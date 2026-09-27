//! Bounded streaming PCM adapter. The device-facing side only consumes frames;
//! curl is the simulator's HTTP transport and is never run on the audio thread.
use super::*;
use std::{
    collections::VecDeque,
    io::Read,
    process::{Command, Stdio},
    time::Duration,
};
const QUEUE_FRAMES: usize = 48000;
pub(super) struct RadioStream {
    queue: Mutex<VecDeque<[f32; 2]>>,
    pub rate: AtomicF32,
    pub ready: AtomicBool,
    pub finished: AtomicBool,
    pub cancel: AtomicBool,
    pub error: Mutex<Option<String>>,
}
impl RadioStream {
    fn empty() -> Arc<Self> {
        Arc::new(Self {
            queue: Mutex::new(VecDeque::with_capacity(QUEUE_FRAMES)),
            rate: AtomicF32::new(48000.),
            ready: AtomicBool::new(false),
            finished: AtomicBool::new(false),
            cancel: AtomicBool::new(false),
            error: Mutex::new(None),
        })
    }
    pub fn start(path: PathBuf) -> Arc<Self> {
        let stream = Self::empty();
        let shared = stream.clone();
        std::thread::spawn(move || {
            let result = (|| -> Result<(), String> {
                let url = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
                let url = url.trim();
                if !(url.starts_with("http://") || url.starts_with("https://")) {
                    return Err("Station needs an HTTP(S) WAV URL".into());
                }
                let mut child = Command::new("curl")
                    .args([
                        "--fail",
                        "--location",
                        "--connect-timeout",
                        "5",
                        "--speed-time",
                        "10",
                        "--speed-limit",
                        "1",
                        "--proto",
                        "=http,https",
                        "--proto-redir",
                        "=http,https",
                        "--silent",
                    ])
                    .arg(url)
                    .stdout(Stdio::piped())
                    .stderr(Stdio::null())
                    .spawn()
                    .map_err(|e| format!("HTTP transport: {e}"))?;
                let reader = child.stdout.take().ok_or("No HTTP audio stream")?;
                let child = Arc::new(Mutex::new(child));
                let supervisor = child.clone();
                let control = shared.clone();
                // Cancellation interrupts a blocked network read, including an
                // idle/disconnected station, rather than waiting for more data.
                let watchdog = std::thread::spawn(move || {
                    while !control.cancel.load(Ordering::Acquire)
                        && !control.finished.load(Ordering::Acquire)
                    {
                        std::thread::sleep(Duration::from_millis(25));
                    }
                    if let Ok(mut child) = supervisor.lock() {
                        let _ = child.kill();
                        let _ = child.wait();
                    }
                });
                let result = decode(reader, &shared);
                shared.finished.store(true, Ordering::Release);
                let _ = watchdog.join();
                result
            })();
            if let Err(error) = result {
                if !shared.cancel.load(Ordering::Relaxed) {
                    *shared.error.lock().unwrap() = Some(error);
                }
            }
            shared.finished.store(true, Ordering::Release);
        });
        stream
    }
    /// Returns true at a drained EOF. Never blocks the device callback waiting
    /// for a network packet, and never holds the queue during network I/O.
    pub fn render(
        &self,
        out: &mut Vec<[f32; 2]>,
        frames: usize,
        rate: f32,
        phase: &mut f32,
        buffer_ms: f32,
    ) -> bool {
        out.clear();
        let Ok(mut queue) = self.queue.try_lock() else {
            out.resize(frames, [0.; 2]);
            return false;
        };
        let ended = self.finished.load(Ordering::Acquire);
        let threshold = ((buffer_ms * self.rate.get() / 1000.) as usize).min(QUEUE_FRAMES - 512);
        if !ended && queue.len() < threshold.max(2) {
            out.resize(frames, [0.; 2]);
            return false;
        }
        let ratio = self.rate.get() / rate.max(1.);
        for _ in 0..frames {
            if queue.is_empty() {
                out.push([0.; 2]);
                continue;
            }
            let a = queue[0];
            let b = queue.get(1).copied().unwrap_or(a);
            out.push([
                a[0] * (1. - *phase) + b[0] * *phase,
                a[1] * (1. - *phase) + b[1] * *phase,
            ]);
            *phase += ratio;
            while *phase >= 1. {
                queue.pop_front();
                *phase -= 1.;
            }
        }
        ended && queue.is_empty()
    }
}
fn decode<R: Read>(reader: R, stream: &RadioStream) -> Result<(), String> {
    let mut wav = hound::WavReader::new(reader)
        .map_err(|e| format!("Station is not a supported WAV stream: {e}"))?;
    let spec = wav.spec();
    if !(1..=2).contains(&spec.channels) || spec.sample_rate == 0 {
        return Err("Use mono/stereo PCM or float WAV".into());
    }
    stream.rate.set(spec.sample_rate as f32);
    stream.ready.store(true, Ordering::Release);
    let samples: Box<dyn Iterator<Item = Result<f32, hound::Error>> + '_> =
        if spec.sample_format == hound::SampleFormat::Float {
            Box::new(wav.samples::<f32>())
        } else {
            let scale = 2f32.powi(spec.bits_per_sample as i32 - 1);
            Box::new(
                wav.samples::<i32>()
                    .map(move |s| s.map(|v| v as f32 / scale)),
            )
        };
    let mut frame = [0.; 2];
    let mut channel = 0;
    let mut batch = Vec::with_capacity(512);
    for sample in samples {
        if stream.cancel.load(Ordering::Acquire) {
            return Ok(());
        }
        let sample = sample.map_err(|e| format!("WAV stream ended: {e}"))?;
        frame[channel] = if sample.is_finite() {
            sample.clamp(-1., 1.)
        } else {
            0.
        };
        channel += 1;
        if channel == spec.channels as usize {
            if channel == 1 {
                frame[1] = frame[0];
            }
            batch.push(frame);
            channel = 0;
        }
        if batch.len() == 512 {
            publish(&mut batch, stream);
        }
    }
    publish(&mut batch, stream);
    Ok(())
}
fn publish(batch: &mut Vec<[f32; 2]>, stream: &RadioStream) {
    while !batch.is_empty() && !stream.cancel.load(Ordering::Acquire) {
        {
            let mut queue = stream.queue.lock().unwrap();
            if queue.len() + batch.len() <= QUEUE_FRAMES {
                queue.extend(batch.drain(..));
                return;
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn streaming_wav_decodes_and_resamples_without_loading_a_file() {
        let mut cursor = std::io::Cursor::new(Vec::new());
        {
            let mut writer = hound::WavWriter::new(
                &mut cursor,
                hound::WavSpec {
                    channels: 1,
                    sample_rate: 24000,
                    bits_per_sample: 16,
                    sample_format: hound::SampleFormat::Int,
                },
            )
            .unwrap();
            for _ in 0..2048 {
                writer.write_sample(8192i16).unwrap();
            }
            writer.finalize().unwrap();
        }
        cursor.set_position(0);
        let stream = RadioStream::empty();
        decode(cursor, &stream).unwrap();
        stream.finished.store(true, Ordering::Relaxed);
        assert!(stream.ready.load(Ordering::Relaxed));
        assert_eq!(stream.rate.get(), 24000.);
        let mut phase = 0.;
        let mut out = Vec::new();
        let mut count = 0;
        loop {
            let ended = stream.render(&mut out, 512, 48000., &mut phase, 80.);
            count += out.iter().filter(|f| f[0] > 0.).count();
            if ended {
                break;
            }
        }
        assert_eq!(count, 4096);
        assert!(out.iter().all(|f| (f[0] - 0.25).abs() < 0.001));
    }
    #[test]
    fn invalid_stream_reports_error_and_empty_buffer_is_silent() {
        let stream = RadioStream::empty();
        assert!(decode(std::io::Cursor::new(b"not audio"), &stream).is_err());
        let mut frames = vec![];
        let mut phase = 0.;
        assert!(!stream.render(&mut frames, 64, 48000., &mut phase, 80.));
        assert_eq!(frames, vec![[0.; 2]; 64]);
    }
}
