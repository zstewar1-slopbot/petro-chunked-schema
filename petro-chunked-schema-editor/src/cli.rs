//! Command-line argument parsing and headless execution modes (`--validate`, `--emit-rust`).

use std::path::PathBuf;

use clap::Parser;
use petro_chunked_schema::codegen::generate_rust_code;
use petro_chunked_schema::resolve::ResolvedSchema;
use petro_chunked_schema::source::SchemaRef;

use crate::gamedata::{load_schema_from_ref, run_batch_validation};
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
