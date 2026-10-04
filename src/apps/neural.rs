//! The Portamax neural runtime: runs the small int8 networks the AI apps
//! use, exactly as the STM32N6's Neural-ART NPU would.
//!
//! The models are trained and quantised by `tools/npu` (PyTorch), which
//! writes each one twice: a `.pmxn` file this runtime loads, and an ONNX
//! file for ST Edge AI, which compiles it for the NPU on the device. Both
//! use the same quantisation (int8 activations with a scale and zero
//! point, int8 weights with a scale per output channel, int32 biases and
//! accumulators, a float multiplier to requantise), so the numbers here
//! match the chip's. Each model ships with test vectors from the Python
//! reference, and the tests below check this runtime reproduces them bit
//! for bit.
//!
//! Layers: 1-D convolution, max pooling, flatten, global average pooling
//! and dense, each with an optional fused ReLU -- all operators the
//! Neural-ART accelerator runs in hardware.
//!
//! In the sim this runs on the computer's CPU. `npu_micros` estimates
//! what a model costs on the NPU from its multiply-accumulates: Neural-ART
//! peaks at 600 GOPS (2 ops per MAC); small layers like these keep only
//! part of it busy, so the estimate assumes 25% of peak.
//!
//! Also here: the audio front ends the models share (a 48 kHz -> 16 kHz
//! decimator and FFT magnitude spectra), mirrored in `tools/npu/features.py`.

use serde_json::Value;
use std::sync::Arc;

const MAGIC: &[u8; 4] = b"PMXN";
/// Neural-ART peak throughput, int8 ops per second.
pub const NPU_PEAK_OPS: f64 = 600e9;
/// The share of peak a small model keeps busy (an estimate; ST's tools
/// report the real figure per model).
pub const NPU_EFFICIENCY: f64 = 0.25;

/// Microseconds one inference of `macs` multiply-accumulates takes on
/// the NPU, by the estimate above.
pub fn npu_micros(macs: u64) -> f64 {
    macs as f64 * 2.0 / (NPU_PEAK_OPS * NPU_EFFICIENCY) * 1e6
}

#[derive(Clone, Debug)]
enum Op {
    Conv { cin: usize, cout: usize, k: usize, stride: usize, pad: usize, relu: bool, w: Vec<i8>, b: Vec<i32>, mult: Vec<f32> },
    Dense { nin: usize, nout: usize, relu: bool, w: Vec<i8>, b: Vec<i32>, mult: Vec<f32> },
    Pool { k: usize },
    Flatten,
    Gap,
}

#[derive(Clone, Debug)]
struct Layer {
    op: Op,
    in_zp: i32,
    in_scale: f32,
    out_zp: i32,
    out_scale: f32,
}

/// A loaded network. `run` allocates nothing.
#[derive(Clone)]
pub struct Model {
    /// The model's name, from its file (shown in test failures).
    #[allow(dead_code)]
    pub name: String,
    in_shape: (usize, usize),
    in_scale: f32,
    in_zp: i32,
    out_scale: f32,
    out_zp: i32,
    layers: Vec<Layer>,
    macs: u64,
    pub meta: Value,
    a: Vec<i8>,
    b: Vec<i8>,
    out_len: usize,
}

#[inline]
fn round_half_up(x: f32) -> f32 {
    (x + 0.5).floor()
}

#[inline]
fn requant(acc: i32, mult: f32, zp: i32, relu: bool) -> i8 {
    let v = round_half_up(acc as f32 * mult) as i32 + zp;
    let lo = if relu { zp.max(-128) } else { -128 };
    v.clamp(lo, 127) as i8
}

fn num(v: &Value, k: &str) -> Result<f64, String> {
    v.get(k).and_then(Value::as_f64).ok_or_else(|| format!("missing {k}"))
}

fn usize_of(v: &Value, k: &str) -> Result<usize, String> {
    v.get(k).and_then(Value::as_u64).map(|x| x as usize).ok_or_else(|| format!("missing {k}"))
}

