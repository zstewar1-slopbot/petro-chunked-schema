//! Shared reactive editor state and domain actions provided via Dioxus/Freya context.

use std::collections::BTreeSet;

use freya::prelude::*;
use petro_chunked_schema::dynamic::{DynamicFile, LoadOptions, load_dynamic_file};
use petro_chunked_schema::known_formats;
use petro_chunked_schema::resolve::{ResolvedSchema, SchemaLoader};
use petro_chunked_schema::schema::Schema;
use petro_chunked_schema::source::SchemaRef;

use crate::gamedata::{load_file_bytes, load_schema_from_ref};
use crate::state::{FileSource, PersistedState};

/// Which collapsible drawer modal is currently active above the main 3-pane workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActiveModal {
    /// No modal open.
    None,
    /// Loose file & `.meg` archive browser.
    FileBrowser,
    /// `GameData` directory configuration panel.
    GameDataDirs,
    /// Batch schema validator across all configured `GameData` directories.
    BatchValidate,
    /// Live `petro-chunked-derive` Rust code generator & `.rs` exporter.
    RustCodegen,
    /// Schema URI / YAML loader, saver, and source viewer.
    YamlFile,
}

/// Shared reactive state handle for the editor, accessible in any child component via
/// `use_context::<EditorState>()`.
#[derive(Clone, Copy)]
pub struct EditorState {
    /// Persistent session settings saved to disk on edits.
    pub persisted: Signal<PersistedState>,
    /// Active editable root [`Schema`].
    pub schema: Signal<Schema>,
    /// Resolved multi-module graph for [`Self::schema`] (including any `imports`).
    pub resolved_schema: Signal<ResolvedSchema>,
    /// Raw bytes of the currently opened chunked binary file.
    pub raw_file_bytes: Signal<Vec<u8>>,
    /// Dynamically parsed & schema-annotated representation of [`Self::raw_file_bytes`].
    pub dynamic_file: Signal<Option<DynamicFile>>,
    /// Currently selected chunk/mini-chunk tree path (e.g. `"0"`, `"0/1"`, `"0/1/mc2"`).
    pub selected_tree_path: Signal<String>,
    /// Tree paths currently collapsed in the left-pane tree view.
    pub collapsed_paths: Signal<BTreeSet<String>>,
    /// Selected byte offset inside the currently inspected leaf chunk or mini-chunk payload.
    pub selected_byte_offset: Signal<usize>,
    /// Active 512-byte page index in the hex grid.
    pub hex_page: Signal<usize>,
    /// Active top drawer modal.
    pub active_modal: Signal<ActiveModal>,
    /// Single-line status feedback displayed in the status bar.
    pub status_msg: Signal<String>,
}

/// Initializes [`EditorState`], registers it as Freya root context, and restores the previously
/// opened file (if any) once on mount.
pub fn use_init_editor_state() -> EditorState {
    let initial_persisted = use_hook(PersistedState::load);
    let (initial_schema, initial_resolved) = use_hook(|| initial_persisted.resolve_initial_schema());

    let persisted = use_signal(|| initial_persisted.clone());
    let schema = use_signal(|| initial_schema);
    let resolved_schema = use_signal(|| initial_resolved.clone());
    let raw_file_bytes = use_signal(Vec::<u8>::new);
    let dynamic_file = use_signal(|| None::<DynamicFile>);
    let selected_tree_path = use_signal(|| "0".to_owned());
    let collapsed_paths = use_signal(BTreeSet::<String>::new);
    let selected_byte_offset = use_signal(|| 0usize);
    let hex_page = use_signal(|| 0usize);
    let active_modal = use_signal(|| ActiveModal::None);
    let status_msg = use_signal(|| "Ready.".to_owned());

    let mut state = EditorState {
        persisted,
        schema,
        resolved_schema,
        raw_file_bytes,
        dynamic_file,
        selected_tree_path,
        collapsed_paths,
        selected_byte_offset,
        hex_page,
        active_modal,
        status_msg,
    };

    use_context_provider(|| state);

    use_hook(move || {
        if let Some(src) = &initial_persisted.last_file {
            match load_file_bytes(src) {
                Ok(bytes) => {
                    let opts =
                        LoadOptions::for_ui(initial_persisted.forced_plain_paths_for(Some(src)));
                    match load_dynamic_file(&bytes, Some(&initial_resolved), &opts) {
                        Ok(df) => {
                            state.status_msg.set(format!(
                                "Restored {} ({} bytes, {} diagnostics)",
                                src.display_label(),
                                bytes.len(),
                                df.diagnostics.len()
                            ));
                            state.raw_file_bytes.set(bytes);
                            state.dynamic_file.set(Some(df));
                        }
                        Err(e) => {
                            state
                                .status_msg
                                .set(format!("Failed to parse {}: {e}", src.display_label()));
                        }
                    }
                }
                Err(e) => {
                    state
                        .status_msg
                        .set(format!("Could not open previous file: {e}"));
                }
            }
        }
    });

    state
}

