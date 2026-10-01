//! Getting beats out of Pulsar: Standard MIDI Files, rendered WAV (mix or
//! one stem per lane), and saved beats (JSON) that load back in.

use super::engine::{Engine, LaneSettings, Model, Pattern, SampleBuf, Settings, Step, BANK, LANES, MAX_STEPS};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub fn export_dir() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/exports/pulsar"))
}

pub fn beats_dir() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/apps/pulsar/beats"))
}

pub fn slug(name: &str) -> String {
    let s: String = name.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
    let s = s.trim_matches('_').to_string();
    if s.is_empty() {
        "beat".into()
    } else {
        s.chars().take(40).collect()
    }
}

/// A path in `dir` named `base.ext` that doesn't exist yet.
pub fn fresh_path(dir: &Path, base: &str, ext: &str) -> PathBuf {
    let mut p = dir.join(format!("{base}.{ext}"));
    let mut k = 2;
    while p.exists() {
        p = dir.join(format!("{base}_{k}.{ext}"));
        k += 1;
    }
    p
}

// ---------------------------------------------------------------- MIDI

const PPQ: u32 = 96;

fn vlq(mut v: u32, out: &mut Vec<u8>) {
    let mut buf = [0u8; 5];
    let mut n = 0;
    loop {
        buf[n] = (v & 0x7F) as u8;
        n += 1;
        v >>= 7;
        if v == 0 {
            break;
        }
    }
    for i in (0..n).rev() {
        out.push(buf[i] | if i > 0 { 0x80 } else { 0 });
    }
}

/// Type-0 SMF on channel 10 (GM drums), swing/nudge/rolls baked in.
pub fn midi_bytes(pat: &Pattern, s: &Settings, repeats: usize) -> Vec<u8> {
    let tick16 = PPQ / 4; // 24 ticks per 16th
    let mut events: Vec<(u32, [u8; 3])> = Vec::new();
    let len = pat.length.clamp(1, MAX_STEPS);
    for rep in 0..repeats.max(1) {
        for step in 0..len {
            let swing = if s.swing8 {
                if step % 4 == 2 {
                    s.swing * tick16 as f32
                } else {
                    0.0
                }
            } else if step % 2 == 1 {
                s.swing * 0.5 * tick16 as f32
            } else {
                0.0
            };
            for lane in 0..LANES {
                let st = pat.steps[lane][step];
                if !st.on() || s.lanes[lane].mute {
                    continue;
                }
                let note = midi_note(&s.lanes[lane], lane);
                let base = ((rep * len + step) as f32 * tick16 as f32 + swing + st.micro as f32 / 100.0 * tick16 as f32 * 0.5).max(0.0);
                let r = st.ratchet.clamp(1, 4) as u32;
                for k in 0..r {
                    let t = base as u32 + k * tick16 / r;
                    let v = (st.vel as f32 * (1.0 - k as f32 * 0.12)).clamp(1.0, 127.0) as u8;
                    events.push((t, [0x99, note, v]));
                    events.push((t + (tick16 / r).max(2) - 1, [0x89, note, 0]));
                }
            }
        }
    }
    events.sort_by_key(|e| (e.0, e.1[0] == 0x99)); // note-offs before note-ons at equal time

    let mut trk = Vec::new();
    // tempo
    let us = (60_000_000.0 / s.bpm.clamp(20.0, 400.0)) as u32;
    trk.extend_from_slice(&[0x00, 0xFF, 0x51, 0x03, (us >> 16) as u8, (us >> 8) as u8, us as u8]);
    // track name
    let name = b"Pulsar";
    trk.extend_from_slice(&[0x00, 0xFF, 0x03, name.len() as u8]);
    trk.extend_from_slice(name);
    let mut last = 0;
    for (t, ev) in events {
        vlq(t - last, &mut trk);
        trk.extend_from_slice(&ev);
        last = t;
    }
    trk.extend_from_slice(&[0x00, 0xFF, 0x2F, 0x00]);

    let mut out = Vec::new();
    out.extend_from_slice(b"MThd");
    out.extend_from_slice(&6u32.to_be_bytes());
    out.extend_from_slice(&0u16.to_be_bytes());
    out.extend_from_slice(&1u16.to_be_bytes());
    out.extend_from_slice(&(PPQ as u16).to_be_bytes());
    out.extend_from_slice(b"MTrk");
    out.extend_from_slice(&(trk.len() as u32).to_be_bytes());
    out.extend_from_slice(&trk);
    out
}