impl Model {
    pub fn from_bytes(bytes: &[u8]) -> Result<Model, String> {
        if bytes.len() < 12 || &bytes[..4] != MAGIC {
            return Err("not a .pmxn model".into());
        }
        let hlen = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
        let header: Value = serde_json::from_slice(bytes.get(12..12 + hlen).ok_or("short header")?).map_err(|e| e.to_string())?;
        let blob = &bytes[12 + hlen..];
        let input = header.get("input").ok_or("no input")?;
        let shape: Vec<usize> = input.get("shape").and_then(Value::as_array).ok_or("no shape")?.iter().filter_map(Value::as_u64).map(|x| x as usize).collect();
        if shape.len() != 2 {
            return Err("input must be [C, L]".into());
        }
        let mut layers = Vec::new();
        let (mut c, mut l) = (shape[0], shape[1]);
        let mut largest = c * l;
        for d in header.get("layers").and_then(Value::as_array).ok_or("no layers")? {
            let kind = d.get("type").and_then(Value::as_str).ok_or("layer type")?;
            let weights = |n: usize, cout: usize| -> Result<(Vec<i8>, Vec<i32>, Vec<f32>), String> {
                let wo = usize_of(d, "w_off")?;
                let bo = usize_of(d, "b_off")?;
                let w = blob.get(wo..wo + n).ok_or("weights out of range")?.iter().map(|&x| x as i8).collect();
                let b = blob.get(bo..bo + 4 * cout).ok_or("bias out of range")?.chunks_exact(4).map(|c| i32::from_le_bytes(c.try_into().unwrap())).collect();
                let mult: Vec<f32> = d.get("mult").and_then(Value::as_array).ok_or("mult")?.iter().filter_map(Value::as_f64).map(|m| m as f32).collect();
                if mult.len() != cout {
                    return Err("multiplier count".into());
                }
                Ok((w, b, mult))
            };
            let op = match kind {
                "conv" => {
                    let (cin, cout, k, stride, pad) = (usize_of(d, "cin")?, usize_of(d, "cout")?, usize_of(d, "k")?, usize_of(d, "stride")?, usize_of(d, "pad")?);
                    if cin != c {
                        return Err(format!("conv expects {cin} channels, has {c}"));
                    }
                    let (w, b, mult) = weights(cout * cin * k, cout)?;
                    l = (l + 2 * pad - k) / stride + 1;
                    c = cout;
                    Op::Conv { cin, cout, k, stride, pad, relu: d.get("relu").and_then(Value::as_bool).unwrap_or(false), w, b, mult }
                }
                "dense" => {
                    let (nin, nout) = (usize_of(d, "nin")?, usize_of(d, "nout")?);
                    if nin != c * l {
                        return Err(format!("dense expects {nin} inputs, has {}", c * l));
                    }
                    let (w, b, mult) = weights(nout * nin, nout)?;
                    c = nout;
                    l = 1;
                    Op::Dense { nin, nout, relu: d.get("relu").and_then(Value::as_bool).unwrap_or(false), w, b, mult }
                }
                "pool" => {
                    let k = usize_of(d, "k")?;
                    l /= k;
                    Op::Pool { k }
                }
                "flatten" => {
                    c *= l;
                    l = 1;
                    Op::Flatten
                }
                "gap" => {
                    l = 1;
                    Op::Gap
                }
                other => return Err(format!("unknown layer {other}")),
            };
            largest = largest.max(c * l);
            layers.push(Layer { op, in_zp: num(d, "in_zp")? as i32, in_scale: num(d, "in_scale")? as f32, out_zp: num(d, "out_zp")? as i32, out_scale: num(d, "out_scale")? as f32 });
        }
        let output = header.get("output").ok_or("no output")?;
        Ok(Model {
            name: header.get("name").and_then(Value::as_str).unwrap_or("").to_string(),
            in_shape: (shape[0], shape[1]),
            in_scale: num(input, "scale")? as f32,
            in_zp: num(input, "zp")? as i32,
            out_scale: num(output, "scale")? as f32,
            out_zp: num(output, "zp")? as i32,
            layers,
            macs: header.get("macs").and_then(Value::as_u64).unwrap_or(0),
            meta: header.get("meta").cloned().unwrap_or(Value::Null),
            a: vec![0; largest],
            b: vec![0; largest],
            out_len: c * l,
        })
    }

    pub fn input_len(&self) -> usize {
        self.in_shape.0 * self.in_shape.1
    }
    pub fn output_len(&self) -> usize {
        self.out_len
    }
    pub fn macs(&self) -> u64 {
        self.macs
    }

