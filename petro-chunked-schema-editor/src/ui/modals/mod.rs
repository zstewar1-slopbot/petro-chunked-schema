//! Centered overlay modal dialogs for browsing disk & `.meg` files, configuring `GameData`
//! directories, opening/saving YAML & built-in schemas with unsaved-changes safeguards,
//! running batch validation, and emitting `petro-chunked-derive` Rust code.

mod batch_validate;
mod confirm_discard;
mod file_browser;
mod gamedata_dirs;
mod open_schema;
mod rust_codegen;
mod save_schema_as;
mod yaml_schema;

use freya::prelude::*;

use crate::browser::{
    BrowseLocation, QuickPlace, breadcrumbs, parent_location,
};

use super::context::{ActiveModal, EditorState};
use super::theme;
use super::widgets::{ActionButton, ButtonStyle, GroupCaption};

use batch_validate::BatchValidateModal;
use confirm_discard::ConfirmDiscardModal;
use file_browser::FileBrowserModal;
use gamedata_dirs::GameDataDirsModal;
use open_schema::OpenSchemaModal;
use rust_codegen::RustCodegenModal;
use save_schema_as::SaveSchemaAsModal;
use yaml_schema::YamlPreviewModal;

/// Routes the active centered overlay modal selected in [`EditorState::active_modal`].
#[component]
pub fn ModalHost() -> Element {
    let state = use_context::<EditorState>();
    match *state.active_modal.read() {
        ActiveModal::None => rsx! {},
        ActiveModal::OpenFile => rsx! { FileBrowserModal {} },
        ActiveModal::GameDataDirs => rsx! { GameDataDirsModal {} },
        ActiveModal::OpenSchema => rsx! { OpenSchemaModal {} },
        ActiveModal::SaveSchemaAs => rsx! { SaveSchemaAsModal {} },
        ActiveModal::ConfirmDiscard => rsx! { ConfirmDiscardModal {} },
        ActiveModal::YamlPreview => rsx! { YamlPreviewModal {} },
        ActiveModal::BatchValidate => rsx! { BatchValidateModal {} },
        ActiveModal::RustCodegen => rsx! { RustCodegenModal {} },
    }
}

/// Centered overlay dialog window with a dimmed fullscreen backdrop, crisp title bar, and
/// click-outside / `[X]` dismissal.
#[component]
pub fn ModalDialog(
    /// Dialog window title shown in the top header bar.
    title: String,
    /// Fixed or percentage dialog width (e.g. `"920"`).
    width: String,
    /// Fixed or percentage dialog height (e.g. `"600"`).
    height: String,
    /// Dialog body elements.
    children: Element,
) -> Element {
    let mut state = use_context::<EditorState>();

    rsx! {
        rect {
            position: "global",
            position_top: "0",
            position_left: "0",
            width: "100%",
            height: "100%",
            background: theme::BG_MODAL_BACKDROP,
            main_align: "center",
            cross_align: "center",
            onpointerdown: move |_| state.close_modal(),

            rect {
                width: "{width}",
                height: "{height}",
                content: "flex",
                padding: "14 16",
                corner_radius: "8",
                background: theme::BG_MODAL,
                border: theme::BORDER_MODAL,
                shadow: "0 12 32 4 rgba(0, 0, 0, 0.55)",
                direction: "vertical",
                spacing: "10",
                onpointerdown: move |e: PointerEvent| e.stop_propagation(),

                rect {
                    width: "fill",
                    height: "28",
                    padding: "0 0 6 0",
                    border: "0 0 1 0 solid rgb(52, 57, 74)",
                    direction: "horizontal",
                    main_align: "space-between",
                    cross_align: "center",

                    label {
                        font_weight: "bold",
                        font_size: "14",
                        color: theme::ACCENT_BLUE,
                        max_lines: "1",
                        "{title}"
                    }

                    ActionButton {
                        label: "Close [X]".to_owned(),
                        style: ButtonStyle::Secondary,
                        compact: Some(true),
                        onpress: move |_| state.close_modal(),
                    }
                }

                {children}
            }
        }
    }
}

