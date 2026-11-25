//! Command-line argument parsing and headless execution modes (`--validate`, `--emit-rust`,
//! and `--find-strings`).

use std::path::PathBuf;

use clap::Parser;
use petro_chunked_schema::codegen::generate_rust_code;
use petro_chunked_schema::dynamic::{FindStringsOptions, LoadOptions, find_strings, load_dynamic_file};
use petro_chunked_schema::resolve::ResolvedSchema;
use petro_chunked_schema::source::SchemaRef;

use crate::gamedata::{load_file_bytes, load_schema_from_ref, run_batch_validation};
use crate::state::{FileSource, PersistedState};

/// Command-line arguments for `petro-chunked-schema-editor`.
#[derive(Debug, Parser)]
#[command(
    name = "petro-chunked-schema-editor",
    about = "Freya-based visual schema editor and hex inspector for Petroglyph chunked binary files"
)]
pub struct CliArgs {
    /// Open a specific chunked binary file on launch.
    #[arg(long)]
    pub file: Option<PathBuf>,

    /// Load a schema by URI (`builtin:Alo`, `builtin:AloModel`, `builtin:AloParticle`,
    /// `builtin:AlaAnimation`) or filesystem path (`./custom.yaml`, `file:builtin:foo`).
    #[arg(long)]
    pub schema: Option<SchemaRef>,

    /// Add one or more `GameData` directories for browsing `.meg` archives and batch validation.
    #[arg(long = "game-data")]
    pub game_data: Vec<PathBuf>,

    /// Run headless batch validation over all files with this extension (e.g. `alo` or `ala`)
    /// across configured `GameData` directories and exit.
    #[arg(long)]
    pub validate: Option<String>,

    /// Print the generated `petro-chunked-derive` Rust code for the active schema to stdout and exit.
    #[arg(long)]
    pub emit_rust: bool,

    /// Scan `--file` (or the previously opened file) for ASCII and NUL-terminated ASCIIZ strings,
    /// print matches to stdout, and exit.
    #[arg(long)]
    pub find_strings: bool,

    /// Include embedded ASCII/ASCIIZ runs inside larger non-ASCII payloads when running
    /// `--find-strings`.
    #[arg(long)]
    pub include_embedded: bool,

    /// Override the minimum character length used by `--find-strings`.
    #[arg(long)]
    pub min_string_len: Option<usize>,
}

/// Result of processing CLI flags before launching the GUI.
pub enum CliOutcome {
    /// Handled a headless command and succeeded.
    ExitSuccess,
    /// Handled a headless command and detected validation failures.
    ExitFailure,
    /// No headless exit requested; proceed to launch the GUI.
    LaunchGui,
}

/// Parses CLI flags, updates persisted session state, and executes any headless sub-mode.
pub fn run_cli() -> anyhow::Result<CliOutcome> {
    let args = CliArgs::parse();
    let mut persisted = PersistedState::load();

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
    if let Some(schema_ref) = &args.schema {
        let (loaded_schema, _) = load_schema_from_ref(schema_ref)?;
        persisted.schema_ref = Some(schema_ref.clone());
        persisted.current_schema_yaml = loaded_schema.to_yaml_string().ok();
    }
    persisted.save();

    let (_active_schema, resolved) = persisted.resolve_initial_schema();

    if args.emit_rust {
        run_emit_rust(&resolved)?;
        return Ok(CliOutcome::ExitSuccess);
    }

    if args.find_strings {
        let Some(src) = persisted.last_file.as_ref() else {
            anyhow::bail!("`--find-strings` requires `--file <path>` (or a previously opened file)");
        };
        run_headless_find_strings(
            src,
            &persisted,
            &resolved,
            args.include_embedded,
            args.min_string_len,
        )?;
        return Ok(CliOutcome::ExitSuccess);
    }

    if let Some(ext) = &args.validate {
        let failed = run_headless_validate(&resolved, &persisted.game_data_dirs, ext);
        return Ok(if failed {
            CliOutcome::ExitFailure
        } else {
            CliOutcome::ExitSuccess
        });
    }

    Ok(CliOutcome::LaunchGui)
}

fn run_emit_rust(resolved: &ResolvedSchema) -> anyhow::Result<()> {
    let code = generate_rust_code(resolved)?;
    println!("{code}");
    Ok(())
}

fn run_headless_find_strings(
    src: &FileSource,
    persisted: &PersistedState,
    resolved: &ResolvedSchema,
    include_embedded: bool,
    min_string_len: Option<usize>,
) -> anyhow::Result<()> {
    let bytes = load_file_bytes(src)?;
    let load_opts = LoadOptions::for_ui(persisted.forced_plain_paths_for(Some(src)));
    let df = load_dynamic_file(&bytes, Some(resolved), &load_opts)?;

    let mut opts = FindStringsOptions {
        include_embedded,
        ..FindStringsOptions::default()
    };
    if let Some(min_len) = min_string_len {
        opts.min_whole_len = min_len;
        opts.min_embedded_len = min_len;
    }

    let hits = find_strings(&df, &opts);
    println!(
        "Found {} string(s) in `{}` ({}):",
        hits.len(),
        src.display_label(),
        if include_embedded {
            "whole blocks + embedded"
        } else {
            "whole blocks only"
        }
    );
    for hit in hits {
        let kind = hit.kind.short_label();
        let node = if hit.is_mini_chunk {
            format!("mc {:02X}", hit.node_id)
        } else {
            format!("0x{:04X}", hit.node_id)
        };
        let field = hit.schema_field_name.as_deref().unwrap_or("<unassigned>");
        let escaped: String = hit
            .text
            .chars()
            .flat_map(|c| match c {
                '\t' => "\\t".chars().collect::<Vec<_>>(),
                '\r' => "\\r".chars().collect(),
                '\n' => "\\n".chars().collect(),
                '\\' => "\\\\".chars().collect(),
                '"' => "\\\"".chars().collect(),
                other => vec![other],
            })
            .collect();
        println!(
            "{kind:<8} {node:<8} {:<16} 0x{:06X} (+0x{:X}, {}B) {field:<18} \"{escaped}\"",
            hit.tree_path, hit.file_offset, hit.payload_offset, hit.byte_len
        );
    }
    Ok(())
}

fn run_headless_validate(
    resolved: &ResolvedSchema,
    game_data_dirs: &[PathBuf],
    ext: &str,
) -> bool {
    let report = run_batch_validation(resolved, game_data_dirs, ext);
    println!(
        "Batch Validation for schema `{}` on `.{}` across {} GameData dir(s):",
        report.schema_name,
        report.extension,
        game_data_dirs.len()
    );
    println!(
        "  Checked: {} | Passed: {} | Failed: {} | Skipped: {}",
        report.total_checked, report.passed_count, report.failed_count, report.skipped_count
    );
    for fail in &report.failures {
        println!("  FAIL: {}", fail.display_name);
        for d in fail.diagnostics.iter().take(5) {
            println!("    - [{} @ 0x{:X}] {}", d.tree_path, d.file_offset, d.message);
        }
    }
    report.failed_count > 0
}