    /// Runs the network on `input` (float, channel-major [C, L]) and
    /// returns the raw int8 output.
    pub fn run_q(&mut self, input: &[f32]) -> &[i8] {
        let n = self.input_len();
        for i in 0..n {
            let x = input.get(i).copied().unwrap_or(0.0);
            let q = round_half_up(x / self.in_scale) as i32 + self.in_zp;
            self.a[i] = q.clamp(-128, 127) as i8;
        }
        let (mut c, mut l) = self.in_shape;
        let (mut src, mut dst) = (std::mem::take(&mut self.a), std::mem::take(&mut self.b));
        for layer in &self.layers {
            let zp = layer.in_zp;
            match &layer.op {
                Op::Conv { cin, cout, k, stride, pad, relu, w, b, mult } => {
                    let out_l = (l + 2 * pad - k) / stride + 1;
                    for co in 0..*cout {
                        let wrow = &w[co * cin * k..(co + 1) * cin * k];
                        for t in 0..out_l {
                            let mut acc = b[co];
                            let start = (t * stride) as isize - *pad as isize;
                            for ci in 0..*cin {
                                let xin = &src[ci * l..(ci + 1) * l];
                                let wk = &wrow[ci * k..(ci + 1) * k];
                                for kk in 0..*k {
                                    let idx = start + kk as isize;
                                    if idx >= 0 && (idx as usize) < l {
                                        acc += (xin[idx as usize] as i32 - zp) * wk[kk] as i32;
                                    }
                                }
                            }
                            dst[co * out_l + t] = requant(acc, mult[co], layer.out_zp, *relu);
                        }
                    }
                    c = *cout;
                    l = out_l;
                }
                Op::Dense { nin, nout, relu, w, b, mult } => {
                    for o in 0..*nout {
                        let row = &w[o * nin..(o + 1) * nin];
                        let mut acc = b[o];
                        for i in 0..*nin {
                            acc += (src[i] as i32 - zp) * row[i] as i32;
                        }
                        dst[o] = requant(acc, mult[o], layer.out_zp, *relu);
                    }
                    c = *nout;
                    l = 1;
                }
                Op::Pool { k } => {
                    let out_l = l / k;
                    for ch in 0..c {
                        for t in 0..out_l {
                            let s = &src[ch * l + t * k..ch * l + t * k + k];
                            dst[ch * out_l + t] = *s.iter().max().unwrap();
                        }
                    }
                    l = out_l;
                }
                Op::Flatten => {
                    dst[..c * l].copy_from_slice(&src[..c * l]);
                    c *= l;
                    l = 1;
                }
                Op::Gap => {
                    let m = layer.in_scale / (l as f32 * layer.out_scale);
                    for ch in 0..c {
                        let acc: i32 = src[ch * l..(ch + 1) * l].iter().map(|&x| x as i32 - zp).sum();
                        dst[ch] = (round_half_up(acc as f32 * m) as i32 + layer.out_zp).clamp(-128, 127) as i8;
                    }
                    l = 1;
                }
            }
            std::mem::swap(&mut src, &mut dst);
        }
        self.a = src;
        self.b = dst;
        &self.a[..c * l]
    }

    /// Runs the network and writes the dequantised outputs.
    pub fn run(&mut self, input: &[f32], out: &mut [f32]) {
        let (s, zp) = (self.out_scale, self.out_zp);
        let q = self.run_q(input);
        for (o, &v) in out.iter_mut().zip(q.iter()) {
            *o = (v as i32 - zp) as f32 * s;
        }
    }
}

/// Softmax in place.
pub fn softmax(x: &mut [f32]) {
    let m = x.iter().cloned().fold(f32::MIN, f32::max);
    let mut sum = 0.0;
    for v in x.iter_mut() {
        *v = (*v - m).exp();
        sum += *v;
    }
    for v in x.iter_mut() {
        *v /= sum.max(1e-30);
    }
}

pub fn argmax(x: &[f32]) -> usize {
    let mut best = 0;
    for i in 1..x.len() {
        if x[i] > x[best] {
            best = i;
        }
    }
    best
}

/// Keeps count of what a model does, for the "NPU" line each AI app shows.
pub struct Load {
    macs: u64,
    count: u64,
    since: std::time::Instant,
    /// Inferences per second over the last second.
    pub rate: f64,
    /// Microseconds the last inference took on this computer's CPU.
    pub cpu_us: f64,
}

