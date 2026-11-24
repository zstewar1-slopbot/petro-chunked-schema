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

    rsx! {
        rect {
            width: "fill",
            padding: "8 12",
            background: theme::BG_TOOLBAR,
            direction: "horizontal",
            cross_align: "center",
            spacing: "8",

            label {
                font_size: "15",
                font_weight: "bold",
                color: theme::ACCENT_BLUE,
                "Petro Schema Editor"
            }

            Button {
                onpress: move |_| state.toggle_modal(ActiveModal::FileBrowser),
                label { "Browse Files / .meg..." }
            }

            for &preset in known_formats::BUILTIN_NAMES {
                Button {
                    onpress: move |_| state.load_builtin(preset),
                    label { "Schema: {preset}" }
                }
            }

            Button {
                onpress: move |_| state.start_blank_schema(),
                label { "+ Blank Schema" }
            }

            Button {
                onpress: move |_| state.toggle_modal(ActiveModal::YamlFile),
                label { "YAML / Schema..." }
            }

            Button {
                onpress: move |_| state.toggle_modal(ActiveModal::GameDataDirs),
                label { "GameData Dirs ({dir_count})..." }
            }

            Button {
                onpress: move |_| state.toggle_modal(ActiveModal::BatchValidate),
                label { "Validate All..." }
            }

            Button {
                onpress: move |_| state.toggle_modal(ActiveModal::RustCodegen),
                label { "Emit Rust Code" }
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
