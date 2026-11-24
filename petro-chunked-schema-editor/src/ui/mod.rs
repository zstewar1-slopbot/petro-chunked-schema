//! Modular Freya 0.3 GUI tree for `petro-chunked-schema-editor`.

pub mod context;
pub mod modals;
pub mod theme;
pub mod toolbar;
pub mod widgets;
pub mod workspace;

use freya::prelude::*;

use context::{ActiveModal, use_init_editor_state};
use modals::ModalHost;
use toolbar::{StatusBar, Toolbar};
use workspace::Workspace;

/// Top-level Freya application component with global keyboard shortcuts (`Ctrl+S`,
/// `Ctrl+Shift+S`, `Escape`) and centered overlay modal hosting.
pub fn app() -> Element {
    use_init_theme(|| DARK_THEME);
    let mut state = use_init_editor_state();

    rsx! {
        rect {
            width: "fill",
            height: "fill",
            content: "flex",
            background: theme::BG_ROOT,
            color: theme::TEXT_PRIMARY,
            font_size: "12",
            direction: "vertical",
            onglobalkeydown: move |e: KeyboardEvent| {
                let ctrl = e.modifiers.contains(Modifiers::CONTROL);
                let shift = e.modifiers.contains(Modifiers::SHIFT);
                if ctrl {
                    if let Key::Character(ch) = &e.key
                        && ch.eq_ignore_ascii_case("s")
                    {
                        if shift {
                            state.pending_schema_action.set(None);
                            state.active_modal.set(ActiveModal::SaveSchemaAs);
                        } else {
                            state.save_schema();
                        }
                    }
                } else if e.key == Key::Escape
                    && *state.active_modal.read() != ActiveModal::None
                {
                    state.cancel_pending_action();
                }
            },

            Toolbar {}
            StatusBar {}
            Workspace {}
            ModalHost {}
        }
    }
}