impl Load {
    pub fn new(macs: u64) -> Load {
        Load { macs, count: 0, since: std::time::Instant::now(), rate: 0.0, cpu_us: 0.0 }
    }
    pub fn tick(&mut self, cpu: std::time::Duration) {
        self.count += 1;
        self.cpu_us = self.cpu_us * 0.9 + cpu.as_secs_f64() * 1e6 * 0.1;
        let e = self.since.elapsed().as_secs_f64();
        if e >= 1.0 {
            self.rate = self.count as f64 / e;
            self.count = 0;
            self.since = std::time::Instant::now();
        }
    }
    /// The share of the NPU these inferences would keep busy, 0..1.
    pub fn npu_busy(&self) -> f64 {
        self.rate * npu_micros(self.macs) * 1e-6
    }
    pub fn summary(&self) -> String {
        format!("{:.0}/s x {:.0}k MAC = {:.1} M MAC/s  NPU ~{:.2}%", self.rate, self.macs as f64 / 1000.0, self.rate * self.macs as f64 / 1e6, (self.npu_busy() * 100.0).abs())
    }
}

// ---------------------------------------------------------------------
// Audio front ends
// ---------------------------------------------------------------------

/// The sample rate the audio models listen at.
pub const MODEL_SR: f32 = 16_000.0;

/// 48 kHz to 16 kHz: a 63-tap windowed-sinc low-pass at 7 kHz (Blackman
/// window), keeping every third sample. Other device rates are resampled
/// by linear interpolation first.
pub struct Decimator {
    taps: Vec<f32>,
    hist: Vec<f32>,
    pos: usize,
    phase: usize,
    /// Fractional read position for non-48 kHz input.
    frac: f32,
    prev: f32,
}

impl Default for Decimator {
    fn default() -> Self {
        Self::new()
    }
}

impl Decimator {
    pub fn new() -> Decimator {
        let n = 63;
        let fc = 7000.0 / 48_000.0;
        let mut taps: Vec<f32> = (0..n)
            .map(|i| {
                let m = i as f32 - (n - 1) as f32 / 2.0;
                let sinc = if m == 0.0 { 2.0 * fc } else { (std::f32::consts::TAU * fc * m).sin() / (std::f32::consts::PI * m) };
                let w = 0.42 - 0.5 * (std::f32::consts::TAU * i as f32 / (n - 1) as f32).cos() + 0.08 * (2.0 * std::f32::consts::TAU * i as f32 / (n - 1) as f32).cos();
                sinc * w
            })
            .collect();
        let sum: f32 = taps.iter().sum();
        taps.iter_mut().for_each(|t| *t /= sum);
        Decimator { hist: vec![0.0; n], taps, pos: 0, phase: 0, frac: 0.0, prev: 0.0 }
    }

    fn push48(&mut self, x: f32, out: &mut Vec<f32>) {
        let n = self.hist.len();
        self.hist[self.pos] = x;
        self.pos = (self.pos + 1) % n;
        self.phase += 1;
        if self.phase == 3 {
            self.phase = 0;
            let mut acc = 0.0;
            for i in 0..n {
                acc += self.taps[i] * self.hist[(self.pos + i) % n];
            }
            out.push(acc);
        }
    }

    /// Feeds input at `sr`, appending 16 kHz samples to `out`.
    pub fn push(&mut self, input: &[f32], sr: f32, out: &mut Vec<f32>) {
        if (sr - 48_000.0).abs() < 1.0 {
            for &x in input {
                self.push48(x, out);
            }
            return;
        }
        let step = sr / 48_000.0;
        for &x in input {
            while self.frac < 1.0 {
                let y = self.prev + (x - self.prev) * self.frac;
                self.push48(y, out);
                self.frac += step;
            }
            self.frac -= 1.0;
            self.prev = x;
        }
    }
}

/// FFT magnitudes of Hann-windowed frames.
pub struct Spectrum {
    window: Vec<f32>,
    fft: Arc<dyn rustfft::Fft<f32>>,
    buf: Vec<rustfft::num_complex::Complex<f32>>,
    scratch: Vec<rustfft::num_complex::Complex<f32>>,
    pub mag: Vec<f32>,
}

