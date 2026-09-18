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

#[derive(Deserialize, Debug, Clone)]
pub struct AppManifest {
    pub id: String,
    pub name: String,
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
            Ok(manifest) => manifests.push(manifest),
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
