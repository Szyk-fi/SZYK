//! Voice input for the Oracle: record a spoken request from the audio
//! input, turn it into text with a speech-to-text service, and hand the
//! words to the Oracle exactly as if they had been typed.
//!
//! Speech-to-text providers, picked from the environment:
//!   ORACLE_STT_URL   an OpenAI-compatible transcription server -- a local
//!                    whisper.cpp / faster-whisper server works fully
//!                    offline (e.g. http://127.0.0.1:8080/v1, or a full
//!                    URL ending in /inference or /audio/transcriptions)
//!   ORACLE_STT_KEY   optional bearer key for that server
//!   OPENAI_API_KEY   otherwise, OpenAI's hosted transcription
//!   ORACLE_STT_MODEL model name (default "whisper-1")
//!   ORACLE_STT_LANG  optional language hint, e.g. "en"
//! With none of these, the recording is still saved to
//! apps/oracle/voice/last.wav so it can be transcribed by hand.
//!
//! Everything here runs on its own thread: the cpal input stream (which
//! isn't `Send` on every platform) is created, used and dropped there.

use crate::util::AtomicF32;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::mpsc::{channel, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Hard cap on one take.
pub const MAX_SECONDS: f32 = 15.0;
/// After speech has been heard, this much quiet ends the take.
const SILENCE_STOP_S: f32 = 1.6;
/// Block RMS above this counts as speech.
const SPEECH_RMS: f32 = 0.02;
const TARGET_RATE: u32 = 16_000;

#[derive(Clone, Debug)]
pub struct SttProvider {
    pub url: String,
    pub key: Option<String>,
    pub model: String,
    pub lang: Option<String>,
}

impl SttProvider {
    pub fn from_env() -> Option<SttProvider> {
        let var = super::llm::config_var;
        let model = var("ORACLE_STT_MODEL").unwrap_or_else(|| "whisper-1".into());
        let lang = var("ORACLE_STT_LANG");
        if let Some(url) = var("ORACLE_STT_URL") {
            return Some(SttProvider { url: endpoint(&url), key: var("ORACLE_STT_KEY"), model, lang });
        }
        var("OPENAI_API_KEY").map(|key| SttProvider { url: endpoint("https://api.openai.com/v1"), key: Some(key), model, lang })
    }

    pub fn label(&self) -> String {
        if self.url.contains("openai.com") {
            format!("OpenAI ({})", self.model)
        } else {
            "Local speech server".into()
        }
    }
}

/// Accepts a base URL (".../v1") or a full endpoint.
fn endpoint(url: &str) -> String {
    let u = url.trim().trim_end_matches('/');
    if u.ends_with("/inference") || u.ends_with("/transcriptions") {
        u.to_string()
    } else {
        format!("{u}/audio/transcriptions")
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Intent {
    /// The words describe a new patch.
    New,
    /// The words are a change to the current patch.
    Change,
}

pub enum VoiceResult {
    Text(String),
    /// Recorded, but nothing to transcribe with; the take was saved.
    Saved(String),
    Error(String),
}

const PHASE_IDLE: u8 = 0;
const PHASE_RECORDING: u8 = 1;
const PHASE_TRANSCRIBING: u8 = 2;

/// One take in flight. The UI thread polls it; the thread does the rest.
pub struct Take {
    pub intent: Intent,
    stop: Arc<AtomicBool>,
    phase: Arc<AtomicU8>,
    level: Arc<AtomicF32>,
    started: Instant,
    rx: Receiver<VoiceResult>,
}

impl Take {
    pub fn start(intent: Intent, provider: Option<SttProvider>) -> Take {
        let stop = Arc::new(AtomicBool::new(false));
        let phase = Arc::new(AtomicU8::new(PHASE_RECORDING));
        let level = Arc::new(AtomicF32::new(0.0));
        let (tx, rx) = channel();
        let (s, p, l) = (Arc::clone(&stop), Arc::clone(&phase), Arc::clone(&level));
        std::thread::spawn(move || {
            let result = run(&s, &p, &l, provider.as_ref());
            p.store(PHASE_IDLE, Ordering::Relaxed);
            let _ = tx.send(result);
        });
        Take { intent, stop, phase, level, started: Instant::now(), rx }
    }

    /// Ask the take to finish recording (it then transcribes).
    pub fn finish(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    pub fn recording(&self) -> bool {
        self.phase.load(Ordering::Relaxed) == PHASE_RECORDING
    }

    pub fn transcribing(&self) -> bool {
        self.phase.load(Ordering::Relaxed) == PHASE_TRANSCRIBING
    }

    /// Live input level, 0..1 (for meters).
    pub fn level(&self) -> f32 {
        self.level.get()
    }

    pub fn elapsed(&self) -> f32 {
        self.started.elapsed().as_secs_f32()
    }

    pub fn poll(&self) -> Option<VoiceResult> {
        self.rx.try_recv().ok()
    }
}

fn run(stop: &AtomicBool, phase: &AtomicU8, level: &AtomicF32, provider: Option<&SttProvider>) -> VoiceResult {
    let (samples, rate) = match record(stop, level) {
        Ok(v) => v,
        Err(e) => return VoiceResult::Error(e),
    };
    let mono16 = resample(&samples, rate, TARGET_RATE);
    if mono16.len() < (TARGET_RATE as usize) / 4 {
        return VoiceResult::Error("Didn't hear anything -- check the input".into());
    }
    let wav = match encode_wav(&mono16, TARGET_RATE) {
        Ok(w) => w,
        Err(e) => return VoiceResult::Error(e),
    };
    let Some(provider) = provider else {
        let dir = std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/apps/oracle/voice"));
        let path = dir.join("last.wav");
        return match std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(&path, &wav)) {
            Ok(()) => VoiceResult::Saved("apps/oracle/voice/last.wav".into()),
            Err(e) => VoiceResult::Error(format!("Couldn't save the recording: {e}")),
        };
    };
    phase.store(PHASE_TRANSCRIBING, Ordering::Relaxed);
    match transcribe(provider, &wav) {
        Ok(t) if t.trim().is_empty() => VoiceResult::Error("Couldn't make out any words".into()),
        Ok(t) => VoiceResult::Text(clean(&t)),
        Err(e) => VoiceResult::Error(e),
    }
}

/// Records from the default input until `stop`, the time cap, or a pause
/// after speech. Returns mono samples at the device rate.
fn record(stop: &AtomicBool, level: &AtomicF32) -> Result<(Vec<f32>, u32), String> {
    let host = cpal::default_host();
    let device = host.default_input_device().ok_or("No audio input found (on a Mac, allow microphone access for your terminal)")?;
    let config = device.default_input_config().map_err(|e| format!("Input config: {e}"))?;
    let rate = config.sample_rate().0;
    let channels = config.channels().max(1) as usize;
    let buf: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::with_capacity(rate as usize * MAX_SECONDS as usize)));
    let block_rms = Arc::new(AtomicF32::new(0.0));

    fn push(data: impl Iterator<Item = f32>, channels: usize, buf: &Mutex<Vec<f32>>, rms: &AtomicF32) {
        let mut acc = 0.0f32;
        let mut n = 0usize;
        let mut frame = 0.0f32;
        let mut k = 0usize;
        let mut mono = Vec::new();
        for s in data {
            frame += s;
            k += 1;
            if k == channels {
                let m = frame / channels as f32;
                mono.push(m);
                acc += m * m;
                n += 1;
                frame = 0.0;
                k = 0;
            }
        }
        if n > 0 {
            rms.set((acc / n as f32).sqrt());
        }
        if let Ok(mut b) = buf.lock() {
            b.extend_from_slice(&mono);
        }
    }

    let err = |e| eprintln!("[Oracle] input stream error: {e}");
    let (b, r) = (Arc::clone(&buf), Arc::clone(&block_rms));
    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => device.build_input_stream(&config.into(), move |d: &[f32], _| push(d.iter().copied(), channels, &b, &r), err, None),
        cpal::SampleFormat::I16 => {
            device.build_input_stream(&config.into(), move |d: &[i16], _| push(d.iter().map(|s| *s as f32 / 32768.0), channels, &b, &r), err, None)
        }
        cpal::SampleFormat::U16 => device.build_input_stream(
            &config.into(),
            move |d: &[u16], _| push(d.iter().map(|s| (*s as f32 - 32768.0) / 32768.0), channels, &b, &r),
            err,
            None,
        ),
        f => return Err(format!("Unsupported input format {f:?}")),
    }
    .map_err(|e| format!("Couldn't open the input: {e}"))?;
    stream.play().map_err(|e| format!("Couldn't start the input: {e}"))?;

    let started = Instant::now();
    let mut heard = false;
    let mut quiet_since: Option<Instant> = None;
    loop {
        std::thread::sleep(Duration::from_millis(40));
        let rms = block_rms.get();
        level.set((rms * 6.0).min(1.0));
        if rms > SPEECH_RMS {
            heard = true;
            quiet_since = None;
        } else if heard {
            let q = *quiet_since.get_or_insert_with(Instant::now);
            if q.elapsed().as_secs_f32() > SILENCE_STOP_S {
                break;
            }
        }
        if stop.load(Ordering::Relaxed) || started.elapsed().as_secs_f32() > MAX_SECONDS {
            break;
        }
    }
    drop(stream);
    level.set(0.0);
    let samples = std::mem::take(&mut *buf.lock().map_err(|_| "recording buffer poisoned")?);
    Ok((samples, rate))
}

