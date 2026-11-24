//! `petro-chunked-schema-editor`: Graphical schema editor and binary chunk inspector for
//! Petroglyph's chunked file formats, built with [`freya`].
//!
//! Features:
//! - Remembers the last opened binary file, active schema, `GameData` directories, and
//!   `force_plain_data` overrides across launches (`~/.config/petro-chunked-schema-editor/state.json`).
//! - Loads chunked files directly from disk or in-memory from Petroglyph `.meg` archives (`petro-meg`).
//! - Runs batch **"Validate"** across all loose files and `.meg` archives in configured `GameData`
//!   directories to identify gaps in a schema definition.
//! - Emits `petro-chunked-derive` Rust code implementing the active schema.
//! - Provides a color-coded hex editor that highlights bytes assigned to structural fields, allows
//!   forcing auto-detected mini-chunk blocks to read as plain data, and previews bytes at the cursor
//!   across primitive, boolean, vector, and string formats with one-click field addition.
//! - Interactive tools to name chunks/mini-chunks, configure container ordering (`map` vs `sequence`),
//!   set cardinality (`required`, `optional`, `repeated`, `default`), and build `ByteBlock` structs.

mod editor_ops;
mod gamedata;
mod state;

use std::collections::BTreeSet;
use std::path::PathBuf;

use clap::Parser;
use freya::prelude::*;
use petro_chunked_schema::codegen::generate_rust_code;
use petro_chunked_schema::dynamic::{
    ByteSpanAnnotation, DynamicFile, LoadOptions, inspect_bytes_at_offset, load_dynamic_file,
};
use petro_chunked_schema::known_formats;
use petro_chunked_schema::schema::{
    ByteFieldDef, Cardinality, ChunkFieldSource, ChunkPayloadSpec, DataType, OrderMode, Schema,
    TypeDef, WireFormat,
};

use crate::editor_ops::{
    ParentContainerRef, SelectedNodeDetails, append_field_to_node_byte_block, build_flat_tree,
    find_existing_chunk_field, find_existing_mini_chunk_field, resolve_selected_node,
    to_pascal_case, upsert_chunk_field, upsert_mini_chunk_field,
};
use crate::gamedata::{
    BatchValidationReport, DiscoveredFile, discover_files, load_file_bytes, load_schema_from_path,
    run_batch_validation, save_schema_to_path,
};
use crate::state::{ActiveSchemaSource, FileSource, PersistedState};

/// Command-line arguments for `petro-chunked-schema-editor`.
#[derive(Debug, Parser)]
#[command(
    name = "petro-chunked-schema-editor",
    about = "Freya-based visual schema editor and hex inspector for Petroglyph chunked binary files"
)]
struct CliArgs {
    /// Open a specific chunked binary file on launch.
    #[arg(long)]
    file: Option<PathBuf>,

    /// Load a YAML schema file from disk on launch.
    #[arg(long)]
    schema: Option<PathBuf>,

    /// Load a built-in schema on launch (`AloModel`, `AloParticle`, or `AlaAnimation`).
    #[arg(long)]
    builtin_schema: Option<String>,

    /// Add one or more `GameData` directories for browsing `.meg` archives and batch validation.
    #[arg(long = "game-data")]
    game_data: Vec<PathBuf>,

    /// Run headless batch validation over all files with this extension (e.g. `alo` or `ala`) across
    /// configured `GameData` directories and exit.
    #[arg(long)]
    validate: Option<String>,

    /// Print the generated `petro-chunked-derive` Rust code for the active schema to stdout and exit.
    #[arg(long)]
    emit_rust: bool,
}

fn load_builtin_schema(name: &str) -> Option<Schema> {
    if name.eq_ignore_ascii_case("AloModel") || name.eq_ignore_ascii_case("alo_model") {
        Some(known_formats::alo_model())
    } else if name.eq_ignore_ascii_case("AloParticle") || name.eq_ignore_ascii_case("alo_particle")
    {
        Some(known_formats::alo_particle())
    } else if name.eq_ignore_ascii_case("AlaAnimation")
        || name.eq_ignore_ascii_case("ala_animation")
    {
        Some(known_formats::ala_animation())
    } else {
        None
    }
}

fn main() -> anyhow::Result<()> {
    let args = CliArgs::parse();
    let mut persisted = PersistedState::load();

    // Apply any CLI overrides to persisted state
    for dir in &args.game_data {
        if !persisted.game_data_dirs.contains(dir) {
            persisted.game_data_dirs.push(dir.clone());
        }
    }
    if let Some(file_path) = &args.file {
        persisted.last_file = Some(FileSource::Disk {
            path: file_path.clone(),
        });
    }
    if let Some(builtin) = &args.builtin_schema {
        if let Some(s) = load_builtin_schema(builtin) {
            persisted.schema_source = ActiveSchemaSource::Builtin {
                name: s.name.clone(),
            };
            persisted.current_schema_yaml = s.to_yaml_string().ok();
        } else {
            anyhow::bail!(
                "Unknown built-in schema `{builtin}` (expected AloModel, AloParticle, or AlaAnimation)"
            );
        }
    }
    if let Some(schema_path) = &args.schema {
        let s = load_schema_from_path(schema_path)?;
        persisted.schema_source = ActiveSchemaSource::File {
            path: schema_path.clone(),
        };
        persisted.current_schema_yaml = s.to_yaml_string().ok();
    }
    persisted.save();

    // Resolve active schema for headless commands
    let active_schema = resolve_initial_schema(&persisted);

    if args.emit_rust {
        let code = generate_rust_code(&active_schema);
        println!("{code}");
        return Ok(());
    }

    if let Some(ext) = &args.validate {
        let report = run_batch_validation(&active_schema, &persisted.game_data_dirs, ext);
        println!(
            "Batch Validation for schema `{}` on `.{}` across {} GameData dir(s):",
            report.schema_name,
            report.extension,
            persisted.game_data_dirs.len()
        );
        println!(
            "  Checked: {} | Passed: {} | Failed: {} | Skipped: {}",
            report.total_checked, report.passed_count, report.failed_count, report.skipped_count
        );
        for fail in &report.failures {
            println!("  FAIL: {}", fail.display_name);
            for d in fail.diagnostics.iter().take(5) {
                println!(
                    "    - [{} @ 0x{:X}] {}",
                    d.tree_path, d.file_offset, d.message
                );
            }
        }
        if report.failed_count > 0 {
            std::process::exit(1);
        }
        return Ok(());
    }

    // Launch the Freya GUI
    launch_cfg(
        app,
        LaunchConfig::<()>::new()
            .with_size(1480.0, 920.0)
            .with_title("Petro Chunked Schema Editor"),
    );
    Ok(())
}

fn resolve_initial_schema(persisted: &PersistedState) -> Schema {
    if let Some(yaml) = &persisted.current_schema_yaml
        && let Ok(s) = Schema::from_yaml_str(yaml)
    {
        return s;
    }
    match &persisted.schema_source {
        ActiveSchemaSource::Builtin { name } => {
            load_builtin_schema(name).unwrap_or_else(known_formats::alo_model)
        }
        ActiveSchemaSource::File { path } => {
            load_schema_from_path(path).unwrap_or_else(|_| known_formats::alo_model())
        }
        ActiveSchemaSource::Custom => known_formats::alo_model(),
    }
}

/// Which modal/drawer panel is currently open in the top workspace area.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActiveModal {
    None,
    FileBrowser,
    GameDataDirs,
    BatchValidate,
    RustCodegen,
    YamlFile,
}

/// Color palette for [`ByteSpanAnnotation`] highlighting in the hex editor.
const SPAN_BG_COLORS: [&str; 8] = [
    "rgb(30, 88, 135)",
    "rgb(38, 110, 68)",
    "rgb(125, 78, 28)",
    "rgb(102, 52, 130)",
    "rgb(28, 112, 115)",
    "rgb(130, 48, 78)",
    "rgb(85, 105, 32)",
    "rgb(70, 75, 135)",
];

fn span_bg_color(span: &ByteSpanAnnotation) -> &'static str {
    if span.is_padding {
        "rgb(65, 65, 72)"
    } else {
        SPAN_BG_COLORS[span.color_index % SPAN_BG_COLORS.len()]
    }
}

