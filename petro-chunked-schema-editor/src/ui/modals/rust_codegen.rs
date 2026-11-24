//! `petro-chunked-derive` Rust code generation centered modal.

use std::path::PathBuf;

use freya::prelude::*;
use petro_chunked_schema::codegen::generate_rust_code;

use crate::ui::context::EditorState;
use crate::ui::theme;
use crate::ui::widgets::{ActionButton, ButtonStyle, StyledInput};

use super::ModalDialog;

/// Centered dialog displaying `prettyplease`-formatted `petro-chunked-derive` Rust code for the
/// active resolved schema with one-click export to a `.rs` file.
#[component]
pub fn RustCodegenModal() -> Element {
    let mut state = use_context::<EditorState>();
    let mut save_path_input = use_signal(|| "generated_format.rs".to_owned());

    let code_result = generate_rust_code(&state.resolved_schema.read());
    let (display_text, text_color) = match &code_result {
        Ok(src) => (src.clone(), theme::TEXT_PRIMARY),
        Err(err) => (format!("// Codegen error: {err}"), theme::ACCENT_RED),
    };

    rsx! {
        ModalDialog {
            title: "Generated Rust Code (`petro-chunked-derive`)".to_owned(),
            width: "940".to_owned(),
            height: "610".to_owned(),

            rect {
                direction: "horizontal",
                spacing: "8",
                cross_align: "center",
                label { font_size: "12", color: theme::TEXT_MUTED, "Export path:" }
                StyledInput {
                    width: Some("340".to_owned()),
                    value: save_path_input.read().clone(),
                    placeholder: Some("output.rs".to_owned()),
                    onchange: move |txt| save_path_input.set(txt),
                }
                ActionButton {
                    label: "Save .rs File".to_owned(),
                    style: ButtonStyle::Primary,
                    onpress: move |_| {
                        let path = PathBuf::from(save_path_input.read().trim());
                        let res = {
                            let r = state.resolved_schema.read();
                            generate_rust_code(&r)
                        };
                        match res {
                            Ok(code) => match std::fs::write(&path, code) {
                                Ok(()) => state.set_status(format!("Saved Rust code to {}", path.display())),
                                Err(e) => state.set_status(format!("Failed to write Rust code: {e}")),
                            },
                            Err(e) => state.set_status(format!("Codegen error: {e}")),
                        }
                    },
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
                        color: "{text_color}",
                        "{display_text}"
                    }
                }
            }
        }
    }
}
