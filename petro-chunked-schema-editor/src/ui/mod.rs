//! Modular Freya 0.3 GUI tree for `petro-chunked-schema-editor`.

pub mod context;
pub mod modals;
pub mod theme;
pub mod toolbar;
pub mod workspace;

use freya::prelude::*;

use context::use_init_editor_state;
use modals::ModalHost;
use toolbar::{StatusBar, Toolbar};
use workspace::Workspace;

/// Top-level Freya application component.
pub fn app() -> Element {
    let _state = use_init_editor_state();

    rsx! {
        rect {
            width: "fill",
            height: "fill",
            background: theme::BG_ROOT,
            color: theme::TEXT_PRIMARY,
            direction: "vertical",

            Toolbar {}
            StatusBar {}
            ModalHost {}
            Workspace {}
        }
    }
}
