//! Freestanding Freya 0.5 modal windows for browsing disk & `.meg` files, configuring `GameData`
//! directories, opening/saving YAML & built-in schemas with unsaved-changes safeguards,
//! previewing live YAML, running batch validation, and emitting `petro-chunked-derive` Rust code.

mod batch_validate;
mod confirm_discard;
mod file_browser;
mod gamedata_dirs;
mod open_schema;
mod rust_codegen;
mod save_schema_as;
mod yaml_schema;

use freya::prelude::*;

use crate::browser::{BrowseLocation, QuickPlace, breadcrumbs, parent_location};

use super::context::{EditorState, ModalKind};
use super::theme;
use super::widgets::{ButtonStyle, action_button, group_caption};

use batch_validate::BatchValidateWindow;
use confirm_discard::ConfirmDiscardWindow;
use file_browser::FileBrowserWindow;
use gamedata_dirs::GameDataDirsWindow;
use open_schema::OpenSchemaWindow;
use rust_codegen::RustCodegenWindow;
use save_schema_as::SaveSchemaAsWindow;
use yaml_schema::YamlPreviewWindow;

/// Root component mounted inside every freestanding modal OS window spawned by
/// [`EditorState::open_modal`].
pub fn modal_window_root(kind: ModalKind) -> impl IntoElement {
    use_init_theme(dark_theme);
    let mut state = EditorState::current();

    let body: Element = match kind {
        ModalKind::OpenFile => FileBrowserWindow.into(),
        ModalKind::GameDataDirs => GameDataDirsWindow.into(),
        ModalKind::OpenSchema => OpenSchemaWindow.into(),
        ModalKind::SaveSchemaAs => SaveSchemaAsWindow.into(),
        ModalKind::ConfirmDiscard => ConfirmDiscardWindow.into(),
        ModalKind::YamlPreview => YamlPreviewWindow.into(),
        ModalKind::BatchValidate => BatchValidateWindow.into(),
        ModalKind::RustCodegen => RustCodegenWindow.into(),
    };

    rect()
        .width(Size::fill())
        .height(Size::fill())
        .content(Content::Flex)
        .padding((12.0, 14.0))
        .background(theme::BG_MODAL)
        .color(theme::TEXT_PRIMARY)
        .font_size(12.0)
        .vertical()
        .spacing(10.0)
        .on_global_key_down(move |e: Event<KeyboardEventData>| {
            if e.key == Key::Named(NamedKey::Escape) {
                if kind == ModalKind::ConfirmDiscard || kind == ModalKind::SaveSchemaAs {
                    state.cancel_pending_action();
                } else {
                    state.close_current_modal_window();
                }
            }
        })
        .child(body)
}

