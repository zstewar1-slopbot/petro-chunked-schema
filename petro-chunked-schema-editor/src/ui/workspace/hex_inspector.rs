//! Center pane: color-coded hex editor, span legend, pagination, and multi-format cursor
//! previewer with one-click `+ Add to ByteBlock`.

use freya::prelude::*;
use petro_chunked_schema::dynamic::{ByteSpanAnnotation, inspect_bytes_at_offset};

use crate::editor_ops::{SelectedNodeDetails, append_field_to_node_byte_block};
use crate::ui::context::EditorState;
use crate::ui::theme;

const HEX_PAGE_SIZE: usize = 512;

/// Center pane rendering metadata, span legend, hex dump grid, and cursor byte inspector for
/// the selected chunk or mini-chunk.
#[component]
pub fn HexInspectorPane(node: Option<SelectedNodeDetails>) -> Element {
    let state = use_context::<EditorState>();
    let mut new_bb_field_name = use_signal(|| "offset_00".to_owned());

    use_effect(move || {
        let _path = state.selected_tree_path.read();
        new_bb_field_name.set("offset_00".to_owned());
    });

    rsx! {
        rect {
            width: "flex(1)",
            height: "fill",
            content: "flex",
            padding: "8 12",
            background: theme::BG_ROOT,
            direction: "vertical",
            spacing: "8",

            if let Some(node) = node {
                NodeHeaderBar { node: node.clone() }

                if !node.spans.is_empty() {
                    SpanLegend { spans: node.spans.clone() }
                }

                HexPaginationBar { total_bytes: node.raw_bytes.len() }

                HexGrid {
                    node: node.clone(),
                    on_select_byte: move |byte_idx: usize| {
                        new_bb_field_name.set(format!("offset_{byte_idx:02x}"));
                    },
                }

                BytePreviewPanel {
                    node,
                    new_bb_field_name,
                }
            } else {
                label {
                    font_size: "13",
                    color: theme::TEXT_MUTED,
                    "Select a chunk or mini-chunk from the left tree to inspect its bytes and edit its schema."
                }
            }
        }
    }
}

#[component]
fn NodeHeaderBar(node: SelectedNodeDetails) -> Element {
    let mut state = use_context::<EditorState>();
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

    rsx! {
        rect {
            width: "fill",
            padding: "8 10",
            corner_radius: "4",
            background: theme::BG_ROW,
            direction: "vertical",
            spacing: "4",

            rect {
                width: "fill",
                direction: "horizontal",
                spacing: "12",
                cross_align: "center",

                label {
                    font_size: "13",
                    font_weight: "bold",
                    color: theme::ACCENT_BLUE,
                    if node.is_mini_chunk {
                        "Mini-Chunk 0x{node.id:02X} ({node.id})"
                    } else {
                        "Chunk 0x{node.id:04X}"
                    }
                }
                if let Some(tlabel) = &node.schema_type_label {
                    label {
                        font_size: "12",
                        color: theme::ACCENT_GREEN,
                        "Type: {tlabel}"
                    }
                }
                if total_bytes > 0 {
                    label {
                        font_size: "12",
                        color: if assigned_bytes >= total_bytes { theme::ACCENT_GREEN } else { theme::ACCENT_AMBER },
                        "Assigned: {assigned_bytes}/{total_bytes}B"
                    }
                }
                if !node.is_container && !node.is_mini_chunk && (node.looks_like_mini_chunks || is_forced_plain) {
                    Button {
                        onpress: move |_| state.toggle_force_plain_data(&node_path),
                        label {
                            font_size: "11",
                            if is_forced_plain {
                                "Forced Plain (Toggle)"
                            } else {
                                "Auto Mini-Chunks (Force Plain)"
                            }
                        }
                    }
                }
            }

            rect {
                width: "fill",
                direction: "horizontal",
                spacing: "12",
                cross_align: "center",

                label {
                    font_size: "12",
                    color: theme::TEXT_MUTED,
                    "Path: {node.tree_path} | Header: 0x{node.header_offset:X} | Payload: 0x{node.payload_offset:X} | Size: {node.size} bytes"
                }
                if let Some(dec) = &node.decoded {
                    label {
                        font_size: "12",
                        color: theme::ACCENT_BLUE,
                        "Value: {dec.short_summary()}"
                    }
                }
                if !node.errors.is_empty() {
                    label {
                        font_size: "12",
                        color: theme::ACCENT_RED,
                        "Node Errors: {node.errors.join(\"; \")}"
                    }
                }
            }
        }
    }
}

