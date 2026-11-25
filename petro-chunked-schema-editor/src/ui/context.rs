//! Shared global reactive editor state and multi-window modal management for Freya 0.5.

use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;

use freya::prelude::*;
use freya::winit::window::WindowId;
use petro_chunked_schema::dynamic::{DynamicFile, LoadOptions, load_dynamic_file};
use petro_chunked_schema::resolve::{ResolvedSchema, SchemaLoader};
use petro_chunked_schema::schema::Schema;
use petro_chunked_schema::source::SchemaRef;

use crate::gamedata::{load_file_bytes, load_schema_from_ref, save_schema_to_path};
use crate::schema_nav::{
    HoveredSchemaTarget, SchemaChildItem, SchemaStackStep, build_root_stack, refresh_schema_stack,
    resolve_schema_stack_for_file_node,
};
use crate::state::{FileSource, PersistedState};

use super::modals::modal_window_root;
use super::theme;

/// Freestanding secondary OS window kinds managed by [`EditorState`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModalKind {
    /// Hierarchical disk & browse-into-`.meg` file browser window.
    OpenFile,
    /// `GameData` directory configuration and visual folder picker window.
    GameDataDirs,
    /// Unified Open Schema window (built-in formats + YAML file browser + manual URI).
    OpenSchema,
    /// Visual Save Schema As window for choosing a destination YAML path.
    SaveSchemaAs,
    /// Unsaved schema changes confirmation dialog (`Save & Continue` / `Discard` / `Cancel`).
    ConfirmDiscard,
    /// Live read-only YAML schema serialization viewer window.
    YamlPreview,
    /// Batch schema validator window scanning all configured `GameData` directories.
    BatchValidate,
    /// Live `petro-chunked-derive` Rust code generator & `.rs` exporter window.
    RustCodegen,
    /// Interactive ASCII and NUL-terminated ASCIIZ string finder window.
    FindStrings,
}

impl ModalKind {
    /// Native OS window title for this modal kind.
    pub fn window_title(self) -> &'static str {
        match self {
            Self::OpenFile => "Open Game File or Browse .meg Archive",
            Self::GameDataDirs => "Configure GameData Directories",
            Self::OpenSchema => "Open Schema (Built-in Formats & YAML Files)",
            Self::SaveSchemaAs => "Save Schema As YAML",
            Self::ConfirmDiscard => "Unsaved Schema Changes",
            Self::YamlPreview => "Live Schema YAML",
            Self::BatchValidate => "Batch Validate Across GameData & .meg Archives",
            Self::RustCodegen => "Generated Rust Code (petro-chunked-derive)",
            Self::FindStrings => "Find Strings (ASCII / ASCIIZ)",
        }
    }

    /// Initial `(width, height)` logical window size in pixels.
    pub fn window_size(self) -> (f64, f64) {
        match self {
            Self::OpenFile => (940.0, 620.0),
            Self::GameDataDirs => (900.0, 610.0),
            Self::OpenSchema => (920.0, 610.0),
            Self::SaveSchemaAs => (880.0, 580.0),
            Self::ConfirmDiscard => (540.0, 225.0),
            Self::YamlPreview => (880.0, 600.0),
            Self::BatchValidate => (900.0, 580.0),
            Self::RustCodegen => (940.0, 620.0),
            Self::FindStrings => (1040.0, 640.0),
        }
    }
}

/// Schema-replacing action deferred while asking the user whether to save or discard unsaved
/// in-memory schema edits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PendingSchemaAction {
    /// Load a built-in or file schema via [`SchemaRef`].
    LoadSchemaRef(SchemaRef),
    /// Replace the current schema with a blank `CustomFormat` schema.
    NewBlankSchema,
}

impl PendingSchemaAction {
    /// Short human-readable phrase describing what will happen after confirmation.
    pub fn describe(&self) -> String {
        match self {
            Self::LoadSchemaRef(sref) => format!("open schema `{sref}`"),
            Self::NewBlankSchema => "start a new blank schema".to_owned(),
        }
    }
}

