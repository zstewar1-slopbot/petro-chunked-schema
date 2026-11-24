//! Center pane: container children overview for parent chunks, plus color-coded hex editor,
//! span legend, pagination, and multi-format cursor previewer for leaf payloads.

use freya::prelude::*;
use petro_chunked_schema::dynamic::{ByteSpanAnnotation, inspect_bytes_at_offset};

use crate::editor_ops::{
    ChildSummaryRow, SelectedNodeDetails, append_field_to_node_byte_block,
};
use crate::ui::context::EditorState;
use crate::ui::theme;
use crate::ui::widgets::{ButtonStyle, action_button, segmented_chip, styled_input};

const HEX_PAGE_SIZE: usize = 512;

/// Center pane rendering a container's direct-children overview or, for leaf chunks and
/// mini-chunks, metadata, span legend, hex dump grid, and cursor byte inspector.
#[derive(PartialEq)]
pub struct HexInspectorPane {
    /// Currently selected node in the left tree, if any.
    pub node: Option<SelectedNodeDetails>,
}

impl Component for HexInspectorPane {
    fn render(&self) -> impl IntoElement {
        let state = EditorState::current();
        let mut synced_path = use_state(String::new);
        let mut new_bb_field_name = use_state(|| "offset_00".to_owned());

        let cur_path = state.selected_tree_path.read().clone();
        if *synced_path.peek() != cur_path {
            synced_path.set(cur_path);
            new_bb_field_name.set("offset_00".to_owned());
        }

        let Some(node) = &self.node else {
            return rect()
                .width(Size::flex(1.0))
                .height(Size::fill())
                .padding((8.0, 12.0))
                .background(theme::BG_ROOT)
                .child(
                    label()
                        .font_size(12.0)
                        .color(theme::TEXT_MUTED)
                        .text("Select a chunk or mini-chunk from the left tree to inspect its bytes and edit its schema."),
                );
        };

        let mut pane = rect()
            .width(Size::flex(1.0))
            .height(Size::fill())
            .content(Content::Flex)
            .padding((8.0, 12.0))
            .background(theme::BG_ROOT)
            .vertical()
            .spacing(8.0)
            .child(render_node_header_bar(node, state));

        if node.is_container {
            pane = pane.child(render_container_overview(
                &node.direct_children,
                node.size,
                state,
            ));
        } else {
            if !node.direct_children.is_empty() {
                pane = pane.child(render_mini_chunk_strip(&node.direct_children, state));
            }
            if !node.spans.is_empty() {
                pane = pane.child(render_span_legend(&node.spans, state));
            }
            if let Some(pager) = render_hex_pagination_bar(node.raw_bytes.len(), state) {
                pane = pane.child(pager);
            }
            pane = pane
                .child(render_hex_grid(node, state, new_bb_field_name))
                .child(render_byte_preview_panel(node, state, new_bb_field_name));
        }

        pane
    }
}

