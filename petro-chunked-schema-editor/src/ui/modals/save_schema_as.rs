//! Centered Save Schema As dialog with a visual directory & `.yaml` file browser.

use std::path::PathBuf;

use freya::prelude::*;
use petro_chunked_schema::source::SchemaRef;

use crate::browser::{
    BrowseEntryKind, BrowseLocation, BrowseOptions, default_quick_places, format_size_compact,
    list_location,
};
use crate::ui::context::EditorState;
use crate::ui::theme;
use crate::ui::widgets::{ActionButton, ButtonStyle, StyledInput};

use super::{BrowserNavBar, ModalDialog, QuickPlacesSidebar};

/// Centered dialog allowing the user to pick a directory and filename to save the active schema
/// as a `.yaml` file (`Ctrl+Shift+S`, or `Ctrl+S` on built-in/unsaved schemas).
#[component]
pub fn SaveSchemaAsModal() -> Element {
    let mut state = use_context::<EditorState>();
    let current_ref = state.persisted.read().schema_ref.clone();
    let game_data_dirs = state.persisted.read().game_data_dirs.clone();
    let schema_name = state.schema.read().name.clone();

    let (init_dir, init_filename) = use_hook(|| match &current_ref {
        Some(SchemaRef::File(p)) => {
            let dir = p
                .parent()
                .filter(|d| d.is_dir())
                .map(std::path::Path::to_path_buf)
                .or_else(|| std::env::current_dir().ok())
                .unwrap_or_else(|| PathBuf::from("/"));
            let name = p
                .file_name()
                .and_then(|s| s.to_str())
                .map(str::to_owned)
                .unwrap_or_else(|| format!("{schema_name}.yaml"));
            (dir, name)
        }
        _ => {
            let dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
            let slug = schema_name
                .chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() {
                        c.to_ascii_lowercase()
                    } else {
                        '_'
                    }
                })
                .collect::<String>();
            let name = if slug.is_empty() {
                "custom_schema.yaml".to_owned()
            } else {
                format!("{slug}.yaml")
            };
            (dir, name)
        }
    });

    let mut location = use_signal(|| BrowseLocation::DiskDir(init_dir));
    let mut filename = use_signal(|| init_filename);

    let current_loc = location.read().clone();
    let current_dir = match &current_loc {
        BrowseLocation::DiskDir(p) => p.clone(),
        BrowseLocation::MegDir { meg_path, .. } => meg_path
            .parent()
            .map(std::path::Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("/")),
    };

    let places = default_quick_places(&game_data_dirs);
    let listing = list_location(
        &current_loc,
        &BrowseOptions {
            dirs_only: false,
            allow_meg_browse: false,
            extensions: vec!["yaml".to_owned(), "yml".to_owned()],
            ..Default::default()
        },
    );

    let pending_note = state
        .pending_schema_action
        .read()
        .as_ref()
        .map(|a| format!("After saving, editor will {}.", a.describe()));

    rsx! {
        ModalDialog {
            title: format!("Save Schema `{schema_name}` As YAML"),
            width: "880".to_owned(),
            height: "570".to_owned(),

            if let Some(note) = pending_note {
                label {
                    font_size: "11",
                    color: theme::ACCENT_AMBER,
                    "{note}"
                }
            }

            BrowserNavBar {
                location: current_loc.clone(),
                onnavigate: move |next| location.set(next),
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
                    onnavigate: move |next| location.set(next),
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
                                padding: "14",
                                label {
                                    font_size: "12",
                                    color: theme::TEXT_MUTED,
                                    "No existing .yaml files or subdirectories in this folder."
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
                                            let entry_name = entry.name.clone();
                                            let is_dir = entry.kind == BrowseEntryKind::Directory;
                                            let badge_color = if is_dir {
                                                theme::ACCENT_BLUE
                                            } else {
                                                theme::ACCENT_GREEN
                                            };
                                            let size_txt = entry
                                                .size
                                                .map(format_size_compact)
                                                .unwrap_or_else(|| "—".to_owned());
                                            let badge = entry.kind.badge();
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
                                                            location.set(loc);
                                                        } else {
                                                            filename.set(entry_name.clone());
                                                        }
                                                    },

                                                    label {
                                                        width: "44",
                                                        font_size: "10",
                                                        font_weight: "bold",
                                                        font_family: "monospace",
                                                        color: "{badge_color}",
                                                        max_lines: "1",
                                                        "[{badge}]"
                                                    }
                                                    label {
                                                        width: "480",
                                                        font_size: "12",
                                                        font_weight: if is_dir { "bold" } else { "normal" },
                                                        color: theme::TEXT_PRIMARY,
                                                        max_lines: "1",
                                                        "{entry.name}"
                                                    }
                                                    label {
                                                        width: "70",
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
                            label { font_size: "12", color: theme::ACCENT_RED, "{err}" }
                        },
                    }
                }
            }

            rect {
                width: "fill",
                height: "34",
                direction: "horizontal",
                main_align: "space-between",
                cross_align: "center",
                spacing: "10",

                rect {
                    direction: "horizontal",
                    cross_align: "center",
                    spacing: "8",
                    label {
                        font_size: "12",
                        color: theme::TEXT_MUTED,
                        "Filename:"
                    }
                    StyledInput {
                        width: Some("360".to_owned()),
                        value: filename.read().clone(),
                        placeholder: Some("my_format.yaml".to_owned()),
                        onchange: move |txt| filename.set(txt),
                    }
                }

                rect {
                    direction: "horizontal",
                    cross_align: "center",
                    spacing: "8",
                    ActionButton {
                        label: "Cancel".to_owned(),
                        style: ButtonStyle::Secondary,
                        onpress: move |_| state.cancel_pending_action(),
                    }
                    ActionButton {
                        label: "Save Schema".to_owned(),
                        style: ButtonStyle::Primary,
                        onpress: move |_| {
                            let raw_name = filename.read().trim().to_owned();
                            if raw_name.is_empty() {
                                state.set_status("Enter a filename before saving.".to_owned());
                                return;
                            }
                            let final_name = if raw_name.contains('.') {
                                raw_name
                            } else {
                                format!("{raw_name}.yaml")
                            };
                            let target_path = if PathBuf::from(&final_name).is_absolute() {
                                PathBuf::from(&final_name)
                            } else {
                                current_dir.join(&final_name)
                            };
                            state.save_schema_as(target_path);
                        },
                    }
                }
            }
        }
    }
}
