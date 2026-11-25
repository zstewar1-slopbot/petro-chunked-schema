//! Top grouped command toolbar and session status bar built with Freya 0.5.

use freya::prelude::*;

use super::context::{EditorState, ModalKind};
use super::theme;
use super::widgets::{ButtonStyle, action_button, group_caption, toolbar_divider};

/// Grouped top command bar organizing file/archive actions, schema lifecycle (`Open`, `Save`,
/// `Save As`, `View YAML`), and validation/codegen tools.
#[derive(PartialEq)]
pub struct Toolbar;

impl Component for Toolbar {
    fn render(&self) -> impl IntoElement {
        let mut state = EditorState::current();
        let dir_count = state.persisted.read().game_data_dirs.len();
        let is_dirty = *state.is_schema_dirty.read();

        let btn_style = |kind: ModalKind| {
            if state.is_modal_open(kind) {
                ButtonStyle::Active
            } else {
                ButtonStyle::Secondary
            }
        };

        rect()
            .width(Size::fill())
            .padding((6.0, 12.0))
            .background(theme::BG_TOOLBAR)
            .border(theme::border_bottom(theme::BORDER_SUBTLE))
            .horizontal()
            .cross_align(Alignment::Center)
            .spacing(10.0)
            .child(
                label()
                    .font_size(14.0)
                    .font_weight(FontWeight::BOLD)
                    .color(theme::ACCENT_BLUE)
                    .max_lines(1)
                    .text("Petro Schema Editor"),
            )
            .child(toolbar_divider())
            .child(group_caption("FILE"))
            .child(action_button(
                "Open File / .meg...",
                btn_style(ModalKind::OpenFile),
                false,
                move || state.open_modal(ModalKind::OpenFile),
            ))
            .child(action_button(
                format!("GameData Dirs ({dir_count})..."),
                btn_style(ModalKind::GameDataDirs),
                false,
                move || state.open_modal(ModalKind::GameDataDirs),
            ))
            .child(toolbar_divider())
            .child(group_caption("SCHEMA"))
            .child(action_button(
                "Open Schema...",
                btn_style(ModalKind::OpenSchema),
                false,
                move || state.open_modal(ModalKind::OpenSchema),
            ))
            .child(action_button(
                if is_dirty { "Save*" } else { "Save" },
                if is_dirty {
                    ButtonStyle::Primary
                } else {
                    ButtonStyle::Secondary
                },
                false,
                move || state.save_schema(),
            ))
            .child(action_button(
                "Save As...",
                btn_style(ModalKind::SaveSchemaAs),
                false,
                move || {
                    state.pending_schema_action.set(None);
                    state.open_modal(ModalKind::SaveSchemaAs);
                },
            ))
            .child(action_button(
                "View YAML...",
                btn_style(ModalKind::YamlPreview),
                false,
                move || state.open_modal(ModalKind::YamlPreview),
            ))
            .child(toolbar_divider())
            .child(group_caption("TOOLS"))
            .child(action_button(
                "Find Strings...",
                btn_style(ModalKind::FindStrings),
                false,
                move || state.open_modal(ModalKind::FindStrings),
            ))
            .child(action_button(
                "Validate All...",
                btn_style(ModalKind::BatchValidate),
                false,
                move || state.open_modal(ModalKind::BatchValidate),
            ))
            .child(action_button(
                "Emit Rust...",
                btn_style(ModalKind::RustCodegen),
                false,
                move || state.open_modal(ModalKind::RustCodegen),
            ))
    }
}

/// Session status strip showing the active binary file, active schema (with unsaved `*`
/// indicator), live status feedback, and keyboard shortcut hints.
#[derive(PartialEq)]
pub struct StatusBar;

impl Component for StatusBar {
    fn render(&self) -> impl IntoElement {
        let state = EditorState::current();
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
        let status_text = state.status_msg.read().clone();

        rect()
            .width(Size::fill())
            .content(Content::Flex)
            .padding((5.0, 12.0))
            .background(theme::BG_STATUS)
            .border(theme::border_bottom(theme::BORDER_SUBTLE))
            .horizontal()
            .cross_align(Alignment::Center)
            .spacing(14.0)
            .child(
                label()
                    .font_size(12.0)
                    .color(theme::ACCENT_ASCII)
                    .max_lines(1)
                    .text(format!("File: {active_file_label}")),
            )
            .child(toolbar_divider())
            .child(
                label()
                    .font_size(12.0)
                    .font_weight(if is_dirty {
                        FontWeight::BOLD
                    } else {
                        FontWeight::NORMAL
                    })
                    .color(schema_color)
                    .max_lines(1)
                    .text(format!(
                        "Schema: {}{dirty_mark} ({schema_ref_label}, {} types)",
                        schema.name,
                        schema.types.len()
                    )),
            )
            .child(toolbar_divider())
            .child(
                rect()
                    .width(Size::flex(1.0))
                    .overflow(Overflow::Clip)
                    .child(
                        label()
                            .font_size(11.0)
                            .color(theme::TEXT_MUTED)
                            .max_lines(1)
                            .text(status_text),
                    ),
            )
            .child(
                label()
                    .font_size(10.0)
                    .color(theme::TEXT_DIM)
                    .max_lines(1)
                    .text("Ctrl+S Save · Ctrl+Shift+S Save As · Ctrl+F Find Strings"),
            )
    }
}