fn render_node_header_bar(node: &SelectedNodeDetails, mut state: EditorState) -> Element {
    let node_path = node.tree_path.clone();
    let is_forced_plain = {
        let p = state.persisted.read();
        p.forced_plain_paths_for(p.last_file.as_ref())
            .contains(&node_path)
    };
    let total_bytes = node.raw_bytes.len();
    let assigned_bytes: usize = node
        .spans
        .iter()
        .map(|s| s.end.saturating_sub(s.start).min(total_bytes))
        .sum();

    let title_str = if node.is_mini_chunk {
        format!("Mini-Chunk 0x{:02X} ({})", node.id, node.id)
    } else {
        format!("Chunk 0x{:04X}", node.id)
    };

    let show_plain_toggle =
        !node.is_container && !node.is_mini_chunk && (node.looks_like_mini_chunks || is_forced_plain);

    rect()
        .width(Size::fill())
        .padding((8.0, 10.0))
        .corner_radius(4.0)
        .background(theme::BG_ROW)
        .vertical()
        .spacing(4.0)
        .child(
            rect()
                .width(Size::fill())
                .horizontal()
                .spacing(12.0)
                .cross_align(Alignment::Center)
                .child(
                    label()
                        .font_size(13.0)
                        .font_weight(FontWeight::BOLD)
                        .color(theme::ACCENT_BLUE)
                        .text(title_str),
                )
                .maybe_child(node.schema_type_label.as_ref().map(|tlabel| {
                    label()
                        .font_size(12.0)
                        .color(theme::ACCENT_GREEN)
                        .text(format!("Type: {tlabel}"))
                }))
                .maybe_child(if node.is_container {
                    Some(
                        label()
                            .font_size(12.0)
                            .color(theme::ACCENT_AMBER)
                            .text(format!("Children: {}", node.direct_children.len())),
                    )
                } else if total_bytes > 0 {
                    Some(
                        label()
                            .font_size(12.0)
                            .color(if assigned_bytes >= total_bytes {
                                theme::ACCENT_GREEN
                            } else {
                                theme::ACCENT_AMBER
                            })
                            .text(format!("Assigned: {assigned_bytes}/{total_bytes}B")),
                    )
                } else {
                    None
                })
                .maybe_child(show_plain_toggle.then(|| {
                    action_button(
                        if is_forced_plain {
                            "Forced Plain (Allow Mini-Chunks)"
                        } else {
                            "Auto Mini-Chunks (Force Plain)"
                        },
                        if is_forced_plain {
                            ButtonStyle::Active
                        } else {
                            ButtonStyle::Secondary
                        },
                        true,
                        move || state.toggle_force_plain_data(&node_path),
                    )
                })),
        )
        .child(
            rect()
                .width(Size::fill())
                .horizontal()
                .spacing(12.0)
                .cross_align(Alignment::Center)
                .overflow(Overflow::Clip)
                .child(
                    label()
                        .font_size(12.0)
                        .color(theme::TEXT_MUTED)
                        .max_lines(1)
                        .text(format!(
                            "Path: {} | Header: 0x{:X} | Payload: 0x{:X} | Size: {} bytes",
                            node.tree_path, node.header_offset, node.payload_offset, node.size
                        )),
                )
                .maybe_child(node.decoded.as_ref().map(|dec| {
                    label()
                        .font_size(12.0)
                        .color(theme::ACCENT_BLUE)
                        .max_lines(1)
                        .text(format!("Value: {}", dec.short_summary()))
                }))
                .maybe_child((!node.errors.is_empty()).then(|| {
                    label()
                        .font_size(12.0)
                        .color(theme::ACCENT_RED)
                        .max_lines(1)
                        .text(format!("Node Errors: {}", node.errors.join("; ")))
                })),
        )
        .into()
}

