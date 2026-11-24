//! Shared reactive editor state and domain actions provided via Dioxus/Freya context.

use std::collections::BTreeSet;
use std::path::PathBuf;

use freya::prelude::*;
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

/// Which centered overlay modal dialog is currently active above the main 3-pane workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActiveModal {
    /// No modal open.
    None,
    /// Hierarchical disk & browse-into-`.meg` file browser dialog.
    OpenFile,
    /// `GameData` directory configuration and visual folder picker dialog.
    GameDataDirs,
    /// Unified Open Schema dialog (built-in formats + YAML file browser + manual URI).
    OpenSchema,
    /// Visual Save Schema As dialog for choosing a destination YAML path.
    SaveSchemaAs,
    /// Unsaved schema changes confirmation dialog (`Save & Continue` / `Discard` / `Cancel`).
    ConfirmDiscard,
    /// Live read-only YAML schema serialization viewer.
    YamlPreview,
    /// Batch schema validator across all configured `GameData` directories.
    BatchValidate,
    /// Live `petro-chunked-derive` Rust code generator & `.rs` exporter.
    RustCodegen,
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

/// Shared reactive state handle for the editor, accessible in any child component via
/// `use_context::<EditorState>()`.
#[derive(Clone, Copy)]
pub struct EditorState {
    /// Persistent session settings saved to disk on edits.
    pub persisted: Signal<PersistedState>,
    /// Active editable root [`Schema`] (updated live in memory).
    pub schema: Signal<Schema>,
    /// Resolved multi-module graph for [`Self::schema`] (including any `imports`).
    pub resolved_schema: Signal<ResolvedSchema>,
    /// Baseline YAML snapshot corresponding to the last loaded or saved schema state.
    pub saved_schema_yaml: Signal<String>,
    /// `true` when the active in-memory schema differs from [`Self::saved_schema_yaml`].
    pub is_schema_dirty: Signal<bool>,
    /// Deferred schema replacement awaiting confirmation when [`Self::is_schema_dirty`] is `true`.
    pub pending_schema_action: Signal<Option<PendingSchemaAction>>,
    /// Raw bytes of the currently opened chunked binary file.
    pub raw_file_bytes: Signal<Vec<u8>>,
    /// Dynamically parsed & schema-annotated representation of [`Self::raw_file_bytes`].
    pub dynamic_file: Signal<Option<DynamicFile>>,
    /// Currently selected chunk/mini-chunk tree path (e.g. `"0"`, `"0/1"`, `"0/1/mc2"`).
    pub selected_tree_path: Signal<String>,
    /// Tree paths currently collapsed in the left-pane tree view.
    pub collapsed_paths: Signal<BTreeSet<String>>,
    /// Right-rail non-indenting schema stack from root down to the currently focused level.
    pub schema_stack: Signal<Vec<SchemaStackStep>>,
    /// Indices of ancestor cards in [`Self::schema_stack`] currently expanded by the user.
    pub expanded_stack_levels: Signal<BTreeSet<usize>>,
    /// Schema type or field currently hovered in the right rail for left-rail match highlighting.
    pub hovered_schema_target: Signal<Option<HoveredSchemaTarget>>,
    /// Selected byte offset inside the currently inspected leaf chunk or mini-chunk payload.
    pub selected_byte_offset: Signal<usize>,
    /// Active 512-byte page index in the hex grid.
    pub hex_page: Signal<usize>,
    /// Active centered overlay dialog.
    pub active_modal: Signal<ActiveModal>,
    /// Single-line status feedback displayed in the status bar.
    pub status_msg: Signal<String>,
}