impl EditorState {
    /// Loads a chunked binary file from disk or `.meg`, evaluates it against the active schema,
    /// and updates persisted session state.
    pub fn open_file(&mut self, src: FileSource) {
        match load_file_bytes(&src) {
            Ok(bytes) => {
                let resolved = self.resolved_schema.read().clone();
                let opts =
                    LoadOptions::for_ui(self.persisted.read().forced_plain_paths_for(Some(&src)));
                match load_dynamic_file(&bytes, Some(&resolved), &opts) {
                    Ok(df) => {
                        let diag_count = df.diagnostics.len();
                        let len = bytes.len();
                        self.raw_file_bytes.set(bytes);
                        self.dynamic_file.set(Some(df));
                        self.selected_tree_path.set("0".to_owned());
                        self.selected_byte_offset.set(0);
                        self.hex_page.set(0);
                        {
                            let mut p = self.persisted.write();
                            p.last_file = Some(src.clone());
                            p.save();
                        }
                        self.status_msg.set(format!(
                            "Loaded {} ({len} bytes, {diag_count} schema diagnostics)",
                            src.display_label()
                        ));
                    }
                    Err(e) => {
                        self.status_msg
                            .set(format!("Error parsing chunked file: {e}"));
                    }
                }
            }
            Err(e) => {
                self.status_msg.set(format!("Error loading file: {e}"));
            }
        }
    }

    /// Loads and resolves a schema by [`SchemaRef`] (`builtin:...` or file path).
    pub fn load_schema_ref(&mut self, schema_ref: SchemaRef) {
        match load_schema_from_ref(&schema_ref) {
            Ok((loaded, resolved)) => {
                {
                    let mut p = self.persisted.write();
                    p.schema_ref = Some(schema_ref.clone());
                    p.current_schema_yaml = loaded.to_yaml_string().ok();
                    p.save();
                }
                self.reevaluate_loaded_bytes(&resolved);
                self.resolved_schema.set(resolved);
                self.schema.set(loaded);
                self.status_msg
                    .set(format!("Loaded schema `{schema_ref}`"));
            }
            Err(e) => {
                self.status_msg
                    .set(format!("Error loading schema `{schema_ref}`: {e}"));
            }
        }
    }

    /// Switches to one of the embedded built-in schemas by canonical name.
    pub fn load_builtin(&mut self, name: &str) {
        if let Some(s) = known_formats::by_name(name) {
            let canon = known_formats::canonical_name(name).unwrap_or(name);
            let sref = SchemaRef::Builtin(canon.to_owned());
            self.persisted.write().schema_ref = Some(sref);
            self.apply_schema(s, format!("Switched to built-in {canon} schema"));
        } else {
            self.status_msg
                .set(format!("Unknown built-in schema `{name}`"));
        }
    }

    /// Starts a fresh blank `CustomFormat` schema.
    pub fn start_blank_schema(&mut self) {
        let s = Schema::new_empty("CustomFormat");
        self.persisted.write().schema_ref = None;
        self.apply_schema(s, "Started a blank CustomFormat schema");
    }

