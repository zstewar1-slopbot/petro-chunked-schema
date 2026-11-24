//! Left pane: expandable chunk and mini-chunk tree view with right-rail hover match highlighting
//! plus per-file schema diagnostics.

use freya::prelude::*;
use petro_chunked_schema::dynamic::DynamicFile;

use crate::editor_ops::{FlatTreeRow, build_flat_tree};
use crate::schema_nav::matches_hover_target;
use crate::ui::context::EditorState;
use crate::ui::theme;
use crate::ui::widgets::{ActionButton, ButtonStyle};

/// Left-hand pane displaying the hierarchical chunk/mini-chunk tree and file schema diagnostics.
#[component]
pub fn ChunkTreePane() -> Element {
    let mut state = use_context::<EditorState>();
    let df_opt = state.dynamic_file.read().clone();
    let collapsed = state.collapsed_paths.read().clone();
    let selected_path = state.selected_tree_path.read().clone();
    let resolved = state.resolved_schema.read().clone();
    let hovered = state.hovered_schema_target.read().clone();

    let flat_tree = df_opt
        .as_ref()
        .map(|df| build_flat_tree(df, &collapsed, 600))
        .unwrap_or_default();

    let matched_count = hovered.as_ref().map(|target| {
        flat_tree
            .iter()
            .filter(|row| matches_hover_target(&resolved, row, target))
            .count()
    });

    rsx! {
        rect {
            width: "355",
            height: "fill",
            content: "flex",
            padding: "8",
            background: theme::BG_PANE_LEFT,
            direction: "vertical",
            spacing: "6",

            rect {
                width: "fill",
                direction: "horizontal",
                spacing: "8",
                cross_align: "center",
                label {
                    font_size: "13",
                    font_weight: "bold",
                    color: theme::ACCENT_BLUE,
                    "Chunk Tree ({flat_tree.len()})"
                }
                ActionButton {
                    label: "Expand All".to_owned(),
                    style: ButtonStyle::Secondary,
                    compact: Some(true),
                    onpress: move |_| state.expand_all(),
                }
                if let Some(count) = matched_count {
                    label {
                        font_size: "11",
                        font_weight: "bold",
                        color: if count > 0 { theme::ACCENT_BLUE } else { theme::TEXT_DIM },
                        max_lines: "1",
                        "{count} highlighted"
                    }
                }
            }

            ScrollView {
                width: "fill",
                height: "flex(1)",
                rect {
                    width: "fill",
                    direction: "vertical",
                    spacing: "2",

                    if flat_tree.is_empty() {
                        label {
                            font_size: "12",
                            color: theme::TEXT_MUTED,
                            "No file loaded. Click 'Open File / .meg...' above to open an .alo or .ala file."
                        }
                    }
                    for row in flat_tree {
                        {
                            let is_selected = row.tree_path == selected_path;
                            let is_hover_match = hovered
                                .as_ref()
                                .is_some_and(|target| matches_hover_target(&resolved, &row, target));
                            rsx! {
                                ChunkTreeRow {
                                    key: "{row.tree_path}",
                                    row,
                                    is_selected,
                                    is_hover_match,
                                }
                            }
                        }
                    }
                }
            }

            if let Some(df) = df_opt {
                DiagnosticsPanel { file: df }
            }
        }
    }
}