fn midi_note(l: &LaneSettings, lane: usize) -> u8 {
    if l.use_sample {
        [36, 38, 39, 42, 46, 45, 63, 49][lane]
    } else {
        l.model.gm_note()
    }
}

// ----------------------------------------------------------------- WAV

/// Render `bars` worth of the pattern (repeating) plus a tail, offline,
/// through the same engine the device uses. `solo` = one lane only.
pub fn render(
    bank: &[Pattern; BANK],
    slot: usize,
    s: &Settings,
    samples: &[Option<Arc<SampleBuf>>; LANES],
    loops: usize,
    solo: Option<usize>,
    sr: f32,
) -> (Vec<f32>, Vec<f32>) {
    let mut s = *s;
    s.playing = true;
    s.slot = slot;
    s.chain = false;
    s.audition = [0; LANES];
    if let Some(l) = solo {
        for (i, ls) in s.lanes.iter_mut().enumerate() {
            ls.solo = i == l;
            ls.mute = false;
        }
    }
    let mut e = Engine::new(sr);
    e.seed(0xC0FFEE);
    // A zero-length stopped block makes the engine jump straight to `slot`.
    let mut warm = s;
    warm.playing = false;
    e.process(&warm, bank, samples, &mut [], &mut [], sr);
    let len = bank[slot].length.clamp(1, MAX_STEPS);
    let step_len = sr as f64 * 60.0 / s.bpm as f64 / 4.0;
    let play = (len * loops.max(1)) as f64 * step_len;
    let total = (play + sr as f64 * 2.0) as usize;
    let (mut l, mut r) = (Vec::with_capacity(total), Vec::with_capacity(total));
    let block = 512;
    let (mut bl, mut br) = (vec![0.0; block], vec![0.0; block]);
    let mut done = 0;
    while done < total {
        if done as f64 >= play {
            s.playing = false; // let tails ring out, no new hits
        }
        e.process(&s, bank, samples, &mut bl, &mut br, sr);
        l.extend_from_slice(&bl);
        r.extend_from_slice(&br);
        done += block;
    }
    l.truncate(total);
    r.truncate(total);
    (l, r)
}

pub fn write_wav(path: &Path, l: &[f32], r: &[f32], sr: u32) -> Result<(), String> {
    let spec = hound::WavSpec { channels: 2, sample_rate: sr, bits_per_sample: 24, sample_format: hound::SampleFormat::Int };
    let mut w = hound::WavWriter::create(path, spec).map_err(|e| e.to_string())?;
    for (a, b) in l.iter().zip(r) {
        for x in [a, b] {
            let v = (x.clamp(-1.0, 1.0) * 8_388_607.0) as i32;
            w.write_sample(v).map_err(|e| e.to_string())?;
        }
    }
    w.finalize().map_err(|e| e.to_string())
}