fn render_container_overview(
    rows: &[ChildSummaryRow],
    container_size: usize,
    state: EditorState,
) -> Element {
    let total = rows.len();
    let matched = rows.iter().filter(|c| c.field_name.is_some()).count();

    let row_els: Vec<Element> = rows
        .iter()
        .map(|child| {
            let row_bg = if child.has_error {
                theme::BG_ROW_ERROR
            } else if child.index % 2 == 0 {
                theme::BG_ROW
            } else {
                theme::BG_INSET
            };
            let field_ty = match (&child.field_name, &child.type_label) {
                (Some(f), Some(t)) => format!("{f}: {t}"),
                (Some(f), None) => f.clone(),
                (None, Some(t)) => format!("<unassigned>: {t}"),
                (None, None) => "<unassigned>".to_owned(),
            };
            let field_color = if child.has_error {
                theme::ACCENT_RED
            } else if child.field_name.is_some() {
                theme::ACCENT_GREEN
            } else {
                theme::TEXT_MUTED
            };
            let sz_off = format!("{}B @ 0x{:X}", child.size, child.header_offset);
            let preview = child.value_summary.clone().unwrap_or_default();
            let click_path = child.tree_path.clone();
            let mut st = state;

            rect()
                .key(child.tree_path.clone())
                .width(Size::fill())
                .height(Size::px(26.0))
                .overflow(Overflow::Clip)
                .padding((4.0, 8.0))
                .corner_radius(4.0)
                .background(row_bg)
                .horizontal()
                .spacing(6.0)
                .cross_align(Alignment::Center)
                .on_pointer_down(move |_| st.select_node(click_path.clone()))
                .child(
                    label()
                        .width(Size::px(28.0))
                        .font_size(11.0)
                        .font_family("monospace")
                        .color(theme::TEXT_DIM)
                        .max_lines(1)
                        .text(format!("{}", child.index)),
                )
                .child(
                    label()
                        .width(Size::px(64.0))
                        .font_size(11.0)
                        .font_family("monospace")
                        .color(theme::ACCENT_BLUE)
                        .max_lines(1)
                        .text(child.id_label.clone()),
                )
                .child(
                    label()
                        .width(Size::px(185.0))
                        .font_size(12.0)
                        .color(field_color)
                        .max_lines(1)
                        .text(field_ty),
                )
                .child(
                    label()
                        .width(Size::px(96.0))
                        .font_size(11.0)
                        .color(theme::TEXT_MUTED)
                        .max_lines(1)
                        .text(child.kind_label.clone()),
                )
                .child(
                    label()
                        .width(Size::px(106.0))
                        .font_size(11.0)
                        .font_family("monospace")
                        .color(theme::TEXT_DIM)
                        .max_lines(1)
                        .text(sz_off),
                )
                .child(
                    label()
                        .font_size(11.0)
                        .font_family("monospace")
                        .color(theme::TEXT_PRIMARY)
                        .max_lines(1)
                        .text(preview),
                )
                .into()
        })
        .collect();

    rect()
        .width(Size::fill())
        .height(Size::flex(1.0))
        .content(Content::Flex)
        .padding((8.0, 10.0))
        .corner_radius(4.0)
        .background(theme::BG_SUB_CARD)
        .vertical()
        .spacing(6.0)
        .child(
            rect()
                .width(Size::fill())
                .horizontal()
                .spacing(12.0)
                .cross_align(Alignment::Center)
                .child(
                    label()
                        .font_size(12.0)
                        .font_weight(FontWeight::BOLD)
                        .color(theme::ACCENT_BLUE)
                        .text(format!("Direct Children ({total})")),
                )
                .child(
                    label()
                        .font_size(12.0)
                        .color(if matched == total {
                            theme::ACCENT_GREEN
                        } else {
                            theme::ACCENT_AMBER
                        })
                        .text(format!(
                            "{matched}/{total} matched in schema ({container_size}B payload)"
                        )),
                )
                .child(
                    label()
                        .font_size(11.0)
                        .color(theme::TEXT_DIM)
                        .text("Click any row to inspect"),
                ),
        )
        .child(
            rect()
                .width(Size::fill())
                .height(Size::px(24.0))
                .overflow(Overflow::Clip)
                .padding((4.0, 8.0))
                .corner_radius(3.0)
                .background(theme::BG_TOOLBAR)
                .horizontal()
                .spacing(6.0)
                .cross_align(Alignment::Center)
                .child(hdr_col(28.0, "#"))
                .child(hdr_col(64.0, "ID"))
                .child(hdr_col(185.0, "Field / Type"))
                .child(hdr_col(96.0, "Kind"))
                .child(hdr_col(106.0, "Size / Offset"))
                .child(
                    label()
                        .font_size(11.0)
                        .font_weight(FontWeight::BOLD)
                        .color(theme::TEXT_DIM)
                        .max_lines(1)
                        .text("Preview"),
                ),
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
                        .maybe_child(rows.is_empty().then(|| {
                            label()
                                .font_size(12.0)
                                .color(theme::TEXT_MUTED)
                                .text("This container chunk holds 0 child chunks.")
                        }))
                        .children(row_els),
                ),
        )
        .into()
}

