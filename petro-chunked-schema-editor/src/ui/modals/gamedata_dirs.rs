//! `GameData` directory configuration drawer modal.

use std::path::PathBuf;

use freya::prelude::*;

use crate::gamedata::detect_default_gamedata_dirs;
use crate::ui::context::EditorState;
use crate::ui::theme;
use crate::ui::widgets::{ActionButton, ButtonStyle, StyledInput};

use super::ModalFrame;

/// Modal drawer for adding, auto-detecting, and removing `GameData` directories used by the file
/// browser and batch validator.
#[component]
pub fn GameDataDirsModal() -> Element {
    let mut state = use_context::<EditorState>();
    let mut new_dir_input = use_signal(String::new);
    let dirs = state.persisted.read().game_data_dirs.clone();

    rsx! {
        ModalFrame {
            title: "Configured GameData Directories (scans loose files and .meg archives)".to_owned(),
            height: "245".to_owned(),

            rect {
                direction: "horizontal",
                spacing: "8",
                cross_align: "center",
                StyledInput {
                    width: Some("360".to_owned()),
                    value: new_dir_input.read().clone(),
                    placeholder: Some("/path/to/GameData/Data".to_owned()),
                    onchange: move |txt| new_dir_input.set(txt),
                }
                ActionButton {
                    label: "+ Add Directory".to_owned(),
                    style: ButtonStyle::Primary,
                    onpress: move |_| {
                        let raw = new_dir_input.read().trim().to_owned();
                        if !raw.is_empty() {
                            let pb = PathBuf::from(&raw);
                            {
                                let mut p = state.persisted.write();
                                if !p.game_data_dirs.contains(&pb) {
                                    p.game_data_dirs.push(pb);
                                    p.save();
                                }
                            }
                            new_dir_input.set(String::new());
                            state.set_status(format!("Added GameData directory `{raw}`"));
                        }
                    },
                }
                ActionButton {
                    label: "+ Auto-Detect Local EAW/FoC Data".to_owned(),
                    style: ButtonStyle::Secondary,
                    onpress: move |_| {
                        let detected = detect_default_gamedata_dirs();
                        let mut added = 0usize;
                        {
                            let mut p = state.persisted.write();
                            for pb in detected {
                                if !p.game_data_dirs.contains(&pb) {
                                    p.game_data_dirs.push(pb);
                                    added += 1;
                                }
                            }
                            p.save();
                        }
                        state.set_status(format!("Auto-detected {added} new GameData directory(ies)"));
                    },
                }
            }

            ScrollView {
                width: "fill",
                height: "flex(1)",
                rect {
                    width: "fill",
                    direction: "vertical",
                    spacing: "3",
                    for (idx, dir) in dirs.iter().enumerate() {
                        {
                            let dir_str = dir.display().to_string();
                            rsx! {
                                rect {
                                    width: "fill",
                                    padding: "4 8",
                                    corner_radius: "4",
                                    background: theme::BG_INSET,
                                    direction: "horizontal",
                                    spacing: "10",
                                    cross_align: "center",
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
                                    label { font_size: "12", color: theme::TEXT_PRIMARY, "{dir_str}" }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
