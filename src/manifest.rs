//! Reads the on-disk `apps/` directory into a list of installed apps.
//!
//! Each entry is either:
//!   - a folder: `apps/<name>/manifest.toml`  (room to grow — icons, assets)
//!   - a single file: `apps/<name>.toml`      (nothing to an app but metadata)
//!
//! This is what makes the app list data on disk instead of a hardcoded Vec
//! in main.rs. It does not load *code* from disk — see registry.rs for why.

use serde::Deserialize;
use std::path::Path;

#[derive(Deserialize, Debug, Clone, Default)]
#[allow(dead_code)] // not used by the main binary
pub struct AppManifest {
    pub id: String,
    pub name: String,
    /// The code that runs it: a module in src/apps exporting
    /// `pub fn create(&AppContext, &str)` (see `apps::FACTORIES`). Defaults
    /// to the id. Several manifests can share one module -- the Collection
    /// apps do, and so can data-only "cartridges": a folder holding a
    /// manifest with `module = "atlas"` and a `data` patch file is a new
    /// instrument with no new code.
    #[serde(default)]
    pub module: Option<String>,
    /// Launcher section: instrument, effect, sequencer, library, utility,
    /// game or system.
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub description: String,
    /// The audio outputs it publishes on the AudioBus, by name.
    #[serde(default)]
    pub audio_outputs: Vec<String>,
    /// Other apps can play it: it appears as an instrument on the note bus
    /// (see note_bus.rs) and receives notes like a MIDI keyboard.
    #[serde(default)]
    pub notes_in: bool,
    /// The note outputs it can send to an instrument, by name.
    #[serde(default)]
    pub note_outputs: Vec<String>,
    /// The modulation inputs this app registers on the ModBus, by full
    /// name ("Plaits: Harmonics"). Declared at startup so every source can
    /// patch to them before the app has ever been opened. Kept in step
    /// with the code by `registry::manifest_contract_tests`.
    #[serde(default)]
    pub mod_inputs: Vec<String>,
    /// A data file for the module (e.g. an Atlas patch), relative to the
    /// manifest's folder.
    #[serde(default)]
    pub data: Option<String>,
    /// The folder the manifest was found in (set by `discover`).
    #[serde(skip)]
    pub dir: std::path::PathBuf,
}

impl AppManifest {
    pub fn module(&self) -> &str {
        self.module.as_deref().unwrap_or(&self.id)
    }

    /// `data`, resolved against the manifest's folder.
    #[allow(dead_code)] // cartridge apps
    pub fn data_path(&self) -> Option<std::path::PathBuf> {
        self.data.as_ref().map(|d| self.dir.join(d))
    }
}

/// Scans `dir` for app manifests. Missing directory, unreadable entries, or
/// malformed TOML are logged and skipped rather than treated as fatal —
/// a broken app on the cartridge shouldn't take down the whole menu.
pub fn discover(dir: &Path) -> Vec<AppManifest> {
    let mut manifests = Vec::new();

    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) => {
            eprintln!("apps: couldn't read {}: {e}", dir.display());
            return manifests;
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let manifest_path = if path.is_dir() {
            path.join("manifest.toml")
        } else if path.extension().and_then(|e| e.to_str()) == Some("toml") {
            path.clone()
        } else {
            continue;
        };

        match load_one(&manifest_path) {
            Ok(mut manifest) => {
                manifest.dir = manifest_path.parent().map(|p| p.to_path_buf()).unwrap_or_default();
                manifests.push(manifest)
            }
            Err(e) => eprintln!("apps: skipping {}: {e}", manifest_path.display()),
        }
    }

    manifests.sort_by(|a, b| a.id.cmp(&b.id));
    manifests
}

fn load_one(path: &Path) -> Result<AppManifest, Box<dyn std::error::Error>> {
    let text = std::fs::read_to_string(path)?;
    let manifest: AppManifest = toml::from_str(&text)?;
    Ok(manifest)
}

/// Every manifest from several roots -- the built-in `apps/` and the SD
/// card's `apps/` folder -- in that order. An id found twice keeps the
/// first; the registry reports the duplicate.
pub fn discover_all(dirs: &[&Path]) -> Vec<AppManifest> {
    let mut all = Vec::new();
    for d in dirs {
        if d.is_dir() {
            all.extend(discover(d));
        }
    }
    all
}

/// Where cartridges dropped on the SD card live (in the sim, `saves/apps`).
pub fn sd_apps_dir() -> std::path::PathBuf {
    std::env::var_os("PORTAMAX_SD_APPS").map(std::path::PathBuf::from).unwrap_or_else(|| std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/saves/apps")))
}
