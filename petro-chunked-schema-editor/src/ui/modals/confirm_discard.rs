//! Freestanding unsaved schema changes confirmation window.

use freya::prelude::*;

use crate::ui::context::EditorState;
use crate::ui::theme;
use crate::ui::widgets::{ButtonStyle, action_button};

/// Freestanding safeguard window shown whenever an action would replace a schema that has unsaved
/// in-memory edits.
#[derive(PartialEq)]
pub struct ConfirmDiscardWindow;

impl Component for ConfirmDiscardWindow {
    fn render(&self) -> impl IntoElement {
        let mut st_cancel = EditorState::current();
        let mut st_discard = st_cancel;
        let mut st_save = st_cancel;
        let schema_name = st_cancel.schema.read().name.clone();
        let pending_desc = st_cancel
            .pending_schema_action
            .read()
            .as_ref()
            .map(|a| a.describe())
            .unwrap_or_else(|| "switch schemas".to_owned());

        rect()
            .width(Size::fill())
            .height(Size::fill())
            .content(Content::Flex)
            .vertical()
            .spacing(12.0)
            .child(
                rect()
                    .width(Size::fill())
                    .height(Size::flex(1.0))
                    .padding((10.0, 12.0))
                    .corner_radius(5.0)
                    .background(theme::BG_SUB_CARD)
                    .border(theme::border_all(theme::BORDER_AMBER))
                    .vertical()
                    .spacing(6.0)
                    .child(
                        label()
                            .font_size(13.0)
                            .font_weight(FontWeight::BOLD)
                            .color(theme::ACCENT_AMBER)
                            .text(format!("Schema `{schema_name}` has unsaved edits in memory.")),
                    )
                    .child(
                        label()
                            .font_size(12.0)
                            .color(theme::TEXT_PRIMARY)
                            .text(format!(
                                "Do you want to save your changes before you {pending_desc}?"
                            )),
                    )
                    .child(
                        label()
                            .font_size(11.0)
                            .color(theme::TEXT_MUTED)
                            .text("Choosing Discard will permanently lose your unsaved edits to the active schema."),
                    ),
            )
            .child(
                rect()
                    .width(Size::fill())
                    .height(Size::px(32.0))
                    .horizontal()
                    .main_align(Alignment::End)
                    .cross_align(Alignment::Center)
                    .spacing(8.0)
                    .child(action_button("Cancel", ButtonStyle::Secondary, false, move || {
                        st_cancel.cancel_pending_action();
                    }))
                    .child(action_button(
                        "Discard Changes",
                        ButtonStyle::Danger,
                        false,
                        move || st_discard.discard_and_continue_pending(),
                    ))
                    .child(action_button(
                        "Save & Continue",
                        ButtonStyle::Primary,
                        false,
                        move || st_save.save_and_continue_pending(),
                    )),
            )
    }
}