// ----------------------------------------------------------- beat files

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct LaneFile {
    pub model: String,
    #[serde(default)]
    pub sample: Option<String>,
    pub level: f32,
    pub pan: f32,
    pub tune: f32,
    pub decay: f32,
    pub tone: f32,
    pub punch: f32,
    pub send: f32,
    pub mute: bool,
    pub chance: f32,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct PatternFile {
    pub length: usize,
    /// Per lane: steps as [velocity, chance, roll, nudge]
    pub lanes: Vec<Vec<[i16; 4]>>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct BeatFile {
    pub name: String,
    pub genre: String,
    pub bpm: f32,
    pub swing: f32,
    pub swing8: bool,
    pub human_time: f32,
    pub human_vel: f32,
    pub fatten: f32,
    pub tone_shift: f32,
    pub glue: f32,
    pub filter: f32,
    pub room: f32,
    pub level: f32,
    pub lanes: Vec<LaneFile>,
    pub slots: Vec<PatternFile>,
}

pub fn pattern_to_file(p: &Pattern) -> PatternFile {
    PatternFile {
        length: p.length,
        lanes: (0..LANES)
            .map(|l| p.steps[l][..p.length.min(MAX_STEPS)].iter().map(|s| [s.vel as i16, s.prob as i16, s.ratchet as i16, s.micro as i16]).collect())
            .collect(),
    }
}

pub fn pattern_from_file(f: &PatternFile) -> Pattern {
    let mut p = Pattern { length: (f.length.clamp(16, MAX_STEPS) / 16) * 16, ..Pattern::default() };
    for (l, lane) in f.lanes.iter().enumerate().take(LANES) {
        for (i, s) in lane.iter().enumerate().take(MAX_STEPS) {
            p.steps[l][i] = Step {
                vel: s[0].clamp(0, 127) as u8,
                prob: s[1].clamp(0, 100) as u8,
                ratchet: s[2].clamp(1, 4) as u8,
                micro: s[3].clamp(-50, 50) as i8,
            };
        }
    }
    p
}

pub fn model_from_name(n: &str) -> Model {
    super::engine::MODELS.iter().copied().find(|m| m.name() == n).unwrap_or(Model::Kick909)
}

pub fn save_beat(b: &BeatFile) -> Result<PathBuf, String> {
    let dir = beats_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let path = fresh_path(&dir, &slug(&b.name), "json");
    let json = serde_json::to_string_pretty(b).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(path)
}

pub fn list_beats() -> Vec<(String, PathBuf)> {
    let mut v: Vec<(String, PathBuf)> = std::fs::read_dir(beats_dir())
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("json"))
        .filter_map(|e| {
            let b: BeatFile = serde_json::from_str(&std::fs::read_to_string(e.path()).ok()?).ok()?;
            Some((b.name, e.path()))
        })
        .collect();
    v.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));
    v
}

pub fn load_beat(path: &Path) -> Result<BeatFile, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("bad beat file: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn midi_file_is_well_formed_with_one_note_pair_per_hit() {
        let mut p = Pattern::default();
        for i in [0, 4, 8, 12] {
            p.steps[0][i] = Step::hit(100);
        }
        p.steps[3][2] = Step { vel: 90, prob: 100, ratchet: 3, micro: 0 };
        let bytes = midi_bytes(&p, &Settings::default(), 2);
        assert_eq!(&bytes[0..4], b"MThd");
        assert_eq!(&bytes[14..18], b"MTrk");
        let ons = bytes.windows(2).filter(|w| w[0] == 0x99).count();
        // (4 kicks + 3 rolled hats) * 2 repeats
        assert_eq!(ons, 14);
        assert_eq!(&bytes[bytes.len() - 3..], &[0xFF, 0x2F, 0x00]);
    }

    #[test]
    fn vlq_matches_the_spec_examples() {
        for (v, want) in [(0u32, vec![0x00]), (0x7F, vec![0x7F]), (0x80, vec![0x81, 0x00]), (0x3FFF, vec![0xFF, 0x7F])] {
            let mut out = Vec::new();
            vlq(v, &mut out);
            assert_eq!(out, want);
        }
    }

    #[test]
    fn offline_render_has_the_right_length_and_sound() {
        let mut bank = [Pattern::default(); BANK];
        for i in [0, 4, 8, 12] {
            bank[0].steps[0][i] = Step::hit(110);
        }
        let none: [Option<Arc<SampleBuf>>; LANES] = Default::default();
        let s = Settings { bpm: 120.0, ..Settings::default() };
        let (l, r) = render(&bank, 0, &s, &none, 1, None, 48_000.0);
        assert_eq!(l.len(), r.len());
        assert_eq!(l.len(), 48_000 * 2 + 48_000 * 2); // one bar at 120 + 2 s tail
        assert!(l.iter().any(|x| x.abs() > 0.1));
        let (sl, _) = render(&bank, 0, &s, &none, 1, Some(3), 48_000.0);
        assert!(sl.iter().all(|x| x.abs() < 1e-4), "soloing an empty lane is silent");
    }

    #[test]
    fn pattern_file_round_trip() {
        let mut p = Pattern { length: 32, ..Pattern::default() };
        p.steps[2][17] = Step { vel: 77, prob: 60, ratchet: 2, micro: -20 };
        assert_eq!(pattern_from_file(&pattern_to_file(&p)), p);
    }
}