/// Global reactive state shared across the main editor window and all freestanding modal windows.
#[derive(Clone, Copy)]
pub struct EditorState {
    /// Persistent session settings saved to disk on edits.
    pub persisted: State<PersistedState>,
    /// Active editable root [`Schema`] (updated live in memory).
    pub schema: State<Schema>,
    /// Resolved multi-module graph for [`Self::schema`] (including any `imports`).
    pub resolved_schema: State<ResolvedSchema>,
    /// Baseline YAML snapshot corresponding to the last loaded or saved schema state.
    pub saved_schema_yaml: State<String>,
    /// `true` when the active in-memory schema differs from [`Self::saved_schema_yaml`].
    pub is_schema_dirty: State<bool>,
    /// Deferred schema replacement awaiting confirmation when [`Self::is_schema_dirty`] is `true`.
    pub pending_schema_action: State<Option<PendingSchemaAction>>,
    /// Raw bytes of the currently opened chunked binary file.
    pub raw_file_bytes: State<Vec<u8>>,
    /// Dynamically parsed & schema-annotated representation of [`Self::raw_file_bytes`].
    pub dynamic_file: State<Option<DynamicFile>>,
    /// Currently selected chunk/mini-chunk tree path (e.g. `"0"`, `"0/1"`, `"0/1/mc2"`).
    pub selected_tree_path: State<String>,
    /// Tree paths currently collapsed in the left-pane tree view.
    pub collapsed_paths: State<BTreeSet<String>>,
    /// Right-rail non-indenting schema stack from root down to the currently focused level.
    pub schema_stack: State<Vec<SchemaStackStep>>,
    /// Indices of ancestor cards in [`Self::schema_stack`] currently expanded by the user.
    pub expanded_stack_levels: State<BTreeSet<usize>>,
    /// Schema type or field currently hovered in the right rail for left-rail match highlighting.
    pub hovered_schema_target: State<Option<HoveredSchemaTarget>>,
    /// Selected byte offset inside the currently inspected leaf chunk or mini-chunk payload.
    pub selected_byte_offset: State<usize>,
    /// Active 512-byte page index in the hex grid.
    pub hex_page: State<usize>,
    /// Currently open freestanding modal windows keyed by [`ModalKind`].
    pub open_windows: State<HashMap<ModalKind, WindowId>>,
    /// Single-line status feedback displayed in the status bar.
    pub status_msg: State<String>,
}

impl EditorState {
    /// Creates the global [`EditorState`] prior to launching the Freya 0.5 event loop and restores
    /// the previously opened binary file and schema snapshot if present.
    pub fn new_global() -> Self {
        let initial_persisted = PersistedState::load();
        let (initial_schema, initial_resolved) = initial_persisted.resolve_initial_schema();
        let current_yaml = initial_schema.to_yaml_string().unwrap_or_default();
        let baseline_yaml = match &initial_persisted.schema_ref {
            Some(sref) => load_schema_from_ref(sref)
                .ok()
                .and_then(|(s, _)| s.to_yaml_string().ok())
                .unwrap_or_else(|| current_yaml.clone()),
            None => Schema::new_empty("CustomFormat")
                .to_yaml_string()
                .unwrap_or_else(|_| current_yaml.clone()),
        };
        let initial_dirty = current_yaml != baseline_yaml;

        let mut initial_bytes = Vec::new();
        let mut initial_df = None;
        let mut initial_stack = build_root_stack(&initial_resolved);
        let mut initial_status = "Ready.".to_owned();

        if let Some(src) = &initial_persisted.last_file {
            match load_file_bytes(src) {
                Ok(bytes) => {
                    let opts =
                        LoadOptions::for_ui(initial_persisted.forced_plain_paths_for(Some(src)));
                    match load_dynamic_file(&bytes, Some(&initial_resolved), &opts) {
                        Ok(df) => {
                            initial_status = format!(
                                "Restored {} ({} bytes, {} diagnostics)",
                                src.display_label(),
                                bytes.len(),
                                df.diagnostics.len()
                            );
                            initial_stack =
                                resolve_schema_stack_for_file_node(&df, &initial_resolved, "0");
                            initial_bytes = bytes;
                            initial_df = Some(df);
                        }
                        Err(e) => {
                            initial_status = format!("Failed to parse {}: {e}", src.display_label());
                        }
                    }
                }
                Err(e) => {
                    initial_status = format!("Could not open previous file: {e}");
                }
            }
        }

        Self {
            persisted: State::create_global(initial_persisted),
            schema: State::create_global(initial_schema),
            resolved_schema: State::create_global(initial_resolved),
            saved_schema_yaml: State::create_global(baseline_yaml),
            is_schema_dirty: State::create_global(initial_dirty),
            pending_schema_action: State::create_global(None),
            raw_file_bytes: State::create_global(initial_bytes),
            dynamic_file: State::create_global(initial_df),
            selected_tree_path: State::create_global("0".to_owned()),
            collapsed_paths: State::create_global(BTreeSet::new()),
            schema_stack: State::create_global(initial_stack),
            expanded_stack_levels: State::create_global(BTreeSet::new()),
            hovered_schema_target: State::create_global(None),
            selected_byte_offset: State::create_global(0),
            hex_page: State::create_global(0),
            open_windows: State::create_global(HashMap::new()),
            status_msg: State::create_global(initial_status),
        }
    }