/// Root Freya application component.
fn app() -> Element {
    let initial_persisted = use_hook(PersistedState::load);
    let initial_schema = use_hook(|| resolve_initial_schema(&initial_persisted));

    let mut persisted = use_signal(|| initial_persisted.clone());
    let mut schema = use_signal(|| initial_schema.clone());
    let mut raw_file_bytes = use_signal(Vec::<u8>::new);
    let mut dynamic_file = use_signal(|| None::<DynamicFile>);
    let mut status_msg = use_signal(|| "Ready.".to_owned());

    // Navigation & selection signals
    let mut selected_tree_path = use_signal(|| "0".to_owned());
    let mut collapsed_paths = use_signal(BTreeSet::<String>::new);
    let mut selected_byte_offset = use_signal(|| 0usize);
    let mut hex_page = use_signal(|| 0usize);
    let mut active_modal = use_signal(|| ActiveModal::None);

    // Modal state signals: File & .meg browser
    let mut disk_file_input = use_signal(|| {
        if let Some(FileSource::Disk { path }) = &initial_persisted.last_file {
            path.display().to_string()
        } else {
            String::new()
        }
    });
    let mut browser_ext_filter = use_signal(|| "alo".to_owned());
    let mut browser_search_query = use_signal(String::new);
    let mut discovered_files = use_signal(Vec::<DiscoveredFile>::new);

    // Modal state signals: GameData directories
    let mut new_gamedata_input = use_signal(String::new);

    // Modal state signals: Batch validation
    let mut validate_ext_input = use_signal(|| initial_persisted.validate_extension.clone());
    let mut validation_report = use_signal(|| None::<BatchValidationReport>);

    // Modal state signals: Rust codegen & YAML load/save
    let mut rust_save_path_input = use_signal(|| "generated_format.rs".to_owned());
    let mut yaml_file_path_input = use_signal(|| {
        if let ActiveSchemaSource::File { path } = &initial_persisted.schema_source {
            path.display().to_string()
        } else {
            "custom_schema.yaml".to_owned()
        }
    });
    let mut raw_yaml_editor_text =
        use_signal(|| initial_schema.to_yaml_string().unwrap_or_default());

    // Right-pane interactive schema form signals
    let mut edit_field_name = use_signal(String::new);
    let mut edit_cardinality = use_signal(|| Cardinality::Required);
    let mut edit_chunk_payload_kind = use_signal(|| "data".to_owned()); // "data", "mini_chunks", "children", "ignore"
    let mut edit_payload_type_str = use_signal(|| "u32".to_owned());
    let mut edit_wire_format = use_signal(|| None::<WireFormat>);

    // Right-pane ByteBlock field addition signals
    let mut new_bb_field_name = use_signal(|| "field_0".to_owned());
    let mut new_bb_field_type = use_signal(|| "u32".to_owned());
    let mut new_bb_field_pad = use_signal(|| "0".to_owned());

    // Load initial file once on startup if persistedState had `last_file`
    use_hook(|| {
        if let Some(src) = &initial_persisted.last_file {
            match load_file_bytes(src) {
                Ok(bytes) => {
                    let opts = LoadOptions {
                        force_plain_data_paths: initial_persisted.force_plain_data_paths.clone(),
                    };
                    match load_dynamic_file(&bytes, Some(&initial_schema), &opts) {
                        Ok(df) => {
                            status_msg.set(format!(
                                "Restored {} ({} bytes, {} diagnostics)",
                                src.display_label(),
                                bytes.len(),
                                df.diagnostics.len()
                            ));
                            raw_file_bytes.set(bytes);
                            dynamic_file.set(Some(df));
                        }
                        Err(e) => {
                            status_msg.set(format!("Failed to parse {}: {e}", src.display_label()));
                        }
                    }
                }
                Err(e) => {
                    status_msg.set(format!("Could not open previous file: {e}"));
                }
            }
        }
    });

    // Helper closure to re-evaluate the currently loaded file bytes against the current schema
    // and save PersistedState.
    let mut refresh_and_persist = move |new_schema: Schema, msg: String| {
        if let Ok(yaml) = new_schema.to_yaml_string() {
            raw_yaml_editor_text.set(yaml.clone());
            persisted.write().current_schema_yaml = Some(yaml);
        }
        persisted.read().save();

        let bytes = raw_file_bytes.read();
        if !bytes.is_empty() {
            let opts = LoadOptions {
                force_plain_data_paths: persisted.read().force_plain_data_paths.clone(),
            };
            if let Ok(df) = load_dynamic_file(&bytes, Some(&new_schema), &opts) {
                dynamic_file.set(Some(df));
            }
        }
        schema.set(new_schema);
        status_msg.set(msg);
    };

    // Helper to load a FileSource into the editor
    let mut open_file_source = move |src: FileSource| {
        match load_file_bytes(&src) {
            Ok(bytes) => {
                let cur_schema = schema.read().clone();
                let opts = LoadOptions {
                    force_plain_data_paths: persisted.read().force_plain_data_paths.clone(),
                };
                match load_dynamic_file(&bytes, Some(&cur_schema), &opts) {
                    Ok(df) => {
                        let diag_count = df.diagnostics.len();
                        let len = bytes.len();
                        raw_file_bytes.set(bytes);
                        dynamic_file.set(Some(df));
                        selected_tree_path.set("0".to_owned());
                        selected_byte_offset.set(0);
                        hex_page.set(0);
                        let mut p = persisted.write();
                        p.last_file = Some(src.clone());
                        p.save();
                        status_msg.set(format!(
                            "Loaded {} ({len} bytes, {diag_count} schema diagnostics)",
                            src.display_label()
                        ));
                    }
                    Err(e) => {
                        status_msg.set(format!("Error parsing chunked file: {e}"));
                    }
                }
            }
            Err(e) => {
                status_msg.set(format!("Error loading file: {e}"));
            }
        }
    };

    // Populate the right-pane form whenever a user selects a new node in the tree
    let mut populate_form_for_node = move |node: &SelectedNodeDetails, cur_schema: &Schema| {
        if node.is_mini_chunk {
            if let Some(existing) = find_existing_mini_chunk_field(
                cur_schema,
                &node.parent_container,
                node.id as u8,
            ) {
                edit_field_name.set(existing.name.clone());
                edit_cardinality.set(existing.cardinality.clone());
                edit_payload_type_str.set(existing.ty.label());
                edit_wire_format.set(existing.wire_format);
            } else {
                edit_field_name.set(
                    node.schema_field_name
                        .clone()
                        .unwrap_or_else(|| format!("mc_{:02x}", node.id)),
                );
                edit_cardinality.set(Cardinality::Required);
                let default_ty = match node.size {
                    1 => "u8",
                    2 => "u16",
                    4 => "u32",
                    8 => "vec2",
                    12 => "vec3",
                    16 => "vec4",
                    _ => "c_string",
                };
                edit_payload_type_str.set(default_ty.to_owned());
                edit_wire_format.set(None);
            }
        } else if let Some(existing) =
            find_existing_chunk_field(cur_schema, &node.parent_container, node.id)
        {
            edit_field_name.set(existing.name.clone());
            edit_cardinality.set(existing.cardinality.clone());
            edit_wire_format.set(existing.wire_format);
            if let ChunkFieldSource::Id { payload, .. } = &existing.source {
                match payload {
                    ChunkPayloadSpec::Data(dt) => {
                        edit_chunk_payload_kind.set("data".to_owned());
                        edit_payload_type_str.set(dt.label());
                    }
                    ChunkPayloadSpec::MiniChunks(tname) => {
                        edit_chunk_payload_kind.set("mini_chunks".to_owned());
                        edit_payload_type_str.set(tname.clone());
                    }
                    ChunkPayloadSpec::Children(tname) => {
                        edit_chunk_payload_kind.set("children".to_owned());
                        edit_payload_type_str.set(tname.clone());
                    }
                    ChunkPayloadSpec::Ignore => {
                        edit_chunk_payload_kind.set("ignore".to_owned());
                    }
                }
            }
        } else {
            let suggested_field = node
                .schema_field_name
                .clone()
                .unwrap_or_else(|| format!("chunk_{:04x}", node.id));
            edit_field_name.set(suggested_field.clone());
            edit_cardinality.set(Cardinality::Required);
            edit_wire_format.set(None);
            if node.is_container {
                edit_chunk_payload_kind.set("children".to_owned());
                edit_payload_type_str.set(format!("{}Container", to_pascal_case(&suggested_field)));
            } else if node.looks_like_mini_chunks {
                edit_chunk_payload_kind.set("mini_chunks".to_owned());
                edit_payload_type_str
                    .set(format!("{}MiniChunks", to_pascal_case(&suggested_field)));
            } else {
                edit_chunk_payload_kind.set("data".to_owned());
                edit_payload_type_str.set(format!("{}Block", to_pascal_case(&suggested_field)));
            }
        }
    };

    // Snapshot current state for rendering
    let cur_persisted = persisted.read().clone();
    let cur_schema = schema.read().clone();
    let cur_df = dynamic_file.read().clone();
    let cur_tree_path = selected_tree_path.read().clone();
    let cur_collapsed = collapsed_paths.read().clone();
    let cur_byte_offset = *selected_byte_offset.read();
    let cur_hex_page = *hex_page.read();
    let cur_modal = *active_modal.read();

    let selected_node = cur_df
        .as_ref()
        .and_then(|df| resolve_selected_node(df, &cur_schema, &cur_tree_path));

    let flat_tree = cur_df
        .as_ref()
        .map(|df| build_flat_tree(df, &cur_collapsed, 600))
        .unwrap_or_default();

    let active_file_label = cur_persisted
        .last_file
        .as_ref()
        .map(|f| f.display_label())
        .unwrap_or_else(|| "<no file loaded>".to_owned());

    rsx! {
        rect {
            width: "fill",
            height: "fill",
            background: "rgb(18, 19, 24)",
            color: "rgb(235, 237, 245)",
            direction: "vertical",

            // =================================================================================
            // 1. TOP TOOLBAR
            // =================================================================================
            rect {
                width: "fill",
                padding: "8 12",
                background: "rgb(28, 30, 38)",
                direction: "horizontal",
                cross_align: "center",
                spacing: "8",

                label {
                    font_size: "15",
                    font_weight: "bold",
                    color: "rgb(115, 195, 255)",
                    "Petro Schema Editor"
                }

                // File Browser button
                Button {
                    onpress: move |_| {
                        if *active_modal.read() == ActiveModal::FileBrowser {
                            active_modal.set(ActiveModal::None);
                        } else {
                            let dirs = persisted.read().game_data_dirs.clone();
                            let ext = browser_ext_filter.read().clone();
                            let q = browser_search_query.read().clone();
                            let ext_opt = if ext.is_empty() || ext == "all" { None } else { Some(ext.as_str()) };
                            discovered_files.set(discover_files(&dirs, ext_opt, &q, 150));
                            active_modal.set(ActiveModal::FileBrowser);
                        }
                    },
                    label { "Browse Files / .meg..." }
                }

                // Built-in Schema presets
                Button {
                    onpress: move |_| {
                        let s = known_formats::alo_model();
                        persisted.write().schema_source = ActiveSchemaSource::Builtin { name: "AloModel".to_owned() };
                        refresh_and_persist(s, "Switched to built-in AloModel schema".to_owned());
                    },
                    label { "Schema: AloModel" }
                }
                Button {
                    onpress: move |_| {
                        let s = known_formats::alo_particle();
                        persisted.write().schema_source = ActiveSchemaSource::Builtin { name: "AloParticle".to_owned() };
                        refresh_and_persist(s, "Switched to built-in AloParticle schema".to_owned());
                    },
                    label { "Schema: AloParticle" }
                }
                Button {
                    onpress: move |_| {
                        let s = known_formats::ala_animation();
                        persisted.write().schema_source = ActiveSchemaSource::Builtin { name: "AlaAnimation".to_owned() };
                        refresh_and_persist(s, "Switched to built-in AlaAnimation schema".to_owned());
                    },
                    label { "Schema: AlaAnimation" }
                }
                Button {
                    onpress: move |_| {
                        let s = Schema::new_empty("CustomFormat");
                        persisted.write().schema_source = ActiveSchemaSource::Custom;
                        refresh_and_persist(s, "Started a blank CustomFormat schema".to_owned());
                    },
                    label { "+ Blank Schema" }
                }

                // Load / Save / Raw YAML button
                Button {
                    onpress: move |_| {
                        if *active_modal.read() == ActiveModal::YamlFile {
                            active_modal.set(ActiveModal::None);
                        } else {
                            if let Ok(y) = schema.read().to_yaml_string() {
                                raw_yaml_editor_text.set(y);
                            }
                            active_modal.set(ActiveModal::YamlFile);
                        }
                    },
                    label { "YAML File / Source..." }
                }

                // GameData directories configuration button
                Button {
                    onpress: move |_| {
                        if *active_modal.read() == ActiveModal::GameDataDirs {
                            active_modal.set(ActiveModal::None);
                        } else {
                            active_modal.set(ActiveModal::GameDataDirs);
                        }
                    },
                    label { "GameData Dirs ({cur_persisted.game_data_dirs.len()})..." }
                }

                // Batch Validate button
                Button {
                    onpress: move |_| {
                        if *active_modal.read() == ActiveModal::BatchValidate {
                            active_modal.set(ActiveModal::None);
                        } else {
                            active_modal.set(ActiveModal::BatchValidate);
                        }
                    },
                    label { "Validate All..." }
                }

                // Emit Rust Code button
                Button {
                    onpress: move |_| {
                        if *active_modal.read() == ActiveModal::RustCodegen {
                            active_modal.set(ActiveModal::None);
                        } else {
                            active_modal.set(ActiveModal::RustCodegen);
                        }
                    },
                    label { "Emit Rust Code" }
                }
            }

            // =================================================================================
            // 2. STATUS BAR
            // =================================================================================
            rect {
                width: "fill",
                padding: "4 12",
                background: "rgb(23, 25, 32)",
                direction: "horizontal",
                spacing: "16",

                label {
                    font_size: "12",
                    color: "rgb(175, 220, 175)",
                    "File: {active_file_label}"
                }
                label {
                    font_size: "12",
                    color: "rgb(145, 200, 250)",
                    "Active Schema: {cur_schema.name} ({cur_schema.types.len()} types)"
                }
                label {
                    font_size: "12",
                    color: "rgb(215, 215, 225)",
                    "{status_msg.read()}"
                }
            }

            // =================================================================================
            // 3. COLLAPSIBLE MODAL / DRAWER PANELS
            // =================================================================================
            if cur_modal == ActiveModal::FileBrowser {
                rect {
                    width: "fill",
                    height: "280",
                    padding: "10 14",
                    background: "rgb(32, 35, 46)",
                    direction: "vertical",
                    spacing: "8",

                    rect {
                        direction: "horizontal",
                        spacing: "8",
                        cross_align: "center",
                        label { font_weight: "bold", font_size: "13", "Open File from Disk or Petroglyph .meg Archive" }
                        Input {
                            value: disk_file_input.read().clone(),
                            placeholder: "/path/to/file.alo or .ala",
                            onchange: move |txt| disk_file_input.set(txt),
                        }
                        Button {
                            onpress: move |_| {
                                let p = PathBuf::from(disk_file_input.read().trim());
                                if !p.as_os_str().is_empty() {
                                    open_file_source(FileSource::Disk { path: p });
                                }
                            },
                            label { "Open Disk Path" }
                        }
                        Button {
                            onpress: move |_| active_modal.set(ActiveModal::None),
                            label { "Close [X]" }
                        }
                    }

                    rect {
                        direction: "horizontal",
                        spacing: "8",
                        cross_align: "center",
                        label { font_size: "12", "Filter GameData & .meg Archives:" }
                        Input {
                            value: browser_search_query.read().clone(),
                            placeholder: "Search filename substring (e.g. at_at, particle,walk)...",
                            onchange: move |txt| browser_search_query.set(txt),
                        }
                        for ext_choice in ["alo", "ala", "ted", "all"] {
                            Button {
                                onpress: move |_| {
                                    browser_ext_filter.set(ext_choice.to_owned());
                                    let dirs = persisted.read().game_data_dirs.clone();
                                    let q = browser_search_query.read().clone();
                                    let ext_opt = if ext_choice == "all" { None } else { Some(ext_choice) };
                                    discovered_files.set(discover_files(&dirs, ext_opt, &q, 150));
                                },
                                label { ".{ext_choice}" }
                            }
                        }
                        Button {
                            onpress: move |_| {
                                let dirs = persisted.read().game_data_dirs.clone();
                                let ext = browser_ext_filter.read().clone();
                                let q = browser_search_query.read().clone();
                                let ext_opt = if ext.is_empty() || ext == "all" { None } else { Some(ext.as_str()) };
                                discovered_files.set(discover_files(&dirs, ext_opt, &q, 150));
                            },
                            label { "Search" }
                        }
                    }

                    ScrollView {
                        width: "fill",
                        height: "fill",
                        if discovered_files.read().is_empty() {
                            label {
                                font_size: "12",
                                color: "rgb(170, 175, 190)",
                                "No matching files found. Configure GameData directories under 'GameData Dirs...' to browse loose files and .meg archives."
                            }
                        }
                        for item in discovered_files.read().iter() {
                            {
                                let src = item.source.clone();
                                let disp = item.display_name.clone();
                                let sz = item.size;
                                rsx! {
                                    rect {
                                        width: "fill",
                                        padding: "3 6",
                                        direction: "horizontal",
                                        spacing: "10",
                                        cross_align: "center",
                                        Button {
                                            onpress: move |_| {
                                                open_file_source(src.clone());
                                                active_modal.set(ActiveModal::None);
                                            },
                                            label { "Load" }
                                        }
                                        label {
                                            font_size: "12",
                                            "{disp}  ({sz} bytes)"
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            if cur_modal == ActiveModal::GameDataDirs {
                rect {
                    width: "fill",
                    height: "240",
                    padding: "10 14",
                    background: "rgb(32, 35, 46)",
                    direction: "vertical",
                    spacing: "8",

                    rect {
                        direction: "horizontal",
                        spacing: "8",
                        cross_align: "center",
                        label {
                            font_weight: "bold",
                            font_size: "13",
                            "Configured GameData Directories (scans loose files and inside .meg archives)"
                        }
                        Input {
                            value: new_gamedata_input.read().clone(),
                            placeholder: "/path/to/GameData/Data",
                            onchange: move |txt| new_gamedata_input.set(txt),
                        }
                        Button {
                            onpress: move |_| {
                                let raw = new_gamedata_input.read().trim().to_owned();
                                if !raw.is_empty() {
                                    let pb = PathBuf::from(&raw);
                                    let mut p = persisted.write();
                                    if !p.game_data_dirs.contains(&pb) {
                                        p.game_data_dirs.push(pb);
                                        p.save();
                                    }
                                    new_gamedata_input.set(String::new());
                                }
                            },
                            label { "+ Add Directory" }
                        }
                        Button {
                            onpress: move |_| {
                                // Quick-add workspace EAW & FoC GameData paths if present
                                let defaults = [
                                    "/var/home/slopbot/Projects/eaw/Star Wars Empire at War/GameData/Data",
                                    "/var/home/slopbot/Projects/eaw/Star Wars Empire at War/corruption/Data",
                                ];
                                let mut p = persisted.write();
                                for d in defaults {
                                    let pb = PathBuf::from(d);
                                    if pb.exists() && !p.game_data_dirs.contains(&pb) {
                                        p.game_data_dirs.push(pb);
                                    }
                                }
                                p.save();
                            },
                            label { "+ Auto-Detect Local EAW/FoC Data" }
                        }
                        Button {
                            onpress: move |_| active_modal.set(ActiveModal::None),
                            label { "Close [X]" }
                        }
                    }

                    ScrollView {
                        width: "fill",
                        height: "fill",
                        for (idx, dir) in cur_persisted.game_data_dirs.iter().enumerate() {
                            {
                                let dir_str = dir.display().to_string();
                                rsx! {
                                    rect {
                                        padding: "4 6",
                                        direction: "horizontal",
                                        spacing: "10",
                                        cross_align: "center",
                                        Button {
                                            onpress: move |_| {
                                                let mut p = persisted.write();
                                                if idx < p.game_data_dirs.len() {
                                                    p.game_data_dirs.remove(idx);
                                                    p.save();
                                                }
                                            },
                                            label { "Remove" }
                                        }
                                        label { font_size: "12", "{dir_str}" }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            if cur_modal == ActiveModal::BatchValidate {
                rect {
                    width: "fill",
                    height: "300",
                    padding: "10 14",
                    background: "rgb(32, 35, 46)",
                    direction: "vertical",
                    spacing: "8",

                    rect {
                        direction: "horizontal",
                        spacing: "8",
                        cross_align: "center",
                        label {
                            font_weight: "bold",
                            font_size: "13",
                            "Batch Validate Schema `{cur_schema.name}` Across GameData Directories & .meg Archives"
                        }
                        label { font_size: "12", "Extension:" }
                        Input {
                            value: validate_ext_input.read().clone(),
                            placeholder: "alo",
                            onchange: move |txt| validate_ext_input.set(txt),
                        }
                        Button {
                            onpress: move |_| {
                                let ext = validate_ext_input.read().trim().to_owned();
                                let mut p = persisted.write();
                                p.validate_extension = ext.clone();
                                p.save();
                                let dirs = p.game_data_dirs.clone();
                                drop(p);
                                let cur_s = schema.read().clone();
                                let rep = run_batch_validation(&cur_s, &dirs, &ext);
                                status_msg.set(format!(
                                    "Batch validation on .{}: {}/{} passed ({} failed, {} skipped)",
                                    rep.extension, rep.passed_count, rep.total_checked, rep.failed_count, rep.skipped_count
                                ));
                                validation_report.set(Some(rep));
                            },
                            label { "Run Validate Now" }
                        }
                        Button {
                            onpress: move |_| active_modal.set(ActiveModal::None),
                            label { "Close [X]" }
                        }
                    }

                    if let Some(rep) = validation_report.read().as_ref() {
                        label {
                            font_size: "12",
                            color: if rep.failed_count == 0 { "rgb(130, 235, 150)" } else { "rgb(255, 165, 130)" },
                            "Schema `{rep.schema_name}` on `.{rep.extension}` -> Checked: {rep.total_checked} | Passed: {rep.passed_count} | Failed: {rep.failed_count} | Skipped: {rep.skipped_count}"
                        }
                        ScrollView {
                            width: "fill",
                            height: "fill",
                            for fail in &rep.failures {
                                {
                                    let f_src = fail.source.clone();
                                    let f_disp = fail.display_name.clone();
                                    let first_diag = fail
                                        .diagnostics
                                        .first()
                                        .map(|d| format!("[{} @ 0x{:X}] {}", d.tree_path, d.file_offset, d.message))
                                        .unwrap_or_default();
                                    let diag_len = fail.diagnostics.len();
                                    rsx! {
                                        rect {
                                            padding: "3 6",
                                            direction: "horizontal",
                                            spacing: "8",
                                            cross_align: "center",
                                            Button {
                                                onpress: move |_| {
                                                    open_file_source(f_src.clone());
                                                    active_modal.set(ActiveModal::None);
                                                },
                                                label { "Open in Editor" }
                                            }
                                            label {
                                                font_size: "12",
                                                color: "rgb(255, 180, 175)",
                                                "{f_disp} ({diag_len} error(s)) — {first_diag}"
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            if cur_modal == ActiveModal::RustCodegen {
                {
                    let generated_code = generate_rust_code(&cur_schema);
                    rsx! {
                        rect {
                            width: "fill",
                            height: "330",
                            padding: "10 14",
                            background: "rgb(32, 35, 46)",
                            direction: "vertical",
                            spacing: "8",

                            rect {
                                direction: "horizontal",
                                spacing: "8",
                                cross_align: "center",
                                label {
                                    font_weight: "bold",
                                    font_size: "13",
                                    "Generated Rust Code (`petro-chunked-derive`)"
                                }
                                Input {
                                    value: rust_save_path_input.read().clone(),
                                    placeholder: "output.rs",
                                    onchange: move |txt| rust_save_path_input.set(txt),
                                }
                                Button {
                                    onpress: move |_| {
                                        let path = PathBuf::from(rust_save_path_input.read().trim());
                                        let code = generate_rust_code(&schema.read());
                                        match std::fs::write(&path, code) {
                                            Ok(()) => status_msg.set(format!("Saved Rust code to {}", path.display())),
                                            Err(e) => status_msg.set(format!("Failed to write Rust code: {e}")),
                                        }
                                    },
                                    label { "Save .rs File" }
                                }
                                Button {
                                    onpress: move |_| active_modal.set(ActiveModal::None),
                                    label { "Close [X]" }
                                }
                            }

                            ScrollView {
                                width: "fill",
                                height: "fill",
                                rect {
                                    padding: "8",
                                    background: "rgb(18, 20, 26)",
                                    label {
                                        font_size: "12",
                                        font_family: "monospace",
                                        "{generated_code}"
                                    }
                                }
                            }
                        }
                    }
                }
            }

            if cur_modal == ActiveModal::YamlFile {
                rect {
                    width: "fill",
                    height: "300",
                    padding: "10 14",
                    background: "rgb(32, 35, 46)",
                    direction: "vertical",
                    spacing: "8",

                    rect {
                        direction: "horizontal",
                        spacing: "8",
                        cross_align: "center",
                        label { font_weight: "bold", font_size: "13", "Load / Save Schema YAML File" }
                        Input {
                            value: yaml_file_path_input.read().clone(),
                            placeholder: "schema.yaml",
                            onchange: move |txt| yaml_file_path_input.set(txt),
                        }
                        Button {
                            onpress: move |_| {
                                let p = PathBuf::from(yaml_file_path_input.read().trim());
                                match load_schema_from_path(&p) {
                                    Ok(loaded) => {
                                        persisted.write().schema_source = ActiveSchemaSource::File { path: p.clone() };
                                        refresh_and_persist(loaded, format!("Loaded schema from {}", p.display()));
                                    }
                                    Err(e) => status_msg.set(format!("Error loading YAML schema: {e}")),
                                }
                            },
                            label { "Load YAML File" }
                        }
                        Button {
                            onpress: move |_| {
                                let p = PathBuf::from(yaml_file_path_input.read().trim());
                                let cur_s = schema.read().clone();
                                match save_schema_to_path(&cur_s, &p) {
                                    Ok(()) => {
                                        persisted.write().schema_source = ActiveSchemaSource::File { path: p.clone() };
                                        persisted.read().save();
                                        status_msg.set(format!("Saved schema YAML to {}", p.display()));
                                    }
                                    Err(e) => status_msg.set(format!("Error saving YAML schema: {e}")),
                                }
                            },
                            label { "Save YAML File" }
                        }
                        Button {
                            onpress: move |_| active_modal.set(ActiveModal::None),
                            label { "Close [X]" }
                        }
                    }

                    ScrollView {
                        width: "fill",
                        height: "fill",
                        rect {
                            padding: "8",
                            background: "rgb(18, 20, 26)",
                            label {
                                font_size: "12",
                                font_family: "monospace",
                                "{raw_yaml_editor_text.read()}"
                            }
                        }
                    }
                }
            }

            // =================================================================================
            // 4. MAIN 3-PANE WORKSPACE
            // =================================================================================
            rect {
                width: "fill",
                height: "fill",
                direction: "horizontal",

                // -----------------------------------------------------------------------------
                // LEFT PANE: Chunk & Mini-Chunk Tree + Diagnostics
                // -----------------------------------------------------------------------------
                rect {
                    width: "340",
                    height: "fill",
                    padding: "8",
                    background: "rgb(22, 24, 31)",
                    direction: "vertical",
                    spacing: "6",

                    rect {
                        width: "fill",
                        direction: "horizontal",
                        spacing: "6",
                        cross_align: "center",
                        label {
                            font_size: "13",
                            font_weight: "bold",
                            color: "rgb(190, 205, 235)",
                            "Chunk & Mini-Chunk Tree ({flat_tree.len()})"
                        }
                        Button {
                            onpress: move |_| collapsed_paths.write().clear(),
                            label { "Expand All" }
                        }
                    }

                    ScrollView {
                        width: "fill",
                        height: "fill",
                        if flat_tree.is_empty() {
                            label {
                                font_size: "12",
                                color: "rgb(145, 150, 165)",
                                "No file loaded. Click 'Browse Files / .meg...' above to open an .alo or .ala file."
                            }
                        }
                        for row in &flat_tree {
                            {
                                let row_path = row.tree_path.clone();
                                let row_path_toggle = row.tree_path.clone();
                                let is_selected = row.tree_path == cur_tree_path;
                                let bg = if is_selected {
                                    "rgb(48, 72, 110)"
                                } else if row.has_error {
                                    "rgb(68, 32, 36)"
                                } else {
                                    "rgb(28, 31, 40)"
                                };
                                let indent_px = row.depth * 14;
                                let id_color = if row.is_mini_chunk {
                                    "rgb(235, 185, 105)"
                                } else {
                                    "rgb(120, 195, 255)"
                                };
                                let name_str = row
                                    .field_name
                                    .clone()
                                    .unwrap_or_else(|| "<unassigned>".to_owned());
                                let name_color = if row.field_name.is_some() {
                                    "rgb(140, 235, 165)"
                                } else {
                                    "rgb(140, 145, 160)"
                                };
                                let type_suffix = row
                                    .type_label
                                    .as_ref()
                                    .map(|t| format!(": {t}"))
                                    .unwrap_or_default();
                                let toggle_label = if row.child_count > 0 {
                                    if row.is_expanded { "[-]" } else { "[+]" }
                                } else {
                                    " · "
                                };
                                let sz = row.size;
                                let id_lbl = row.id_label.clone();

                                rsx! {
                                    rect {
                                        width: "fill",
                                        padding: "3 4",
                                        margin: "1 0",
                                        background: "{bg}",
                                        direction: "horizontal",
                                        spacing: "4",
                                        cross_align: "center",

                                        rect { width: "{indent_px}" }

                                        if row.child_count > 0 {
                                            Button {
                                                onpress: move |_| {
                                                    let mut c = collapsed_paths.write();
                                                    if !c.remove(&row_path_toggle) {
                                                        c.insert(row_path_toggle.clone());
                                                    }
                                                },
                                                label { font_size: "11", "{toggle_label}" }
                                            }
                                        } else {
                                            label { font_size: "11", color: "rgb(110, 115, 130)", "{toggle_label}" }
                                        }

                                        Button {
                                            onpress: move |_| {
                                                selected_tree_path.set(row_path.clone());
                                                selected_byte_offset.set(0);
                                                hex_page.set(0);
                                                if let Some(df) = dynamic_file.read().as_ref() {
                                                    let s = schema.read().clone();
                                                    if let Some(nd) = resolve_selected_node(df, &s, &row_path) {
                                                        populate_form_for_node(&nd, &s);
                                                    }
                                                }
                                            },
                                            label {
                                                font_size: "11",
                                                font_family: "monospace",
                                                color: "{id_color}",
                                                "{id_lbl} "
                                            }
                                            label {
                                                font_size: "11",
                                                color: "{name_color}",
                                                "{name_str}{type_suffix} "
                                            }
                                            label {
                                                font_size: "10",
                                                color: "rgb(155, 160, 175)",
                                                "({sz}B)"
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }

                    // Bottom of Left Pane: Schema Diagnostics for Active File
                    if let Some(df) = &cur_df {
                        rect {
                            width: "fill",
                            height: "155",
                            padding: "6",
                            background: "rgb(18, 20, 26)",
                            direction: "vertical",
                            spacing: "4",

                            label {
                                font_size: "12",
                                font_weight: "bold",
                                color: if df.diagnostics.is_empty() { "rgb(130, 230, 155)" } else { "rgb(255, 160, 145)" },
                                "File Schema Diagnostics ({df.diagnostics.len()})"
                            }
                            ScrollView {
                                width: "fill",
                                height: "fill",
                                if df.diagnostics.is_empty() {
                                    label {
                                        font_size: "11",
                                        color: "rgb(130, 210, 150)",
                                        "All chunks and bytes in this file conform cleanly to the active schema!"
                                    }
                                }
                                for diag in df.diagnostics.iter().take(60) {
                                    {
                                        let d_msg = diag.message.clone();
                                        let d_path = diag.tree_path.clone();
                                        let d_off = diag.file_offset;
                                        rsx! {
                                            label {
                                                font_size: "11",
                                                color: "rgb(255, 175, 165)",
                                                "• [{d_path} @ 0x{d_off:X}] {d_msg}"
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                // -----------------------------------------------------------------------------
                // CENTER PANE: Color-Coded Hex Editor & Multi-Format Byte Inspector
                // -----------------------------------------------------------------------------
                rect {
                    width: "fill",
                    height: "fill",
                    padding: "8 12",
                    background: "rgb(18, 19, 24)",
                    direction: "vertical",
                    spacing: "8",

                    if let Some(node) = &selected_node {
                        {
                            let node_path = node.tree_path.clone();
                            let is_forced_plain = cur_persisted.force_plain_data_paths.contains(&node_path);
                            let total_bytes = node.raw_bytes.len();
                            let assigned_bytes: usize = node
                                .spans
                                .iter()
                                .map(|s| s.end.saturating_sub(s.start).min(total_bytes))
                                .sum();
                            let page_size = 512usize;
                            let total_pages = if total_bytes == 0 { 1 } else { total_bytes.div_ceil(page_size) };
                            let safe_page = cur_hex_page.min(total_pages.saturating_sub(1));
                            let page_start = safe_page * page_size;
                            let page_end = (page_start + page_size).min(total_bytes);
                            let previews = inspect_bytes_at_offset(&node.raw_bytes, cur_byte_offset);
                            let node_for_inspector = node.clone();

                            rsx! {
                                // Selected Chunk/Mini-Chunk Header Bar
                                rect {
                                    width: "fill",
                                    padding: "8 10",
                                    background: "rgb(28, 31, 40)",
                                    direction: "horizontal",
                                    spacing: "12",
                                    cross_align: "center",

                                    label {
                                        font_size: "13",
                                        font_weight: "bold",
                                        color: "rgb(130, 205, 255)",
                                        if node.is_mini_chunk {
                                            "Mini-Chunk 0x{node.id:02X} ({node.id})"
                                        } else {
                                            "Chunk 0x{node.id:04X}"
                                        }
                                    }
                                    label {
                                        font_size: "12",
                                        color: "rgb(185, 190, 205)",
                                        "Path: {node.tree_path} | Header: 0x{node.header_offset:X} | Payload: 0x{node.payload_offset:X} | Size: {node.size} bytes"
                                    }
                                    if let Some(tlabel) = &node.schema_type_label {
                                        label {
                                            font_size: "12",
                                            color: "rgb(140, 230, 175)",
                                            "Type: {tlabel}"
                                        }
                                    }
                                    if let Some(dec) = &node.decoded {
                                        label {
                                            font_size: "12",
                                            color: "rgb(180, 225, 255)",
                                            "Value: {dec.short_summary()}"
                                        }
                                    }
                                    if !node.errors.is_empty() {
                                        label {
                                            font_size: "12",
                                            color: "rgb(255, 150, 145)",
                                            "Node Errors: {node.errors.join(\"; \")}"
                                        }
                                    }
                                    if total_bytes > 0 {
                                        label {
                                            font_size: "12",
                                            color: if assigned_bytes >= total_bytes { "rgb(135, 235, 160)" } else { "rgb(245, 200, 120)" },
                                            "Assigned: {assigned_bytes}/{total_bytes} bytes"
                                        }
                                    }

                                    // Force Plain Data vs Mini-Chunks Toggle button on leaf chunks
                                    if !node.is_container && !node.is_mini_chunk && (node.looks_like_mini_chunks || is_forced_plain) {
                                        Button {
                                            onpress: move |_| {
                                                let mut p = persisted.write();
                                                if !p.force_plain_data_paths.remove(&node_path) {
                                                    p.force_plain_data_paths.insert(node_path.clone());
                                                }
                                                let now_forced = p.force_plain_data_paths.contains(&node_path);
                                                drop(p);
                                                let cur_s = schema.read().clone();
                                                refresh_and_persist(
                                                    cur_s,
                                                    if now_forced {
                                                        format!("Forced chunk `{node_path}` to decode as plain byte data")
                                                    } else {
                                                        format!("Restored mini-chunk decoding on chunk `{node_path}`")
                                                    },
                                                );
                                            },
                                            label {
                                                if is_forced_plain {
                                                    "Mode: Forced Plain Data (Click to allow Mini-Chunks)"
                                                } else {
                                                    "Mode: Auto Mini-Chunks (Click to Force Plain Data)"
                                                }
                                            }
                                        }
                                    }
                                }

                                // Color-Coded Field Span Legend
                                if !node.spans.is_empty() {
                                    rect {
                                        width: "fill",
                                        height: "88",
                                        padding: "6 8",
                                        background: "rgb(24, 26, 34)",
                                        direction: "vertical",
                                        spacing: "4",

                                        label {
                                            font_size: "11",
                                            font_weight: "bold",
                                            color: "rgb(180, 195, 220)",
                                            "Assigned Structural Field Spans (click a span to jump hex cursor):"
                                        }
                                        ScrollView {
                                            width: "fill",
                                            height: "fill",
                                            rect {
                                                direction: "horizontal",
                                                spacing: "6",
                                                for span in &node.spans {
                                                    {
                                                        let bg = span_bg_color(span);
                                                        let sp_start = span.start;
                                                        let sp_text = format!(
                                                            "[0x{:02X}..0x{:02X}] {}: {} = {}",
                                                            span.start, span.end, span.field_path, span.type_label, span.value_summary
                                                        );
                                                        rsx! {
                                                            rect {
                                                                padding: "2 6",
                                                                background: "{bg}",
                                                                Button {
                                                                    onpress: move |_| {
                                                                        selected_byte_offset.set(sp_start);
                                                                        hex_page.set(sp_start / 512);
                                                                    },
                                                                    label {
                                                                        font_size: "11",
                                                                        font_family: "monospace",
                                                                        "{sp_text}"
                                                                    }
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }

                                // Hex View Pagination Bar (when > 512 bytes)
                                if total_pages > 1 {
                                    rect {
                                        direction: "horizontal",
                                        spacing: "8",
                                        cross_align: "center",
                                        label {
                                            font_size: "11",
                                            "Hex Page {safe_page + 1} / {total_pages} (bytes 0x{page_start:04X}..0x{page_end:04X})"
                                        }
                                        Button {
                                            onpress: move |_| {
                                                let cur = *hex_page.read();
                                                if cur > 0 {
                                                    hex_page.set(cur - 1);
                                                }
                                            },
                                            label { "< Prev 512B" }
                                        }
                                        Button {
                                            onpress: move |_| {
                                                let cur = *hex_page.read();
                                                if cur + 1 < total_pages {
                                                    hex_page.set(cur + 1);
                                                }
                                            },
                                            label { "Next 512B >" }
                                        }
                                    }
                                }

                                // Hex Grid Area
                                ScrollView {
                                    width: "fill",
                                    height: "fill",
                                    rect {
                                        width: "fill",
                                        padding: "6",
                                        background: "rgb(14, 15, 20)",
                                        direction: "vertical",
                                        spacing: "2",

                                        if node.is_container {
                                            label {
                                                font_size: "12",
                                                color: "rgb(160, 170, 190)",
                                                "This is a 32-bit container chunk holding child chunks. Select any leaf chunk or mini-chunk in the tree to inspect its bytes."
                                            }
                                        } else if node.raw_bytes.is_empty() {
                                            label {
                                                font_size: "12",
                                                color: "rgb(160, 170, 190)",
                                                "Empty payload (0 bytes)."
                                            }
                                        } else {
                                            for row_start in (page_start..page_end).step_by(16) {
                                                {
                                                    let row_end = (row_start + 16).min(page_end);
                                                    let ascii_str: String = (row_start..row_end)
                                                        .map(|idx| {
                                                            let b = node.raw_bytes[idx];
                                                            if b.is_ascii_graphic() || b == b' ' {
                                                                b as char
                                                            } else {
                                                                '.'
                                                            }
                                                        })
                                                        .collect();

                                                    rsx! {
                                                        rect {
                                                            direction: "horizontal",
                                                            spacing: "4",
                                                            cross_align: "center",

                                                            // Row offset label
                                                            label {
                                                                width: "54",
                                                                font_size: "12",
                                                                font_family: "monospace",
                                                                color: "rgb(130, 140, 165)",
                                                                "{row_start:04X}:"
                                                            }

                                                            // 16 clickable color-coded hex byte cells
                                                            for byte_idx in row_start..row_end {
                                                                {
                                                                    let b_val = node.raw_bytes[byte_idx];
                                                                    let covering_span = node
                                                                        .spans
                                                                        .iter()
                                                                        .find(|s| byte_idx >= s.start && byte_idx < s.end);
                                                                    let is_cursor = byte_idx == cur_byte_offset;
                                                                    let cell_bg = if is_cursor {
                                                                        "rgb(225, 165, 40)"
                                                                    } else if let Some(sp) = covering_span {
                                                                        span_bg_color(sp)
                                                                    } else {
                                                                        "rgb(28, 31, 40)"
                                                                    };
                                                                    let cell_fg = if is_cursor {
                                                                        "rgb(15, 15, 20)"
                                                                    } else if covering_span.is_some() {
                                                                        "rgb(245, 248, 255)"
                                                                    } else {
                                                                        "rgb(175, 180, 195)"
                                                                    };

                                                                    rsx! {
                                                                        rect {
                                                                            padding: "2 4",
                                                                            background: "{cell_bg}",
                                                                            onpointerdown: move |_| {
                                                                                selected_byte_offset.set(byte_idx);
                                                                                new_bb_field_name.set(format!("offset_{byte_idx:02x}"));
                                                                            },
                                                                            label {
                                                                                font_size: "12",
                                                                                font_family: "monospace",
                                                                                color: "{cell_fg}",
                                                                                "{b_val:02X}"
                                                                            }
                                                                        }
                                                                    }
                                                                }
                                                            }

                                                            // Pad missing cells on final partial row
                                                            for _ in row_end..(row_start + 16) {
                                                                rect {
                                                                    width: "24",
                                                                }
                                                            }

                                                            // ASCII column
                                                            label {
                                                                font_size: "12",
                                                                font_family: "monospace",
                                                                color: "rgb(165, 210, 180)",
                                                                "  |{ascii_str}|"
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }

                                // -----------------------------------------------------------------
                                // Bottom of Center Pane: Multi-Format Byte Inspector at Cursor
                                // -----------------------------------------------------------------
                                rect {
                                    width: "fill",
                                    height: "215",
                                    padding: "8 10",
                                    background: "rgb(24, 27, 35)",
                                    direction: "vertical",
                                    spacing: "6",

                                    rect {
                                        direction: "horizontal",
                                        spacing: "10",
                                        cross_align: "center",
                                        label {
                                            font_size: "12",
                                            font_weight: "bold",
                                            color: "rgb(250, 205, 110)",
                                            "Multi-Format Byte Inspector at Payload Offset 0x{cur_byte_offset:04X} ({cur_byte_offset})"
                                        }
                                        label {
                                            font_size: "11",
                                            color: "rgb(175, 180, 195)",
                                            "New Field Name:"
                                        }
                                        Input {
                                            value: new_bb_field_name.read().clone(),
                                            placeholder: "field_name",
                                            onchange: move |txt| new_bb_field_name.set(txt),
                                        }
                                    }

                                    ScrollView {
                                        width: "fill",
                                        height: "fill",
                                        if previews.is_empty() {
                                            label {
                                                font_size: "11",
                                                color: "rgb(145, 150, 165)",
                                                "Select a leaf chunk or mini-chunk with payload bytes to preview decoded values."
                                            }
                                        }
                                        for entry in &previews {
                                            {
                                                let lbl = entry.label;
                                                let blen = entry.byte_len;
                                                let val_str = entry.value.clone();
                                                let suggested = entry.suggested_type.clone();
                                                let node_clone = node_for_inspector.clone();

                                                rsx! {
                                                    rect {
                                                        padding: "2 4",
                                                        direction: "horizontal",
                                                        spacing: "10",
                                                        cross_align: "center",

                                                        if let Some((dt, wf)) = suggested {
                                                            Button {
                                                                onpress: move |_| {
                                                                    let mut s = schema.read().clone();
                                                                    let fname = new_bb_field_name.read().clone();
                                                                    match append_field_to_node_byte_block(
                                                                        &mut s,
                                                                        &node_clone,
                                                                        &fname,
                                                                        dt.clone(),
                                                                        wf,
                                                                        None,
                                                                    ) {
                                                                        Ok(bb_name) => {
                                                                            let next_off = cur_byte_offset + blen;
                                                                            selected_byte_offset.set(next_off);
                                                                            new_bb_field_name.set(format!("offset_{next_off:02x}"));
                                                                            refresh_and_persist(
                                                                                s,
                                                                                format!("Added `{fname}: {}` to ByteBlock `{bb_name}`", dt.label()),
                                                                            );
                                                                        }
                                                                        Err(e) => status_msg.set(format!("Could not add field: {e}")),
                                                                    }
                                                                },
                                                                label { font_size: "11", "+ Add to ByteBlock" }
                                                            }
                                                        }

                                                        label {
                                                            width: "145",
                                                            font_size: "12",
                                                            font_family: "monospace",
                                                            color: "rgb(135, 205, 255)",
                                                            "{lbl} ({blen}B)"
                                                        }
                                                        label {
                                                            font_size: "12",
                                                            font_family: "monospace",
                                                            color: "rgb(230, 235, 245)",
                                                            "= {val_str}"
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    } else {
                        label {
                            font_size: "13",
                            color: "rgb(155, 160, 175)",
                            "Select a chunk or mini-chunk from the left tree to inspect its bytes and edit its schema."
                        }
                    }
                }

                // -----------------------------------------------------------------------------
                // RIGHT PANE: Interactive Schema Editor
                // -----------------------------------------------------------------------------
                rect {
                    width: "390",
                    height: "fill",
                    padding: "8 10",
                    background: "rgb(23, 25, 33)",
                    direction: "vertical",
                    spacing: "10",

                    label {
                        font_size: "13",
                        font_weight: "bold",
                        color: "rgb(190, 210, 245)",
                        "Interactive Schema Definition"
                    }

                    ScrollView {
                        width: "fill",
                        height: "fill",
                        rect {
                            width: "fill",
                            direction: "vertical",
                            spacing: "10",

                            if let Some(node) = &selected_node {
                                {
                                    let parent_ref = node.parent_container.clone();
                                    let parent_ref_for_apply = node.parent_container.clone();
                                    let node_id = node.id;
                                    let is_mc = node.is_mini_chunk;

                                    // Inspect parent container mode & ignore_unknown
                                    let parent_summary = match &parent_ref {
                                        ParentContainerRef::Root => {
                                            Some((
                                                "root (Top-Level File Container)".to_owned(),
                                                cur_schema.root.mode,
                                                cur_schema.root.ignore_unknown,
                                            ))
                                        }
                                        ParentContainerRef::ChunksType { type_name } => {
                                            match cur_schema.types.get(type_name) {
                                                Some(TypeDef::Chunks(c) | TypeDef::ChunkStreamSeq(c)) => {
                                                    Some((format!("Chunks `{type_name}`"), c.mode, c.ignore_unknown))
                                                }
                                                _ => None,
                                            }
                                        }
                                        ParentContainerRef::MiniChunksType { type_name } => {
                                            match cur_schema.types.get(type_name) {
                                                Some(TypeDef::MiniChunks(c)) => {
                                                    Some((format!("MiniChunks `{type_name}`"), c.mode, c.ignore_unknown))
                                                }
                                                _ => None,
                                            }
                                        }
                                        ParentContainerRef::UnassignedParentChunk { .. } => {
                                            None
                                        }
                                    };

                                    rsx! {
                                        // 1. Parent Container Settings (Map vs Sequence, ignore_unknown)
                                        rect {
                                            width: "fill",
                                            padding: "8",
                                            background: "rgb(30, 33, 44)",
                                            direction: "vertical",
                                            spacing: "6",

                                            label {
                                                font_size: "12",
                                                font_weight: "bold",
                                                color: "rgb(145, 205, 255)",
                                                "1. Parent Container Relationship"
                                            }

                                            if let Some((p_title, p_mode, p_ignore)) = parent_summary {
                                                {
                                                    let p_ref_mode = parent_ref.clone();
                                                    let p_ref_ign = parent_ref.clone();
                                                    rsx! {
                                                        label { font_size: "11", "Container: {p_title}" }
                                                        rect {
                                                            direction: "horizontal",
                                                            spacing: "6",
                                                            Button {
                                                                onpress: move |_| {
                                                                    let mut s = schema.read().clone();
                                                                    let next_mode = if p_mode == OrderMode::Map {
                                                                        OrderMode::Sequence
                                                                    } else {
                                                                        OrderMode::Map
                                                                    };
                                                                    match &p_ref_mode {
                                                                        ParentContainerRef::Root => s.root.mode = next_mode,
                                                                        ParentContainerRef::ChunksType { type_name } => {
                                                                            if let Some(TypeDef::Chunks(c) | TypeDef::ChunkStreamSeq(c)) = s.types.get_mut(type_name) {
                                                                                c.mode = next_mode;
                                                                            }
                                                                        }
                                                                        ParentContainerRef::MiniChunksType { type_name } => {
                                                                            if let Some(TypeDef::MiniChunks(c)) = s.types.get_mut(type_name) {
                                                                                c.mode = next_mode;
                                                                            }
                                                                        }
                                                                        _ => {}
                                                                    }
                                                                    refresh_and_persist(s, format!("Set container ordering mode to {next_mode:?}"));
                                                                },
                                                                label {
                                                                    font_size: "11",
                                                                    "Order Mode: {p_mode:?} (Toggle Map/Seq)"
                                                                }
                                                            }
                                                            Button {
                                                                onpress: move |_| {
                                                                    let mut s = schema.read().clone();
                                                                    let next_ign = !p_ignore;
                                                                    match &p_ref_ign {
                                                                        ParentContainerRef::Root => s.root.ignore_unknown = next_ign,
                                                                        ParentContainerRef::ChunksType { type_name } => {
                                                                            if let Some(TypeDef::Chunks(c) | TypeDef::ChunkStreamSeq(c)) = s.types.get_mut(type_name) {
                                                                                c.ignore_unknown = next_ign;
                                                                            }
                                                                        }
                                                                        ParentContainerRef::MiniChunksType { type_name } => {
                                                                            if let Some(TypeDef::MiniChunks(c)) = s.types.get_mut(type_name) {
                                                                                c.ignore_unknown = next_ign;
                                                                            }
                                                                        }
                                                                        _ => {}
                                                                    }
                                                                    refresh_and_persist(s, format!("Set ignore_unknown = {next_ign}"));
                                                                },
                                                                label {
                                                                    font_size: "11",
                                                                    "Ignore Unknown: {p_ignore}"
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                            } else {
                                                label {
                                                    font_size: "11",
                                                    color: "rgb(255, 195, 130)",
                                                    "Parent chunk does not have a container type assigned yet. Select the parent chunk first and assign it a `Children` or `MiniChunks` container."
                                                }
                                            }
                                        }

                                        // 2. Selected Chunk / Mini-Chunk Field Assignment
                                        rect {
                                            width: "fill",
                                            padding: "8",
                                            background: "rgb(30, 33, 44)",
                                            direction: "vertical",
                                            spacing: "6",

                                            label {
                                                font_size: "12",
                                                font_weight: "bold",
                                                color: "rgb(140, 235, 170)",
                                                if is_mc {
                                                    "2. Assign Mini-Chunk 0x{node_id:02X} ({node_id})"
                                                } else {
                                                    "2. Assign Chunk 0x{node_id:04X}"
                                                }
                                            }

                                            label { font_size: "11", "Field Name:" }
                                            Input {
                                                value: edit_field_name.read().clone(),
                                                placeholder: "field_name",
                                                onchange: move |txt| edit_field_name.set(txt),
                                            }

                                            label {
                                                font_size: "11",
                                                "Cardinality (currently: {edit_cardinality.read():?}):"
                                            }
                                            rect {
                                                direction: "horizontal",
                                                spacing: "4",
                                                Button {
                                                    onpress: move |_| edit_cardinality.set(Cardinality::Required),
                                                    label { font_size: "11", "Required" }
                                                }
                                                Button {
                                                    onpress: move |_| edit_cardinality.set(Cardinality::Optional),
                                                    label { font_size: "11", "Optional" }
                                                }
                                                Button {
                                                    onpress: move |_| edit_cardinality.set(Cardinality::Repeated),
                                                    label { font_size: "11", "Repeated" }
                                                }
                                                Button {
                                                    onpress: move |_| edit_cardinality.set(Cardinality::Default),
                                                    label { font_size: "11", "Default" }
                                                }
                                            }

                                            if !is_mc {
                                                label {
                                                    font_size: "11",
                                                    "Chunk Payload Kind (currently: {edit_chunk_payload_kind.read()}):"
                                                }
                                                rect {
                                                    direction: "horizontal",
                                                    spacing: "4",
                                                    Button {
                                                        onpress: move |_| edit_chunk_payload_kind.set("data".to_owned()),
                                                        label { font_size: "11", "Data" }
                                                    }
                                                    Button {
                                                        onpress: move |_| {
                                                            edit_chunk_payload_kind.set("mini_chunks".to_owned());
                                                            let fname = edit_field_name.read().clone();
                                                            edit_payload_type_str.set(format!("{}MiniChunks", to_pascal_case(&fname)));
                                                        },
                                                        label { font_size: "11", "MiniChunks" }
                                                    }
                                                    Button {
                                                        onpress: move |_| {
                                                            edit_chunk_payload_kind.set("children".to_owned());
                                                            let fname = edit_field_name.read().clone();
                                                            edit_payload_type_str.set(format!("{}Children", to_pascal_case(&fname)));
                                                        },
                                                        label { font_size: "11", "Children" }
                                                    }
                                                    Button {
                                                        onpress: move |_| edit_chunk_payload_kind.set("ignore".to_owned()),
                                                        label { font_size: "11", "Ignore" }
                                                    }
                                                }
                                            }

                                            label {
                                                font_size: "11",
                                                "Type / Target Struct Name (e.g. u32, f32, c_string, vec3, Vec<u16>, MyBlock):"
                                            }
                                            Input {
                                                value: edit_payload_type_str.read().clone(),
                                                placeholder: "u32 or MyByteBlock",
                                                onchange: move |txt| edit_payload_type_str.set(txt),
                                            }

                                            // Quick type presets
                                            rect {
                                                direction: "horizontal",
                                                spacing: "4",
                                                for preset in ["u8", "u16", "u32", "f32", "c_string", "vec3"] {
                                                    Button {
                                                        onpress: move |_| {
                                                            edit_chunk_payload_kind.set("data".to_owned());
                                                            edit_payload_type_str.set(preset.to_owned());
                                                        },
                                                        label { font_size: "10", "{preset}" }
                                                    }
                                                }
                                            }

                                            label {
                                                font_size: "11",
                                                "Wire Format (currently: {edit_wire_format.read():?}):"
                                            }
                                            rect {
                                                direction: "horizontal",
                                                spacing: "4",
                                                Button {
                                                    onpress: move |_| edit_wire_format.set(None),
                                                    label { font_size: "10", "None" }
                                                }
                                                Button {
                                                    onpress: move |_| edit_wire_format.set(Some(WireFormat::U8Bool)),
                                                    label { font_size: "10", "U8Bool" }
                                                }
                                                Button {
                                                    onpress: move |_| edit_wire_format.set(Some(WireFormat::U32Bool)),
                                                    label { font_size: "10", "U32Bool" }
                                                }
                                                Button {
                                                    onpress: move |_| edit_wire_format.set(Some(WireFormat::U32Usize)),
                                                    label { font_size: "10", "U32Usize" }
                                                }
                                            }

                                            Button {
                                                onpress: move |_| {
                                                    let mut s = schema.read().clone();
                                                    let fname = edit_field_name.read().clone();
                                                    let card = edit_cardinality.read().clone();
                                                    let wf = *edit_wire_format.read();
                                                    let ty_str = edit_payload_type_str.read().clone();

                                                    let res = if is_mc {
                                                        match DataType::parse_str(&ty_str) {
                                                            Ok(dt) => upsert_mini_chunk_field(
                                                                &mut s,
                                                                &parent_ref_for_apply,
                                                                node_id as u8,
                                                                &fname,
                                                                card,
                                                                dt,
                                                                wf,
                                                            ),
                                                            Err(e) => Err(e),
                                                        }
                                                    } else {
                                                        let pkind = edit_chunk_payload_kind.read().clone();
                                                        let spec_res = match pkind.as_str() {
                                                            "mini_chunks" => Ok(ChunkPayloadSpec::MiniChunks(ty_str.trim().to_owned())),
                                                            "children" => Ok(ChunkPayloadSpec::Children(ty_str.trim().to_owned())),
                                                            "ignore" => Ok(ChunkPayloadSpec::Ignore),
                                                            _ => DataType::parse_str(&ty_str).map(ChunkPayloadSpec::Data),
                                                        };
                                                        match spec_res {
                                                            Ok(spec) => upsert_chunk_field(
                                                                &mut s,
                                                                &parent_ref_for_apply,
                                                                node_id,
                                                                &fname,
                                                                card,
                                                                spec,
                                                                wf,
                                                            ),
                                                            Err(e) => Err(e),
                                                        }
                                                    };

                                                    match res {
                                                        Ok(()) => refresh_and_persist(
                                                            s,
                                                            format!("Updated schema field `{fname}` for ID 0x{node_id:X}"),
                                                        ),
                                                        Err(e) => status_msg.set(format!("Schema update error: {e}")),
                                                    }
                                                },
                                                label { font_weight: "bold", "Save Field to Schema" }
                                            }
                                        }

                                        // 3. Active ByteBlock Struct Fields Editor
                                        if let Some(bb_name) = &node.active_byte_block_type {
                                            if let Some(TypeDef::ByteBlock(bb_def)) = cur_schema.types.get(bb_name) {
                                                {
                                                    let bb_name_clone = bb_name.clone();
                                                    let bb_name_add = bb_name.clone();
                                                    rsx! {
                                                        rect {
                                                            width: "fill",
                                                            padding: "8",
                                                            background: "rgb(30, 33, 44)",
                                                            direction: "vertical",
                                                            spacing: "6",

                                                            label {
                                                                font_size: "12",
                                                                font_weight: "bold",
                                                                color: "rgb(245, 200, 120)",
                                                                "3. ByteBlock Struct `{bb_name_clone}` ({bb_def.fields.len()} fields)"
                                                            }

                                                            for (f_idx, field) in bb_def.fields.iter().enumerate() {
                                                                {
                                                                    let f_summary = format!(
                                                                        "#{f_idx} {}: {}{}{}",
                                                                        field.name,
                                                                        field.ty.label(),
                                                                        field
                                                                            .wire_format
                                                                            .map(|w| format!(" ({})", w.rust_type_name()))
                                                                            .unwrap_or_default(),
                                                                        field
                                                                            .pad_after
                                                                            .map(|p| format!(" [+{p}B pad]"))
                                                                            .unwrap_or_default()
                                                                    );
                                                                    let bb_del = bb_name.clone();
                                                                    rsx! {
                                                                        rect {
                                                                            padding: "2 4",
                                                                            background: "rgb(22, 24, 32)",
                                                                            direction: "horizontal",
                                                                            spacing: "6",
                                                                            cross_align: "center",

                                                                            Button {
                                                                                onpress: move |_| {
                                                                                    let mut s = schema.read().clone();
                                                                                    if let Some(TypeDef::ByteBlock(b)) = s.types.get_mut(&bb_del)
                                                                                        && f_idx < b.fields.len()
                                                                                    {
                                                                                        let removed = b.fields.remove(f_idx);
                                                                                        refresh_and_persist(
                                                                                            s,
                                                                                            format!("Removed field `{}` from `{bb_del}`", removed.name),
                                                                                        );
                                                                                    }
                                                                                },
                                                                                label { font_size: "10", "Del" }
                                                                            }
                                                                            label {
                                                                                font_size: "11",
                                                                                font_family: "monospace",
                                                                                "{f_summary}"
                                                                            }
                                                                        }
                                                                    }
                                                                }
                                                            }

                                                            // Manual add field row
                                                            label { font_size: "11", "Append Field (name / type / pad_after):" }
                                                            Input {
                                                                value: new_bb_field_name.read().clone(),
                                                                placeholder: "field_name",
                                                                onchange: move |txt| new_bb_field_name.set(txt),
                                                            }
                                                            Input {
                                                                value: new_bb_field_type.read().clone(),
                                                                placeholder: "u32, f32, vec3, [u8; 4]...",
                                                                onchange: move |txt| new_bb_field_type.set(txt),
                                                            }
                                                            Input {
                                                                value: new_bb_field_pad.read().clone(),
                                                                placeholder: "0 (pad_after bytes)",
                                                                onchange: move |txt| new_bb_field_pad.set(txt),
                                                            }
                                                            Button {
                                                                onpress: move |_| {
                                                                    let mut s = schema.read().clone();
                                                                    let fname = new_bb_field_name.read().trim().to_owned();
                                                                    let fty_str = new_bb_field_type.read().trim().to_owned();
                                                                    let pad: Option<usize> = new_bb_field_pad.read().trim().parse().ok().filter(|&p| p > 0);
                                                                    match DataType::parse_str(&fty_str) {
                                                                        Ok(dt) => {
                                                                            let wf = dt.default_wire_format();
                                                                            if let Some(TypeDef::ByteBlock(b)) = s.types.get_mut(&bb_name_add) {
                                                                                b.fields.push(ByteFieldDef {
                                                                                    name: fname.clone(),
                                                                                    description: None,
                                                                                    ty: dt,
                                                                                    wire_format: wf,
                                                                                    pad_after: pad,
                                                                                });
                                                                                refresh_and_persist(
                                                                                    s,
                                                                                    format!("Appended field `{fname}` to `{bb_name_add}`"),
                                                                                );
                                                                            }
                                                                        }
                                                                        Err(e) => status_msg.set(format!("Invalid DataType `{fty_str}`: {e}")),
                                                                    }
                                                                },
                                                                label { "+ Append Field to {bb_name}" }
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