fn hdr_col(w: f32, text: &'static str) -> Element {
    label()
        .width(Size::px(w))
        .font_size(11.0)
        .font_weight(FontWeight::BOLD)
        .color(theme::TEXT_DIM)
        .max_lines(1)
        .text(text)
        .into()
}

fn render_mini_chunk_strip(rows: &[ChildSummaryRow], state: EditorState) -> Element {
    let chips: Vec<Element> = rows
        .iter()
        .map(|mc| {
            let lbl = match &mc.field_name {
                Some(name) => format!("{}: {name} ({}B)", mc.id_label, mc.size),
                None => format!("{} ({}B)", mc.id_label, mc.size),
            };
            let path = mc.tree_path.clone();
            let mut st = state;
            segmented_chip(lbl, false, true, move || st.select_node(path.clone()))
        })
        .collect();

    rect()
        .width(Size::fill())
        .height(Size::px(62.0))
        .content(Content::Flex)
        .padding((6.0, 8.0))
        .corner_radius(4.0)
        .background(theme::BG_SUB_CARD)
        .vertical()
        .spacing(4.0)
        .child(
            label()
                .font_size(11.0)
                .font_weight(FontWeight::BOLD)
                .color(theme::ACCENT_AMBER)
                .text(format!(
                    "Parsed Mini-Chunks ({}) — click to inspect a mini-chunk:",
                    rows.len()
                )),
        )
        .child(
            ScrollView::new()
                .width(Size::fill())
                .height(Size::flex(1.0))
                .direction(Direction::Horizontal)
                .child(
                    rect()
                        .horizontal()
                        .spacing(6.0)
                        .cross_align(Alignment::Center)
                        .children(chips),
                ),
        )
        .into()
}

fn render_span_legend(spans: &[ByteSpanAnnotation], state: EditorState) -> Element {
    let items: Vec<Element> = spans
        .iter()
        .map(|span| {
            let bg = theme::span_bg_color(span);
            let sp_start = span.start;
            let sp_text = format!(
                "[0x{:02X}..0x{:02X}] {}: {} = {}",
                span.start, span.end, span.field_path, span.type_label, span.value_summary
            );
            let mut st = state;
            rect()
                .width(Size::fill())
                .padding((3.0, 7.0))
                .corner_radius(4.0)
                .background(bg)
                .on_pointer_down(move |_| st.jump_to_byte(sp_start))
                .child(
                    label()
                        .font_size(11.0)
                        .font_family("monospace")
                        .color(theme::TEXT_PRIMARY)
                        .max_lines(1)
                        .text(sp_text),
                )
                .into()
        })
        .collect();

    rect()
        .width(Size::fill())
        .height(Size::px(96.0))
        .content(Content::Flex)
        .padding((6.0, 8.0))
        .corner_radius(4.0)
        .background(theme::BG_SUB_CARD)
        .vertical()
        .spacing(4.0)
        .child(
            label()
                .font_size(11.0)
                .font_weight(FontWeight::BOLD)
                .color(theme::ACCENT_BLUE)
                .text("Assigned Structural Field Spans (click a span to jump hex cursor):"),
        )
        .child(
            ScrollView::new()
                .width(Size::fill())
                .height(Size::flex(1.0))
                .child(rect().width(Size::fill()).vertical().spacing(3.0).children(items)),
        )
        .into()
}