    /// Retrieves the global [`EditorState`] inside any component in any window.
    pub fn current() -> Self {
        consume_root_context::<Self>()
    }

    /// Returns `true` if a freestanding window of `kind` is currently open.
    pub fn is_modal_open(&self, kind: ModalKind) -> bool {
        self.open_windows.read().contains_key(&kind)
    }

    /// Opens a freestanding OS window for `kind`, or focuses it if it is already open.
    pub fn open_modal(&mut self, kind: ModalKind) {
        if let Some(&wid) = self.open_windows.read().get(&kind) {
            Platform::get().focus_window(wid);
            return;
        }
        let mut state = *self;
        let (w, h) = kind.window_size();
        let title = kind.window_title();
        spawn_in_window(async move {
            let wid = Platform::get()
                .launch_window(
                    WindowConfig::new(move || modal_window_root(kind))
                        .with_root_context(state)
                        .with_size(w, h)
                        .with_title(title)
                        .with_background(theme::BG_MODAL)
                        .with_on_close(move |_, closed_id| {
                            state.on_modal_window_closed(kind, closed_id);
                            CloseDecision::Close
                        }),
                )
                .await;
            state.open_windows.write().insert(kind, wid);
        });
    }

    /// Closes the freestanding window for `kind` if it is currently open.
    pub fn close_modal(&mut self, kind: ModalKind) {
        let wid = self.open_windows.write().remove(&kind);
        if let Some(wid) = wid {
            Platform::get().close_window(wid);
        }
    }

    /// Closes the currently executing freestanding modal window.
    pub fn close_current_modal_window(&mut self) {
        let wid = Platform::window_id();
        self.open_windows.write().retain(|_, id| *id != wid);
        Platform::get().close_window(wid);
    }

    /// Closes all open freestanding modal windows (called when the main editor window closes).
    pub fn close_all_modals(&mut self) {
        let wids: Vec<WindowId> = self.open_windows.write().drain().map(|(_, id)| id).collect();
        let platform = Platform::get();
        for wid in wids {
            platform.close_window(wid);
        }
    }

    fn on_modal_window_closed(&mut self, kind: ModalKind, closed_id: WindowId) {
        let mut map = self.open_windows.write();
        if map.get(&kind) == Some(&closed_id) {
            map.remove(&kind);
        }
        if kind == ModalKind::ConfirmDiscard {
            self.pending_schema_action.set(None);
        }
    }

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
                        let stack = resolve_schema_stack_for_file_node(&df, &resolved, "0");
                        self.raw_file_bytes.set(bytes);
                        self.dynamic_file.set(Some(df));
                        self.selected_tree_path.set("0".to_owned());
                        self.collapsed_paths.write().clear();
                        self.schema_stack.set(stack);
                        self.expanded_stack_levels.write().clear();
                        self.hovered_schema_target.set(None);
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

