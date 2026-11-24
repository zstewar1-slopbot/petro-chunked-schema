//! Centered `GameData` directory management modal with a visual directory picker.

use std::path::PathBuf;

use freya::prelude::*;

use crate::browser::{
    BrowseLocation, BrowseOptions, default_quick_places, list_location, location_display,
};
use crate::gamedata::detect_default_gamedata_dirs;
use crate::ui::context::EditorState;
use crate::ui::theme;
use crate::ui::widgets::{ActionButton, ButtonStyle, GroupCaption};

use super::{BrowserNavBar, ModalDialog, QuickPlacesSidebar};

/// Centered modal for adding, auto-detecting, and removing Petroglyph `GameData` directories
/// using a visual filesystem folder browser.
#[component]
pub fn GameDataDirsModal() -> Element {
    let mut state = use_context::<EditorState>();
    let dirs = state.persisted.read().game_data_dirs.clone();

    let mut browse_loc = use_signal(|| {
        dirs.first()
            .and_then(|d| d.parent().map(std::path::Path::to_path_buf))
            .filter(|d| d.is_dir())
            .or_else(|| std::env::current_dir().ok())
            .map(BrowseLocation::DiskDir)
            .unwrap_or_else(|| BrowseLocation::DiskDir(PathBuf::from("/")))
    });

    let current_loc = browse_loc.read().clone();
    let current_disk_dir = match &current_loc {
        BrowseLocation::DiskDir(p) => p.clone(),
        BrowseLocation::MegDir { meg_path, .. } => meg_path
            .parent()
            .map(std::path::Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("/")),
    };

    let places = default_quick_places(&dirs);
    let listing = list_location(
        &current_loc,
        &BrowseOptions {
            dirs_only: true,
            allow_meg_browse: false,
            ..Default::default()
        },
    );

    let already_added = dirs.iter().any(|existing| {
        let a = std::fs::canonicalize(existing).unwrap_or_else(|_| existing.clone());
        let b = std::fs::canonicalize(&current_disk_dir)
            .unwrap_or_else(|_| current_disk_dir.clone());
        a == b
    });

    rsx! {
        ModalDialog {
            title: "Configure GameData Directories".to_owned(),
            width: "900".to_owned(),
            height: "600".to_owned(),

            rect {
                width: "fill",
                padding: "8 10",
                corner_radius: "5",
                background: theme::BG_SUB_CARD,
                border: "1 solid rgb(48, 53, 68)",
                direction: "vertical",
                spacing: "6",

                rect {
                    width: "fill",
                    direction: "horizontal",
                    main_align: "space-between",
                    cross_align: "center",
                    GroupCaption { text: format!("CONFIGURED DIRECTORIES ({})", dirs.len()) }
                    ActionButton {
                        label: "Auto-Detect EAW / FoC".to_owned(),
                        style: ButtonStyle::Secondary,
                        compact: Some(true),
                        onpress: move |_| {
                            let found = detect_default_gamedata_dirs();
                            let mut added = 0usize;
                            {
                                let mut p = state.persisted.write();
                                for d in found {
                                    if !p.game_data_dirs.contains(&d) {
                                        p.game_data_dirs.push(d);
                                        added += 1;
                                    }
                                }
                                p.save();
                            }
                            state.set_status(format!("Auto-detected {added} new GameData directory(s)"));
                        },
                    }
                }

                if dirs.is_empty() {
                    label {
                        font_size: "12",
                        color: theme::TEXT_MUTED,
                        "No GameData directories configured yet. Browse to a GameData/Data folder below and click Add."
                    }
                } else {
                    ScrollView {
                        width: "fill",
                        height: "110",
                        rect {
                            width: "fill",
                            direction: "vertical",
                            spacing: "3",
                            for (idx, d) in dirs.iter().enumerate() {
                                {
                                    let disp = d.display().to_string();
                                    let jump_dir = d.clone();
                                    rsx! {
                                        rect {
                                            width: "fill",
                                            height: "28",
                                            padding: "0 8",
                                            corner_radius: "4",
                                            background: theme::BG_INSET,
                                            direction: "horizontal",
                                            cross_align: "center",
                                            spacing: "8",
                                            overflow: "clip",

                                            label {
                                                width: "670",
                                                font_size: "12",
                                                color: theme::ACCENT_ASCII,
                                                max_lines: "1",
                                                "{disp}"
                                            }
                                            ActionButton {
                                                label: "Browse".to_owned(),
                                                style: ButtonStyle::Secondary,
                                                compact: Some(true),
                                                onpress: move |_| {
                                                    if jump_dir.is_dir() {
                                                        browse_loc.set(BrowseLocation::DiskDir(jump_dir.clone()));
                                                    }
                                                },
                                            }
                                            ActionButton {
                                                label: "Remove".to_owned(),
                                                style: ButtonStyle::Danger,
                                                compact: Some(true),
                                                onpress: move |_| {
                                                    let mut p = state.persisted.write();
                                                    if idx < p.game_data_dirs.len() {
                                                        p.game_data_dirs.remove(idx);
                                                        p.save();
                                                    }
                                                },
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            GroupCaption { text: "PICK FOLDER TO ADD".to_owned() }

            BrowserNavBar {
                location: current_loc.clone(),
                onnavigate: move |next| browse_loc.set(next),
            }

            rect {
                width: "fill",
                height: "flex(1)",
                content: "flex",
                direction: "horizontal",
                spacing: "10",

                QuickPlacesSidebar {
                    places: places,
                    current_location: current_loc.clone(),
                    onnavigate: move |next| browse_loc.set(next),
                }

                rect {
                    width: "flex(1)",
                    height: "fill",
                    content: "flex",
                    padding: "6",
                    corner_radius: "5",
                    background: theme::BG_INSET,
                    border: "1 solid rgb(46, 51, 66)",
                    direction: "vertical",
                    spacing: "4",

                    match listing {
                        Ok(entries) if entries.is_empty() => rsx! {
                            rect {
                                width: "fill",
                                padding: "12",
                                label {
                                    font_size: "12",
                                    color: theme::TEXT_MUTED,
                                    "No subdirectories inside {location_display(&current_loc)}."
                                }
                            }
                        },
                        Ok(entries) => rsx! {
                            ScrollView {
                                width: "fill",
                                height: "flex(1)",
                                rect {
                                    width: "fill",
                                    direction: "vertical",
                                    spacing: "2",

                                    for entry in entries {
                                        {
                                            let next_loc = entry.target_location.clone();
                                            let dir_path = entry.disk_path.clone();
                                            rsx! {
                                                rect {
                                                    width: "fill",
                                                    height: "28",
                                                    padding: "0 8",
                                                    corner_radius: "3",
                                                    background: theme::BG_ROW,
                                                    direction: "horizontal",
                                                    cross_align: "center",
                                                    spacing: "8",
                                                    overflow: "clip",
                                                    onpointerdown: move |_| {
                                                        if let Some(loc) = next_loc.clone() {
                                                            browse_loc.set(loc);
                                                        }
                                                    },

                                                    label {
                                                        width: "44",
                                                        font_size: "10",
                                                        font_weight: "bold",
                                                        font_family: "monospace",
                                                        color: theme::ACCENT_BLUE,
                                                        max_lines: "1",
                                                        "[DIR]"
                                                    }
                                                    label {
                                                        width: "510",
                                                        font_size: "12",
                                                        font_weight: "bold",
                                                        color: theme::TEXT_PRIMARY,
                                                        max_lines: "1",
                                                        "{entry.name}"
                                                    }
                                                    if let Some(add_path) = dir_path {
                                                        ActionButton {
                                                            label: "+ Add".to_owned(),
                                                            style: ButtonStyle::Secondary,
                                                            compact: Some(true),
                                                            onpress: move |_| {
                                                                let mut p = state.persisted.write();
                                                                if !p.game_data_dirs.contains(&add_path) {
                                                                    p.game_data_dirs.push(add_path.clone());
                                                                    p.save();
                                                                }
                                                                drop(p);
                                                                state.set_status(format!(
                                                                    "Added GameData directory {}",
                                                                    add_path.display()
                                                                ));
                                                            },
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        },
                        Err(err) => rsx! {
                            label { font_size: "12", color: theme::ACCENT_RED, "{err}" }
                        },
                    }
                }
            }

            rect {
                width: "fill",
                height: "32",
                direction: "horizontal",
                main_align: "space-between",
                cross_align: "center",
                spacing: "10",

                label {
                    font_size: "11",
                    color: theme::TEXT_MUTED,
                    max_lines: "1",
                    "Current folder: {current_disk_dir.display()}"
                }

                rect {
                    direction: "horizontal",
                    spacing: "8",
                    cross_align: "center",

                    ActionButton {
                        label: if already_added {
                            "Current Folder Already Added".to_owned()
                        } else {
                            "+ Add Current Folder".to_owned()
                        },
                        style: if already_added {
                            ButtonStyle::Secondary
                        } else {
                            ButtonStyle::Primary
                        },
                        onpress: move |_| {
                            if !already_added {
                                let target = current_disk_dir.clone();
                                {
                                    let mut p = state.persisted.write();
                                    p.game_data_dirs.push(target.clone());
                                    p.save();
                                }
                                state.set_status(format!(
                                    "Added GameData directory {}",
                                    target.display()
                                ));
                            }
                        },
                    }

                    ActionButton {
                        label: "Done".to_owned(),
                        style: ButtonStyle::Secondary,
                        onpress: move |_| state.close_modal(),
                    }
                }
            }
        }
    }
}
