//! Persistence for per-file reading state (last page, zoom level, etc.).
//!
//! State is stored as a JSON file in the user's config directory:
//!   - Linux:   `~/.config/shosai/reading_state.json`
//!   - macOS:   `~/Library/Application Support/shosai/reading_state.json`
//!
//! The file maps absolute file paths to their reading state.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

const APP_DIR: &str = "shosai";
const STATE_FILE: &str = "reading_state.json";

/// Per-file reading state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileReadingState {
    /// Last viewed page index (0-based).
    pub page: usize,
    /// Last zoom scale (1.0 = 100%).
    pub zoom: f32,
}

/// Collection of reading states for all opened files.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReadingStateStore {
    /// Map from canonical file path to reading state.
    files: HashMap<String, FileReadingState>,
}

impl ReadingStateStore {
    /// Load the reading state store from disk. Returns a default (empty) store
    /// if the file doesn't exist yet.
    pub fn load() -> Result<Self> {
        let path = state_file_path()?;
        if !path.exists() {
            return Ok(Self::default());
        }

        let data = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;

        let store: Self =
            serde_json::from_str(&data).with_context(|| "failed to parse reading state JSON")?;

        Ok(store)
    }

    /// Save the reading state store to disk.
    pub fn save(&self) -> Result<()> {
        let path = state_file_path()?;

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create config dir {}", parent.display()))?;
        }

        let json = serde_json::to_string_pretty(self)
            .with_context(|| "failed to serialize reading state")?;

        std::fs::write(&path, json)
            .with_context(|| format!("failed to write {}", path.display()))?;

        Ok(())
    }

    /// Get the reading state for a file.
    pub fn get(&self, file_path: &Path) -> Option<&FileReadingState> {
        let key = canonical_key(file_path);
        self.files.get(&key)
    }

    /// Update or insert the reading state for a file.
    pub fn set(&mut self, file_path: &Path, state: FileReadingState) {
        let key = canonical_key(file_path);
        self.files.insert(key, state);
    }
}

/// Convert a file path to a canonical string key for the map.
fn canonical_key(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .to_string()
}

/// Get the path to the reading state JSON file.
fn state_file_path() -> Result<PathBuf> {
    let config_dir = config_dir()?;
    Ok(config_dir.join(APP_DIR).join(STATE_FILE))
}

/// Get the platform-specific config directory.
fn config_dir() -> Result<PathBuf> {
    // Try XDG_CONFIG_HOME first, then fall back to ~/.config (Linux)
    // or ~/Library/Application Support (macOS).
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        return Ok(PathBuf::from(xdg));
    }

    let home = std::env::var("HOME")
        .map(PathBuf::from)
        .with_context(|| "HOME environment variable not set")?;

    #[cfg(target_os = "macos")]
    {
        Ok(home.join("Library").join("Application Support"))
    }

    #[cfg(not(target_os = "macos"))]
    {
        Ok(home.join(".config"))
    }
}
