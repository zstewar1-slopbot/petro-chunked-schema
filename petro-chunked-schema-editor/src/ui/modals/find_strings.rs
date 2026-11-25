//! Freestanding string finder window scanning leaf chunks, mini-chunks, and byte blocks
//! for whole-block and embedded ASCII / ASCIIZ strings.

use freya::prelude::*;
use petro_chunked_schema::dynamic::{FindStringsOptions, FoundString, StringMatchKind, find_strings};

use crate::ui::context::EditorState;
use crate::ui::theme;
use crate::ui::widgets::{
    ButtonStyle, action_button, group_caption, segmented_chip, styled_input, toolbar_divider,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScopeFilter {
    WholeAndEmbeddedZ,
    WholeOnly,
    WholeAndAllEmbedded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KindFilter {
    All,
    AsciizOnly,
    AsciiOnly,
}

fn format_node_id(hit: &FoundString) -> String {
    if hit.is_mini_chunk {
        format!("mc {:02X}", hit.node_id)
    } else {
        format!("0x{:04X}", hit.node_id)
    }
}

fn format_field_label(hit: &FoundString) -> (String, Color) {
    if let Some(sp) = &hit.span_field_path {
        let label = match &hit.schema_field_name {
            Some(sf)
                if sf != sp
                    && !sp.starts_with(&format!("{sf}."))
                    && !sp.starts_with(&format!("{sf}[")) =>
            {
                format!("{sf} ({sp})")
            }
            _ => sp.clone(),
        };
        let color = if hit.is_schema_assigned {
            theme::ACCENT_GREEN
        } else {
            theme::ACCENT_AMBER
        };
        (label, color)
    } else if let Some(sf) = &hit.schema_field_name {
        if hit.is_schema_assigned {
            (sf.clone(), theme::ACCENT_GREEN)
        } else {
            (format!("{sf} (unmapped)"), theme::ACCENT_AMBER)
        }
    } else {
        ("<unassigned>".to_owned(), theme::TEXT_DIM)
    }
}

fn escape_string_preview(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            '\n' => out.push_str("\\n"),
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            other => out.push(other),
        }
    }
    out
}

fn kind_badge_color(kind: StringMatchKind) -> Color {
    match kind {
        StringMatchKind::WholeAsciiz => theme::ACCENT_GREEN,
        StringMatchKind::WholeAsciizPadded => theme::ACCENT_AMBER,
        StringMatchKind::WholeAscii => theme::ACCENT_BLUE,
        StringMatchKind::EmbeddedAsciiz => theme::ACCENT_ASCII,
        StringMatchKind::EmbeddedAscii => theme::TEXT_MUTED,
    }
}

/// Freestanding window displaying whole-block and embedded ASCII / ASCIIZ string hits across the
/// active binary file, with span-aware typed-binary filtering and click-to-jump navigation.
#[derive(PartialEq)]
pub struct FindStringsWindow;

