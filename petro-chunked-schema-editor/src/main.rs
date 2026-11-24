//! `petro-chunked-schema-editor`: Graphical schema editor, binary chunk inspector, and batch
//! validator for Petroglyph's chunked file formats, built with [`freya`] 0.5 and
//! [`petro_chunked_schema`].
//!
//! # Capabilities
//!
//! - **Interactive 3-Pane Editor**: browse the chunk/mini-chunk tree (left), inspect color-coded
//!   structural byte spans and cursor decodings in a hex grid (center), and explore/edit a
//!   non-indenting vertical schema stack with live in-memory updates and hover synchronization
//!   (right).
//! - **Freestanding Multi-Window Dialogs**: open separate native OS windows for browsing local
//!   folders and stepping inside `.meg` archives, configuring `GameData` directories, opening
//!   built-in or YAML schemas, saving schemas (`Ctrl+S` / `Ctrl+Shift+S`), previewing live YAML,
//!   running batch validation, and emitting `petro-chunked-derive` Rust code.
//! - **Session Persistence**: automatically restores the active file, schema reference, unsaved
//!   schema edits, `GameData` paths, and per-file `force_plain_data_paths` overrides.

#![warn(missing_docs)]

mod browser;
mod cli;
mod editor_ops;
mod gamedata;
mod schema_nav;
mod state;
mod ui;

use cli::CliOutcome;

fn main() -> anyhow::Result<()> {
    match cli::run_cli()? {
        CliOutcome::ExitSuccess => Ok(()),
        CliOutcome::ExitFailure => std::process::exit(1),
        CliOutcome::LaunchGui => {
            ui::launch_editor();
            Ok(())
        }
    }
}
