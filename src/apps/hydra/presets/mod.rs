//! Hydra's sound library: factory presets in category folders, the user's
//! own presets saved into the same folders, favorites, plus Randomize and
//! Mutate.
//!
//! A preset is a list of (parameter index, value) overrides applied on top
//! of the initial patch, so each one only says what makes it that sound. It
//! also names its four macros and wires them to what they should move, so
//! the four dials on the play view mean something for every sound.
//!
//! Factory loudness is levelled by `levels.rs` (written by the
//! `calibrate_levels` test), so moving through the library does not jump in
//! volume.

mod arp;
mod atmosphere;
mod bass;
mod bells;
mod brass;
mod drums;
mod fx;
mod keys;
mod lead;
mod levels;
mod organ;
mod pad;
mod pluck;
mod retro;
mod strings;
mod vocal;
mod wavetable;

use super::dsp::Rng;
use super::params::*;
use super::store::{from_norm, to_norm, Params};
use std::collections::HashSet;
use std::path::PathBuf;

/// The folders, in the order they are browsed.
pub const CATEGORIES: [&str; 17] = ["Init", "Bass", "Lead", "Pad", "Keys", "Pluck", "Strings", "Brass", "Organ", "Bells", "Vocal", "Atmosphere", "Arp", "Drums", "FX", "Retro", "Wavetable"];