fn render_hex_pagination_bar(total_bytes: usize, state: EditorState) -> Option<Element> {
    let total_pages = if total_bytes == 0 {
        1
    } else {
        total_bytes.div_ceil(HEX_PAGE_SIZE)
    };
    if total_pages <= 1 {
        return None;
    }

    let safe_page = (*state.hex_page.read()).min(total_pages.saturating_sub(1));
    let page_start = safe_page * HEX_PAGE_SIZE;
    let page_end = (page_start + HEX_PAGE_SIZE).min(total_bytes);
    let mut st_prev = state;
    let mut st_next = state;

    Some(
        rect()
            .horizontal()
            .spacing(8.0)
            .cross_align(Alignment::Center)
            .child(
                label()
                    .font_size(11.0)
                    .color(theme::TEXT_MUTED)
                    .text(format!(
                        "Hex Page {} / {total_pages} (bytes 0x{page_start:04X}..0x{page_end:04X})",
                        safe_page + 1
                    )),
            )
            .child(action_button(
                "< Prev 512B",
                ButtonStyle::Secondary,
                true,
                move || {
                    let cur = *st_prev.hex_page.read();
                    if cur > 0 {
                        st_prev.hex_page.set(cur - 1);
                    }
                },
            ))
            .child(action_button(
                "Next 512B >",
                ButtonStyle::Secondary,
                true,
                move || {
                    let cur = *st_next.hex_page.read();
                    if cur + 1 < total_pages {
                        st_next.hex_page.set(cur + 1);
                    }
                },
            ))
            .into(),
    )
}

fn render_hex_grid(
    node: &SelectedNodeDetails,
    state: EditorState,
    new_bb_field_name: State<String>,
) -> Element {
    let cur_byte_offset = *state.selected_byte_offset.read();
    let cur_hex_page = *state.hex_page.read();

    let total_bytes = node.raw_bytes.len();
    let total_pages = if total_bytes == 0 {
        1
    } else {
        total_bytes.div_ceil(HEX_PAGE_SIZE)
    };
    let safe_page = cur_hex_page.min(total_pages.saturating_sub(1));
    let page_start = safe_page * HEX_PAGE_SIZE;
    let page_end = (page_start + HEX_PAGE_SIZE).min(total_bytes);

    let mut hex_rows: Vec<Element> = Vec::new();
    for row_start in (page_start..page_end).step_by(16) {
        let row_end = (row_start + 16).min(page_end);
        let ascii_str: String = (row_start..row_end)
            .map(|idx| {
                let b = node.raw_bytes[idx];
                if b.is_ascii_graphic() || b == b' ' {
                    b as char
                } else {
                    '.'
                }
            })
            .collect();

        let mut cells: Vec<Element> = Vec::with_capacity(16);
        for byte_idx in row_start..row_end {
            let b_val = node.raw_bytes[byte_idx];
            let covering_span = node
                .spans
                .iter()
                .find(|s| byte_idx >= s.start && byte_idx < s.end);
            let is_cursor = byte_idx == cur_byte_offset;
            let cell_bg = if is_cursor {
                theme::BG_CURSOR
            } else if let Some(sp) = covering_span {
                theme::span_bg_color(sp)
            } else {
                theme::BG_ROW
            };
            let cell_fg = if is_cursor {
                theme::TEXT_DARK
            } else if covering_span.is_some() {
                theme::TEXT_PRIMARY
            } else {
                theme::TEXT_MUTED
            };
            let mut st = state;
            let mut fname_st = new_bb_field_name;
            cells.push(
                rect()
                    .padding((2.0, 4.0))
                    .corner_radius(3.0)
                    .background(cell_bg)
                    .on_pointer_down(move |_| {
                        st.selected_byte_offset.set(byte_idx);
                        fname_st.set(format!("offset_{byte_idx:02x}"));
                    })
                    .child(
                        label()
                            .font_size(12.0)
                            .font_family("monospace")
                            .color(cell_fg)
                            .text(format!("{b_val:02X}")),
                    )
                    .into(),
            );
        }
        for _ in row_end..(row_start + 16) {
            cells.push(rect().width(Size::px(24.0)).into());
        }

        hex_rows.push(
            rect()
                .horizontal()
                .spacing(4.0)
                .cross_align(Alignment::Center)
                .child(
                    label()
                        .width(Size::px(54.0))
                        .font_size(12.0)
                        .font_family("monospace")
                        .color(theme::TEXT_DIM)
                        .text(format!("{row_start:04X}:")),
                )
                .children(cells)
                .child(
                    label()
                        .font_size(12.0)
                        .font_family("monospace")
                        .color(theme::ACCENT_ASCII)
                        .text(format!("  |{ascii_str}|")),
                )
                .into(),
        );
    }

    ScrollView::new()
        .width(Size::fill())
        .height(Size::flex(1.0))
        .child(
            rect()
                .width(Size::fill())
                .padding(6.0)
                .corner_radius(4.0)
                .background(theme::BG_INSET)
                .vertical()
                .spacing(2.0)
                .maybe_child(node.raw_bytes.is_empty().then(|| {
                    label()
                        .font_size(12.0)
                        .color(theme::TEXT_MUTED)
                        .text("Empty payload (0 bytes).")
                }))
                .children(hex_rows),
        )
        .into()
}

