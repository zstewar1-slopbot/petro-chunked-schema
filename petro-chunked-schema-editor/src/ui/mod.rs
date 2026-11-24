//! Modular Freya 0.5 multi-window GUI for `petro-chunked-schema-editor`.

pub mod context;
pub mod modals;
pub mod theme;
pub mod toolbar;
pub mod widgets;
pub mod workspace;

use freya::prelude::*;

use context::{EditorState, ModalKind};
use toolbar::{StatusBar, Toolbar};
use workspace::Workspace;

/// Launches the multi-window Freya 0.5 editor application with shared global [`EditorState`].
pub fn launch_editor() {
    let mut state = EditorState::new_global();
    launch(
        LaunchConfig::new()
            .with_global_context(state)
            .with_exit_on_close(true)
            .with_window(
                WindowConfig::new(main_window_app)
                    .with_size(1480.0, 920.0)
                    .with_title("Petro Chunked Schema Editor")
                    .with_background(theme::BG_ROOT)
                    .with_on_close(move |_, _| {
                        state.close_all_modals();
                        CloseDecision::Close
                    }),
            ),
    );
}

fn main_window_app() -> impl IntoElement {
    use_init_theme(dark_theme);
    let mut state = EditorState::current();

    rect()
        .width(Size::fill())
        .height(Size::fill())
        .content(Content::Flex)
        .background(theme::BG_ROOT)
        .color(theme::TEXT_PRIMARY)
        .font_size(12.0)
        .vertical()
        .on_global_key_down(move |e: Event<KeyboardEventData>| {
            let ctrl = e.modifiers.contains(Modifiers::CONTROL);
            let shift = e.modifiers.contains(Modifiers::SHIFT);
            if ctrl
                && let Key::Character(ch) = &e.key
                && ch.eq_ignore_ascii_case("s")
            {
                if shift {
                    state.pending_schema_action.set(None);
                    state.open_modal(ModalKind::SaveSchemaAs);
                } else {
                    state.save_schema();
                }
            }
        })
        .child(Toolbar)
        .child(StatusBar)
        .child(Workspace)
}
