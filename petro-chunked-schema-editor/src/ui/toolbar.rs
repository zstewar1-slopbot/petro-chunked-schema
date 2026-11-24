//! Top toolbar and status bar components.

use freya::prelude::*;
use petro_chunked_schema::known_formats;

use super::context::{ActiveModal, EditorState};
use super::theme;

/// Top application toolbar with file browser trigger, built-in schema presets, and modal tools.
#[component]
pub fn Toolbar() -> Element {
    let mut state = use_context::<EditorState>();
    let dir_count = state.persisted.read().game_data_dirs.len();
    let active_schema_name = state.schema.read().name.clone();

    rsx! {
        rect {
            width: "fill",
            padding: "6 12",
            background: theme::BG_TOOLBAR,
            direction: "horizontal",
            cross_align: "center",
            spacing: "6",

            label {
                font_size: "14",
                font_weight: "bold",
                color: theme::ACCENT_BLUE,
                "Petro Schema Editor"
            }

            Button {
                onpress: move |_| state.toggle_modal(ActiveModal::FileBrowser),
                label { font_size: "12", "Files / .meg..." }
            }

            label {
                font_size: "11",
                color: theme::TEXT_MUTED,
                "Schemas:"
            }

            for &preset in known_formats::BUILTIN_NAMES {
                {
                    let is_active = active_schema_name == preset;
                    rsx! {
                        Button {
                            onpress: move |_| state.load_builtin(preset),
                            label {
                                font_size: "12",
                                font_weight: if is_active { "bold" } else { "normal" },
                                color: if is_active { theme::ACCENT_BLUE } else { theme::TEXT_PRIMARY },
                                "{preset}"
                            }
                        }
                    }
                }
            }

            Button {
                onpress: move |_| state.start_blank_schema(),
                label { font_size: "12", "+ Blank" }
            }

            Button {
                onpress: move |_| state.toggle_modal(ActiveModal::YamlFile),
                label { font_size: "12", "YAML..." }
            }

            Button {
                onpress: move |_| state.toggle_modal(ActiveModal::GameDataDirs),
                label { font_size: "12", "GameData ({dir_count})" }
            }

            Button {
                onpress: move |_| state.toggle_modal(ActiveModal::BatchValidate),
                label { font_size: "12", "Validate..." }
            }

            Button {
                onpress: move |_| state.toggle_modal(ActiveModal::RustCodegen),
                label { font_size: "12", "Emit Rust" }
            }
        }
    }
}

/// Single-line status bar displaying the open file, active schema, and latest operation result.
#[component]
pub fn StatusBar() -> Element {
    let state = use_context::<EditorState>();
    let persisted = state.persisted.read();
    let schema = state.schema.read();

    let active_file_label = persisted
        .last_file
        .as_ref()
        .map(|f| f.display_label())
        .unwrap_or_else(|| "<no file loaded>".to_owned());

    let schema_ref_label = persisted
        .schema_ref
        .as_ref()
        .map(|r| r.to_string())
        .unwrap_or_else(|| "unsaved".to_owned());

    rsx! {
        rect {
            width: "fill",
            padding: "4 12",
            background: theme::BG_STATUS,
            direction: "horizontal",
            spacing: "16",

            label {
                font_size: "12",
                color: theme::ACCENT_ASCII,
                "File: {active_file_label}"
            }
            label {
                font_size: "12",
                color: theme::ACCENT_BLUE,
                "Schema: {schema.name} ({schema_ref_label}, {schema.types.len()} types)"
            }
            label {
                font_size: "12",
                color: theme::TEXT_PRIMARY,
                "{state.status_msg.read()}"
            }
        }
    }
}
