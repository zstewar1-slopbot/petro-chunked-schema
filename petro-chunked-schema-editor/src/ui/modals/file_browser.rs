//! File and Petroglyph `.meg` archive browser drawer modal.

use std::path::PathBuf;

use freya::prelude::*;

use crate::gamedata::discover_files;
use crate::state::FileSource;
use crate::ui::context::EditorState;
use crate::ui::theme;

use super::ModalFrame;

/// Modal drawer allowing users to open a loose file by path or search `.meg` archives inside
/// configured `GameData` directories.
#[component]
pub fn FileBrowserModal() -> Element {
    let mut state = use_context::<EditorState>();

    let initial_disk = use_hook(|| match &state.persisted.read().last_file {
        Some(FileSource::Disk { path }) => path.display().to_string(),
        _ => String::new(),
    });
    let mut disk_file_input = use_signal(|| initial_disk);
    let mut ext_filter = use_signal(|| "alo".to_owned());
    let mut search_query = use_signal(String::new);
    let initial_files = use_hook(|| {
        let dirs = state.persisted.read().game_data_dirs.clone();
        discover_files(&dirs, Some("alo"), "", 150)
    });
    let mut discovered_files = use_signal(|| initial_files);

    let mut run_search = move |ext_str: &str, query: &str| {
        let dirs = state.persisted.read().game_data_dirs.clone();
        let ext_opt = if ext_str.is_empty() || ext_str == "all" {
            None
        } else {
            Some(ext_str)
        };
        discovered_files.set(discover_files(&dirs, ext_opt, query, 150));
    };

    rsx! {
        ModalFrame {
            title: "Open File from Disk or Petroglyph .meg Archive".to_owned(),
            height: "290".to_owned(),

            rect {
                direction: "horizontal",
                spacing: "8",
                cross_align: "center",
                Input {
                    value: disk_file_input.read().clone(),
                    placeholder: "/path/to/file.alo or .ala",
                    onchange: move |txt| disk_file_input.set(txt),
                }
                Button {
                    onpress: move |_| {
                        let p = PathBuf::from(disk_file_input.read().trim());
                        if !p.as_os_str().is_empty() {
                            state.open_file(FileSource::Disk { path: p });
                            state.close_modal();
                        }
                    },
                    label { "Open Disk Path" }
                }
            }

            rect {
                direction: "horizontal",
                spacing: "8",
                cross_align: "center",
                label { font_size: "12", "Filter GameData & .meg Archives:" }
                Input {
                    value: search_query.read().clone(),
                    placeholder: "Search filename substring...",
                    onchange: move |txt| search_query.set(txt),
                }
                for ext_choice in ["alo", "ala", "ted", "all"] {
                    Button {
                        onpress: move |_| {
                            ext_filter.set(ext_choice.to_owned());
                            let q = search_query.read().clone();
                            run_search(ext_choice, &q);
                        },
                        label { ".{ext_choice}" }
                    }
                }
                Button {
                    onpress: move |_| {
                        let ext = ext_filter.read().clone();
                        let q = search_query.read().clone();
                        run_search(&ext, &q);
                    },
                    label { "Search" }
                }
            }

            ScrollView {
                width: "fill",
                height: "fill",
                if discovered_files.read().is_empty() {
                    label {
                        font_size: "12",
                        color: theme::TEXT_MUTED,
                        "No matching files found. Configure GameData directories under 'GameData Dirs...' to browse loose files and .meg archives."
                    }
                }
                for item in discovered_files.read().iter() {
                    {
                        let src = item.source.clone();
                        let disp = item.display_name.clone();
                        let sz = item.size;
                        rsx! {
                            rect {
                                width: "fill",
                                padding: "3 6",
                                direction: "horizontal",
                                spacing: "10",
                                cross_align: "center",
                                Button {
                                    onpress: move |_| {
                                        state.open_file(src.clone());
                                        state.close_modal();
                                    },
                                    label { "Load" }
                                }
                                label {
                                    font_size: "12",
                                    "{disp}  ({sz} bytes)"
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
