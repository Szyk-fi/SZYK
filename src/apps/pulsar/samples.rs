//! Drum samples from the project's `samples/` folder, sorted into the
//! roles Pulsar's lanes play (kick, snare, hat...) by file and folder name.

use super::engine::SampleBuf;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct SampleFile {
    pub name: String,
    pub path: PathBuf,
    pub role: Role,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Kick,
    Snare,
    Clap,
    ClosedHat,
    OpenHat,
    Perc,
    Cymbal,
    Other,
}

/// Which roles suit each lane, best first.
pub fn lane_roles(lane: usize) -> &'static [Role] {
    match lane {
        0 => &[Role::Kick],
        1 => &[Role::Snare, Role::Clap],
        2 => &[Role::Clap, Role::Snare],
        3 => &[Role::ClosedHat, Role::Perc],
        4 => &[Role::OpenHat, Role::Cymbal, Role::ClosedHat],
        5 | 6 => &[Role::Perc, Role::ClosedHat, Role::Other],
        _ => &[Role::Cymbal, Role::OpenHat, Role::Other],
    }
}

fn classify(path: &Path) -> Role {
    let s = path.to_string_lossy().to_lowercase();
    let file = path.file_name().map(|f| f.to_string_lossy().to_lowercase()).unwrap_or_default();
    let has = |k: &str| file.contains(k);
    if has("kick") || has("bd ") || has("bassdrum") {
        Role::Kick
    } else if has("snare") || has("sd ") {
        Role::Snare
    } else if has("clap") || has("snap") {
        Role::Clap
    } else if has("open") && (has("hat") || has("hh")) {
        Role::OpenHat
    } else if has("hat") || has("hh") {
        Role::ClosedHat
    } else if has("crash") || has("cymbal") || has("ride") || has("splash") {
        Role::Cymbal
    } else if ["shaker", "tom", "perc", "rim", "cowbell", "tamb", "conga", "bongo", "wood", "stick", "clave", "bell", "click", "hand"]
        .iter()
        .any(|k| has(k))
        || s.contains("perc")
    {
        Role::Perc
    } else if s.contains("kick") {
        Role::Kick
    } else if s.contains("snare") {
        Role::Snare
    } else if s.contains("hat") {
        Role::ClosedHat
    } else if s.contains("clap") {
        Role::Clap
    } else {
        Role::Other
    }
}

pub fn samples_dir() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/samples"))
}

/// Recursively list WAVs (bass/stab/melodic one-shots are skipped -- this
/// is a drum machine).
pub fn scan(dir: &Path) -> Vec<SampleFile> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().and_then(|x| x.to_str()).is_some_and(|x| x.eq_ignore_ascii_case("wav")) {
                let lower = p.to_string_lossy().to_lowercase();
                if lower.contains("bass") || lower.contains("stab") {
                    continue;
                }
                let name = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                out.push(SampleFile { role: classify(&p), name, path: p });
            }
        }
    }
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    out
}

/// Samples for a lane, best-matching roles first.
pub fn for_lane(all: &[SampleFile], lane: usize) -> Vec<usize> {
    let mut idx = Vec::new();
    for role in lane_roles(lane) {
        idx.extend(all.iter().enumerate().filter(|(_, s)| s.role == *role).map(|(i, _)| i));
    }
    idx
}

/// Decode to mono f32 (max 10 s). Runs on the UI thread.
pub fn load(path: &Path) -> Result<SampleBuf, String> {
    let mut r = hound::WavReader::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let spec = r.spec();
    let ch = spec.channels.max(1) as usize;
    let max = spec.sample_rate as usize * 10 * ch;
    let raw: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => r.samples::<f32>().take(max).filter_map(Result::ok).collect(),
        hound::SampleFormat::Int => {
            let scale = 1.0 / (1i64 << (spec.bits_per_sample.clamp(8, 32) - 1)) as f32;
            r.samples::<i32>().take(max).filter_map(Result::ok).map(|v| v as f32 * scale).collect()
        }
    };
    let data: Vec<f32> = raw.chunks(ch).map(|c| c.iter().sum::<f32>() / ch as f32).collect();
    if data.len() < 8 {
        return Err(format!("{}: empty", path.display()));
    }
    let name = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    Ok(SampleBuf { name, data, rate: spec.sample_rate as f32 })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_common_drum_names() {
        let c = |s: &str| classify(Path::new(s));
        assert_eq!(c("Kicks 67/Kick Dusty 04.wav"), Role::Kick);
        assert_eq!(c("Snares/Snare Crack.wav"), Role::Snare);
        assert_eq!(c("Hats/Hat Open 2.wav"), Role::OpenHat);
        assert_eq!(c("Hats/Hat Closed.wav"), Role::ClosedHat);
        assert_eq!(c("Percs/Shaker Egg Up.wav"), Role::Perc);
        assert_eq!(c("Percs/Crash High.wav"), Role::Cymbal);
        assert_eq!(c("Claps & Snaps/Snap 3.wav"), Role::Clap);
    }

    #[test]
    fn round_trips_a_wav() {
        let dir = std::env::temp_dir().join(format!("pulsar_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("Kick Test.wav");
        let spec = hound::WavSpec { channels: 2, sample_rate: 44_100, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
        let mut w = hound::WavWriter::create(&p, spec).unwrap();
        for i in 0..4410 {
            let v = ((i as f32 * 0.05).sin() * 16000.0) as i16;
            w.write_sample(v).unwrap();
            w.write_sample(v).unwrap();
        }
        w.finalize().unwrap();
        let s = load(&p).unwrap();
        assert_eq!(s.data.len(), 4410);
        assert_eq!(s.rate, 44_100.0);
        assert!(s.data.iter().any(|x| x.abs() > 0.4));
        let found = scan(&dir);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].role, Role::Kick);
        std::fs::remove_dir_all(&dir).ok();
    }
}
