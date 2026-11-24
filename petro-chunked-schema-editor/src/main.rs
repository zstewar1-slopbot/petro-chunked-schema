//! `petro-chunked-schema-editor`: Graphical schema editor, binary chunk inspector, and batch
//! validator for Petroglyph's chunked file formats, built with [`freya`] and
//! [`petro_chunked_schema`].
//!
//! # Capabilities
//!
//! - **Interactive 3-Pane Editor**: browse the chunk/mini-chunk tree (left), inspect color-coded
//!   structural byte spans and cursor decodings in a hex grid (center), and define/update schema
//!   containers, fields, cardinalities, wire formats, and `ByteBlock` structs (right).
//! - **Unified Schema References**: load built-in schemas (`builtin:Alo`, `builtin:AloModel`,
//!   `builtin:AloParticle`, `builtin:AlaAnimation`) or local YAML files (including `file:`-escaped
//!   paths) via `--schema` or the YAML modal.
//! - **In-Memory `.meg` & Batch Validation**: scan `GameData` directories and `.meg` archives
//!   without extraction, validate all files matching an extension (`--validate` CLI or GUI modal),
//!   and emit `petro-chunked-derive` Rust code (`--emit-rust` CLI or GUI modal).
//! - **Session Persistence**: automatically restores the active file, schema reference, unsaved
//!   schema edits, `GameData` paths, and per-file `force_plain_data_paths` overrides.

#![warn(missing_docs)]

mod browser;
mod cli;
mod editor_ops;
mod gamedata;
#[allow(dead_code)]
mod schema_nav;
mod state;
mod ui;

use cli::CliOutcome;
use freya::prelude::*;

fn main() -> anyhow::Result<()> {
    match cli::run_cli()? {
        CliOutcome::ExitSuccess => Ok(()),
        CliOutcome::ExitFailure => std::process::exit(1),
        CliOutcome::LaunchGui => {
            launch_cfg(
                ui::app,
                LaunchConfig::<()>::new()
                    .with_size(1480.0, 920.0)
                    .with_title("Petro Chunked Schema Editor"),
            );
            Ok(())
        }
    }
}
