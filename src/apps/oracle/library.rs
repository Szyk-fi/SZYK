//! Built-in starter patches plus the user's saved patches on disk.
//!
//! Starters are compiled into the binary. Saved patches live as plain JSON
//! in `apps/oracle/patches/` next to the app's manifest, so they can be
//! shared, versioned, or dropped in by hand.

use super::patch::Patch;
use std::path::PathBuf;

pub const STARTERS: &[(&str, &str)] = &[
    ("velvet_drift", include_str!("starters/velvet_drift.json")),
    ("acid_worm", include_str!("starters/acid_worm.json")),
    ("karplus_harp", include_str!("starters/karplus_harp.json")),
    ("fm_bells", include_str!("starters/fm_bells.json")),
    ("rubber_drums", include_str!("starters/rubber_drums.json")),
    ("drone_engine", include_str!("starters/drone_engine.json")),
    ("chaos_garden", include_str!("starters/chaos_garden.json")),
    ("tape_echo", include_str!("starters/tape_echo.json")),
    ("shimmer_cathedral", include_str!("starters/shimmer_cathedral.json")),
    ("bit_mangler", include_str!("starters/bit_mangler.json")),
];

#[derive(Clone, Debug)]
pub struct Entry {
    pub name: String,
    pub kind: &'static str,
    pub source: Source,
}

#[derive(Clone, Debug)]
pub enum Source {
    Starter(usize),
    File(PathBuf),
}

pub fn patch_dir() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/apps/oracle/patches"))
}

/// Starters first, then saved files (sorted). Unreadable files are skipped.
pub fn scan() -> Vec<Entry> {
    let mut out: Vec<Entry> = STARTERS
        .iter()
        .enumerate()
        .filter_map(|(i, (_, json))| {
            let p: Patch = serde_json::from_str(json).ok()?;
            Some(Entry { name: p.name, kind: p.kind.label(), source: Source::Starter(i) })
        })
        .collect();
    let mut files: Vec<Entry> = std::fs::read_dir(patch_dir())
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("json"))
        .filter_map(|e| {
            let text = std::fs::read_to_string(e.path()).ok()?;
            let p: Patch = serde_json::from_str(&text).ok()?;
            Some(Entry { name: p.name, kind: p.kind.label(), source: Source::File(e.path()) })
        })
        .collect();
    files.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    out.extend(files);
    out
}

pub fn load(entry: &Entry) -> Result<Patch, String> {
    let text = match &entry.source {
        Source::Starter(i) => STARTERS.get(*i).map(|s| s.1.to_string()).ok_or("missing starter")?,
        Source::File(p) => std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))?,
    };
    serde_json::from_str(&text).map_err(|e| format!("bad patch file: {e}"))
}

pub fn slug(name: &str) -> String {
    let s: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    let s = s.trim_matches('_').to_string();
    if s.is_empty() {
        "patch".into()
    } else {
        s.chars().take(40).collect()
    }
}

/// Save (with current control state), never overwriting: name_2.json etc.
pub fn save(patch: &Patch) -> Result<PathBuf, String> {
    let dir = patch_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let base = slug(&patch.name);
    let mut path = dir.join(format!("{base}.json"));
    let mut k = 2;
    while path.exists() {
        path = dir.join(format!("{base}_{k}.json"));
        k += 1;
    }
    std::fs::write(&path, patch.to_json()).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::super::engine::{compile, Controls};
    use super::*;

    #[test]
    fn every_starter_compiles_and_makes_sound_safely() {
        for (file, json) in STARTERS {
            let patch: Patch = serde_json::from_str(json).unwrap_or_else(|e| panic!("{file}: {e}"));
            let mut e = compile(&patch, 48_000.0).unwrap_or_else(|errs| panic!("{file}: {errs:#?}"));
            assert!(patch.macros.len() >= 4, "{file}: starters should show off macros");
            let mut ctl = Controls::default();
            // Default knob positions.
            for (i, p) in patch.params.iter().enumerate() {
                ctl.norms[i] = super::super::patch::ParamMap::from_spec(p).norm(p.default);
            }
            ctl.held[12] = true;
            ctl.held[9] = true;
            let (mut l, mut r) = (vec![0.0; 256], vec![0.0; 256]);
            let input: Vec<f32> = (0..256).map(|i| (i as f32 * 0.07).sin() * 0.4).collect();
            let mut peak = 0.0f32;
            for _ in 0..(48_000 * 2 / 256) {
                e.process(&ctl, &input, &mut l, &mut r, 48_000.0);
                for x in l.iter().chain(r.iter()) {
                    assert!(x.is_finite(), "{file}: non-finite output");
                    peak = peak.max(x.abs());
                }
            }
            assert!(peak > 0.02, "{file}: silent (peak {peak})");
            assert!(!e.unstable, "{file}: a node blew up");
        }
    }

    #[test]
    fn slugs_are_filesystem_safe() {
        assert_eq!(slug("Velvet Drift!"), "velvet_drift");
        assert_eq!(slug("../../etc"), "etc");
        assert_eq!(slug("   "), "patch");
    }
}
