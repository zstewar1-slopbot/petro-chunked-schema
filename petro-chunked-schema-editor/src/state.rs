//! Persistent editor session state management for `petro-chunked-schema-editor`.
//!
//! Saves and restores the active file, schema reference, unsaved schema YAML snapshot,
//! `GameData` directories, and per-file `force_plain_data_paths` overrides across launches.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use petro_chunked_schema::known_formats;
use petro_chunked_schema::resolve::{ResolvedSchema, SchemaLoader};
use petro_chunked_schema::schema::Schema;
use petro_chunked_schema::source::SchemaRef;
use serde::{Deserialize, Serialize};

/// Identifies a chunked binary file either on disk or inside a Petroglyph `.meg` archive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FileSource {
    /// A loose file on the local filesystem.
    Disk {
        /// Filesystem path to the chunked file.
        path: PathBuf,
    },
    /// A file entry stored inside a `.meg` archive (loaded directly in-memory without extracting).
    MegEntry {
        /// Filesystem path to the enclosing `.meg` archive.
        meg_path: PathBuf,
        /// Archive-relative entry path inside `meg_path`.
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

    /// Stable map key identifying this file for per-file persistent overrides.
    pub fn persistence_key(&self) -> String {
        match self {
            Self::Disk { path } => {
                let canon = std::fs::canonicalize(path).unwrap_or_else(|_| path.clone());
                format!("disk:{}", canon.display())
            }
            Self::MegEntry {
                meg_path,
                entry_name,
            } => {
                let canon = std::fs::canonicalize(meg_path).unwrap_or_else(|_| meg_path.clone());
                format!("meg:{}::{entry_name}", canon.display())
            }
        }
    }
}

/// Persistent session state stored in the user's configuration directory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedState {
    /// Last opened chunked binary file (on disk or inside a `.meg` archive).
    pub last_file: Option<FileSource>,
    /// URI or path reference of the active schema (`None` for an unsaved blank custom schema).
    #[serde(default = "default_schema_ref")]
    pub schema_ref: Option<SchemaRef>,
    /// Snapshot of the current schema YAML so unsaved edits persist across launches.
    pub current_schema_yaml: Option<String>,
    /// Configured `GameData` directories for games and mods.
    #[serde(default)]
    pub game_data_dirs: Vec<PathBuf>,
    /// Per-file chunk tree paths forced to decode as plain byte data rather than mini-chunks.
    #[serde(default)]
    pub force_plain_data_paths: BTreeMap<String, BTreeSet<String>>,
    /// File extension filter last used for batch validation (e.g. `"alo"` or `"ala"`).
    #[serde(default = "default_validate_ext")]
    pub validate_extension: String,
}

fn default_schema_ref() -> Option<SchemaRef> {
    Some(SchemaRef::Builtin("Alo".to_owned()))
}

fn default_validate_ext() -> String {
    "alo".to_owned()
}

impl Default for PersistedState {
    fn default() -> Self {
        Self {
            last_file: None,
            schema_ref: default_schema_ref(),
            current_schema_yaml: None,
            game_data_dirs: Vec::new(),
            force_plain_data_paths: BTreeMap::new(),
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

    /// Loads persisted state from disk, falling back to [`Default::default`] if absent or corrupt.
    pub fn load() -> Self {
        let path = Self::config_file_path();
        let Ok(content) = std::fs::read_to_string(&path) else {
            return Self::default();
        };
        serde_json::from_str(&content).unwrap_or_default()
    }

    /// Saves persisted state to disk.
    pub fn save(&self) {
        let path = Self::config_file_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(&path, json);
        }
    }

    /// Returns the `force_plain_data_paths` set for `src` (empty if `src` is `None`).
    pub fn forced_plain_paths_for(&self, src: Option<&FileSource>) -> BTreeSet<String> {
        let Some(src) = src else {
            return BTreeSet::new();
        };
        self.force_plain_data_paths
            .get(&src.persistence_key())
            .cloned()
            .unwrap_or_default()
    }

    /// Toggles `tree_path` in the per-file `force_plain_data_paths` set for `src`, returning `true`
    /// if the path is now forced to plain data or `false` if restored.
    pub fn toggle_forced_plain_path(&mut self, src: &FileSource, tree_path: &str) -> bool {
        let key = src.persistence_key();
        let set = self.force_plain_data_paths.entry(key.clone()).or_default();
        let now_forced = if set.remove(tree_path) {
            false
        } else {
            set.insert(tree_path.to_owned());
            true
        };
        if set.is_empty() {
            self.force_plain_data_paths.remove(&key);
        }
        now_forced
    }

    /// Resolves the initial `(Schema, ResolvedSchema)` from persisted state, falling back cleanly
    /// to `builtin:Alo` if the stored YAML or reference fails to load.
    pub fn resolve_initial_schema(&self) -> (Schema, ResolvedSchema) {
        if let Some(yaml) = &self.current_schema_yaml
            && let Ok(s) = Schema::from_yaml_str(yaml)
            && let Ok(resolved) = SchemaLoader::resolve(s.clone(), self.schema_ref.as_ref())
        {
            return (s, resolved);
        }

        if let Some(schema_ref) = &self.schema_ref
            && let Ok(resolved) = SchemaLoader::load(schema_ref)
        {
            return (resolved.root_schema().clone(), resolved);
        }

        let fallback = known_formats::alo();
        let fallback_ref = SchemaRef::Builtin("Alo".to_owned());
        let resolved = SchemaLoader::resolve(fallback.clone(), Some(&fallback_ref))
            .expect("built-in Alo schema must resolve");
        (fallback, resolved)
    }
}