    /// Requests loading `schema_ref` (`builtin:...` or YAML file path), opening the freestanding
    /// confirmation dialog first if the active schema has unsaved in-memory edits.
    pub fn request_load_schema_ref(&mut self, schema_ref: SchemaRef) {
        if *self.is_schema_dirty.read() {
            self.pending_schema_action
                .set(Some(PendingSchemaAction::LoadSchemaRef(schema_ref)));
            self.open_modal(ModalKind::ConfirmDiscard);
        } else {
            self.load_schema_ref_now(schema_ref);
            self.close_modal(ModalKind::OpenSchema);
        }
    }

    /// Requests starting a fresh blank `CustomFormat` schema, opening the freestanding
    /// confirmation dialog first if the active schema has unsaved in-memory edits.
    pub fn request_new_blank_schema(&mut self) {
        if *self.is_schema_dirty.read() {
            self.pending_schema_action
                .set(Some(PendingSchemaAction::NewBlankSchema));
            self.open_modal(ModalKind::ConfirmDiscard);
        } else {
            self.start_blank_schema_now();
            self.close_modal(ModalKind::OpenSchema);
        }
    }

    /// Saves the active schema to its current YAML file path (`Ctrl+S`), or opens the
    /// freestanding `Save Schema As` window if the active schema is built-in or unsaved.
    pub fn save_schema(&mut self) {
        let current_ref = self.persisted.read().schema_ref.clone();
        match current_ref {
            Some(SchemaRef::File(path)) => {
                self.save_schema_as(path);
            }
            _ => {
                self.open_modal(ModalKind::SaveSchemaAs);
                self.status_msg.set(
                    "Built-in or unsaved schema — choose a YAML destination file to save."
                        .to_owned(),
                );
            }
        }
    }

    /// Saves the active schema to `path`, clears the dirty flag, updates the persisted
    /// [`SchemaRef`], and completes any deferred [`PendingSchemaAction`] queued from
    /// [`ModalKind::ConfirmDiscard`].
    pub fn save_schema_as(&mut self, path: PathBuf) -> bool {
        let cur_schema = self.schema.read().clone();
        match save_schema_to_path(&cur_schema, &path) {
            Ok(()) => {
                let yaml = cur_schema.to_yaml_string().unwrap_or_default();
                let sref = SchemaRef::File(path.clone());
                {
                    let mut p = self.persisted.write();
                    p.schema_ref = Some(sref.clone());
                    p.current_schema_yaml = Some(yaml.clone());
                    p.save();
                }
                self.saved_schema_yaml.set(yaml);
                self.is_schema_dirty.set(false);
                if let Ok(resolved) = SchemaLoader::resolve(cur_schema, Some(&sref)) {
                    self.reevaluate_loaded_bytes(&resolved);
                    self.resolved_schema.set(resolved);
                }
                self.status_msg
                    .set(format!("Saved schema to {}", path.display()));

                let pending = self.pending_schema_action.write().take();
                self.close_modal(ModalKind::SaveSchemaAs);
                self.close_modal(ModalKind::ConfirmDiscard);
                if let Some(action) = pending {
                    self.execute_pending_action(action);
                    self.close_modal(ModalKind::OpenSchema);
                }
                true
            }
            Err(e) => {
                self.status_msg.set(format!("Error saving schema: {e}"));
                false
            }
        }
    }

    /// Handles `"Save & Continue"` in [`ModalKind::ConfirmDiscard`].
    pub fn save_and_continue_pending(&mut self) {
        let current_ref = self.persisted.read().schema_ref.clone();
        match current_ref {
            Some(SchemaRef::File(path)) => {
                self.save_schema_as(path);
            }
            _ => {
                self.close_modal(ModalKind::ConfirmDiscard);
                self.open_modal(ModalKind::SaveSchemaAs);
                self.status_msg
                    .set("Choose a YAML file to save changes before continuing.".to_owned());
            }
        }
    }

    /// Handles `"Discard Changes"` in [`ModalKind::ConfirmDiscard`].
    pub fn discard_and_continue_pending(&mut self) {
        let pending = self.pending_schema_action.write().take();
        self.close_modal(ModalKind::ConfirmDiscard);
        if let Some(action) = pending {
            self.execute_pending_action(action);
            self.close_modal(ModalKind::OpenSchema);
        }
    }

