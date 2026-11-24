//! Freestanding hierarchical file browser window with native browse-into-`.meg` archive support.

use freya::prelude::*;

use crate::browser::{
    BrowseEntryKind, BrowseLocation, BrowseOptions, default_quick_places, format_size_compact,
    initial_browse_location, list_location, location_display,
};
use crate::state::FileSource;
use crate::ui::context::EditorState;
use crate::ui::theme;
use crate::ui::widgets::{ButtonStyle, action_button, segmented_chip, styled_input};

use super::{browser_nav_bar, quick_places_sidebar};

const EXT_PRESETS: [(&str, Option<&str>); 5] = [
    ("All", None),
    (".alo", Some("alo")),
    (".ala", Some("ala")),
    (".ted", Some("ted")),
    (".bui", Some("bui")),
];

/// Freestanding window for browsing local directories and stepping inside Petroglyph `.meg`
/// archives as virtual folders to open game files.
#[derive(PartialEq)]
pub struct FileBrowserWindow;

impl Component for FileBrowserWindow {
    fn render(&self) -> impl IntoElement {
        let state = EditorState::current();
        let game_data_dirs = state.persisted.read().game_data_dirs.clone();
        let active_file = state.persisted.read().last_file.clone();

        let mut location = use_state(|| initial_browse_location(&game_data_dirs, active_file.as_ref()));
        let ext_filter = use_state(|| Some("alo".to_owned()));
        let name_filter = use_state(String::new);
        let mut recursive_in_meg = use_state(|| true);
        let mut selected_source = use_state(|| None::<(FileSource, String)>);

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

        let ext_chips: Vec<Element> = EXT_PRESETS
            .into_iter()
            .map(|(preset_label, preset_ext)| {
                let is_sel = ext_filter.read().as_deref() == preset_ext;
                let target_ext = preset_ext.map(str::to_owned);
                let mut ef = ext_filter;
                let mut sel = selected_source;
                segmented_chip(preset_label, is_sel, true, move || {
                    ef.set(target_ext.clone());
                    sel.set(None);
                })
            })
            .collect();

        let list_body: Element = match listing {
            Ok(entries) if entries.is_empty() => rect()
                .width(Size::fill())
                .padding(16.0)
                .child(
                    label()
                        .font_size(12.0)
                        .color(theme::TEXT_MUTED)
                        .text(format!(
                            "No matching files or folders in {}.",
                            location_display(&current_loc)
                        )),
                )
                .into(),
            Ok(entries) => {
                let rows: Vec<Element> = entries
                    .into_iter()
                    .map(|entry| {
                        let is_sel = entry.file_source.is_some()
                            && selected_now.as_ref().map(|(s, _)| s) == entry.file_source.as_ref();
                        let bg = if is_sel {
                            theme::BG_ROW_SELECTED
                        } else {
                            theme::BG_ROW
                        };
                        let badge_color = match entry.kind {
                            BrowseEntryKind::Directory | BrowseEntryKind::MegVirtualDir => {
                                theme::ACCENT_BLUE
                            }
                            BrowseEntryKind::MegArchive => theme::ACCENT_AMBER,
                            BrowseEntryKind::DiskFile | BrowseEntryKind::MegFile => {
                                theme::ACCENT_GREEN
                            }
                        };
                        let name_color = match entry.kind {
                            BrowseEntryKind::MegArchive => theme::ACCENT_AMBER,
                            _ => theme::TEXT_PRIMARY,
                        };
                        let size_txt = entry
                            .size
                            .map(format_size_compact)
                            .unwrap_or_else(|| "—".to_owned());
                        let target_loc = entry.target_location.clone();
                        let file_src = entry.file_source.clone();
                        let entry_name = entry.name.clone();
                        let badge_txt = entry.kind.badge();
                        let is_nav = entry.kind.is_navigable();
                        let mut st = state;
                        let mut loc_st = location;
                        let mut sel_st = selected_source;

                        rect()
                            .width(Size::fill())
                            .height(Size::px(28.0))
                            .padding((0.0, 6.0))
                            .corner_radius(3.0)
                            .background(bg)
                            .horizontal()
                            .cross_align(Alignment::Center)
                            .spacing(8.0)
                            .overflow(Overflow::Clip)
                            .on_pointer_down(move |_| {
                                if let Some(next_loc) = target_loc.clone() {
                                    sel_st.set(None);
                                    loc_st.set(next_loc);
                                } else if let Some(src) = file_src.clone() {
                                    if is_sel {
                                        st.open_file(src);
                                        st.close_current_modal_window();
                                    } else {
                                        sel_st.set(Some((src, entry_name.clone())));
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
                                    .text(format!("[{badge_txt}]")),
                            )
                            .child(
                                label()
                                    .width(Size::px(540.0))
                                    .font_size(12.0)
                                    .font_weight(if is_nav {
                                        FontWeight::BOLD
                                    } else {
                                        FontWeight::NORMAL
                                    })
                                    .color(name_color)
                                    .max_lines(1)
                                    .text(entry.name),
                            )
                            .child(
                                label()
                                    .width(Size::px(80.0))
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
            Err(err) => rect()
                .width(Size::fill())
                .padding(12.0)
                .child(label().font_size(12.0).color(theme::ACCENT_RED).text(err))
                .into(),
        };

        let nav_handler = move |next: BrowseLocation| {
            selected_source.set(None);
            location.set(next);
        };

        let mut st_cancel = state;
        let mut st_open = state;

        rect()
            .width(Size::fill())
            .height(Size::fill())
            .content(Content::Flex)
            .vertical()
            .spacing(10.0)
            .child(browser_nav_bar(&current_loc, nav_handler))
            .child(
                rect()
                    .width(Size::fill())
                    .horizontal()
                    .cross_align(Alignment::Center)
                    .spacing(8.0)
                    .child(
                        label()
                            .font_size(11.0)
                            .color(theme::TEXT_MUTED)
                            .text("Type:"),
                    )
                    .children(ext_chips)
                    .child(
                        label()
                            .font_size(11.0)
                            .color(theme::TEXT_MUTED)
                            .text("Filter:"),
                    )
                    .child(styled_input(
                        Size::px(220.0),
                        name_filter,
                        "name substring (e.g. tiefighter)...",
                    ))
                    .maybe_child(in_meg.then(|| {
                        let cur = *recursive_in_meg.read();
                        segmented_chip("Search Inside .meg", cur, true, move || {
                            recursive_in_meg.set(!cur);
                        })
                    }))
                    .child(
                        label()
                            .font_size(11.0)
                            .color(theme::TEXT_DIM)
                            .max_lines(1)
                            .text(format!("{entry_count} item(s)")),
                    ),
            )
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
                            .child(
                                rect()
                                    .width(Size::fill())
                                    .height(Size::px(22.0))
                                    .padding((0.0, 6.0))
                                    .border(theme::border_bottom(theme::BORDER_CARD))
                                    .horizontal()
                                    .cross_align(Alignment::Center)
                                    .spacing(8.0)
                                    .overflow(Overflow::Clip)
                                    .child(hdr(44.0, "KIND"))
                                    .child(hdr(540.0, "NAME"))
                                    .child(hdr(80.0, "SIZE")),
                            )
                            .child(list_body),
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
                    .child(if let Some((_, sel_label)) = &selected_now {
                        label()
                            .font_size(12.0)
                            .color(theme::ACCENT_GREEN)
                            .max_lines(1)
                            .text(format!(
                                "Selected: {sel_label}  (click again or press Open File)"
                            ))
                    } else {
                        label()
                            .font_size(11.0)
                            .color(theme::TEXT_DIM)
                            .max_lines(1)
                            .text("Click a folder or .meg archive to browse inside; click a file to select.")
                    })
                    .child(
                        rect()
                            .horizontal()
                            .spacing(8.0)
                            .cross_align(Alignment::Center)
                            .child(action_button("Cancel", ButtonStyle::Secondary, false, move || {
                                st_cancel.close_current_modal_window();
                            }))
                            .maybe_child(selected_now.map(|(src, _)| {
                                action_button("Open File", ButtonStyle::Primary, false, move || {
                                    st_open.open_file(src.clone());
                                    st_open.close_current_modal_window();
                                })
                            })),
                    ),
            )
    }
}

fn hdr(w: f32, text: &'static str) -> Element {
    label()
        .width(Size::px(w))
        .font_size(10.0)
        .font_weight(FontWeight::BOLD)
        .color(theme::TEXT_DIM)
        .max_lines(1)
        .text(text)
        .into()
}
