//! Unsaved schema changes confirmation dialog (`Save & Continue` / `Discard` / `Cancel`).

use freya::prelude::*;

use crate::ui::context::EditorState;
use crate::ui::theme;
use crate::ui::widgets::{ActionButton, ButtonStyle};

use super::ModalDialog;

/// Centered safeguard dialog shown whenever an action would replace a schema that has unsaved
/// in-memory edits.
#[component]
pub fn ConfirmDiscardModal() -> Element {
    let mut state = use_context::<EditorState>();
    let schema_name = state.schema.read().name.clone();
    let pending_desc = state
        .pending_schema_action
        .read()
        .as_ref()
        .map(|a| a.describe())
        .unwrap_or_else(|| "switch schemas".to_owned());

    rsx! {
        ModalDialog {
            title: "Unsaved Schema Changes".to_owned(),
            width: "540".to_owned(),
            height: "215".to_owned(),

            rect {
                width: "fill",
                height: "flex(1)",
                padding: "8 10",
                corner_radius: "5",
                background: theme::BG_SUB_CARD,
                border: "1 solid rgb(165, 125, 55)",
                direction: "vertical",
                spacing: "6",

                label {
                    font_size: "13",
                    font_weight: "bold",
                    color: theme::ACCENT_AMBER,
                    "Schema `{schema_name}` has unsaved edits in memory."
                }
                label {
                    font_size: "12",
                    color: theme::TEXT_PRIMARY,
                    "Do you want to save your changes before you {pending_desc}?"
                }
                label {
                    font_size: "11",
                    color: theme::TEXT_MUTED,
                    "Choosing Discard will permanently lose your unsaved edits to the active schema."
                }
            }

            rect {
                width: "fill",
                height: "32",
                direction: "horizontal",
                main_align: "end",
                cross_align: "center",
                spacing: "8",

                ActionButton {
                    label: "Cancel".to_owned(),
                    style: ButtonStyle::Secondary,
                    onpress: move |_| state.cancel_pending_action(),
                }
                ActionButton {
                    label: "Discard Changes".to_owned(),
                    style: ButtonStyle::Danger,
                    onpress: move |_| state.discard_and_continue_pending(),
                }
                ActionButton {
                    label: "Save & Continue".to_owned(),
                    style: ButtonStyle::Primary,
                    onpress: move |_| state.save_and_continue_pending(),
                }
            }
        }
    }
}
