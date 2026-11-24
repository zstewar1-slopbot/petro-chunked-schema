//! Top grouped command toolbar and session status bar.

use freya::prelude::*;

use super::context::{ActiveModal, EditorState};
use super::theme;
use super::widgets::{ActionButton, ButtonStyle, GroupCaption, ToolbarDivider};

/// Grouped top command bar organizing file/archive actions, schema lifecycle (`Open`, `Save`,
/// `Save As`, `View YAML`), and validation/codegen tools.
#[component]
pub fn Toolbar() -> Element {
    let mut state = use_context::<EditorState>();
    let dir_count = state.persisted.read().game_data_dirs.len();
    let active_modal = *state.active_modal.read();
    let is_dirty = *state.is_schema_dirty.read();

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
            GroupCaption { text: "FILE".to_owned() }

            ActionButton {
                label: "Open File / .meg...".to_owned(),
                style: if active_modal == ActiveModal::OpenFile {
                    ButtonStyle::Active
                } else {
                    ButtonStyle::Secondary
                },
                onpress: move |_| state.toggle_modal(ActiveModal::OpenFile),
            }

            ActionButton {
                label: format!("GameData Dirs ({dir_count})..."),
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
                label: "Open Schema...".to_owned(),
                style: if active_modal == ActiveModal::OpenSchema {
                    ButtonStyle::Active
                } else {
                    ButtonStyle::Secondary
                },
                onpress: move |_| state.toggle_modal(ActiveModal::OpenSchema),
            }

            ActionButton {
                label: if is_dirty { "Save*".to_owned() } else { "Save".to_owned() },
                style: if is_dirty {
                    ButtonStyle::Primary
                } else {
                    ButtonStyle::Secondary
                },
                onpress: move |_| state.save_schema(),
            }

            ActionButton {
                label: "Save As...".to_owned(),
                style: if active_modal == ActiveModal::SaveSchemaAs {
                    ButtonStyle::Active
                } else {
                    ButtonStyle::Secondary
                },
                onpress: move |_| state.toggle_modal(ActiveModal::SaveSchemaAs),
            }

            ActionButton {
                label: "View YAML...".to_owned(),
                style: if active_modal == ActiveModal::YamlPreview {
                    ButtonStyle::Active
                } else {
                    ButtonStyle::Secondary
                },
                onpress: move |_| state.toggle_modal(ActiveModal::YamlPreview),
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

/// Session status strip showing the active binary file, active schema (with unsaved `*`
/// indicator), live status feedback, and keyboard shortcut hints.
#[component]
pub fn StatusBar() -> Element {
    let state = use_context::<EditorState>();
    let persisted = state.persisted.read();
    let schema = state.schema.read();
    let is_dirty = *state.is_schema_dirty.read();

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

    let dirty_mark = if is_dirty { " *" } else { "" };
    let schema_color = if is_dirty {
        theme::ACCENT_AMBER
    } else {
        theme::ACCENT_BLUE
    };

    rsx! {
        rect {
            width: "fill",
            content: "flex",
            padding: "5 12",
            background: theme::BG_STATUS,
            border: "0 0 1 0 solid rgb(52, 57, 74)",
            direction: "horizontal",
            cross_align: "center",
            spacing: "14",

            label {
                font_size: "12",
                color: theme::ACCENT_ASCII,
                max_lines: "1",
                "File: {active_file_label}"
            }

            ToolbarDivider {}

            label {
                font_size: "12",
                font_weight: if is_dirty { "bold" } else { "normal" },
                color: "{schema_color}",
                max_lines: "1",
                "Schema: {schema.name}{dirty_mark} ({schema_ref_label}, {schema.types.len()} types)"
            }

            ToolbarDivider {}

            rect {
                width: "flex(1)",
                overflow: "clip",
                label {
                    font_size: "11",
                    color: theme::TEXT_MUTED,
                    max_lines: "1",
                    "{state.status_msg.read()}"
                }
            }

            label {
                font_size: "10",
                color: theme::TEXT_DIM,
                max_lines: "1",
                "Ctrl+S Save · Ctrl+Shift+S Save As"
            }
        }
    }
}
