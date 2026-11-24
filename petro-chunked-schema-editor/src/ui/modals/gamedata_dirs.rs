//! Freestanding `GameData` directory management window with a visual directory picker.

use std::path::PathBuf;

use freya::prelude::*;

use crate::browser::{
    BrowseLocation, BrowseOptions, default_quick_places, list_location, location_display,
};
use crate::gamedata::detect_default_gamedata_dirs;
use crate::ui::context::EditorState;
use crate::ui::theme;
use crate::ui::widgets::{ButtonStyle, action_button, group_caption};

use super::{browser_nav_bar, quick_places_sidebar};

/// Freestanding window for adding, auto-detecting, and removing Petroglyph `GameData` directories
/// using a visual filesystem folder browser.
#[derive(PartialEq)]
pub struct GameDataDirsWindow;

impl Component for GameDataDirsWindow {
    fn render(&self) -> impl IntoElement {
        let mut state = EditorState::current();
        let dirs = state.persisted.read().game_data_dirs.clone();

        let mut browse_loc = use_state(|| {
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

        let configured_rows: Vec<Element> = dirs
            .iter()
            .enumerate()
            .map(|(idx, d)| {
                let disp = d.display().to_string();
                let jump_dir = d.clone();
                let mut bl = browse_loc;
                let mut st = state;

                rect()
                    .width(Size::fill())
                    .height(Size::px(28.0))
                    .padding((0.0, 8.0))
                    .corner_radius(4.0)
                    .background(theme::BG_INSET)
                    .horizontal()
                    .cross_align(Alignment::Center)
                    .spacing(8.0)
                    .overflow(Overflow::Clip)
                    .child(
                        label()
                            .width(Size::px(670.0))
                            .font_size(12.0)
                            .color(theme::ACCENT_ASCII)
                            .max_lines(1)
                            .text(disp),
                    )
                    .child(action_button("Browse", ButtonStyle::Secondary, true, move || {
                        if jump_dir.is_dir() {
                            bl.set(BrowseLocation::DiskDir(jump_dir.clone()));
                        }
                    }))
                    .child(action_button("Remove", ButtonStyle::Danger, true, move || {
                        let mut p = st.persisted.write();
                        if idx < p.game_data_dirs.len() {
                            p.game_data_dirs.remove(idx);
                            p.save();
                        }
                    }))
                    .into()
            })
            .collect();

        let dir_list_el: Element = match listing {
            Ok(entries) if entries.is_empty() => rect()
                .width(Size::fill())
                .padding(12.0)
                .child(
                    label()
                        .font_size(12.0)
                        .color(theme::TEXT_MUTED)
                        .text(format!(
                            "No subdirectories inside {}.",
                            location_display(&current_loc)
                        )),
                )
                .into(),
            Ok(entries) => {
                let rows: Vec<Element> = entries
                    .into_iter()
                    .map(|entry| {
                        let next_loc = entry.target_location.clone();
                        let dir_path = entry.disk_path.clone();
                        let mut bl = browse_loc;
                        let mut st = state;

                        rect()
                            .width(Size::fill())
                            .height(Size::px(28.0))
                            .padding((0.0, 8.0))
                            .corner_radius(3.0)
                            .background(theme::BG_ROW)
                            .horizontal()
                            .cross_align(Alignment::Center)
                            .spacing(8.0)
                            .overflow(Overflow::Clip)
                            .on_pointer_down(move |_| {
                                if let Some(loc) = next_loc.clone() {
                                    bl.set(loc);
                                }
                            })
                            .child(
                                label()
                                    .width(Size::px(44.0))
                                    .font_size(10.0)
                                    .font_weight(FontWeight::BOLD)
                                    .font_family("monospace")
                                    .color(theme::ACCENT_BLUE)
                                    .max_lines(1)
                                    .text("[DIR]"),
                            )
                            .child(
                                label()
                                    .width(Size::px(510.0))
                                    .font_size(12.0)
                                    .font_weight(FontWeight::BOLD)
                                    .color(theme::TEXT_PRIMARY)
                                    .max_lines(1)
                                    .text(entry.name),
                            )
                            .maybe_child(dir_path.map(|add_path| {
                                action_button("+ Add", ButtonStyle::Secondary, true, move || {
                                    {
                                        let mut p = st.persisted.write();
                                        if !p.game_data_dirs.contains(&add_path) {
                                            p.game_data_dirs.push(add_path.clone());
                                            p.save();
                                        }
                                    }
                                    st.set_status(format!(
                                        "Added GameData directory {}",
                                        add_path.display()
                                    ));
                                })
                            }))
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

        let nav_handler = move |next: BrowseLocation| browse_loc.set(next);
        let mut st_detect = state;
        let mut st_add_cur = state;
        let add_cur_dir = current_disk_dir.clone();

        rect()
            .width(Size::fill())
            .height(Size::fill())
            .content(Content::Flex)
            .vertical()
            .spacing(10.0)
            .child(
                rect()
                    .width(Size::fill())
                    .padding((8.0, 10.0))
                    .corner_radius(5.0)
                    .background(theme::BG_SUB_CARD)
                    .border(theme::border_all(theme::BORDER_CARD))
                    .vertical()
                    .spacing(6.0)
                    .child(
                        rect()
                            .width(Size::fill())
                            .horizontal()
                            .main_align(Alignment::SpaceBetween)
                            .cross_align(Alignment::Center)
                            .child(group_caption(format!(
                                "CONFIGURED DIRECTORIES ({})",
                                dirs.len()
                            )))
                            .child(action_button(
                                "Auto-Detect EAW / FoC",
                                ButtonStyle::Secondary,
                                true,
                                move || {
                                    let found = detect_default_gamedata_dirs();
                                    let mut added = 0usize;
                                    {
                                        let mut p = st_detect.persisted.write();
                                        for d in found {
                                            if !p.game_data_dirs.contains(&d) {
                                                p.game_data_dirs.push(d);
                                                added += 1;
                                            }
                                        }
                                        p.save();
                                    }
                                    st_detect.set_status(format!(
                                        "Auto-detected {added} new GameData directory(s)"
                                    ));
                                },
                            )),
                    )
                    .child(if dirs.is_empty() {
                        label()
                            .font_size(12.0)
                            .color(theme::TEXT_MUTED)
                            .text("No GameData directories configured yet. Browse to a GameData/Data folder below and click Add.")
                            .into_element()
                    } else {
                        ScrollView::new()
                            .width(Size::fill())
                            .height(Size::px(110.0))
                            .child(
                                rect()
                                    .width(Size::fill())
                                    .vertical()
                                    .spacing(3.0)
                                    .children(configured_rows),
                            )
                            .into_element()
                    }),
            )
            .child(group_caption("PICK FOLDER TO ADD"))
            .child(browser_nav_bar(&current_loc, nav_handler))
            .child(
                rect()
                    .width(Size::fill())
                    .height(Size::flex(1.0))
                    .content(Content::Flex)
                    .horizontal()
                    .spacing(10.0)
                    .child(quick_places_sidebar(places, &current_loc, nav_handler))
                    .child(
                        rect()
                            .width(Size::flex(1.0))
                            .height(Size::fill())
                            .content(Content::Flex)
                            .padding(6.0)
                            .corner_radius(5.0)
                            .background(theme::BG_INSET)
                            .border(theme::border_all(theme::BORDER_CARD))
                            .vertical()
                            .spacing(4.0)
                            .child(dir_list_el),
                    ),
            )
            .child(
                rect()
                    .width(Size::fill())
                    .height(Size::px(32.0))
                    .horizontal()
                    .main_align(Alignment::SpaceBetween)
                    .cross_align(Alignment::Center)
                    .spacing(10.0)
                    .child(
                        label()
                            .font_size(11.0)
                            .color(theme::TEXT_MUTED)
                            .max_lines(1)
                            .text(format!("Current folder: {}", current_disk_dir.display())),
                    )
                    .child(
                        rect()
                            .horizontal()
                            .spacing(8.0)
                            .cross_align(Alignment::Center)
                            .child(action_button(
                                if already_added {
                                    "Current Folder Already Added"
                                } else {
                                    "+ Add Current Folder"
                                },
                                if already_added {
                                    ButtonStyle::Secondary
                                } else {
                                    ButtonStyle::Primary
                                },
                                false,
                                move || {
                                    if !already_added {
                                        let target = add_cur_dir.clone();
                                        {
                                            let mut p = st_add_cur.persisted.write();
                                            p.game_data_dirs.push(target.clone());
                                            p.save();
                                        }
                                        st_add_cur.set_status(format!(
                                            "Added GameData directory {}",
                                            target.display()
                                        ));
                                    }
                                },
                            ))
                            .child(action_button("Done", ButtonStyle::Secondary, false, move || {
                                state.close_current_modal_window();
                            })),
                    ),
            )
    }
}
