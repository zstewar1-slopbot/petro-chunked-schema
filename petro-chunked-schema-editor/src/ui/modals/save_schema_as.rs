//! Freestanding Save Schema As window with a visual directory & `.yaml` file browser.

use std::path::PathBuf;

use freya::prelude::*;
use petro_chunked_schema::source::SchemaRef;

use crate::browser::{
    BrowseEntryKind, BrowseLocation, BrowseOptions, default_quick_places, format_size_compact,
    list_location,
};
use crate::ui::context::EditorState;
use crate::ui::theme;
use crate::ui::widgets::{ButtonStyle, action_button, styled_input};

use super::{browser_nav_bar, quick_places_sidebar};

/// Freestanding window allowing the user to pick a directory and filename to save the active
/// schema as a `.yaml` file (`Ctrl+Shift+S`, or `Ctrl+S` on built-in/unsaved schemas).
#[derive(PartialEq)]
pub struct SaveSchemaAsWindow;

impl Component for SaveSchemaAsWindow {
    fn render(&self) -> impl IntoElement {
        let state = EditorState::current();
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

        let mut location = use_state(|| BrowseLocation::DiskDir(init_dir));
        let filename = use_state(|| init_filename);

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

        let list_body: Element = match listing {
            Ok(entries) if entries.is_empty() => rect()
                .width(Size::fill())
                .padding(14.0)
                .child(
                    label()
                        .font_size(12.0)
                        .color(theme::TEXT_MUTED)
                        .text("No existing .yaml files or subdirectories in this folder."),
                )
                .into(),
            Ok(entries) => {
                let rows: Vec<Element> = entries
                    .into_iter()
                    .map(|entry| {
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
                        let mut loc_st = location;
                        let mut fn_st = filename;

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
                                    loc_st.set(loc);
                                } else {
                                    fn_st.set(entry_name.clone());
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
                                    .width(Size::px(480.0))
                                    .font_size(12.0)
                                    .font_weight(if is_dir {
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

        let nav_handler = move |next: BrowseLocation| location.set(next);
        let mut st_cancel = state;
        let mut st_save = state;

        rect()
            .width(Size::fill())
            .height(Size::fill())
            .content(Content::Flex)
            .vertical()
            .spacing(10.0)
            .maybe_child(pending_note.map(|note| {
                label()
                    .font_size(11.0)
                    .color(theme::ACCENT_AMBER)
                    .text(note)
            }))
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
                            .child(list_body),
                    ),
            )
            .child(
                rect()
                    .width(Size::fill())
                    .height(Size::px(34.0))
                    .horizontal()
                    .main_align(Alignment::SpaceBetween)
                    .cross_align(Alignment::Center)
                    .spacing(10.0)
                    .child(
                        rect()
                            .horizontal()
                            .cross_align(Alignment::Center)
                            .spacing(8.0)
                            .child(
                                label()
                                    .font_size(12.0)
                                    .color(theme::TEXT_MUTED)
                                    .text("Filename:"),
                            )
                            .child(styled_input(Size::px(360.0), filename, "my_format.yaml")),
                    )
                    .child(
                        rect()
                            .horizontal()
                            .cross_align(Alignment::Center)
                            .spacing(8.0)
                            .child(action_button("Cancel", ButtonStyle::Secondary, false, move || {
                                st_cancel.cancel_pending_action();
                            }))
                            .child(action_button(
                                "Save Schema",
                                ButtonStyle::Primary,
                                false,
                                move || {
                                    let raw_name = filename.read().trim().to_owned();
                                    if raw_name.is_empty() {
                                        st_save.set_status(
                                            "Enter a filename before saving.".to_owned(),
                                        );
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
                                    st_save.save_schema_as(target_path);
                                },
                            )),
                    ),
            )
    }
}