    /// Cancels any pending discard action and closes the confirmation / save-as dialog.
    pub fn cancel_pending_action(&mut self) {
        self.pending_schema_action.set(None);
        self.close_current_modal_window();
    }

    fn execute_pending_action(&mut self, action: PendingSchemaAction) {
        match action {
            PendingSchemaAction::LoadSchemaRef(sref) => self.load_schema_ref_now(sref),
            PendingSchemaAction::NewBlankSchema => self.start_blank_schema_now(),
        }
    }

    fn load_schema_ref_now(&mut self, schema_ref: SchemaRef) {
        match load_schema_from_ref(&schema_ref) {
            Ok((loaded, resolved)) => {
                let yaml = loaded.to_yaml_string().unwrap_or_default();
                {
                    let mut p = self.persisted.write();
                    p.schema_ref = Some(schema_ref.clone());
                    p.current_schema_yaml = Some(yaml.clone());
                    p.save();
                }
                self.saved_schema_yaml.set(yaml);
                self.is_schema_dirty.set(false);
                self.reevaluate_loaded_bytes(&resolved);
                let sel_path = self.selected_tree_path.read().clone();
                let stack = match self.dynamic_file.read().as_ref() {
                    Some(df) => resolve_schema_stack_for_file_node(df, &resolved, &sel_path),
                    None => build_root_stack(&resolved),
                };
                self.resolved_schema.set(resolved);
                self.schema.set(loaded);
                self.schema_stack.set(stack);
                self.expanded_stack_levels.write().clear();
                self.hovered_schema_target.set(None);
                self.status_msg
                    .set(format!("Loaded schema `{schema_ref}`"));
            }
            Err(e) => {
                self.status_msg
                    .set(format!("Error loading schema `{schema_ref}`: {e}"));
            }
        }
    }

    fn start_blank_schema_now(&mut self) {
        let blank = Schema::new_empty("CustomFormat");
        let yaml = blank.to_yaml_string().unwrap_or_default();
        let resolved = match SchemaLoader::resolve(blank.clone(), None) {
            Ok(r) => r,
            Err(e) => {
                self.status_msg.set(format!("Schema resolution error: {e}"));
                return;
            }
        };
        {
            let mut p = self.persisted.write();
            p.schema_ref = None;
            p.current_schema_yaml = Some(yaml.clone());
            p.save();
        }
        self.saved_schema_yaml.set(yaml);
        self.is_schema_dirty.set(false);
        self.reevaluate_loaded_bytes(&resolved);
        let sel_path = self.selected_tree_path.read().clone();
        let stack = match self.dynamic_file.read().as_ref() {
            Some(df) => resolve_schema_stack_for_file_node(df, &resolved, &sel_path),
            None => build_root_stack(&resolved),
        };
        self.resolved_schema.set(resolved);
        self.schema.set(blank);
        self.schema_stack.set(stack);
        self.expanded_stack_levels.write().clear();
        self.hovered_schema_target.set(None);
        self.status_msg
            .set("Started a blank CustomFormat schema".to_owned());
    }

    /// Resolves `new_schema` in memory, updates dirty tracking and persisted draft YAML,
    /// re-evaluates the open file, and keeps the right-rail schema stack synchronized.
    pub fn apply_schema(&mut self, new_schema: Schema, msg: impl Into<String>) {
        let base_ref = self.persisted.read().schema_ref.clone();
        let resolved = match SchemaLoader::resolve(new_schema.clone(), base_ref.as_ref()) {
            Ok(r) => r,
            Err(e) => {
                self.status_msg.set(format!("Schema resolution error: {e}"));
                return;
            }
        };
        let new_yaml = new_schema.to_yaml_string().unwrap_or_default();
        let dirty = new_yaml != *self.saved_schema_yaml.read();
        {
            let mut p = self.persisted.write();
            p.current_schema_yaml = Some(new_yaml);
            p.save();
        }
        self.is_schema_dirty.set(dirty);
        self.reevaluate_loaded_bytes(&resolved);
        let old_stack = self.schema_stack.read().clone();
        let sel_path = self.selected_tree_path.read().clone();
        let new_stack = refresh_schema_stack(
            &resolved,
            &old_stack,
            self.dynamic_file.read().as_ref(),
            &sel_path,
        );
        self.resolved_schema.set(resolved);
        self.schema.set(new_schema);
        self.schema_stack.set(new_stack);
        self.status_msg.set(msg.into());
    }