/// Shared navigation bar (`Up` button + clickable breadcrumb trail) for disk and `.meg` browsers.
pub fn browser_nav_bar(
    location: &BrowseLocation,
    on_navigate: impl FnMut(BrowseLocation) + Clone + 'static,
) -> Element {
    let parent_opt = parent_location(location);
    let crumbs = breadcrumbs(location);
    let visible_start = crumbs.len().saturating_sub(7);
    let truncated = visible_start > 0;
    let shown_crumbs = &crumbs[visible_start..];
    let in_meg = matches!(location, BrowseLocation::MegDir { .. });

    let mut nav_up = on_navigate.clone();
    let up_btn = action_button("Up", ButtonStyle::Secondary, true, move || {
        if let Some(parent_loc) = parent_opt.clone() {
            nav_up(parent_loc);
        }
    });

    let mut crumb_els: Vec<Element> = Vec::new();
    for (idx, crumb) in shown_crumbs.iter().enumerate() {
        if idx > 0 {
            crumb_els.push(
                label()
                    .font_size(11.0)
                    .color(theme::TEXT_DIM)
                    .max_lines(1)
                    .text("/")
                    .into(),
            );
        }
        let target = crumb.location.clone();
        let label_txt = crumb.label.clone();
        let is_last = idx + 1 == shown_crumbs.len();
        let is_meg_seg = label_txt.to_ascii_lowercase().ends_with(".meg");
        let txt_color = if is_last {
            theme::TEXT_PRIMARY
        } else if is_meg_seg {
            theme::ACCENT_AMBER
        } else {
            theme::ACCENT_BLUE
        };
        let mut nav_crumb = on_navigate.clone();
        crumb_els.push(
            rect()
                .padding((2.0, 5.0))
                .corner_radius(3.0)
                .background(if is_last {
                    theme::BG_ROW
                } else {
                    Color::TRANSPARENT
                })
                .on_pointer_down(move |_| nav_crumb(target.clone()))
                .child(
                    label()
                        .font_size(11.0)
                        .font_weight(if is_last {
                            FontWeight::BOLD
                        } else {
                            FontWeight::NORMAL
                        })
                        .color(txt_color)
                        .max_lines(1)
                        .text(label_txt),
                )
                .into(),
        );
    }

    rect()
        .width(Size::fill())
        .height(Size::px(30.0))
        .padding((3.0, 8.0))
        .corner_radius(4.0)
        .background(theme::BG_SUB_CARD)
        .border(theme::border_all(theme::BORDER_SUBTLE))
        .horizontal()
        .cross_align(Alignment::Center)
        .spacing(6.0)
        .overflow(Overflow::Clip)
        .child(up_btn)
        .maybe_child(in_meg.then(|| {
            rect()
                .padding((2.0, 6.0))
                .corner_radius(3.0)
                .background(theme::BG_MEG_BADGE)
                .border(theme::border_all(theme::BORDER_AMBER))
                .child(
                    label()
                        .font_size(10.0)
                        .font_weight(FontWeight::BOLD)
                        .color(theme::ACCENT_AMBER)
                        .max_lines(1)
                        .text("MEG"),
                )
        }))
        .maybe_child(truncated.then(|| {
            label()
                .font_size(11.0)
                .color(theme::TEXT_DIM)
                .max_lines(1)
                .text("... /")
        }))
        .children(crumb_els)
        .into()
}

/// Shared left sidebar listing quick-navigation shortcuts (`GameData`, `Workspace`, `Home`, `/`).
pub fn quick_places_sidebar(
    places: Vec<QuickPlace>,
    current_location: &BrowseLocation,
    on_navigate: impl FnMut(BrowseLocation) + Clone + 'static,
) -> Element {
    let items: Vec<Element> = places
        .into_iter()
        .map(|place| {
            let is_active = &place.location == current_location;
            let target = place.location.clone();
            let bg = if is_active {
                theme::BG_CHIP_ACTIVE
            } else {
                theme::BG_ROW
            };
            let border = if is_active {
                theme::border_all(theme::BORDER_CHIP_ACTIVE)
            } else {
                theme::border_all(theme::BORDER_CARD)
            };
            let mut nav = on_navigate.clone();
            rect()
                .width(Size::fill())
                .padding((5.0, 7.0))
                .corner_radius(4.0)
                .background(bg)
                .border(border)
                .vertical()
                .spacing(1.0)
                .on_pointer_down(move |_| nav(target.clone()))
                .child(
                    label()
                        .font_size(11.0)
                        .font_weight(if is_active {
                            FontWeight::BOLD
                        } else {
                            FontWeight::NORMAL
                        })
                        .color(theme::TEXT_PRIMARY)
                        .max_lines(1)
                        .text(place.label),
                )
                .maybe_child(place.subtitle.map(|sub| {
                    label()
                        .font_size(10.0)
                        .color(theme::TEXT_DIM)
                        .max_lines(1)
                        .text(sub)
                }))
                .into()
        })
        .collect();

    rect()
        .width(Size::px(190.0))
        .height(Size::fill())
        .content(Content::Flex)
        .padding(8.0)
        .corner_radius(5.0)
        .background(theme::BG_SUB_CARD)
        .border(theme::border_all(theme::BORDER_CARD))
        .vertical()
        .spacing(6.0)
        .child(group_caption("PLACES"))
        .child(
            ScrollView::new()
                .width(Size::fill())
                .height(Size::flex(1.0))
                .child(rect().width(Size::fill()).vertical().spacing(4.0).children(items)),
        )
        .into()
}