fn render_byte_preview_panel(
    node: &SelectedNodeDetails,
    state: EditorState,
    new_bb_field_name: State<String>,
) -> Element {
    let cur_byte_offset = *state.selected_byte_offset.read();
    let previews = inspect_bytes_at_offset(&node.raw_bytes, cur_byte_offset);

    let rows: Vec<Element> = previews
        .into_iter()
        .map(|entry| {
            let lbl = entry.label;
            let blen = entry.byte_len;
            let val_str = entry.value;
            let suggested = entry.suggested_type;
            let node_clone = node.clone();
            let mut st = state;
            let mut fname_st = new_bb_field_name;

            rect()
                .width(Size::fill())
                .padding((2.0, 4.0))
                .horizontal()
                .spacing(10.0)
                .cross_align(Alignment::Center)
                .maybe_child(suggested.map(|(dt, wf)| {
                    action_button("+ Add to ByteBlock", ButtonStyle::Secondary, true, move || {
                        let fname = fname_st.read().clone();
                        let type_label = dt.label();
                        let ok = st.mutate_schema(format!("Added `{fname}: {type_label}`"), |s, r| {
                            append_field_to_node_byte_block(
                                s,
                                r,
                                &node_clone,
                                &fname,
                                dt.clone(),
                                wf,
                                None,
                            )
                            .map(|_| ())
                        });
                        if ok {
                            let next_off = cur_byte_offset + blen;
                            st.jump_to_byte(next_off);
                            fname_st.set(format!("offset_{next_off:02x}"));
                        }
                    })
                }))
                .child(
                    label()
                        .width(Size::px(145.0))
                        .font_size(12.0)
                        .font_family("monospace")
                        .color(theme::ACCENT_BLUE)
                        .text(format!("{lbl} ({blen}B)")),
                )
                .child(
                    label()
                        .font_size(12.0)
                        .font_family("monospace")
                        .color(theme::TEXT_PRIMARY)
                        .max_lines(1)
                        .text(format!("= {val_str}")),
                )
                .into()
        })
        .collect();

    rect()
        .width(Size::fill())
        .height(Size::px(215.0))
        .content(Content::Flex)
        .padding((8.0, 10.0))
        .corner_radius(4.0)
        .background(theme::BG_SUB_CARD)
        .vertical()
        .spacing(6.0)
        .child(
            rect()
                .width(Size::fill())
                .horizontal()
                .spacing(10.0)
                .cross_align(Alignment::Center)
                .child(
                    label()
                        .font_size(12.0)
                        .font_weight(FontWeight::BOLD)
                        .color(theme::ACCENT_AMBER)
                        .text(format!(
                            "Byte Inspector @ 0x{cur_byte_offset:04X} ({cur_byte_offset})"
                        )),
                )
                .child(
                    label()
                        .font_size(12.0)
                        .color(theme::TEXT_MUTED)
                        .text("Field:"),
                )
                .child(styled_input(
                    Size::px(140.0),
                    new_bb_field_name,
                    "field_name",
                )),
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
                        .maybe_child(rows.is_empty().then(|| {
                            label()
                                .font_size(12.0)
                                .color(theme::TEXT_MUTED)
                                .text("Select a leaf chunk or mini-chunk with payload bytes to preview decoded values.")
                        }))
                        .children(rows),
                ),
        )
        .into()
}