/// What one macro is called and what it moves: two destinations
/// (`MD_*`) and how far each goes, as a fraction of the destination's own
/// range at full macro.
pub type MacroSet = [(&'static str, (usize, f32), (usize, f32)); 4];

#[derive(Clone, Debug)]
pub struct Preset {
    pub name: String,
    pub category: String,
    pub values: Vec<(usize, f32)>,
    pub macros: [String; 4],
    pub user: bool,
}

impl Preset {
    pub fn new(name: &str, values: Vec<(usize, f32)>) -> Self {
        Self { name: name.to_string(), category: String::new(), values, macros: Default::default(), user: false }
    }

    /// Adds a mod-matrix slot (1..=12): `source` moves `target` by `amount`.
    pub fn m(mut self, slot: usize, source: usize, target: usize, amount: f32) -> Self {
        let base = P::M1_Src as usize + (slot - 1) * 3;
        self.values.extend([(base, source as f32), (base + 1, target as f32), (base + 2, amount)]);
        self
    }

    /// Names macro `n` (1..=4) and wires it to up to two destinations.
    pub fn mac(mut self, n: usize, name: &str, a: (usize, f32), b: (usize, f32)) -> Self {
        let base = P::Mac1_DA as usize + (n - 1) * 4;
        self.values.extend([(base, a.0 as f32), (base + 1, a.1), (base + 2, b.0 as f32), (base + 3, b.1)]);
        self.macros[n - 1] = name.to_string();
        self
    }

    fn has_macros(&self) -> bool {
        !self.macros[0].is_empty()
    }

    /// The loudness this sound should have (the table's Level), if it sets one.
    fn sets_level(&self) -> bool {
        self.values.iter().any(|&(i, _)| i == P::Level as usize)
    }
}

/// Files a category's presets under its folder and gives those without
/// their own macros the category's.
fn finish(category: &str, macros: &MacroSet, mut list: Vec<Preset>) -> Vec<Preset> {
    for p in &mut list {
        p.category = category.to_string();
        if !p.has_macros() {
            for (n, m) in macros.iter().enumerate() {
                *p = std::mem::replace(p, Preset::new("", Vec::new())).mac(n + 1, m.0, m.1, m.2);
            }
        }
    }
    list
}

macro_rules! pr {
    ($name:expr; $( $p:expr => $v:expr ),* $(,)?) => {
        Preset::new($name, vec![ $( ($p as usize, $v as f32) ),* ])
    };
}
pub(crate) use pr;

// Oscillator shorthand used across the category files.
pub const SINE: f32 = 0.0;
pub const TRI: f32 = 0.333;
pub const SAW: f32 = 0.667;
pub const PULSE: f32 = 1.0;
// Wavetable banks (an oscillator's P2 in Wavetable mode).
pub const BASIC: f32 = 0.0;
pub const ORGAN: f32 = 0.2;
pub const VOWEL: f32 = 0.4;
pub const DIGITAL: f32 = 0.6;
pub const SWEEP: f32 = 0.8;
pub const GLASS: f32 = 1.0;
// FM ratios (an oscillator's P1 in FM mode): index / 15 of the ratio list.
pub const FM_HALF: f32 = 0.067;
pub const FM_1: f32 = 0.2;
pub const FM_1_5: f32 = 0.267;
pub const FM_2: f32 = 0.333;
pub const FM_3: f32 = 0.467;
pub const FM_3_5: f32 = 0.533;
pub const FM_4: f32 = 0.6;
pub const FM_5: f32 = 0.667;
pub const FM_6: f32 = 0.733;
pub const FM_7: f32 = 0.8;
pub const FM_8: f32 = 0.867;
pub const FM_10: f32 = 0.933;

/// The factory presets, folder by folder (levelled).
pub fn factory() -> Vec<Preset> {
    let mut all = finish("Init", &INIT_MACROS, vec![pr!("Init";)]);
    for list in [
        bass::presets(),
        lead::presets(),
        pad::presets(),
        keys::presets(),
        pluck::presets(),
        strings::presets(),
        brass::presets(),
        organ::presets(),
        bells::presets(),
        vocal::presets(),
        atmosphere::presets(),
        arp::presets(),
        drums::presets(),
        fx::presets(),
        retro::presets(),
        wavetable::presets(),
    ] {
        all.extend(list);
    }
    for p in &mut all {
        if !p.sets_level() {
            if let Some(&(_, level)) = levels::LEVELS.iter().find(|(n, _)| *n == p.name) {
                p.values.push((P::Level as usize, level));
            }
        }
    }
    all
}

const INIT_MACROS: MacroSet = [
    ("Bright", (MD_CUTOFF, 0.5), (MD_RES, 0.0)),
    ("Drive", (MD_DRIVE, 0.6), (MD_FDRIVE, 0.0)),
    ("Width", (MD_DETUNE, 0.5), (MD_SPREAD, 0.4)),
    ("Space", (MD_REVERB, 0.35), (MD_DELAY, 0.2)),
];

// ------------------------------------------------------------ user files

/// Where the user's own presets live: one folder per category.
pub fn user_dir() -> PathBuf {
    PathBuf::from("saves/hydra/presets")
}

fn favorites_path() -> PathBuf {
    PathBuf::from("saves/hydra/favorites.json")
}

/// Characters a file name can safely hold.
fn file_stem(name: &str) -> String {
    name.chars().map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' || c == '_' { c } else { '_' }).collect()
}

/// Saves the current sound into `category`'s folder.
pub fn save_user(category: &str, name: &str, values: &[f32], macros: &[String; 4]) -> std::io::Result<PathBuf> {
    save_user_in(&user_dir(), category, name, values, macros)
}

pub fn save_user_in(root: &std::path::Path, category: &str, name: &str, values: &[f32], macros: &[String; 4]) -> std::io::Result<PathBuf> {
    let dir = root.join(file_stem(category));
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}.json", file_stem(name)));
    let doc = serde_json::json!({ "name": name, "values": values, "macros": macros });
    std::fs::write(&path, doc.to_string())?;
    Ok(path)
}

/// Every preset found in the user folders (a file dropped in a category
/// folder shows up there).
pub fn load_user() -> Vec<Preset> {
    load_user_in(&user_dir())
}

pub fn load_user_in(root: &std::path::Path) -> Vec<Preset> {
    let mut out = Vec::new();
    let Ok(folders) = std::fs::read_dir(root) else { return out };
    let mut folders: Vec<_> = folders.flatten().filter(|e| e.path().is_dir()).collect();
    folders.sort_by_key(|e| e.file_name());
    for folder in folders {
        let category = folder.file_name().to_string_lossy().to_string();
        let Ok(files) = std::fs::read_dir(folder.path()) else { continue };
        let mut files: Vec<_> = files.flatten().filter(|e| e.path().extension().is_some_and(|x| x == "json")).collect();
        files.sort_by_key(|e| e.file_name());
        for file in files {
            let Some(doc) = std::fs::read_to_string(file.path()).ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok()) else { continue };
            let Some(values) = doc["values"].as_array() else { continue };
            let stem = file.path().file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            let mut p = Preset::new(doc["name"].as_str().unwrap_or(&stem), values.iter().take(COUNT).enumerate().map(|(i, v)| (i, v.as_f64().unwrap_or(0.0) as f32)).collect());
            p.category = category.clone();
            p.user = true;
            if let Some(m) = doc["macros"].as_array() {
                for (n, name) in m.iter().take(4).enumerate() {
                    p.macros[n] = name.as_str().unwrap_or("").to_string();
                }
            }
            out.push(p);
        }
    }
    out
}