/// Linear-interpolation resample (fine for speech going to an STT model).
pub fn resample(input: &[f32], from: u32, to: u32) -> Vec<f32> {
    if input.is_empty() || from == 0 || from == to {
        return input.to_vec();
    }
    let ratio = from as f64 / to as f64;
    let n = (input.len() as f64 / ratio) as usize;
    (0..n)
        .map(|i| {
            let pos = i as f64 * ratio;
            let j = pos as usize;
            let f = (pos - j as f64) as f32;
            let a = input[j.min(input.len() - 1)];
            let b = input[(j + 1).min(input.len() - 1)];
            a + (b - a) * f
        })
        .collect()
}

pub fn encode_wav(samples: &[f32], rate: u32) -> Result<Vec<u8>, String> {
    // Normalize quiet takes so the recognizer gets a healthy level.
    let peak = samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    let gain = if peak > 1e-4 { (0.9 / peak).min(8.0) } else { 1.0 };
    let spec = hound::WavSpec { channels: 1, sample_rate: rate, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
    let mut cur = std::io::Cursor::new(Vec::new());
    {
        let mut w = hound::WavWriter::new(&mut cur, spec).map_err(|e| e.to_string())?;
        for s in samples {
            w.write_sample(((s * gain).clamp(-1.0, 1.0) * 32767.0) as i16).map_err(|e| e.to_string())?;
        }
        w.finalize().map_err(|e| e.to_string())?;
    }
    Ok(cur.into_inner())
}

/// multipart/form-data body for an OpenAI-style transcription request.
pub fn multipart_body(boundary: &str, fields: &[(&str, &str)], wav: &[u8]) -> Vec<u8> {
    let mut body = Vec::with_capacity(wav.len() + 512);
    for (name, value) in fields {
        body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n").as_bytes());
    }
    body.extend_from_slice(
        format!("--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"voice.wav\"\r\nContent-Type: audio/wav\r\n\r\n").as_bytes(),
    );
    body.extend_from_slice(wav);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    body
}

fn transcribe(p: &SttProvider, wav: &[u8]) -> Result<String, String> {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let boundary = format!("----portamax{nanos:x}");
    let mut fields = vec![("model", p.model.as_str()), ("response_format", "json"), ("temperature", "0")];
    if let Some(l) = p.lang.as_deref() {
        fields.push(("language", l));
    }
    let body = multipart_body(&boundary, &fields, wav);
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(60)).build();
    let mut req = agent.post(&p.url).set("Content-Type", &format!("multipart/form-data; boundary={boundary}"));
    if let Some(k) = p.key.as_deref() {
        req = req.set("Authorization", &format!("Bearer {k}"));
    }
    let resp = match req.send_bytes(&body) {
        Ok(r) => r,
        Err(ureq::Error::Status(code, r)) => {
            let text = r.into_string().unwrap_or_default();
            return Err(format!("Speech-to-text error {code}: {}", text.chars().take(160).collect::<String>()));
        }
        Err(e) => return Err(format!("Speech-to-text unreachable: {e}")),
    };
    let text = resp.into_string().map_err(|e| e.to_string())?;
    parse_transcript(&text)
}