impl Component for FindStringsWindow {
    fn render(&self) -> impl IntoElement {
        let state = EditorState::current();
        let mut scope = use_state(|| ScopeFilter::WholeAndEmbeddedZ);
        let mut kind_filter = use_state(|| KindFilter::All);
        let mut unassigned_only = use_state(|| false);
        let mut skip_typed = use_state(|| true);
        let mut min_whole_len = use_state(|| 2usize);
        let mut min_emb_len = use_state(|| 4usize);
        let mut query = use_state(String::new);
        let mut active_hit_key = use_state(|| None::<(String, usize)>);

        let cur_scope = *scope.read();
        let cur_kind = *kind_filter.read();
        let cur_unassigned = *unassigned_only.read();
        let cur_skip_typed = *skip_typed.read();
        let cur_min_whole = *min_whole_len.read();
        let cur_min_emb = *min_emb_len.read();
        let cur_query = query.read().trim().to_ascii_lowercase();
        let cur_active = active_hit_key.read().clone();

        let opts = FindStringsOptions {
            min_whole_len: cur_min_whole,
            min_embedded_len: cur_min_emb,
            include_asciiz: matches!(cur_kind, KindFilter::All | KindFilter::AsciizOnly),
            include_ascii: matches!(cur_kind, KindFilter::All | KindFilter::AsciiOnly),
            include_embedded_asciiz: matches!(
                cur_scope,
                ScopeFilter::WholeAndEmbeddedZ | ScopeFilter::WholeAndAllEmbedded
            ),
            include_embedded_ascii: matches!(cur_scope, ScopeFilter::WholeAndAllEmbedded),
            skip_typed_non_string_spans: cur_skip_typed,
            allow_whitespace_controls: true,
            max_results: 2000,
        };

        let active_file_label = state
            .persisted
            .read()
            .last_file
            .as_ref()
            .map(|f| f.display_label())
            .unwrap_or_else(|| "<no file loaded>".to_owned());

        let (has_file, all_hits) = {
            let df_guard = state.dynamic_file.read();
            match df_guard.as_ref() {
                Some(df) => (true, find_strings(df, &opts)),
                None => (false, Vec::new()),
            }
        };

        let total_hits = all_hits.len();
        let hit_limit_reached = total_hits == opts.max_results;
        let filtered_hits: Vec<FoundString> = all_hits
            .into_iter()
            .filter(|hit| !cur_unassigned || !hit.is_schema_assigned)
            .filter(|hit| {
                if cur_query.is_empty() {
                    return true;
                }
                let node_id = format_node_id(hit).to_ascii_lowercase();
                hit.text.to_ascii_lowercase().contains(&cur_query)
                    || node_id.contains(&cur_query)
                    || hit.tree_path.to_ascii_lowercase().contains(&cur_query)
                    || hit
                        .schema_field_name
                        .as_deref()
                        .unwrap_or("")
                        .to_ascii_lowercase()
                        .contains(&cur_query)
                    || hit
                        .span_field_path
                        .as_deref()
                        .unwrap_or("")
                        .to_ascii_lowercase()
                        .contains(&cur_query)
            })
            .collect();
        let shown_hits = filtered_hits.len();

        let mut st_close = state;
        let cap_suffix = if hit_limit_reached {
            format!(" (capped at {})", opts.max_results)
        } else {
            String::new()
        };

        let (min_len_caption, min_len_chips): (&'static str, Vec<Element>) =
            if cur_scope == ScopeFilter::WholeOnly {
                let chips = [1usize, 2, 4, 8]
                    .into_iter()
                    .map(|len_val| {
                        segmented_chip(
                            format!("{len_val}"),
                            cur_min_whole == len_val,
                            true,
                            move || min_whole_len.set(len_val),
                        )
                    })
                    .collect();
                ("MIN LEN", chips)
            } else {
                let chips = [3usize, 4, 6, 8]
                    .into_iter()
                    .map(|len_val| {
                        segmented_chip(
                            format!("{len_val}"),
                            cur_min_emb == len_val,
                            true,
                            move || min_emb_len.set(len_val),
                        )
                    })
                    .collect();
                ("EMB MIN", chips)
            };

        let rows: Vec<Element> = filtered_hits
            .into_iter()
            .map(|hit| {
                let row_key = (hit.tree_path.clone(), hit.payload_offset);
                let is_selected = cur_active.as_ref() == Some(&row_key);
                let row_bg = if is_selected {
                    theme::BG_ROW_SELECTED
                } else {
                    theme::BG_ROW
                };
                let kind_label = hit.kind.short_label();
                let kind_color = kind_badge_color(hit.kind);
                let node_str = format_node_id(&hit);
                let off_str = if hit.payload_offset == 0 {
                    format!("0x{:X}", hit.file_offset)
                } else {
                    format!("0x{:X} +{:X}", hit.file_offset, hit.payload_offset)
                };
                let len_str = format!("{}B", hit.byte_len);
                let (field_str, field_color) = format_field_label(&hit);
                let preview_str = format!("\"{}\"", escape_string_preview(&hit.text));

                let jump_path = hit.tree_path.clone();
                let jump_off = hit.payload_offset;
                let byte_len = hit.byte_len;
                let mut st_jump = state;

                rect()
                    .key(format!("{}@{}", hit.tree_path, hit.payload_offset))
                    .width(Size::fill())
                    .height(Size::px(28.0))
                    .overflow(Overflow::Clip)
                    .padding((0.0, 8.0))
                    .corner_radius(4.0)
                    .background(row_bg)
                    .horizontal()
                    .spacing(8.0)
                    .cross_align(Alignment::Center)
                    .on_pointer_down(move |_| {
                        active_hit_key.set(Some((jump_path.clone(), jump_off)));
                        st_jump.select_node_at_offset(jump_path.clone(), jump_off);
                        st_jump.set_status(format!(
                            "Jumped to {kind_label} string at {jump_path} +0x{jump_off:X} ({byte_len} bytes)"
                        ));
                    })
                    .child(
                        label()
                            .width(Size::px(68.0))
                            .font_size(11.0)
                            .font_weight(FontWeight::BOLD)
                            .font_family("monospace")
                            .color(kind_color)
                            .max_lines(1)
                            .text(kind_label),
                    )
                    .child(
                        label()
                            .width(Size::px(82.0))
                            .font_size(11.0)
                            .font_family("monospace")
                            .color(if hit.is_mini_chunk {
                                theme::ACCENT_AMBER
                            } else {
                                theme::ACCENT_BLUE
                            })
                            .max_lines(1)
                            .text(node_str),
                    )
                    .child(
                        label()
                            .width(Size::px(106.0))
                            .font_size(11.0)
                            .font_family("monospace")
                            .color(theme::TEXT_DIM)
                            .max_lines(1)
                            .text(off_str),
                    )
                    .child(
                        label()
                            .width(Size::px(44.0))
                            .font_size(11.0)
                            .font_family("monospace")
                            .color(theme::TEXT_MUTED)
                            .max_lines(1)
                            .text(len_str),
                    )
                    .child(
                        label()
                            .width(Size::px(155.0))
                            .font_size(11.0)
                            .color(field_color)
                            .max_lines(1)
                            .text(field_str),
                    )
                    .child(
                        label()
                            .font_size(12.0)
                            .font_family("monospace")
                            .color(theme::TEXT_PRIMARY)
                            .max_lines(1)
                            .text(preview_str),
                    )
                    .into()
            })
            .collect();

        let results_body: Element = if !has_file {
            rect()
                .width(Size::fill())
                .height(Size::flex(1.0))
                .padding(18.0)
                .corner_radius(5.0)
                .background(theme::BG_INSET)
                .border(theme::border_all(theme::BORDER_CARD))
                .child(
                    label()
                        .font_size(12.0)
                        .color(theme::TEXT_MUTED)
                        .text("No binary file is loaded. Open a chunked file or .meg entry first to scan for ASCII / ASCIIZ strings."),
                )
                .into_element()
        } else if rows.is_empty() {
            rect()
                .width(Size::fill())
                .height(Size::flex(1.0))
                .padding(18.0)
                .corner_radius(5.0)
                .background(theme::BG_INSET)
                .border(theme::border_all(theme::BORDER_CARD))
                .child(
                    label()
                        .font_size(12.0)
                        .color(theme::TEXT_MUTED)
                        .text("No matching ASCII / ASCIIZ strings found with the current filters. Try lowering EMB MIN, toggling `+ Emb Raw`, or disabling `Skip Typed`."),
                )
                .into_element()
        } else {
            ScrollView::new()
                .width(Size::fill())
                .height(Size::flex(1.0))
                .child(
                    rect()
                        .width(Size::fill())
                        .vertical()
                        .spacing(3.0)
                        .children(rows),
                )
                .into_element()
        };

        rect()
            .width(Size::fill())
            .height(Size::fill())
            .content(Content::Flex)
            .vertical()
            .spacing(8.0)
            .child(
                rect()
                    .width(Size::fill())
                    .horizontal()
                    .cross_align(Alignment::Center)
                    .spacing(12.0)
                    .child(
                        label()
                            .font_size(14.0)
                            .font_weight(FontWeight::BOLD)
                            .color(theme::ACCENT_BLUE)
                            .max_lines(1)
                            .text("Find Strings (ASCII / ASCIIZ)"),
                    )
                    .child(
                        label()
                            .font_size(12.0)
                            .color(theme::ACCENT_ASCII)
                            .max_lines(1)
                            .text(active_file_label),
                    )
                    .child(
                        rect()
                            .width(Size::flex(1.0))
                            .overflow(Overflow::Clip)
                            .child(
                                label()
                                    .font_size(12.0)
                                    .color(theme::TEXT_MUTED)
                                    .max_lines(1)
                                    .text(format!("{shown_hits} shown / {total_hits} matched{cap_suffix}")),
                            ),
                    )
                    .child(action_button(
                        "Close",
                        ButtonStyle::Secondary,
                        false,
                        move || st_close.close_current_modal_window(),
                    )),
            )
            .child(
                rect()
                    .width(Size::fill())
                    .padding((6.0, 10.0))
                    .corner_radius(5.0)
                    .background(theme::BG_SUB_CARD)
                    .border(theme::border_all(theme::BORDER_CARD))
                    .horizontal()
                    .spacing(6.0)
                    .cross_align(Alignment::Center)
                    .child(group_caption("SCOPE"))
                    .child(segmented_chip(
                        "Whole + Emb Z",
                        cur_scope == ScopeFilter::WholeAndEmbeddedZ,
                        true,
                        move || scope.set(ScopeFilter::WholeAndEmbeddedZ),
                    ))
                    .child(segmented_chip(
                        "Whole Only",
                        cur_scope == ScopeFilter::WholeOnly,
                        true,
                        move || scope.set(ScopeFilter::WholeOnly),
                    ))
                    .child(segmented_chip(
                        "+ Emb Raw",
                        cur_scope == ScopeFilter::WholeAndAllEmbedded,
                        true,
                        move || scope.set(ScopeFilter::WholeAndAllEmbedded),
                    ))
                    .child(toolbar_divider())
                    .child(group_caption("KIND"))
                    .child(segmented_chip(
                        "All",
                        cur_kind == KindFilter::All,
                        true,
                        move || kind_filter.set(KindFilter::All),
                    ))
                    .child(segmented_chip(
                        "ASCIIZ",
                        cur_kind == KindFilter::AsciizOnly,
                        true,
                        move || kind_filter.set(KindFilter::AsciizOnly),
                    ))
                    .child(segmented_chip(
                        "ASCII",
                        cur_kind == KindFilter::AsciiOnly,
                        true,
                        move || kind_filter.set(KindFilter::AsciiOnly),
                    ))
                    .child(toolbar_divider())
                    .child(segmented_chip(
                        "Unassigned Only",
                        cur_unassigned,
                        true,
                        move || unassigned_only.set(!cur_unassigned),
                    ))
                    .child(segmented_chip(
                        "Skip Typed",
                        cur_skip_typed,
                        true,
                        move || skip_typed.set(!cur_skip_typed),
                    ))
                    .child(toolbar_divider())
                    .child(group_caption(min_len_caption))
                    .children(min_len_chips),
            )
            .child(
                rect()
                    .width(Size::fill())
                    .horizontal()
                    .spacing(8.0)
                    .cross_align(Alignment::Center)
                    .child(group_caption("FILTER"))
                    .child(styled_input(
                        Size::flex(1.0),
                        query,
                        "Filter text, chunk ID (0x0401 / mc 01), tree path, or field name...",
                    ))
                    .child(action_button(
                        "Clear",
                        ButtonStyle::Secondary,
                        true,
                        move || query.set(String::new()),
                    )),
            )
            .child(
                rect()
                    .width(Size::fill())
                    .height(Size::px(24.0))
                    .overflow(Overflow::Clip)
                    .padding((0.0, 8.0))
                    .corner_radius(4.0)
                    .background(theme::BG_SUB_CARD)
                    .border(theme::border_all(theme::BORDER_CARD))
                    .horizontal()
                    .spacing(8.0)
                    .cross_align(Alignment::Center)
                    .child(
                        label()
                            .width(Size::px(68.0))
                            .font_size(10.0)
                            .font_weight(FontWeight::BOLD)
                            .color(theme::TEXT_DIM)
                            .max_lines(1)
                            .text("KIND"),
                    )
                    .child(
                        label()
                            .width(Size::px(82.0))
                            .font_size(10.0)
                            .font_weight(FontWeight::BOLD)
                            .color(theme::TEXT_DIM)
                            .max_lines(1)
                            .text("NODE"),
                    )
                    .child(
                        label()
                            .width(Size::px(106.0))
                            .font_size(10.0)
                            .font_weight(FontWeight::BOLD)
                            .color(theme::TEXT_DIM)
                            .max_lines(1)
                            .text("OFFSET"),
                    )
                    .child(
                        label()
                            .width(Size::px(44.0))
                            .font_size(10.0)
                            .font_weight(FontWeight::BOLD)
                            .color(theme::TEXT_DIM)
                            .max_lines(1)
                            .text("LEN"),
                    )
                    .child(
                        label()
                            .width(Size::px(155.0))
                            .font_size(10.0)
                            .font_weight(FontWeight::BOLD)
                            .color(theme::TEXT_DIM)
                            .max_lines(1)
                            .text("FIELD"),
                    )
                    .child(
                        label()
                            .font_size(10.0)
                            .font_weight(FontWeight::BOLD)
                            .color(theme::TEXT_DIM)
                            .max_lines(1)
                            .text("TEXT"),
                    ),
            )
            .child(results_body)
            .child(
                label()
                    .font_size(11.0)
                    .color(theme::TEXT_DIM)
                    .max_lines(1)
                    .text("Click any row to jump the main editor to that chunk/mini-chunk and byte offset. This window stays open and updates live."),
            )
    }
}