#[component]
fn SpanLegend(spans: Vec<ByteSpanAnnotation>) -> Element {
    let mut state = use_context::<EditorState>();

    rsx! {
        rect {
            width: "fill",
            height: "96",
            content: "flex",
            padding: "6 8",
            corner_radius: "4",
            background: theme::BG_SUB_CARD,
            direction: "vertical",
            spacing: "4",

            label {
                font_size: "11",
                font_weight: "bold",
                color: theme::ACCENT_BLUE,
                "Assigned Structural Field Spans (click a span to jump hex cursor):"
            }
            ScrollView {
                width: "fill",
                height: "flex(1)",
                rect {
                    width: "fill",
                    direction: "vertical",
                    spacing: "3",
                    for span in spans {
                        {
                            let bg = theme::span_bg_color(&span);
                            let sp_start = span.start;
                            let sp_text = format!(
                                "[0x{:02X}..0x{:02X}] {}: {} = {}",
                                span.start, span.end, span.field_path, span.type_label, span.value_summary
                            );
                            rsx! {
                                rect {
                                    width: "fill",
                                    padding: "3 7",
                                    corner_radius: "4",
                                    background: "{bg}",
                                    onpointerdown: move |_| state.jump_to_byte(sp_start),
                                    label {
                                        font_size: "11",
                                        font_family: "monospace",
                                        color: theme::TEXT_PRIMARY,
                                        "{sp_text}"
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn HexPaginationBar(total_bytes: usize) -> Element {
    let mut state = use_context::<EditorState>();
    let total_pages = if total_bytes == 0 {
        1
    } else {
        total_bytes.div_ceil(HEX_PAGE_SIZE)
    };
    if total_pages <= 1 {
        return rsx! {};
    }

    let safe_page = (*state.hex_page.read()).min(total_pages.saturating_sub(1));
    let page_start = safe_page * HEX_PAGE_SIZE;
    let page_end = (page_start + HEX_PAGE_SIZE).min(total_bytes);

    rsx! {
        rect {
            direction: "horizontal",
            spacing: "8",
            cross_align: "center",
            label {
                font_size: "11",
                "Hex Page {safe_page + 1} / {total_pages} (bytes 0x{page_start:04X}..0x{page_end:04X})"
            }
            Button {
                onpress: move |_| {
                    let cur = *state.hex_page.read();
                    if cur > 0 {
                        state.hex_page.set(cur - 1);
                    }
                },
                label { "< Prev 512B" }
            }
            Button {
                onpress: move |_| {
                    let cur = *state.hex_page.read();
                    if cur + 1 < total_pages {
                        state.hex_page.set(cur + 1);
                    }
                },
                label { "Next 512B >" }
            }
        }
    }
}

#[component]
fn HexGrid(node: SelectedNodeDetails, on_select_byte: EventHandler<usize>) -> Element {
    let mut state = use_context::<EditorState>();
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

    rsx! {
        ScrollView {
            width: "fill",
            height: "flex(1)",
            rect {
                width: "fill",
                padding: "6",
                background: theme::BG_INSET,
                direction: "vertical",
                spacing: "2",

                if node.is_container {
                    label {
                        font_size: "12",
                        color: theme::TEXT_MUTED,
                        "This is a 32-bit container chunk holding child chunks. Select any leaf chunk or mini-chunk in the tree to inspect its bytes."
                    }
                } else if node.raw_bytes.is_empty() {
                    label {
                        font_size: "12",
                        color: theme::TEXT_MUTED,
                        "Empty payload (0 bytes)."
                    }
                } else {
                    for row_start in (page_start..page_end).step_by(16) {
                        {
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

                            rsx! {
                                rect {
                                    direction: "horizontal",
                                    spacing: "4",
                                    cross_align: "center",

                                    label {
                                        width: "54",
                                        font_size: "12",
                                        font_family: "monospace",
                                        color: theme::TEXT_DIM,
                                        "{row_start:04X}:"
                                    }

                                    for byte_idx in row_start..row_end {
                                        {
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

                                            rsx! {
                                                rect {
                                                    padding: "2 4",
                                                    corner_radius: "3",
                                                    background: "{cell_bg}",
                                                    onpointerdown: move |_| {
                                                        state.selected_byte_offset.set(byte_idx);
                                                        on_select_byte.call(byte_idx);
                                                    },
                                                    label {
                                                        font_size: "12",
                                                        font_family: "monospace",
                                                        color: "{cell_fg}",
                                                        "{b_val:02X}"
                                                    }
                                                }
                                            }
                                        }
                                    }

                                    for _ in row_end..(row_start + 16) {
                                        rect { width: "24" }
                                    }

                                    label {
                                        font_size: "12",
                                        font_family: "monospace",
                                        color: theme::ACCENT_ASCII,
                                        "  |{ascii_str}|"
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn BytePreviewPanel(
    node: SelectedNodeDetails,
    mut new_bb_field_name: Signal<String>,
) -> Element {
    let mut state = use_context::<EditorState>();
    let cur_byte_offset = *state.selected_byte_offset.read();
    let previews = inspect_bytes_at_offset(&node.raw_bytes, cur_byte_offset);

    rsx! {
        rect {
            width: "fill",
            height: "215",
            content: "flex",
            padding: "8 10",
            corner_radius: "4",
            background: theme::BG_SUB_CARD,
            direction: "vertical",
            spacing: "6",

            rect {
                width: "fill",
                direction: "horizontal",
                spacing: "10",
                cross_align: "center",
                label {
                    font_size: "12",
                    font_weight: "bold",
                    color: theme::ACCENT_AMBER,
                    "Byte Inspector @ 0x{cur_byte_offset:04X} ({cur_byte_offset})"
                }
                label {
                    font_size: "11",
                    color: theme::TEXT_MUTED,
                    "Field:"
                }
                Input {
                    value: new_bb_field_name.read().clone(),
                    placeholder: "field_name",
                    onchange: move |txt| new_bb_field_name.set(txt),
                }
            }

            ScrollView {
                width: "fill",
                height: "flex(1)",
                if previews.is_empty() {
                    label {
                        font_size: "11",
                        color: theme::TEXT_MUTED,
                        "Select a leaf chunk or mini-chunk with payload bytes to preview decoded values."
                    }
                }
                for entry in previews {
                    {
                        let lbl = entry.label;
                        let blen = entry.byte_len;
                        let val_str = entry.value;
                        let suggested = entry.suggested_type;
                        let node_clone = node.clone();

                        rsx! {
                            rect {
                                padding: "2 4",
                                direction: "horizontal",
                                spacing: "10",
                                cross_align: "center",

                                if let Some((dt, wf)) = suggested {
                                    Button {
                                        onpress: move |_| {
                                            let fname = new_bb_field_name.read().clone();
                                            let type_label = dt.label();
                                            let ok = state.mutate_schema(
                                                format!("Added `{fname}: {type_label}`"),
                                                |s, r| {
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
                                                },
                                            );
                                            if ok {
                                                let next_off = cur_byte_offset + blen;
                                                state.jump_to_byte(next_off);
                                                new_bb_field_name.set(format!("offset_{next_off:02x}"));
                                            }
                                        },
                                        label { font_size: "11", "+ Add to ByteBlock" }
                                    }
                                }

                                label {
                                    width: "145",
                                    font_size: "12",
                                    font_family: "monospace",
                                    color: theme::ACCENT_BLUE,
                                    "{lbl} ({blen}B)"
                                }
                                label {
                                    font_size: "12",
                                    font_family: "monospace",
                                    color: theme::TEXT_PRIMARY,
                                    "= {val_str}"
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
