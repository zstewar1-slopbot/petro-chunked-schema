//! Freestanding `petro-chunked-derive` Rust code generation window.

use std::path::PathBuf;

use freya::prelude::*;
use petro_chunked_schema::codegen::generate_rust_code;

use crate::ui::context::EditorState;
use crate::ui::theme;
use crate::ui::widgets::{ButtonStyle, action_button, styled_input};

/// Freestanding window displaying `prettyplease`-formatted `petro-chunked-derive` Rust code for
/// the active resolved schema (updating live as you edit) with one-click export to a `.rs` file.
#[derive(PartialEq)]
pub struct RustCodegenWindow;

impl Component for RustCodegenWindow {
    fn render(&self) -> impl IntoElement {
        let mut st_save = EditorState::current();
        let mut st_close = st_save;
        let save_path_input = use_state(|| "generated_format.rs".to_owned());

        let code_result = generate_rust_code(&st_save.resolved_schema.read());
        let (display_text, text_color) = match &code_result {
            Ok(src) => (src.clone(), theme::TEXT_PRIMARY),
            Err(err) => (format!("// Codegen error: {err}"), theme::ACCENT_RED),
        };

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
                    .child(
                        rect()
                            .horizontal()
                            .spacing(8.0)
                            .cross_align(Alignment::Center)
                            .child(
                                label()
                                    .font_size(12.0)
                                    .color(theme::TEXT_MUTED)
                                    .text("Export path:"),
                            )
                            .child(styled_input(Size::px(340.0), save_path_input, "output.rs"))
                            .child(action_button(
                                "Save .rs File",
                                ButtonStyle::Primary,
                                false,
                                move || {
                                    let path = PathBuf::from(save_path_input.read().trim());
                                    let res = {
                                        let r = st_save.resolved_schema.read();
                                        generate_rust_code(&r)
                                    };
                                    match res {
                                        Ok(code) => match std::fs::write(&path, code) {
                                            Ok(()) => st_save.set_status(format!(
                                                "Saved Rust code to {}",
                                                path.display()
                                            )),
                                            Err(e) => st_save.set_status(format!(
                                                "Failed to write Rust code: {e}"
                                            )),
                                        },
                                        Err(e) => {
                                            st_save.set_status(format!("Codegen error: {e}"))
                                        }
                                    }
                                },
                            )),
                    )
                    .child(action_button("Close", ButtonStyle::Secondary, false, move || {
                        st_close.close_current_modal_window();
                    })),
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
                                    .color(text_color)
                                    .text(display_text),
                            ),
                    ),
            )
    }
}