impl Spectrum {
    /// `frame` samples, zero-padded to `n_fft`. The window is the periodic
    /// Hann, as numpy's `0.5 - 0.5 cos(2 pi n / N)`.
    pub fn new(frame: usize, n_fft: usize) -> Spectrum {
        let fft = rustfft::FftPlanner::new().plan_fft_forward(n_fft);
        let scratch = vec![Default::default(); fft.get_inplace_scratch_len()];
        Spectrum {
            window: (0..frame).map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / frame as f32).cos()).collect(),
            fft,
            buf: vec![Default::default(); n_fft],
            scratch,
            mag: vec![0.0; n_fft / 2 + 1],
        }
    }

    pub fn compute(&mut self, frame: &[f32]) -> &[f32] {
        for (i, c) in self.buf.iter_mut().enumerate() {
            let x = if i < self.window.len() { frame.get(i).copied().unwrap_or(0.0) * self.window[i] } else { 0.0 };
            *c = rustfft::num_complex::Complex::new(x, 0.0);
        }
        self.fft.process_with_scratch(&mut self.buf, &mut self.scratch);
        for (m, c) in self.mag.iter_mut().zip(self.buf.iter()) {
            *m = c.norm();
        }
        &self.mag
    }
}

/// Magnitude at frequency `hz`, linearly interpolated between FFT bins.
pub fn mag_at(mag: &[f32], hz: f32, bin_hz: f32) -> f32 {
    let b = hz / bin_hz;
    let i = b.floor() as usize;
    if i + 1 >= mag.len() {
        return 0.0;
    }
    let f = b - i as f32;
    mag[i] * (1.0 - f) + mag[i + 1] * f
}

/// A log of `x` relative to the frame's peak, clipped to `[floor, 0]`:
/// the models see the shape of a spectrum, not its loudness.
pub fn log_relative(x: &mut [f32], floor: f32) {
    let peak = x.iter().cloned().fold(0.0f32, f32::max).max(1e-9);
    for v in x.iter_mut() {
        *v = ((*v / peak).max(1e-9)).ln().max(floor);
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// Checks a model against its Python test vectors, bit for bit.
    pub fn check_vectors(model_bytes: &[u8], test_json: &str) {
        let mut m = Model::from_bytes(model_bytes).unwrap();
        let t: Value = serde_json::from_str(test_json).unwrap();
        let cases = t["cases"].as_array().unwrap();
        assert!(!cases.is_empty());
        for (n, c) in cases.iter().enumerate() {
            let input: Vec<f32> = c["input"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap() as f32).collect();
            let want: Vec<i8> = c["output"].as_array().unwrap().iter().map(|v| v.as_i64().unwrap() as i8).collect();
            let got = m.run_q(&input).to_vec();
            assert_eq!(got, want, "{} case {n}", m.name);
        }
    }

    #[test]
    fn every_layer_matches_the_python_reference() {
        check_vectors(include_bytes!("../../tools/npu/fixtures/selftest.pmxn"), include_str!("../../tools/npu/fixtures/selftest.test.json"));
        check_vectors(include_bytes!("../../tools/npu/fixtures/selftest_flat.pmxn"), include_str!("../../tools/npu/fixtures/selftest_flat.test.json"));
    }

    #[test]
    fn the_decimator_passes_low_tones_and_stops_high_ones() {
        let tone = |hz: f32| -> f32 {
            let mut d = Decimator::new();
            let x: Vec<f32> = (0..9600).map(|i| (i as f32 * std::f32::consts::TAU * hz / 48_000.0).sin()).collect();
            let mut out = Vec::new();
            d.push(&x, 48_000.0, &mut out);
            assert_eq!(out.len(), 3200);
            (out[400..].iter().map(|v| v * v).sum::<f32>() / (out.len() - 400) as f32).sqrt()
        };
        assert!((tone(440.0) - 0.7071).abs() < 0.01);
        assert!(tone(12_000.0) < 0.01, "aliasing would fold 12 kHz to 4 kHz");
    }

    #[test]
    fn a_spectrum_peaks_at_the_tone() {
        let mut s = Spectrum::new(1024, 2048);
        let x: Vec<f32> = (0..1024).map(|i| (i as f32 * std::f32::consts::TAU * 1000.0 / 16_000.0).sin()).collect();
        let m = s.compute(&x).to_vec();
        assert_eq!(argmax(&m), 128, "1000 Hz / 7.8125 Hz per bin");
    }
}