/// Shared navigation bar (`Up` button + clickable breadcrumb trail) for disk and `.meg` browsers.
#[component]
pub fn BrowserNavBar(
    /// Current browsed directory or `.meg` archive folder.
    location: BrowseLocation,
    /// Callback fired when the user clicks `Up` or any breadcrumb segment.
    onnavigate: EventHandler<BrowseLocation>,
) -> Element {
    let parent_opt = parent_location(&location);
    let crumbs = breadcrumbs(&location);
    let visible_start = crumbs.len().saturating_sub(7);
    let truncated = visible_start > 0;
    let shown_crumbs = &crumbs[visible_start..];
    let in_meg = matches!(location, BrowseLocation::MegDir { .. });

    rsx! {
        rect {
            width: "fill",
            height: "30",
            padding: "3 8",
            corner_radius: "4",
            background: theme::BG_SUB_CARD,
            border: "1 solid rgb(52, 57, 74)",
            direction: "horizontal",
            cross_align: "center",
            spacing: "6",
            overflow: "clip",

            if let Some(parent_loc) = parent_opt {
                ActionButton {
                    label: "Up".to_owned(),
                    style: ButtonStyle::Secondary,
                    compact: Some(true),
                    onpress: move |_| onnavigate.call(parent_loc.clone()),
                }
            } else {
                ActionButton {
                    label: "Up".to_owned(),
                    style: ButtonStyle::Secondary,
                    compact: Some(true),
                    onpress: move |_| {},
                }
            }

            if in_meg {
                rect {
                    padding: "2 6",
                    corner_radius: "3",
                    background: "rgb(72, 54, 24)",
                    border: "1 solid rgb(165, 125, 55)",
                    label {
                        font_size: "10",
                        font_weight: "bold",
                        color: theme::ACCENT_AMBER,
                        max_lines: "1",
                        "MEG"
                    }
                }
            }

            if truncated {
                label {
                    font_size: "11",
                    color: theme::TEXT_DIM,
                    max_lines: "1",
                    "... /"
                }
            }

            for (idx, crumb) in shown_crumbs.iter().enumerate() {
                {
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
                    rsx! {
                        if idx > 0 {
                            label {
                                font_size: "11",
                                color: theme::TEXT_DIM,
                                max_lines: "1",
                                "/"
                            }
                        }
                        rect {
                            padding: "2 5",
                            corner_radius: "3",
                            background: if is_last { theme::BG_ROW } else { "transparent" },
                            onpointerdown: move |_| onnavigate.call(target.clone()),
                            label {
                                font_size: "11",
                                font_weight: if is_last { "bold" } else { "normal" },
                                color: "{txt_color}",
                                max_lines: "1",
                                "{label_txt}"
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Shared left sidebar listing quick-navigation shortcuts (`GameData`, `Workspace`, `Home`, `/`).
#[component]
pub fn QuickPlacesSidebar(
    /// Places to render in the sidebar.
    places: Vec<QuickPlace>,
    /// Current location used to highlight the active place.
    current_location: BrowseLocation,
    /// Callback fired when a shortcut is clicked.
    onnavigate: EventHandler<BrowseLocation>,
) -> Element {
    rsx! {
        rect {
            width: "190",
            height: "fill",
            padding: "8",
            corner_radius: "5",
            background: theme::BG_SUB_CARD,
            border: "1 solid rgb(48, 53, 68)",
            direction: "vertical",
            spacing: "6",

            GroupCaption { text: "PLACES".to_owned() }

            ScrollView {
                width: "fill",
                height: "fill",
                rect {
                    width: "fill",
                    direction: "vertical",
                    spacing: "4",

                    for place in places {
                        {
                            let is_active = place.location == current_location;
                            let target = place.location.clone();
                            let bg = if is_active { theme::BG_CHIP_ACTIVE } else { theme::BG_ROW };
                            let border = if is_active {
                                theme::BORDER_CHIP_ACTIVE
                            } else {
                                "1 solid rgb(42, 46, 60)"
                            };
                            rsx! {
                                rect {
                                    width: "fill",
                                    padding: "5 7",
                                    corner_radius: "4",
                                    background: "{bg}",
                                    border: "{border}",
                                    direction: "vertical",
                                    spacing: "1",
                                    onpointerdown: move |_| onnavigate.call(target.clone()),

                                    label {
                                        font_size: "11",
                                        font_weight: if is_active { "bold" } else { "normal" },
                                        color: theme::TEXT_PRIMARY,
                                        max_lines: "1",
                                        "{place.label}"
                                    }
                                    if let Some(sub) = &place.subtitle {
                                        label {
                                            font_size: "10",
                                            color: theme::TEXT_DIM,
                                            max_lines: "1",
                                            "{sub}"
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
