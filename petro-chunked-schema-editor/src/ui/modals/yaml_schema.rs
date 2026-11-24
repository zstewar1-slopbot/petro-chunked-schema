//! Live YAML schema serialization preview modal.

use freya::prelude::*;

use crate::ui::context::{ActiveModal, EditorState};
use crate::ui::theme;
use crate::ui::widgets::{ActionButton, ButtonStyle};

use super::ModalDialog;

/// Centered dialog displaying the live YAML serialization of the active in-memory schema with
/// quick `Save` and `Save As...` actions.
#[component]
pub fn YamlPreviewModal() -> Element {
    let mut state = use_context::<EditorState>();
    let schema = state.schema.read().clone();
    let is_dirty = *state.is_schema_dirty.read();
    let schema_ref_label = state
        .persisted
        .read()
        .schema_ref
        .as_ref()
        .map(ToString::to_string)
        .unwrap_or_else(|| "unsaved".to_owned());
    let yaml_text = schema
        .to_yaml_string()
        .unwrap_or_else(|e| format!("# YAML serialization error: {e}"));

    rsx! {
        ModalDialog {
            title: format!("Schema YAML — {} ({schema_ref_label})", schema.name),
            width: "880".to_owned(),
            height: "590".to_owned(),

            rect {
                width: "fill",
                direction: "horizontal",
                main_align: "space-between",
                cross_align: "center",
                spacing: "8",

                label {
                    font_size: "12",
                    color: if is_dirty { theme::ACCENT_AMBER } else { theme::TEXT_MUTED },
                    if is_dirty {
                        "In-memory edits pending — press Ctrl+S or Save to write to disk."
                    } else {
                        "Live serialized YAML reflecting all in-memory schema definitions."
                    }
                }

                rect {
                    direction: "horizontal",
                    spacing: "8",
                    ActionButton {
                        label: if is_dirty { "Save*".to_owned() } else { "Save".to_owned() },
                        style: if is_dirty { ButtonStyle::Primary } else { ButtonStyle::Secondary },
                        onpress: move |_| state.save_schema(),
                    }
                    ActionButton {
                        label: "Save As...".to_owned(),
                        style: ButtonStyle::Secondary,
                        onpress: move |_| {
                            state.pending_schema_action.set(None);
                            state.active_modal.set(ActiveModal::SaveSchemaAs);
                        },
                    }
                }
            }

            ScrollView {
                width: "fill",
                height: "flex(1)",
                rect {
                    width: "fill",
                    padding: "12",
                    corner_radius: "5",
                    background: theme::BG_INSET,
                    border: "1 solid rgb(46, 51, 66)",
                    label {
                        font_size: "12",
                        font_family: "monospace",
                        color: theme::TEXT_PRIMARY,
                        "{yaml_text}"
                    }
                }
            }
        }
    }
}
