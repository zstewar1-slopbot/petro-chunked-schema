//! Top grouped command toolbar and built-in schema / status bar.

use freya::prelude::*;
use petro_chunked_schema::known_formats;

use super::context::{ActiveModal, EditorState};
use super::theme;
use super::widgets::{ActionButton, ButtonStyle, GroupCaption, SegmentedChip, ToolbarDivider};

/// Grouped top command bar organizing file/archive asset triggers, schema management, and tools.
#[component]
pub fn Toolbar() -> Element {
    let mut state = use_context::<EditorState>();
    let dir_count = state.persisted.read().game_data_dirs.len();
    let active_modal = *state.active_modal.read();

    rsx! {
        rect {
            width: "fill",
            padding: "6 12",
            background: theme::BG_TOOLBAR,
            border: "0 0 1 0 solid rgb(52, 57, 74)",
            direction: "horizontal",
            cross_align: "center",
            spacing: "10",

            label {
                font_size: "14",
                font_weight: "bold",
                color: theme::ACCENT_BLUE,
                max_lines: "1",
                "Petro Schema Editor"
            }

            ToolbarDivider {}
            GroupCaption { text: "ASSETS".to_owned() }

            ActionButton {
                label: "Open File / .meg...".to_owned(),
                style: if active_modal == ActiveModal::FileBrowser {
                    ButtonStyle::Active
                } else {
                    ButtonStyle::Secondary
                },
                onpress: move |_| state.toggle_modal(ActiveModal::FileBrowser),
            }

            ActionButton {
                label: format!("GameData ({dir_count})"),
                style: if active_modal == ActiveModal::GameDataDirs {
                    ButtonStyle::Active
                } else {
                    ButtonStyle::Secondary
                },
                onpress: move |_| state.toggle_modal(ActiveModal::GameDataDirs),
            }

            ToolbarDivider {}
            GroupCaption { text: "SCHEMA".to_owned() }

            ActionButton {
                label: "YAML / URI...".to_owned(),
                style: if active_modal == ActiveModal::YamlFile {
                    ButtonStyle::Active
                } else {
                    ButtonStyle::Secondary
                },
                onpress: move |_| state.toggle_modal(ActiveModal::YamlFile),
            }

            ActionButton {
                label: "+ New Blank".to_owned(),
                style: ButtonStyle::Secondary,
                onpress: move |_| state.start_blank_schema(),
            }

            ToolbarDivider {}
            GroupCaption { text: "TOOLS".to_owned() }

            ActionButton {
                label: "Validate All...".to_owned(),
                style: if active_modal == ActiveModal::BatchValidate {
                    ButtonStyle::Active
                } else {
                    ButtonStyle::Secondary
                },
                onpress: move |_| state.toggle_modal(ActiveModal::BatchValidate),
            }

            ActionButton {
                label: "Emit Rust...".to_owned(),
                style: if active_modal == ActiveModal::RustCodegen {
                    ButtonStyle::Active
                } else {
                    ButtonStyle::Secondary
                },
                onpress: move |_| state.toggle_modal(ActiveModal::RustCodegen),
            }
        }
    }
}

/// Second top strip combining the built-in schema radio selector with active session context.
#[component]
pub fn StatusBar() -> Element {
    let mut state = use_context::<EditorState>();
    let persisted = state.persisted.read();
    let schema = state.schema.read();
    let active_schema_name = schema.name.clone();

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
            content: "flex",
            padding: "5 12",
            background: theme::BG_STATUS,
            border: "0 0 1 0 solid rgb(52, 57, 74)",
            direction: "horizontal",
            cross_align: "center",
            spacing: "10",

            GroupCaption { text: "BUILT-IN".to_owned() }

            rect {
                direction: "horizontal",
                cross_align: "center",
                spacing: "4",
                for &preset in known_formats::BUILTIN_NAMES {
                    SegmentedChip {
                        key: "{preset}",
                        label: preset.to_owned(),
                        selected: active_schema_name == preset,
                        compact: Some(true),
                        onpress: move |_| state.load_builtin(preset),
                    }
                }
            }

            ToolbarDivider {}

            rect {
                width: "flex(1)",
                overflow: "clip",
                direction: "horizontal",
                cross_align: "center",
                spacing: "14",

                label {
                    font_size: "12",
                    color: theme::ACCENT_ASCII,
                    max_lines: "1",
                    "File: {active_file_label}"
                }
                label {
                    font_size: "12",
                    color: theme::ACCENT_BLUE,
                    max_lines: "1",
                    "Schema: {schema.name} ({schema_ref_label}, {schema.types.len()} types)"
                }
                label {
                    font_size: "11",
                    color: theme::TEXT_MUTED,
                    max_lines: "1",
                    "{state.status_msg.read()}"
                }
            }
        }
    }
}