/// Pulls the text out of the reply ({"text": ...}; plain text accepted).
pub fn parse_transcript(body: &str) -> Result<String, String> {
    match serde_json::from_str::<serde_json::Value>(body) {
        Ok(v) => v.get("text").and_then(|t| t.as_str()).map(str::to_string).ok_or_else(|| "Speech-to-text reply had no text".into()),
        Err(_) if !body.trim().is_empty() && !body.trim_start().starts_with('{') => Ok(body.trim().to_string()),
        Err(e) => Err(format!("Bad speech-to-text reply: {e}")),
    }
}

/// Tidy a transcript: one line, no recognizer tags like "[BLANK_AUDIO]".
pub fn clean(t: &str) -> String {
    let mut out = String::new();
    let mut depth = 0;
    for c in t.chars() {
        match c {
            '[' | '(' => depth += 1,
            ']' | ')' if depth > 0 => depth -= 1,
            _ if depth == 0 => out.push(if c.is_whitespace() { ' ' } else { c }),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ").trim_end_matches(['.', '!', ' ']).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoints_accept_base_or_full_urls() {
        assert_eq!(endpoint("http://127.0.0.1:8080/v1/"), "http://127.0.0.1:8080/v1/audio/transcriptions");
        assert_eq!(endpoint("http://127.0.0.1:8080/inference"), "http://127.0.0.1:8080/inference");
        assert_eq!(endpoint("https://x/v1/audio/transcriptions"), "https://x/v1/audio/transcriptions");
    }

    #[test]
    fn resample_keeps_duration() {
        let s: Vec<f32> = (0..48_000).map(|i| (i as f32 * 0.01).sin()).collect();
        let r = resample(&s, 48_000, 16_000);
        assert!((r.len() as i64 - 16_000).abs() <= 1);
        assert!(r.iter().all(|v| v.abs() <= 1.0));
    }

    #[test]
    fn wav_and_multipart_are_well_formed() {
        let wav = encode_wav(&vec![0.1; 1600], 16_000).unwrap();
        assert_eq!(&wav[0..4], b"RIFF");
        let r = hound::WavReader::new(std::io::Cursor::new(wav.clone())).unwrap();
        assert_eq!(r.spec().sample_rate, 16_000);
        let body = multipart_body("BND", &[("model", "whisper-1")], &wav);
        let text = String::from_utf8_lossy(&body);
        assert!(text.starts_with("--BND\r\nContent-Disposition: form-data; name=\"model\"\r\n\r\nwhisper-1\r\n"));
        assert!(text.contains("filename=\"voice.wav\""));
        assert!(text.ends_with("\r\n--BND--\r\n"));
    }

    #[test]
    fn transcripts_parse_and_clean() {
        assert_eq!(parse_transcript(r#"{"text":" a warm pad "}"#).unwrap(), " a warm pad ");
        assert_eq!(parse_transcript("plain words").unwrap(), "plain words");
        assert!(parse_transcript(r#"{"error":"x"}"#).is_err());
        assert_eq!(clean(" [BLANK_AUDIO] Make it\n darker (music). "), "Make it darker");
    }

    /// Round trip against a running server: `ORACLE_STT_URL=... cargo test
    /// --bin portamax-sim stt_server -- --ignored`.
    #[test]
    #[ignore]
    fn stt_server_round_trip() {
        let p = SttProvider::from_env().expect("set ORACLE_STT_URL");
        let tone: Vec<f32> = (0..16_000).map(|i| (i as f32 * 0.07).sin() * 0.3).collect();
        let wav = encode_wav(&tone, 16_000).unwrap();
        let text = transcribe(&p, &wav).unwrap();
        println!("server said: {text}");
        assert!(!text.is_empty());
    }
}