/// Initializes [`EditorState`], registers it as Freya root context, and restores the previously
/// opened file (if any) once on mount.
pub fn use_init_editor_state() -> EditorState {
    let initial_persisted = use_hook(PersistedState::load);
    let (initial_schema, initial_resolved) = use_hook(|| initial_persisted.resolve_initial_schema());
    let (initial_baseline_yaml, initial_dirty) = use_hook(|| {
        let current_yaml = initial_schema.to_yaml_string().unwrap_or_default();
        let baseline = match &initial_persisted.schema_ref {
            Some(sref) => load_schema_from_ref(sref)
                .ok()
                .and_then(|(s, _)| s.to_yaml_string().ok())
                .unwrap_or_else(|| current_yaml.clone()),
            None => Schema::new_empty("CustomFormat")
                .to_yaml_string()
                .unwrap_or_else(|_| current_yaml.clone()),
        };
        let dirty = current_yaml != baseline;
        (baseline, dirty)
    });

    let persisted = use_signal(|| initial_persisted.clone());
    let schema = use_signal(|| initial_schema);
    let resolved_schema = use_signal(|| initial_resolved.clone());
    let saved_schema_yaml = use_signal(|| initial_baseline_yaml);
    let is_schema_dirty = use_signal(|| initial_dirty);
    let pending_schema_action = use_signal(|| None::<PendingSchemaAction>);
    let raw_file_bytes = use_signal(Vec::<u8>::new);
    let dynamic_file = use_signal(|| None::<DynamicFile>);
    let selected_tree_path = use_signal(|| "0".to_owned());
    let collapsed_paths = use_signal(BTreeSet::<String>::new);
    let schema_stack = use_signal(|| build_root_stack(&initial_resolved));
    let expanded_stack_levels = use_signal(BTreeSet::<usize>::new);
    let hovered_schema_target = use_signal(|| None::<HoveredSchemaTarget>);
    let selected_byte_offset = use_signal(|| 0usize);
    let hex_page = use_signal(|| 0usize);
    let active_modal = use_signal(|| ActiveModal::None);
    let status_msg = use_signal(|| "Ready.".to_owned());

    let mut state = EditorState {
        persisted,
        schema,
        resolved_schema,
        saved_schema_yaml,
        is_schema_dirty,
        pending_schema_action,
        raw_file_bytes,
        dynamic_file,
        selected_tree_path,
        collapsed_paths,
        schema_stack,
        expanded_stack_levels,
        hovered_schema_target,
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
                            let stack =
                                resolve_schema_stack_for_file_node(&df, &initial_resolved, "0");
                            state.status_msg.set(format!(
                                "Restored {} ({} bytes, {} diagnostics)",
                                src.display_label(),
                                bytes.len(),
                                df.diagnostics.len()
                            ));
                            state.raw_file_bytes.set(bytes);
                            state.dynamic_file.set(Some(df));
                            state.schema_stack.set(stack);
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

    /// Requests loading `schema_ref` (`builtin:...` or YAML file path), prompting the user first
    /// if the active schema has unsaved in-memory edits.
    pub fn request_load_schema_ref(&mut self, schema_ref: SchemaRef) {
        if *self.is_schema_dirty.read() {
            self.pending_schema_action
                .set(Some(PendingSchemaAction::LoadSchemaRef(schema_ref)));
            self.active_modal.set(ActiveModal::ConfirmDiscard);
        } else {
            self.load_schema_ref_now(schema_ref);
            self.active_modal.set(ActiveModal::None);
        }
    }

    /// Requests starting a fresh blank `CustomFormat` schema, prompting the user first if the
    /// active schema has unsaved in-memory edits.
    pub fn request_new_blank_schema(&mut self) {
        if *self.is_schema_dirty.read() {
            self.pending_schema_action
                .set(Some(PendingSchemaAction::NewBlankSchema));
            self.active_modal.set(ActiveModal::ConfirmDiscard);
        } else {
            self.start_blank_schema_now();
            self.active_modal.set(ActiveModal::None);
        }
    }

    /// Saves the active schema to its current YAML file path (`Ctrl+S`), or opens the
    /// `SaveSchemaAs` dialog if the active schema is built-in or has not yet been saved to disk.
    pub fn save_schema(&mut self) {
        let current_ref = self.persisted.read().schema_ref.clone();
        match current_ref {
            Some(SchemaRef::File(path)) => {
                self.save_schema_as(path);
            }
            _ => {
                self.active_modal.set(ActiveModal::SaveSchemaAs);
                self.status_msg.set(
                    "Built-in or unsaved schema — choose a YAML destination file to save."
                        .to_owned(),
                );
            }
        }
    }

    /// Saves the active schema to `path`, clears the dirty flag, updates the persisted
    /// [`SchemaRef`], and completes any deferred [`PendingSchemaAction`] queued from
    /// [`ActiveModal::ConfirmDiscard`].
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
                if let Some(action) = pending {
                    self.execute_pending_action(action);
                }
                self.active_modal.set(ActiveModal::None);
                true
            }
            Err(e) => {
                self.status_msg.set(format!("Error saving schema: {e}"));
                false
            }
        }
    }

    /// Handles `"Save & Continue"` in [`ActiveModal::ConfirmDiscard`]: saves directly when a file
    /// path exists, or transitions to [`ActiveModal::SaveSchemaAs`] while keeping the pending
    /// action queued.
    pub fn save_and_continue_pending(&mut self) {
        let current_ref = self.persisted.read().schema_ref.clone();
        match current_ref {
            Some(SchemaRef::File(path)) => {
                self.save_schema_as(path);
            }
            _ => {
                self.active_modal.set(ActiveModal::SaveSchemaAs);
                self.status_msg
                    .set("Choose a YAML file to save changes before continuing.".to_owned());
            }
        }
    }

    /// Handles `"Discard Changes"` in [`ActiveModal::ConfirmDiscard`]: drops unsaved edits and
    /// runs the deferred schema switch immediately.
    pub fn discard_and_continue_pending(&mut self) {
        let pending = self.pending_schema_action.write().take();
        self.active_modal.set(ActiveModal::None);
        if let Some(action) = pending {
            self.execute_pending_action(action);
        }
    }

    /// Cancels any pending discard action and closes the active modal dialog.
    pub fn cancel_pending_action(&mut self) {
        self.pending_schema_action.set(None);
        self.active_modal.set(ActiveModal::None);
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
        if *self.hovered_schema_target.read() != target {
            self.hovered_schema_target.set(target);
        }
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

    /// Opens or toggles a centered modal dialog.
    pub fn toggle_modal(&mut self, modal: ActiveModal) {
        let current = *self.active_modal.read();
        if current == modal {
            self.cancel_pending_action();
        } else {
            self.pending_schema_action.set(None);
            self.active_modal.set(modal);
        }
    }

    /// Closes any open modal dialog and clears any pending discard action.
    pub fn close_modal(&mut self) {
        self.cancel_pending_action();
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
