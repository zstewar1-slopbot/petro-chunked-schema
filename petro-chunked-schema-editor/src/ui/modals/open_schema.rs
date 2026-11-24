//! Unified centered Open Schema dialog combining built-in Petroglyph formats, blank schema
//! creation, a visual YAML file browser, and manual `SchemaRef` URI entry.

use std::path::PathBuf;

use freya::prelude::*;
use petro_chunked_schema::known_formats;
use petro_chunked_schema::source::SchemaRef;

use crate::browser::{
    BrowseEntryKind, BrowseLocation, BrowseOptions, default_quick_places, format_size_compact,
    list_location, location_display,
};
use crate::ui::context::EditorState;
use crate::ui::theme;
use crate::ui::widgets::{ActionButton, ButtonStyle, GroupCaption, StyledInput};

use super::{BrowserNavBar, ModalDialog};

fn builtin_description(name: &str) -> &'static str {
    match name {
        "Alo" => "Combined ALO dispatcher (models + particle systems)",
        "AloModel" => "3D skeletons, meshes, shaders, proxies, and lights",
        "AloParticle" => "Particle emitter systems and simulation graphs",
        "AlaAnimation" => "Bone keyframe animations and compressed tracks",
        _ => "Embedded Petroglyph format schema",
    }
}

/// Unified centered dialog for selecting a built-in schema, starting a blank schema, or browsing
/// disk for a `.yaml` / `.yml` schema file. All actions pass through the unsaved-changes guard.
#[component]
pub fn OpenSchemaModal() -> Element {
    let mut state = use_context::<EditorState>();
    let current_ref = state.persisted.read().schema_ref.clone();
    let game_data_dirs = state.persisted.read().game_data_dirs.clone();

    let mut browse_loc = use_signal(|| {
        if let Some(SchemaRef::File(path)) = &current_ref
            && let Some(parent) = path.parent()
            && parent.is_dir()
        {
            return BrowseLocation::DiskDir(parent.to_path_buf());
        }
        std::env::current_dir()
            .map(BrowseLocation::DiskDir)
            .unwrap_or_else(|_| BrowseLocation::DiskDir(PathBuf::from("/")))
    });
    let mut selected_yaml = use_signal(|| None::<PathBuf>);
    let mut manual_uri = use_signal(|| {
        current_ref
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_default()
    });

    let current_loc = browse_loc.read().clone();
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
    let selected_path = selected_yaml.read().clone();

    rsx! {
        ModalDialog {
            title: "Open Schema (Built-in Formats & YAML Files)".to_owned(),
            width: "920".to_owned(),
            height: "600".to_owned(),

            rect {
                width: "fill",
                height: "flex(1)",
                content: "flex",
                direction: "horizontal",
                spacing: "12",

                // Left column: Built-in Schemas & New Blank
                rect {
                    width: "280",
                    height: "fill",
                    content: "flex",
                    padding: "10",
                    corner_radius: "5",
                    background: theme::BG_SUB_CARD,
                    border: "1 solid rgb(48, 53, 68)",
                    direction: "vertical",
                    spacing: "8",

                    GroupCaption { text: "BUILT-IN FORMATS".to_owned() }

                    ScrollView {
                        width: "fill",
                        height: "flex(1)",
                        rect {
                            width: "fill",
                            direction: "vertical",
                            spacing: "6",

                            for &name in known_formats::BUILTIN_NAMES {
                                {
                                    let target_ref = SchemaRef::Builtin(name.to_owned());
                                    let is_active = current_ref.as_ref() == Some(&target_ref);
                                    let bg = if is_active {
                                        theme::BG_CHIP_ACTIVE
                                    } else {
                                        theme::BG_ROW
                                    };
                                    let border = if is_active {
                                        theme::BORDER_CHIP_ACTIVE
                                    } else {
                                        "1 solid rgb(48, 53, 68)"
                                    };
                                    let desc = builtin_description(name);
                                    rsx! {
                                        rect {
                                            key: "{name}",
                                            width: "fill",
                                            padding: "8 10",
                                            corner_radius: "5",
                                            background: "{bg}",
                                            border: "{border}",
                                            direction: "vertical",
                                            spacing: "3",
                                            onpointerdown: move |_| {
                                                state.request_load_schema_ref(target_ref.clone());
                                            },

                                            rect {
                                                width: "fill",
                                                direction: "horizontal",
                                                main_align: "space-between",
                                                cross_align: "center",
                                                label {
                                                    font_size: "12",
                                                    font_weight: "bold",
                                                    color: theme::TEXT_PRIMARY,
                                                    max_lines: "1",
                                                    "{name}"
                                                }
                                                label {
                                                    font_size: "10",
                                                    font_family: "monospace",
                                                    color: if is_active { theme::ACCENT_GREEN } else { theme::ACCENT_BLUE },
                                                    max_lines: "1",
                                                    if is_active { "active" } else { "builtin:{name}" }
                                                }
                                            }
                                            label {
                                                font_size: "11",
                                                color: theme::TEXT_MUTED,
                                                "{desc}"
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }

                    rect {
                        width: "fill",
                        padding: "8 0 0 0",
                        border: "1 0 0 0 solid rgb(48, 53, 68)",
                        direction: "vertical",
                        spacing: "6",
                        GroupCaption { text: "SCRATCHPAD".to_owned() }
                        ActionButton {
                            label: "+ New Blank Schema".to_owned(),
                            style: ButtonStyle::Secondary,
                            onpress: move |_| state.request_new_blank_schema(),
                        }
                    }
                }

                // Right column: YAML File Browser
                rect {
                    width: "flex(1)",
                    height: "fill",
                    content: "flex",
                    direction: "vertical",
                    spacing: "8",

                    rect {
                        width: "fill",
                        direction: "horizontal",
                        main_align: "space-between",
                        cross_align: "center",
                        GroupCaption { text: "YAML SCHEMA FILES ON DISK".to_owned() }

                        rect {
                            direction: "horizontal",
                            spacing: "4",
                            for place in places.into_iter().take(4) {
                                {
                                    let target = place.location.clone();
                                    let is_cur = target == current_loc;
                                    rsx! {
                                        ActionButton {
                                            label: place.label,
                                            style: if is_cur { ButtonStyle::Active } else { ButtonStyle::Secondary },
                                            compact: Some(true),
                                            onpress: move |_| {
                                                selected_yaml.set(None);
                                                browse_loc.set(target.clone());
                                            },
                                        }
                                    }
                                }
                            }
                        }
                    }

                    BrowserNavBar {
                        location: current_loc.clone(),
                        onnavigate: move |next| {
                            selected_yaml.set(None);
                            browse_loc.set(next);
                        },
                    }

                    rect {
                        width: "fill",
                        height: "flex(1)",
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
                                        "No .yaml/.yml files or folders in {location_display(&current_loc)}."
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
                                                let is_sel = entry.disk_path.is_some()
                                                    && entry.kind == BrowseEntryKind::DiskFile
                                                    && selected_path == entry.disk_path;
                                                let bg = if is_sel {
                                                    theme::BG_ROW_SELECTED
                                                } else {
                                                    theme::BG_ROW
                                                };
                                                let badge_color = match entry.kind {
                                                    BrowseEntryKind::Directory => theme::ACCENT_BLUE,
                                                    _ => theme::ACCENT_GREEN,
                                                };
                                                let size_txt = entry
                                                    .size
                                                    .map(format_size_compact)
                                                    .unwrap_or_else(|| "—".to_owned());
                                                let next_loc = entry.target_location.clone();
                                                let disk_p = entry.disk_path.clone();
                                                let badge = entry.kind.badge();
                                                rsx! {
                                                    rect {
                                                        width: "fill",
                                                        height: "28",
                                                        padding: "0 8",
                                                        corner_radius: "3",
                                                        background: "{bg}",
                                                        direction: "horizontal",
                                                        cross_align: "center",
                                                        spacing: "8",
                                                        overflow: "clip",
                                                        onpointerdown: move |_| {
                                                            if let Some(loc) = next_loc.clone() {
                                                                selected_yaml.set(None);
                                                                browse_loc.set(loc);
                                                            } else if let Some(path) = disk_p.clone() {
                                                                if is_sel {
                                                                    state.request_load_schema_ref(SchemaRef::File(path));
                                                                } else {
                                                                    manual_uri.set(SchemaRef::File(path.clone()).to_string());
                                                                    selected_yaml.set(Some(path));
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
                                                            "[{badge}]"
                                                        }
                                                        label {
                                                            width: "430",
                                                            font_size: "12",
                                                            font_weight: if entry.kind.is_navigable() { "bold" } else { "normal" },
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

                    // Manual URI / Open bar
                    rect {
                        width: "fill",
                        height: "32",
                        direction: "horizontal",
                        cross_align: "center",
                        spacing: "8",

                        label {
                            font_size: "11",
                            color: theme::TEXT_MUTED,
                            "URI / Path:"
                        }
                        StyledInput {
                            width: Some("360".to_owned()),
                            value: manual_uri.read().clone(),
                            placeholder: Some("builtin:Alo or path/to/schema.yaml".to_owned()),
                            onchange: move |txt| manual_uri.set(txt),
                        }
                        ActionButton {
                            label: "Open Schema".to_owned(),
                            style: ButtonStyle::Primary,
                            onpress: move |_| {
                                let raw = manual_uri.read().trim().to_owned();
                                if let Some(path) = selected_yaml.read().clone()
                                    && raw.is_empty()
                                {
                                    state.request_load_schema_ref(SchemaRef::File(path));
                                    return;
                                }
                                match SchemaRef::parse(&raw) {
                                    Ok(sref) => state.request_load_schema_ref(sref),
                                    Err(e) => state.set_status(format!("Invalid schema reference: {e}")),
                                }
                            },
                        }
                    }
                }
            }
        }
    }
}
