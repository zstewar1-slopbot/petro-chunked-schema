//! Persistent editor state management for `petro-chunked-schema-editor`.
//!
//! Automatically saves and restores what files and settings the user was working on so the editor
//! returns right where they left off on launch.

use std::collections::BTreeSet;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Identifies a chunked binary file either on disk or inside a Petroglyph `.meg` archive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FileSource {
    /// A loose file on the local filesystem.
    Disk { path: PathBuf },
    /// A file entry stored inside a `.meg` archive (loaded directly in-memory without extracting).
    MegEntry {
        meg_path: PathBuf,
        entry_name: String,
    },
}

impl FileSource {
    /// Short human-readable label for display in the title bar and file list.
    pub fn display_label(&self) -> String {
        match self {
            Self::Disk { path } => path.display().to_string(),
            Self::MegEntry {
                meg_path,
                entry_name,
            } => {
                let meg_name = meg_path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("archive.meg");
                format!("{meg_name} :: {entry_name}")
            }
        }
    }
}

/// Which schema was active at the end of the last session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ActiveSchemaSource {
    /// One of the built-in known formats (`"AloModel"`, `"AloParticle"`, `"AlaAnimation"`).
    Builtin { name: String },
    /// A YAML schema file on disk.
    File { path: PathBuf },
    /// An unsaved/custom schema held in session state.
    #[default]
    Custom,
}

/// Persistent session state stored in the user's configuration directory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedState {
    /// Last opened chunked binary file (on disk or inside a `.meg` archive).
    pub last_file: Option<FileSource>,
    /// Source of the active schema.
    #[serde(default)]
    pub schema_source: ActiveSchemaSource,
    /// Snapshot of the current schema YAML so unsaved schema edits are preserved across launches.
    pub current_schema_yaml: Option<String>,
    /// Configured `GameData` directories for games and mods.
    #[serde(default)]
    pub game_data_dirs: Vec<PathBuf>,
    /// Tree paths in the active file that the user forced to be read as plain byte data.
    #[serde(default)]
    pub force_plain_data_paths: BTreeSet<String>,
    /// File extension filter last used for batch validation (e.g. `"alo"` or `"ala"`).
    #[serde(default = "default_validate_ext")]
    pub validate_extension: String,
}

fn default_validate_ext() -> String {
    "alo".to_owned()
}

impl Default for PersistedState {
    fn default() -> Self {
        Self {
            last_file: None,
            schema_source: ActiveSchemaSource::Builtin {
                name: "AloModel".to_owned(),
            },
            current_schema_yaml: None,
            game_data_dirs: Vec::new(),
            force_plain_data_paths: BTreeSet::new(),
            validate_extension: default_validate_ext(),
        }
    }
}

impl PersistedState {
    /// Returns the path to `state.json` inside the OS user configuration directory.
    pub fn config_file_path() -> PathBuf {
        let base = if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
            PathBuf::from(xdg)
        } else if let Some(appdata) = std::env::var_os("APPDATA") {
            PathBuf::from(appdata)
        } else if let Some(home) = std::env::var_os("HOME") {
            PathBuf::from(home).join(".config")
        } else {
            PathBuf::from(".")
        };
        base.join("petro-chunked-schema-editor").join("state.json")
    }

    /// Loads the persisted state from disk, falling back to `Default::default()` if absent or corrupt.
    pub fn load() -> Self {
        let path = Self::config_file_path();
        let Ok(content) = std::fs::read_to_string(&path) else {
            return Self::default();
        };
        serde_json::from_str(&content).unwrap_or_default()
    }

    /// Saves the persisted state to disk.
    pub fn save(&self) {
        let path = Self::config_file_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(&path, json);
        }
    }
}