/// The sound library: factory and user presets, and which are favorites.
pub struct Library {
    pub presets: Vec<Preset>,
    favorites: HashSet<String>,
}

impl Library {
    pub fn new() -> Self {
        Self::with(factory(), load_user(), load_favorites())
    }

    pub fn with(factory: Vec<Preset>, user: Vec<Preset>, favorites: HashSet<String>) -> Self {
        let mut presets = factory;
        // user sounds follow the factory ones inside their folder
        for u in user {
            let at = presets.iter().rposition(|p| p.category == u.category).map(|i| i + 1).unwrap_or(presets.len());
            presets.insert(at, u);
        }
        Self { presets, favorites }
    }

    /// The folders to browse: Favorites first, then every folder with sounds.
    pub fn folders(&self) -> Vec<String> {
        let mut f = vec!["Favorites".to_string()];
        for c in CATEGORIES {
            if self.presets.iter().any(|p| p.category == c) {
                f.push(c.to_string());
            }
        }
        // folders the user made that the factory does not have
        for p in &self.presets {
            if !f.contains(&p.category) {
                f.push(p.category.clone());
            }
        }
        f
    }

    /// The presets (as indices into `presets`) in a folder.
    pub fn in_folder(&self, folder: &str) -> Vec<usize> {
        (0..self.presets.len()).filter(|&i| if folder == "Favorites" { self.is_favorite(i) } else { self.presets[i].category == folder }).collect()
    }

    pub fn key(&self, i: usize) -> String {
        format!("{}/{}", self.presets[i].category, self.presets[i].name)
    }

    pub fn is_favorite(&self, i: usize) -> bool {
        self.favorites.contains(&self.key(i))
    }

    pub fn toggle_favorite(&mut self, i: usize) -> bool {
        let key = self.key(i);
        let now = !self.favorites.remove(&key);
        if now {
            self.favorites.insert(key);
        }
        save_favorites(&self.favorites);
        now
    }

    pub fn add_user(&mut self, p: Preset) -> usize {
        let at = self.presets.iter().rposition(|q| q.category == p.category).map(|i| i + 1).unwrap_or(self.presets.len());
        // saving over an existing user sound replaces it
        if let Some(i) = self.presets.iter().position(|q| q.user && q.category == p.category && q.name == p.name) {
            self.presets[i] = p;
            return i;
        }
        self.presets.insert(at, p);
        at
    }

    /// Where a preset sits among its folder's, and how many there are.
    pub fn place(&self, folder: &str, i: usize) -> (usize, usize) {
        let list = self.in_folder(folder);
        (list.iter().position(|&x| x == i).map(|p| p + 1).unwrap_or(0), list.len())
    }
}

fn load_favorites() -> HashSet<String> {
    std::fs::read_to_string(favorites_path())
        .ok()
        .and_then(|t| serde_json::from_str::<Vec<String>>(&t).ok())
        .map(|v| v.into_iter().collect())
        .unwrap_or_default()
}

fn save_favorites(f: &HashSet<String>) {
    let mut list: Vec<&String> = f.iter().collect();
    list.sort();
    let _ = std::fs::create_dir_all("saves/hydra").and_then(|_| std::fs::write(favorites_path(), serde_json::to_string(&list).unwrap_or_default()));
}

/// Every parameter value a preset produces (the initial patch with its
/// overrides on top), for blending between presets.
pub fn resolve(preset: &Preset) -> Vec<f32> {
    let params = Params::new();
    apply(&params, preset);
    params.snapshot().to_vec()
}

/// Loads a preset: the initial patch, then its overrides. Every value is
/// clamped by the table, so a preset can never leave a parameter out of range.
pub fn apply(params: &Params, preset: &Preset) {
    params.reset_all();
    for &(i, v) in &preset.values {
        params.set_at(i, v);
    }
}

