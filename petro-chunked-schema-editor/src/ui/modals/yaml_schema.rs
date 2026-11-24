//! Freestanding live YAML schema serialization preview window.

use freya::prelude::*;

use crate::ui::context::{EditorState, ModalKind};
use crate::ui::theme;
use crate::ui::widgets::{ButtonStyle, action_button};

/// Freestanding window displaying the live YAML serialization of the active in-memory schema
/// (updating automatically as you edit in the main window) with quick `Save` and `Save As...`
/// buttons.
#[derive(PartialEq)]
pub struct YamlPreviewWindow;

impl Component for YamlPreviewWindow {
    fn render(&self) -> impl IntoElement {
        let mut st_save = EditorState::current();
        let mut st_save_as = st_save;
        let mut st_close = st_save;
        let schema = st_save.schema.read().clone();
        let is_dirty = *st_save.is_schema_dirty.read();
        let schema_ref_label = st_save
            .persisted
            .read()
            .schema_ref
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_else(|| "unsaved".to_owned());
        let yaml_text = schema
            .to_yaml_string()
            .unwrap_or_else(|e| format!("# YAML serialization error: {e}"));

        rect()
            .width(Size::fill())
            .height(Size::fill())
            .content(Content::Flex)
            .vertical()
            .spacing(10.0)
            .child(
                rect()
                    .width(Size::fill())
                    .horizontal()
                    .main_align(Alignment::SpaceBetween)
                    .cross_align(Alignment::Center)
                    .spacing(8.0)
                    .child(
                        label()
                            .font_size(12.0)
                            .color(if is_dirty {
                                theme::ACCENT_AMBER
                            } else {
                                theme::TEXT_MUTED
                            })
                            .text(if is_dirty {
                                format!(
                                    "{} ({schema_ref_label}) — unsaved edits pending (Ctrl+S saves)",
                                    schema.name
                                )
                            } else {
                                format!(
                                    "{} ({schema_ref_label}) — live YAML serialization",
                                    schema.name
                                )
                            }),
                    )
                    .child(
                        rect()
                            .horizontal()
                            .spacing(8.0)
                            .child(action_button(
                                if is_dirty { "Save*" } else { "Save" },
                                if is_dirty {
                                    ButtonStyle::Primary
                                } else {
                                    ButtonStyle::Secondary
                                },
                                false,
                                move || st_save.save_schema(),
                            ))
                            .child(action_button(
                                "Save As...",
                                ButtonStyle::Secondary,
                                false,
                                move || {
                                    st_save_as.pending_schema_action.set(None);
                                    st_save_as.open_modal(ModalKind::SaveSchemaAs);
                                },
                            ))
                            .child(action_button("Close", ButtonStyle::Secondary, false, move || {
                                st_close.close_current_modal_window();
                            })),
                    ),
            )
            .child(
                ScrollView::new()
                    .width(Size::fill())
                    .height(Size::flex(1.0))
                    .child(
                        rect()
                            .width(Size::fill())
                            .padding(12.0)
                            .corner_radius(5.0)
                            .background(theme::BG_INSET)
                            .border(theme::border_all(theme::BORDER_CARD))
                            .child(
                                label()
                                    .font_size(12.0)
                                    .font_family("monospace")
                                    .color(theme::TEXT_PRIMARY)
                                    .text(yaml_text),
                            ),
                    ),
            )
    }
}