    /// Resolves `new_schema`, persists its YAML snapshot, and re-evaluates the open file.
    pub fn apply_schema(&mut self, new_schema: Schema, msg: impl Into<String>) {
        let base_ref = self.persisted.read().schema_ref.clone();
        let resolved = match SchemaLoader::resolve(new_schema.clone(), base_ref.as_ref()) {
            Ok(r) => r,
            Err(e) => {
                self.status_msg.set(format!("Schema resolution error: {e}"));
                return;
            }
        };
        {
            let mut p = self.persisted.write();
            if let Ok(yaml) = new_schema.to_yaml_string() {
                p.current_schema_yaml = Some(yaml);
            }
            p.save();
        }
        self.reevaluate_loaded_bytes(&resolved);
        self.resolved_schema.set(resolved);
        self.schema.set(new_schema);
        self.status_msg.set(msg.into());
    }

    /// Clones the active [`Schema`], passes it to `mutate`, and applies/persists on `Ok(())`.
    pub fn mutate_schema(
        &mut self,
        msg: impl Into<String>,
        mutate: impl FnOnce(&mut Schema, &ResolvedSchema) -> Result<(), String>,
    ) -> bool {
        let mut draft = self.schema.read().clone();
        let resolved = self.resolved_schema.read().clone();
        match mutate(&mut draft, &resolved) {
            Ok(()) => {
                self.apply_schema(draft, msg);
                true
            }
            Err(err) => {
                self.status_msg.set(err);
                false
            }
        }
    }

    /// Toggles per-file `force_plain_data_paths` for `tree_path` and re-evaluates the file.
    pub fn toggle_force_plain_data(&mut self, tree_path: &str) {
        let now_forced = {
            let mut p = self.persisted.write();
            let Some(src) = p.last_file.clone() else {
                self.status_msg
                    .set("Open a file before toggling force-plain-data.".to_owned());
                return;
            };
            let forced = p.toggle_forced_plain_path(&src, tree_path);
            p.save();
            forced
        };
        let resolved = self.resolved_schema.read().clone();
        self.reevaluate_loaded_bytes(&resolved);
        self.status_msg.set(if now_forced {
            format!("Forced chunk `{tree_path}` to decode as plain byte data")
        } else {
            format!("Restored mini-chunk decoding on chunk `{tree_path}`")
        });
    }

    /// Selects a tree node by `tree_path` and resets the hex cursor to offset 0.
    pub fn select_node(&mut self, tree_path: String) {
        self.selected_tree_path.set(tree_path);
        self.selected_byte_offset.set(0);
        self.hex_page.set(0);
    }

    /// Jumps the hex cursor and pagination page to payload byte `offset`.
    pub fn jump_to_byte(&mut self, offset: usize) {
        self.selected_byte_offset.set(offset);
        self.hex_page.set(offset / 512);
    }

    /// Collapses or expands a tree node by `tree_path`.
    pub fn toggle_collapse(&mut self, tree_path: &str) {
        let mut collapsed = self.collapsed_paths.write();
        if !collapsed.remove(tree_path) {
            collapsed.insert(tree_path.to_owned());
        }
    }

    /// Expands all nodes in the chunk tree.
    pub fn expand_all(&mut self) {
        self.collapsed_paths.write().clear();
    }

    /// Toggles a drawer modal (closing it if already open).
    pub fn toggle_modal(&mut self, modal: ActiveModal) {
        let current = *self.active_modal.read();
        self.active_modal.set(if current == modal {
            ActiveModal::None
        } else {
            modal
        });
    }

    /// Closes any open drawer modal.
    pub fn close_modal(&mut self) {
        self.active_modal.set(ActiveModal::None);
    }

    /// Updates the status bar text.
    pub fn set_status(&mut self, msg: impl Into<String>) {
        self.status_msg.set(msg.into());
    }

    fn reevaluate_loaded_bytes(&mut self, resolved: &ResolvedSchema) {
        let bytes = self.raw_file_bytes.read();
        if bytes.is_empty() {
            return;
        }
        let p = self.persisted.read();
        let opts = LoadOptions::for_ui(p.forced_plain_paths_for(p.last_file.as_ref()));
        if let Ok(df) = load_dynamic_file(&bytes, Some(resolved), &opts) {
            self.dynamic_file.set(Some(df));
        }
    }
}