/// A fresh random patch that is meant to be playable: a sounding oscillator,
/// a filter that isn't closed, a sustaining envelope and a few mod routings.
pub fn randomize(params: &Params, rng: &mut Rng) {
    params.reset_all();
    let r = |rng: &mut Rng, lo: f32, hi: f32| lo + rng.unit() * (hi - lo);
    let pick = |rng: &mut Rng, n: usize| (rng.unit() * n as f32) as usize % n;
    for (base, level) in [(P::A_Type as usize, 0.8f32), (P::B_Type as usize, 0.0), (P::C_Type as usize, 0.0)] {
        let on = base == P::A_Type as usize || rng.unit() < 0.6;
        params.set_at(base, pick(rng, 3) as f32);
        params.set_at(base + 1, if on { r(rng, 0.4, 0.9) * (level.max(0.5) / 0.8f32).min(1.0) } else { 0.0 });
        params.set_at(base + 2, [0.0, 0.0, -12.0, 7.0, 12.0][pick(rng, 5)]);
        params.set_at(base + 3, r(rng, -12.0, 12.0));
        params.set_at(base + 4, r(rng, 0.0, 1.0));
        params.set_at(base + 5, r(rng, 0.0, 1.0));
        params.set_at(base + 6, r(rng, 0.0, 0.5));
    }
    params.set(P::Unison, [1, 1, 3, 5][pick(rng, 4)] as f32);
    params.set(P::UniDetune, r(rng, 0.1, 0.5));
    params.set(P::SubLevel, if rng.unit() < 0.3 { r(rng, 0.2, 0.6) } else { 0.0 });
    params.set(P::F1_Type, pick(rng, 4) as f32);
    params.set(P::F1_Cut, 10.0f32.powf(r(rng, 2.6, 4.2)));
    params.set(P::F1_Res, r(rng, 0.05, 0.6));
    params.set(P::F1_Env, r(rng, -0.3, 0.7));
    params.set(P::E2D, r(rng, 0.1, 1.5));
    params.set(P::E2S, r(rng, 0.0, 0.6));
    params.set(P::AmpA, 10.0f32.powf(r(rng, -2.6, -0.3)));
    params.set(P::AmpD, r(rng, 0.2, 1.5));
    params.set(P::AmpS, r(rng, 0.4, 1.0));
    params.set(P::AmpR, r(rng, 0.1, 1.5));
    params.set(P::L1_Rate, 10.0f32.powf(r(rng, -1.0, 0.9)));
    params.set(P::L1_Shape, pick(rng, 6) as f32);
    for slot in 0..3 {
        let base = P::M1_Src as usize + slot * 3;
        params.set_at(base, (2 + pick(rng, 4)) as f32);
        params.set_at(base + 1, [T_A_P1, T_A_P1 + 1, T_F1_CUT, T_PITCH_A, T_A_P1 + 3, T_F1_RES][pick(rng, 6)] as f32);
        params.set_at(base + 2, r(rng, -0.5, 0.5));
    }
    params.set(P::Cho_Mix, if rng.unit() < 0.5 { r(rng, 0.1, 0.4) } else { 0.0 });
    params.set(P::Dly_Mix, if rng.unit() < 0.4 { r(rng, 0.1, 0.35) } else { 0.0 });
    params.set(P::Rev_Mix, r(rng, 0.0, 0.4));
}

/// Nudges a random handful of parameters a little, to evolve the patch.
pub fn mutate(params: &Params, rng: &mut Rng) {
    for (i, d) in DEFS.iter().enumerate() {
        // leave the performance setup, the sound's routing and the arp alone
        let skip = matches!(d.kind, Kind::Toggle | Kind::Int { .. } | Kind::Choice(_)) || [P::Level as usize, P::Tune as usize, P::ModWheel as usize].contains(&i) || (P::Mac1 as usize..=P::Mac4 as usize).contains(&i) || (P::Mac1_DA as usize..=P::Mac4_AB as usize).contains(&i);
        if skip || rng.unit() > 0.18 {
            continue;
        }
        let x = to_norm(d, params.at(i)) + rng.bipolar() * 0.09;
        params.set_at(i, from_norm(d, x.clamp(0.0, 1.0)));
    }
}


#[cfg(test)]
mod tests;
