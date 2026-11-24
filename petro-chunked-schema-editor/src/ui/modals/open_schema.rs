//! Unified freestanding Open Schema window combining built-in Petroglyph formats, blank schema
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
use crate::ui::widgets::{ButtonStyle, action_button, group_caption, styled_input};

use super::browser_nav_bar;

fn builtin_description(name: &str) -> &'static str {
    match name {
        "Alo" => "Combined ALO dispatcher (models + particle systems)",
        "AloModel" => "3D skeletons, meshes, shaders, proxies, and lights",
        "AloParticle" => "Particle emitter systems and simulation graphs",
        "AlaAnimation" => "Bone keyframe animations and compressed tracks",
        _ => "Embedded Petroglyph format schema",
    }
}

/// Freestanding window for selecting a built-in schema, starting a blank schema, or browsing disk
/// for a `.yaml` / `.yml` schema file. All actions pass through the unsaved-changes guard.
#[derive(PartialEq)]
pub struct OpenSchemaWindow;

impl Component for OpenSchemaWindow {
    fn render(&self) -> impl IntoElement {
        let state = EditorState::current();
        let current_ref = state.persisted.read().schema_ref.clone();
        let game_data_dirs = state.persisted.read().game_data_dirs.clone();

        let mut browse_loc = use_state(|| {
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
        let mut selected_yaml = use_state(|| None::<PathBuf>);
        let manual_uri = use_state(|| {
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

        let builtin_cards: Vec<Element> = known_formats::BUILTIN_NAMES
            .iter()
            .map(|&name| {
                let target_ref = SchemaRef::Builtin(name.to_owned());
                let is_active = current_ref.as_ref() == Some(&target_ref);
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
                let desc = builtin_description(name);
                let mut st = state;

                rect()
                    .key(name)
                    .width(Size::fill())
                    .padding((8.0, 10.0))
                    .corner_radius(5.0)
                    .background(bg)
                    .border(border)
                    .vertical()
                    .spacing(3.0)
                    .on_pointer_down(move |_| st.request_load_schema_ref(target_ref.clone()))
                    .child(
                        rect()
                            .width(Size::fill())
                            .horizontal()
                            .main_align(Alignment::SpaceBetween)
                            .cross_align(Alignment::Center)
                            .child(
                                label()
                                    .font_size(12.0)
                                    .font_weight(FontWeight::BOLD)
                                    .color(theme::TEXT_PRIMARY)
                                    .max_lines(1)
                                    .text(name),
                            )
                            .child(
                                label()
                                    .font_size(10.0)
                                    .font_family("monospace")
                                    .color(if is_active {
                                        theme::ACCENT_GREEN
                                    } else {
                                        theme::ACCENT_BLUE
                                    })
                                    .max_lines(1)
                                    .text(if is_active {
                                        "active".to_owned()
                                    } else {
                                        format!("builtin:{name}")
                                    }),
                            ),
                    )
                    .child(
                        label()
                            .font_size(11.0)
                            .color(theme::TEXT_MUTED)
                            .text(desc),
                    )
                    .into()
            })
            .collect();

        let place_btns: Vec<Element> = places
            .into_iter()
            .take(4)
            .map(|place| {
                let target = place.location.clone();
                let is_cur = target == current_loc;
                let mut bl = browse_loc;
                let mut sel = selected_yaml;
                action_button(
                    place.label,
                    if is_cur {
                        ButtonStyle::Active
                    } else {
                        ButtonStyle::Secondary
                    },
                    true,
                    move || {
                        sel.set(None);
                        bl.set(target.clone());
                    },
                )
            })
            .collect();

        let list_body: Element = match listing {
            Ok(entries) if entries.is_empty() => rect()
                .width(Size::fill())
                .padding(14.0)
                .child(
                    label()
                        .font_size(12.0)
                        .color(theme::TEXT_MUTED)
                        .text(format!(
                            "No .yaml/.yml files or folders in {}.",
                            location_display(&current_loc)
                        )),
                )
                .into(),
            Ok(entries) => {
                let rows: Vec<Element> = entries
                    .into_iter()
                    .map(|entry| {
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
                        let is_nav = entry.kind.is_navigable();
                        let mut st = state;
                        let mut bl = browse_loc;
                        let mut sel = selected_yaml;
                        let mut uri = manual_uri;

                        rect()
                            .width(Size::fill())
                            .height(Size::px(28.0))
                            .padding((0.0, 8.0))
                            .corner_radius(3.0)
                            .background(bg)
                            .horizontal()
                            .cross_align(Alignment::Center)
                            .spacing(8.0)
                            .overflow(Overflow::Clip)
                            .on_pointer_down(move |_| {
                                if let Some(loc) = next_loc.clone() {
                                    sel.set(None);
                                    bl.set(loc);
                                } else if let Some(path) = disk_p.clone() {
                                    if is_sel {
                                        st.request_load_schema_ref(SchemaRef::File(path));
                                    } else {
                                        uri.set(SchemaRef::File(path.clone()).to_string());
                                        sel.set(Some(path));
                                    }
                                }
                            })
                            .child(
                                label()
                                    .width(Size::px(44.0))
                                    .font_size(10.0)
                                    .font_weight(FontWeight::BOLD)
                                    .font_family("monospace")
                                    .color(badge_color)
                                    .max_lines(1)
                                    .text(format!("[{badge}]")),
                            )
                            .child(
                                label()
                                    .width(Size::px(420.0))
                                    .font_size(12.0)
                                    .font_weight(if is_nav {
                                        FontWeight::BOLD
                                    } else {
                                        FontWeight::NORMAL
                                    })
                                    .color(theme::TEXT_PRIMARY)
                                    .max_lines(1)
                                    .text(entry.name),
                            )
                            .child(
                                label()
                                    .width(Size::px(70.0))
                                    .font_size(11.0)
                                    .font_family("monospace")
                                    .color(theme::TEXT_DIM)
                                    .max_lines(1)
                                    .text(size_txt),
                            )
                            .into()
                    })
                    .collect();

                ScrollView::new()
                    .width(Size::fill())
                    .height(Size::flex(1.0))
                    .child(rect().width(Size::fill()).vertical().spacing(2.0).children(rows))
                    .into()
            }
            Err(err) => label()
                .font_size(12.0)
                .color(theme::ACCENT_RED)
                .text(err)
                .into(),
        };

        let mut st_blank = state;
        let mut st_open = state;
        let mut st_close = state;

        rect()
            .width(Size::fill())
            .height(Size::fill())
            .content(Content::Flex)
            .horizontal()
            .spacing(12.0)
            // Left column: Built-in Schemas & New Blank
            .child(
                rect()
                    .width(Size::px(280.0))
                    .height(Size::fill())
                    .content(Content::Flex)
                    .padding(10.0)
                    .corner_radius(5.0)
                    .background(theme::BG_SUB_CARD)
                    .border(theme::border_all(theme::BORDER_CARD))
                    .vertical()
                    .spacing(8.0)
                    .child(group_caption("BUILT-IN FORMATS"))
                    .child(
                        ScrollView::new()
                            .width(Size::fill())
                            .height(Size::flex(1.0))
                            .child(
                                rect()
                                    .width(Size::fill())
                                    .vertical()
                                    .spacing(6.0)
                                    .children(builtin_cards),
                            ),
                    )
                    .child(
                        rect()
                            .width(Size::fill())
                            .padding((8.0, 0.0, 0.0, 0.0))
                            .border(theme::border_top(theme::BORDER_CARD))
                            .vertical()
                            .spacing(6.0)
                            .child(group_caption("SCRATCHPAD"))
                            .child(action_button(
                                "+ New Blank Schema",
                                ButtonStyle::Secondary,
                                false,
                                move || st_blank.request_new_blank_schema(),
                            )),
                    ),
            )
            // Right column: YAML File Browser & manual URI bar
            .child(
                rect()
                    .width(Size::flex(1.0))
                    .height(Size::fill())
                    .content(Content::Flex)
                    .vertical()
                    .spacing(8.0)
                    .child(
                        rect()
                            .width(Size::fill())
                            .vertical()
                            .spacing(5.0)
                            .child(group_caption("YAML SCHEMA FILES ON DISK"))
                            .child(
                                rect()
                                    .width(Size::fill())
                                    .horizontal()
                                    .cross_align(Alignment::Center)
                                    .spacing(5.0)
                                    .overflow(Overflow::Clip)
                                    .children(place_btns),
                            ),
                    )
                    .child(browser_nav_bar(&current_loc, move |next| {
                        selected_yaml.set(None);
                        browse_loc.set(next);
                    }))
                    .child(
                        rect()
                            .width(Size::fill())
                            .height(Size::flex(1.0))
                            .content(Content::Flex)
                            .padding(6.0)
                            .corner_radius(5.0)
                            .background(theme::BG_INSET)
                            .border(theme::border_all(theme::BORDER_CARD))
                            .vertical()
                            .spacing(4.0)
                            .child(list_body),
                    )
                    .child(
                        rect()
                            .width(Size::fill())
                            .height(Size::px(32.0))
                            .horizontal()
                            .cross_align(Alignment::Center)
                            .spacing(8.0)
                            .child(
                                label()
                                    .font_size(11.0)
                                    .color(theme::TEXT_MUTED)
                                    .text("URI / Path:"),
                            )
                            .child(styled_input(
                                Size::px(330.0),
                                manual_uri,
                                "builtin:Alo or path/to/schema.yaml",
                            ))
                            .child(action_button("Open Schema", ButtonStyle::Primary, false, move || {
                                let raw = manual_uri.read().trim().to_owned();
                                if let Some(path) = selected_yaml.read().clone()
                                    && raw.is_empty()
                                {
                                    st_open.request_load_schema_ref(SchemaRef::File(path));
                                    return;
                                }
                                match SchemaRef::parse(&raw) {
                                    Ok(sref) => st_open.request_load_schema_ref(sref),
                                    Err(e) => st_open
                                        .set_status(format!("Invalid schema reference: {e}")),
                                }
                            }))
                            .child(action_button("Cancel", ButtonStyle::Secondary, false, move || {
                                st_close.close_current_modal_window();
                            })),
                    ),
            )
    }
}
