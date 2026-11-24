//! Left pane: expandable chunk and mini-chunk tree view with right-rail hover match highlighting
//! plus per-file schema diagnostics.

use freya::prelude::*;
use petro_chunked_schema::dynamic::DynamicFile;

use crate::editor_ops::{FlatTreeRow, build_flat_tree};
use crate::schema_nav::matches_hover_target;
use crate::ui::context::EditorState;
use crate::ui::theme;
use crate::ui::widgets::{ButtonStyle, action_button};

/// Left-hand pane displaying the hierarchical chunk/mini-chunk tree and file schema diagnostics.
#[derive(PartialEq)]
pub struct ChunkTreePane;

impl Component for ChunkTreePane {
    fn render(&self) -> impl IntoElement {
        let mut state = EditorState::current();
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

        let row_elements: Vec<Element> = flat_tree
            .iter()
            .map(|row| {
                let is_selected = row.tree_path == selected_path;
                let is_hover_match = hovered
                    .as_ref()
                    .is_some_and(|target| matches_hover_target(&resolved, row, target));
                render_chunk_tree_row(row, is_selected, is_hover_match, state)
            })
            .collect();

        rect()
            .width(Size::px(355.0))
            .height(Size::fill())
            .content(Content::Flex)
            .padding(8.0)
            .background(theme::BG_PANE_LEFT)
            .vertical()
            .spacing(6.0)
            .child(
                rect()
                    .width(Size::fill())
                    .horizontal()
                    .spacing(8.0)
                    .cross_align(Alignment::Center)
                    .child(
                        label()
                            .font_size(13.0)
                            .font_weight(FontWeight::BOLD)
                            .color(theme::ACCENT_BLUE)
                            .text(format!("Chunk Tree ({})", flat_tree.len())),
                    )
                    .child(action_button(
                        "Expand All",
                        ButtonStyle::Secondary,
                        true,
                        move || state.expand_all(),
                    ))
                    .maybe_child(matched_count.map(|count| {
                        label()
                            .font_size(11.0)
                            .font_weight(FontWeight::BOLD)
                            .color(if count > 0 {
                                theme::ACCENT_BLUE
                            } else {
                                theme::TEXT_DIM
                            })
                            .max_lines(1)
                            .text(format!("{count} highlighted"))
                    })),
            )
            .child(
                ScrollView::new()
                    .width(Size::fill())
                    .height(Size::flex(1.0))
                    .child(
                        rect()
                            .width(Size::fill())
                            .vertical()
                            .spacing(2.0)
                            .maybe_child(flat_tree.is_empty().then(|| {
                                label()
                                    .font_size(12.0)
                                    .color(theme::TEXT_MUTED)
                                    .text("No file loaded. Click 'Open File / .meg...' above to open an .alo or .ala file.")
                            }))
                            .children(row_elements),
                    ),
            )
            .maybe_child(df_opt.map(|df| DiagnosticsPanel { file: df }))
    }
}

fn render_chunk_tree_row(
    row: &FlatTreeRow,
    is_selected: bool,
    is_hover_match: bool,
    mut state: EditorState,
) -> Element {
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
        Some(theme::border_all(theme::ACCENT_BLUE))
    } else {
        Some(theme::border_all(Color::TRANSPARENT))
    };
    let indent_px = (row.depth * 14) as f32;
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

    let toggle_el: Element = if row.child_count > 0 {
        let mut st_toggle = state;
        rect()
            .padding((1.0, 5.0))
            .corner_radius(3.0)
            .background(theme::BG_CARD)
            .on_pointer_down(move |e: Event<PointerEventData>| {
                e.stop_propagation();
                st_toggle.toggle_collapse(&row_path_toggle);
            })
            .child(
                label()
                    .font_size(10.0)
                    .font_family("monospace")
                    .color(theme::TEXT_PRIMARY)
                    .text(toggle_label),
            )
            .into()
    } else {
        label()
            .font_size(11.0)
            .color(theme::TEXT_DIM)
            .text(" · ")
            .into()
    };

    rect()
        .key(row.tree_path.clone())
        .width(Size::fill())
        .overflow(Overflow::Clip)
        .padding((3.0, 6.0))
        .corner_radius(4.0)
        .background(bg)
        .border(border)
        .horizontal()
        .spacing(5.0)
        .cross_align(Alignment::Center)
        .on_pointer_down(move |_| state.select_node(row_path.clone()))
        .maybe_child((indent_px > 0.0).then(|| rect().width(Size::px(indent_px))))
        .child(toggle_el)
        .child(
            label()
                .font_size(11.0)
                .font_family("monospace")
                .color(id_color)
                .max_lines(1)
                .text(id_lbl),
        )
        .child(
            label()
                .font_size(11.0)
                .color(name_color)
                .max_lines(1)
                .text(format!("{name_str}{type_suffix} ({sz}B)")),
        )
        .into()
}

#[derive(PartialEq)]
struct DiagnosticsPanel {
    file: DynamicFile,
}

impl Component for DiagnosticsPanel {
    fn render(&self) -> impl IntoElement {
        let state = EditorState::current();
        let diag_count = self.file.diagnostics.len();

        let diag_rows: Vec<Element> = self
            .file
            .diagnostics
            .iter()
            .take(60)
            .map(|diag| {
                let d_msg = diag.message.clone();
                let d_path = diag.tree_path.clone();
                let d_off = diag.file_offset;
                let jump_path = if diag.tree_path == "root" {
                    "0".to_owned()
                } else {
                    diag.tree_path.clone()
                };
                let mut st = state;
                rect()
                    .width(Size::fill())
                    .padding((3.0, 5.0))
                    .corner_radius(3.0)
                    .background(theme::BG_ROW)
                    .on_pointer_down(move |_| st.select_node(jump_path.clone()))
                    .child(
                        label()
                            .font_size(11.0)
                            .color(theme::ACCENT_RED)
                            .text(format!("• [{d_path} @ 0x{d_off:X}] {d_msg}")),
                    )
                    .into()
            })
            .collect();

        rect()
            .width(Size::fill())
            .height(Size::px(155.0))
            .content(Content::Flex)
            .padding(6.0)
            .corner_radius(4.0)
            .background(theme::BG_INSET)
            .vertical()
            .spacing(4.0)
            .child(
                label()
                    .font_size(12.0)
                    .font_weight(FontWeight::BOLD)
                    .color(if diag_count == 0 {
                        theme::ACCENT_GREEN
                    } else {
                        theme::ACCENT_RED
                    })
                    .text(format!("File Schema Diagnostics ({diag_count})")),
            )
            .child(
                ScrollView::new()
                    .width(Size::fill())
                    .height(Size::flex(1.0))
                    .child(
                        rect()
                            .width(Size::fill())
                            .vertical()
                            .spacing(2.0)
                            .maybe_child((diag_count == 0).then(|| {
                                label()
                                    .font_size(11.0)
                                    .color(theme::ACCENT_GREEN)
                                    .text("All chunks and bytes in this file conform cleanly to the active schema!")
                            }))
                            .children(diag_rows),
                    ),
            )
    }
}