#[component]
fn ChunkTreeRow(row: FlatTreeRow, is_selected: bool, is_hover_match: bool) -> Element {
    let mut state = use_context::<EditorState>();
    let row_path = row.tree_path.clone();
    let row_path_toggle = row.tree_path.clone();

    let bg = if is_selected {
        theme::BG_ROW_SELECTED
    } else if is_hover_match {
        theme::BG_ROW_HOVER_MATCH
    } else if row.has_error {
        theme::BG_ROW_ERROR
    } else {
        theme::BG_ROW
    };
    let border = if is_hover_match {
        theme::BORDER_ROW_HOVER_MATCH
    } else {
        "1 solid transparent"
    };
    let indent_px = row.depth * 14;
    let id_color = if row.is_mini_chunk {
        theme::ACCENT_AMBER
    } else {
        theme::ACCENT_BLUE
    };
    let name_str = row
        .field_name
        .clone()
        .unwrap_or_else(|| "<unassigned>".to_owned());
    let name_color = if row.field_name.is_some() {
        theme::ACCENT_GREEN
    } else {
        theme::TEXT_MUTED
    };
    let type_suffix = row
        .type_label
        .as_ref()
        .map(|t| format!(": {t}"))
        .unwrap_or_default();
    let toggle_label = if row.child_count > 0 {
        if row.is_expanded { "[-]" } else { "[+]" }
    } else {
        "·"
    };
    let sz = row.size;
    let id_lbl = row.id_label.clone();

    rsx! {
        rect {
            width: "fill",
            overflow: "clip",
            padding: "3 6",
            corner_radius: "4",
            background: "{bg}",
            border: "{border}",
            direction: "horizontal",
            spacing: "5",
            cross_align: "center",

            if indent_px > 0 {
                rect { width: "{indent_px}" }
            }

            if row.child_count > 0 {
                rect {
                    padding: "1 5",
                    corner_radius: "3",
                    background: theme::BG_CARD,
                    onpointerdown: move |_| state.toggle_collapse(&row_path_toggle),
                    label {
                        font_size: "10",
                        font_family: "monospace",
                        color: theme::TEXT_PRIMARY,
                        "{toggle_label}"
                    }
                }
            } else {
                label {
                    font_size: "11",
                    color: theme::TEXT_DIM,
                    " · "
                }
            }

            rect {
                padding: "1 2",
                direction: "horizontal",
                spacing: "5",
                cross_align: "center",
                onpointerdown: move |_| state.select_node(row_path.clone()),

                label {
                    font_size: "11",
                    font_family: "monospace",
                    color: "{id_color}",
                    max_lines: "1",
                    "{id_lbl}"
                }
                label {
                    font_size: "11",
                    color: "{name_color}",
                    max_lines: "1",
                    "{name_str}{type_suffix} ({sz}B)"
                }
            }
        }
    }
}

#[component]
fn DiagnosticsPanel(file: DynamicFile) -> Element {
    let mut state = use_context::<EditorState>();
    let diag_count = file.diagnostics.len();

    rsx! {
        rect {
            width: "fill",
            height: "155",
            content: "flex",
            padding: "6",
            corner_radius: "4",
            background: theme::BG_INSET,
            direction: "vertical",
            spacing: "4",

            label {
                font_size: "12",
                font_weight: "bold",
                color: if diag_count == 0 { theme::ACCENT_GREEN } else { theme::ACCENT_RED },
                "File Schema Diagnostics ({diag_count})"
            }
            ScrollView {
                width: "fill",
                height: "flex(1)",
                if diag_count == 0 {
                    label {
                        font_size: "11",
                        color: theme::ACCENT_GREEN,
                        "All chunks and bytes in this file conform cleanly to the active schema!"
                    }
                }
                for diag in file.diagnostics.iter().take(60) {
                    {
                        let d_msg = diag.message.clone();
                        let d_path = diag.tree_path.clone();
                        let d_off = diag.file_offset;
                        let jump_path = if diag.tree_path == "root" {
                            "0".to_owned()
                        } else {
                            diag.tree_path.clone()
                        };
                        rsx! {
                            rect {
                                width: "fill",
                                padding: "3 5",
                                corner_radius: "3",
                                background: theme::BG_ROW,
                                margin: "1 0",
                                onpointerdown: move |_| state.select_node(jump_path.clone()),
                                label {
                                    font_size: "11",
                                    color: theme::ACCENT_RED,
                                    "• [{d_path} @ 0x{d_off:X}] {d_msg}"
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