    /// Clones the active [`Schema`], passes it to `mutate`, and applies in memory on `Ok(())`.
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
        let sel_path = self.selected_tree_path.read().clone();
        if let Some(df) = self.dynamic_file.read().as_ref() {
            let stack = resolve_schema_stack_for_file_node(df, &resolved, &sel_path);
            self.schema_stack.set(stack);
        }
        self.status_msg.set(if now_forced {
            format!("Forced chunk `{tree_path}` to decode as plain byte data")
        } else {
            format!("Restored mini-chunk decoding on chunk `{tree_path}`")
        });
    }

    /// Selects a tree node by `tree_path`, resets the hex cursor to offset 0, and syncs the
    /// right-rail schema stack to that node's full schema ancestry.
    pub fn select_node(&mut self, tree_path: String) {
        let resolved = self.resolved_schema.read().clone();
        let stack = match self.dynamic_file.read().as_ref() {
            Some(df) => resolve_schema_stack_for_file_node(df, &resolved, &tree_path),
            None => build_root_stack(&resolved),
        };
        self.selected_tree_path.set(tree_path);
        self.schema_stack.set(stack);
        self.expanded_stack_levels.write().clear();
        self.hovered_schema_target.set(None);
        self.selected_byte_offset.set(0);
        self.hex_page.set(0);
    }

    /// Navigates the right-rail schema stack to a child field or variant of `parent_level_idx`,
    /// allowing inspection and editing even when that definition has `0` occurrences in the file.
    pub fn select_schema_child(&mut self, parent_level_idx: usize, child: SchemaChildItem) {
        let mut stack = self.schema_stack.read().clone();
        stack.truncate(parent_level_idx + 1);
        stack.push(SchemaStackStep {
            incoming_edge: Some(child.edge),
            target: child.target,
            file_tree_path: None,
        });
        self.schema_stack.set(stack);
        self.expanded_stack_levels.write().clear();
        self.hovered_schema_target.set(None);
    }

    /// Truncates the right-rail schema stack so `level_idx` becomes the active bottom card.
    pub fn focus_schema_level(&mut self, level_idx: usize) {
        let mut stack = self.schema_stack.read().clone();
        if level_idx < stack.len() {
            stack.truncate(level_idx + 1);
            self.schema_stack.set(stack);
            self.expanded_stack_levels.write().clear();
            self.hovered_schema_target.set(None);
        }
    }

    /// Expands or collapses an ancestor card at `level_idx` in the right-rail schema stack.
    pub fn toggle_stack_level_expanded(&mut self, level_idx: usize) {
        let mut expanded = self.expanded_stack_levels.write();
        if !expanded.remove(&level_idx) {
            expanded.insert(level_idx);
        }
    }

    /// Updates [`Self::hovered_schema_target`] only when the value actually changes.
    pub fn set_hovered_schema_target(&mut self, target: Option<HoveredSchemaTarget>) {
        let changed = *self.hovered_schema_target.peek() != target;
        if changed {
            self.hovered_schema_target.set(target);
        }
    }

    /// Jumps the hex cursor and pagination page to payload byte `offset`.
    pub fn jump_to_byte(&mut self, offset: usize) {
        self.selected_byte_offset.set(offset);
        self.hex_page.set(offset / 512);
    }

    /// Expands ancestor nodes in the left tree, selects `tree_path`, and positions the hex cursor
    /// and pagination page on payload byte `payload_offset`.
    pub fn select_node_at_offset(&mut self, tree_path: String, payload_offset: usize) {
        {
            let mut collapsed = self.collapsed_paths.write();
            let mut prefix_end = 0;
            while let Some(slash) = tree_path[prefix_end..].find('/') {
                let end = prefix_end + slash;
                collapsed.remove(&tree_path[..end]);
                prefix_end = end + 1;
            }
        }
        self.select_node(tree_path);
        self.jump_to_byte(payload_offset);
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
