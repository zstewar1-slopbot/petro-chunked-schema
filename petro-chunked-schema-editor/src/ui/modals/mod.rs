//! Top drawer modals for browsing `.meg`/disk files, configuring `GameData` paths, running
//! batch validation, emitting `petro-chunked-derive` Rust code, and loading/saving YAML schemas.

mod batch_validate;
mod file_browser;
mod gamedata_dirs;
mod rust_codegen;
mod yaml_schema;

use freya::prelude::*;

use super::context::{ActiveModal, EditorState};
use super::theme;

use batch_validate::BatchValidateModal;
use file_browser::FileBrowserModal;
use gamedata_dirs::GameDataDirsModal;
use rust_codegen::RustCodegenModal;
use yaml_schema::YamlSchemaModal;

/// Routes the active drawer modal selected in [`EditorState::active_modal`].
#[component]
pub fn ModalHost() -> Element {
    let state = use_context::<EditorState>();
    match *state.active_modal.read() {
        ActiveModal::None => rsx! {},
        ActiveModal::FileBrowser => rsx! { FileBrowserModal {} },
        ActiveModal::GameDataDirs => rsx! { GameDataDirsModal {} },
        ActiveModal::BatchValidate => rsx! { BatchValidateModal {} },
        ActiveModal::RustCodegen => rsx! { RustCodegenModal {} },
        ActiveModal::YamlFile => rsx! { YamlSchemaModal {} },
    }
}

/// Consistent framed container for collapsible drawer modals with a title bar and close button.
#[component]
pub fn ModalFrame(title: String, height: String, children: Element) -> Element {
    let mut state = use_context::<EditorState>();

    rsx! {
        rect {
            width: "fill",
            height: "{height}",
            content: "flex",
            padding: "10 14",
            background: theme::BG_MODAL,
            direction: "vertical",
            spacing: "8",

            rect {
                width: "fill",
                direction: "horizontal",
                main_align: "space-between",
                cross_align: "center",
                label {
                    font_weight: "bold",
                    font_size: "13",
                    color: theme::ACCENT_BLUE,
                    "{title}"
                }
                Button {
                    onpress: move |_| state.close_modal(),
                    label { "Close [X]" }
                }
            }

            {children}
        }
    }
}
