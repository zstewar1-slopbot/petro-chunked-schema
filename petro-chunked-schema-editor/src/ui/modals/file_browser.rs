//! Centered hierarchical file browser modal with native browse-into-`.meg` archive support.

use freya::prelude::*;

use crate::browser::{
    BrowseEntryKind, BrowseLocation, BrowseOptions, default_quick_places, format_size_compact,
    initial_browse_location, list_location, location_display,
};
use crate::state::FileSource;
use crate::ui::context::EditorState;
use crate::ui::theme;
use crate::ui::widgets::{ActionButton, ButtonStyle, SegmentedChip, StyledInput};

use super::{BrowserNavBar, ModalDialog, QuickPlacesSidebar};

const EXT_PRESETS: [(&str, Option<&str>); 5] = [
    ("All", None),
    (".alo", Some("alo")),
    (".ala", Some("ala")),
    (".ted", Some("ted")),
    (".bui", Some("bui")),
];

/// Centered dialog for browsing local directories and stepping inside Petroglyph `.meg` archives
/// as virtual folders to open game files.
#[component]
pub fn FileBrowserModal() -> Element {
    let mut state = use_context::<EditorState>();
    let game_data_dirs = state.persisted.read().game_data_dirs.clone();
    let active_file = state.persisted.read().last_file.clone();

    let mut location = use_signal(|| initial_browse_location(&game_data_dirs, active_file.as_ref()));
    let mut ext_filter = use_signal(|| Some("alo".to_owned()));
    let mut name_filter = use_signal(String::new);
    let mut recursive_in_meg = use_signal(|| true);
    let mut selected_source = use_signal(|| None::<(FileSource, String)>);

    let current_loc = location.read().clone();
    let in_meg = matches!(current_loc, BrowseLocation::MegDir { .. });
    let places = default_quick_places(&game_data_dirs);

    let browse_opts = BrowseOptions {
        dirs_only: false,
        allow_meg_browse: true,
        extensions: ext_filter.read().iter().cloned().collect(),
        name_filter: name_filter.read().clone(),
        recursive_in_meg: *recursive_in_meg.read(),
        show_hidden: false,
        max_entries: 500,
    };

    let listing = list_location(&current_loc, &browse_opts);
    let entry_count = listing.as_ref().map_or(0, Vec::len);
    let selected_now = selected_source.read().clone();

    rsx! {
        ModalDialog {
            title: "Open Game File or Browse Inside .meg Archive".to_owned(),
            width: "940".to_owned(),
            height: "610".to_owned(),

            BrowserNavBar {
                location: current_loc.clone(),
                onnavigate: move |next| {
                    selected_source.set(None);
                    location.set(next);
                },
            }

            rect {
                width: "fill",
                direction: "horizontal",
                cross_align: "center",
                spacing: "8",

                label {
                    font_size: "11",
                    color: theme::TEXT_MUTED,
                    "Type:"
                }
                for (preset_label, preset_ext) in EXT_PRESETS {
                    {
                        let is_sel = ext_filter.read().as_deref() == preset_ext;
                        let target_ext = preset_ext.map(str::to_owned);
                        rsx! {
                            SegmentedChip {
                                key: "{preset_label}",
                                label: preset_label.to_owned(),
                                selected: is_sel,
                                compact: Some(true),
                                onpress: move |_| {
                                    ext_filter.set(target_ext.clone());
                                    selected_source.set(None);
                                },
                            }
                        }
                    }
                }

                label {
                    font_size: "11",
                    color: theme::TEXT_MUTED,
                    "Filter:"
                }
                StyledInput {
                    width: Some("230".to_owned()),
                    value: name_filter.read().clone(),
                    placeholder: Some("name substring (e.g. tiefighter)...".to_owned()),
                    onchange: move |txt| name_filter.set(txt),
                }

                if in_meg {
                    SegmentedChip {
                        label: "Search Inside .meg".to_owned(),
                        selected: *recursive_in_meg.read(),
                        compact: Some(true),
                        onpress: move |_| {
                            let cur = *recursive_in_meg.read();
                            recursive_in_meg.set(!cur);
                        },
                    }
                }

                label {
                    font_size: "11",
                    color: theme::TEXT_DIM,
                    max_lines: "1",
                    "{entry_count} item(s)"
                }
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
                    onnavigate: move |next| {
                        selected_source.set(None);
                        location.set(next);
                    },
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

                    rect {
                        width: "fill",
                        height: "22",
                        padding: "0 6",
                        border: "0 0 1 0 solid rgb(42, 46, 60)",
                        direction: "horizontal",
                        cross_align: "center",
                        spacing: "8",
                        overflow: "clip",
                        label { width: "44", font_size: "10", font_weight: "bold", color: theme::TEXT_DIM, max_lines: "1", "KIND" }
                        label { width: "540", font_size: "10", font_weight: "bold", color: theme::TEXT_DIM, max_lines: "1", "NAME" }
                        label { width: "80", font_size: "10", font_weight: "bold", color: theme::TEXT_DIM, max_lines: "1", "SIZE" }
                    }

                    match listing {
                        Ok(entries) if entries.is_empty() => rsx! {
                            rect {
                                width: "fill",
                                padding: "16",
                                label {
                                    font_size: "12",
                                    color: theme::TEXT_MUTED,
                                    "No matching files or folders in {location_display(&current_loc)}."
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
                                            let is_sel = entry.file_source.is_some()
                                                && selected_now.as_ref().map(|(s, _)| s) == entry.file_source.as_ref();
                                            let bg = if is_sel {
                                                theme::BG_ROW_SELECTED
                                            } else {
                                                theme::BG_ROW
                                            };
                                            let badge_color = match entry.kind {
                                                BrowseEntryKind::Directory | BrowseEntryKind::MegVirtualDir => theme::ACCENT_BLUE,
                                                BrowseEntryKind::MegArchive => theme::ACCENT_AMBER,
                                                BrowseEntryKind::DiskFile | BrowseEntryKind::MegFile => theme::ACCENT_GREEN,
                                            };
                                            let name_color = match entry.kind {
                                                BrowseEntryKind::Directory | BrowseEntryKind::MegVirtualDir => theme::TEXT_PRIMARY,
                                                BrowseEntryKind::MegArchive => theme::ACCENT_AMBER,
                                                BrowseEntryKind::DiskFile | BrowseEntryKind::MegFile => theme::TEXT_PRIMARY,
                                            };
                                            let size_txt = entry
                                                .size
                                                .map(format_size_compact)
                                                .unwrap_or_else(|| "—".to_owned());
                                            let target_loc = entry.target_location.clone();
                                            let file_src = entry.file_source.clone();
                                            let entry_name = entry.name.clone();
                                            let badge_txt = entry.kind.badge();
                                            rsx! {
                                                rect {
                                                    width: "fill",
                                                    height: "28",
                                                    padding: "0 6",
                                                    corner_radius: "3",
                                                    background: "{bg}",
                                                    direction: "horizontal",
                                                    cross_align: "center",
                                                    spacing: "8",
                                                    overflow: "clip",
                                                    onpointerdown: move |_| {
                                                        if let Some(next_loc) = target_loc.clone() {
                                                            selected_source.set(None);
                                                            location.set(next_loc);
                                                        } else if let Some(src) = file_src.clone() {
                                                            if is_sel {
                                                                state.open_file(src);
                                                                state.close_modal();
                                                            } else {
                                                                selected_source.set(Some((src, entry_name.clone())));
                                                            }
                                                        }
                                                    },

                                                    label {
                                                        width: "44",
                                                        font_size: "10",
                                                        font_weight: "bold",
                                                        font_family: "monospace",
                                                        color: "{badge_color}",
                                                        max_lines: "1",
                                                        "[{badge_txt}]"
                                                    }
                                                    label {
                                                        width: "540",
                                                        font_size: "12",
                                                        font_weight: if entry.kind.is_navigable() { "bold" } else { "normal" },
                                                        color: "{name_color}",
                                                        max_lines: "1",
                                                        "{entry.name}"
                                                    }
                                                    label {
                                                        width: "80",
                                                        font_size: "11",
                                                        font_family: "monospace",
                                                        color: theme::TEXT_DIM,
                                                        max_lines: "1",
                                                        "{size_txt}"
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        },
                        Err(err) => rsx! {
                            rect {
                                width: "fill",
                                padding: "12",
                                label {
                                    font_size: "12",
                                    color: theme::ACCENT_RED,
                                    "{err}"
                                }
                            }
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

                if let Some((_, sel_label)) = &selected_now {
                    label {
                        font_size: "12",
                        color: theme::ACCENT_GREEN,
                        max_lines: "1",
                        "Selected: {sel_label}  (click again or press Open)"
                    }
                } else {
                    label {
                        font_size: "11",
                        color: theme::TEXT_DIM,
                        max_lines: "1",
                        "Click a folder or .meg archive to browse inside; click a file to select."
                    }
                }

                rect {
                    direction: "horizontal",
                    spacing: "8",
                    cross_align: "center",

                    ActionButton {
                        label: "Cancel".to_owned(),
                        style: ButtonStyle::Secondary,
                        onpress: move |_| state.close_modal(),
                    }

                    if let Some((src, _)) = selected_now {
                        ActionButton {
                            label: "Open File".to_owned(),
                            style: ButtonStyle::Primary,
                            onpress: move |_| {
                                state.open_file(src.clone());
                                state.close_modal();
                            },
                        }
                    }
                }
            }
        }
    }
}
