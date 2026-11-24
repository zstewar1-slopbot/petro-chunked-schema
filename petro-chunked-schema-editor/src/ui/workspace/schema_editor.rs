//! Right pane: non-indenting vertical schema stack showing all ancestor schema levels from root
//! down to the active definition (parents collapsed by default, expandable to switch sibling
//! definitions including those absent from the current file), live auto-applying edits in memory,
//! and mouseover synchronization with the left-rail chunk tree.

use std::collections::BTreeSet;

use freya::prelude::*;
use petro_chunked_schema::resolve::{ResolvedSchema, SchemaId};
use petro_chunked_schema::schema::{
    ByteBlockDef, ByteFieldDef, Cardinality, ChunkFieldDef, ChunkFieldSource, ChunkPayloadSpec,
    DataType, MiniChunkFieldDef, OrderMode, Schema, TypeDef, WireFormat,
};

use crate::editor_ops::{
    FlatTreeRow, ParentContainerRef, build_flat_tree, delete_chunk_field, delete_mini_chunk_field,
    set_byte_block_pad_after, to_pascal_case, upsert_chunk_field, upsert_mini_chunk_field,
};
use crate::schema_nav::{
    HoveredSchemaTarget, SchemaChildItem, SchemaLevelTarget, SchemaParentEdge, SchemaStackStep,
    count_file_matches, list_level_children, step_hover_target, type_kind_badge,
};
use crate::ui::context::EditorState;
use crate::ui::theme;
use crate::ui::widgets::{ActionButton, ButtonStyle, GroupCaption, SegmentedChip, StyledInput};

/// Returns the wire formats compatible with a parsed `DataType` string, or `&[]` when no explicit
/// wire-format adapter applies to that type.
pub fn compatible_wire_formats(ty_str: &str) -> &'static [(WireFormat, &'static str)] {
    match DataType::parse_str(ty_str.trim()) {
        Ok(DataType::Bool) => &[
            (WireFormat::U8Bool, "U8Bool (1B)"),
            (WireFormat::U32Bool, "U32Bool (4B)"),
        ],
        Ok(DataType::Usize) => &[(WireFormat::U32Usize, "U32Usize (4B)")],
        Ok(DataType::List(inner)) if *inner == DataType::Usize => {
            &[(WireFormat::U32UsizeVec, "U32UsizeVec")]
        }
        _ => &[],
    }
}

/// Normalizes a candidate wire-format selection for `ty_str`, clearing incompatible formats and
/// substituting the type's default adapter when an explicit adapter is required.
pub fn normalize_wire_format_for_type(
    ty_str: &str,
    selected: Option<WireFormat>,
) -> Option<WireFormat> {
    let opts = compatible_wire_formats(ty_str);
    if opts.is_empty() {
        return None;
    }
    match selected {
        Some(wf) if opts.iter().any(|(cand, _)| *cand == wf) => Some(wf),
        _ => DataType::parse_str(ty_str.trim())
            .ok()
            .and_then(|dt| dt.default_wire_format())
            .or_else(|| opts.first().map(|(wf, _)| *wf)),
    }
}

/// Right-hand pane displaying the non-indenting vertical schema stack and live in-memory editors.
#[component]
pub fn SchemaEditorPane() -> Element {
    let mut state = use_context::<EditorState>();
    let resolved = state.resolved_schema.read().clone();
    let schema_name = state.schema.read().name.clone();
    let is_dirty = *state.is_schema_dirty.read();
    let stack = state.schema_stack.read().clone();
    let expanded_levels = state.expanded_stack_levels.read().clone();
    let df_opt = state.dynamic_file.read().clone();

    let all_file_rows: Vec<FlatTreeRow> = df_opt
        .as_ref()
        .map(|df| build_flat_tree(df, &BTreeSet::new(), 4000))
        .unwrap_or_default();

    let stack_len = stack.len();
    let dirty_badge = if is_dirty { " *" } else { "" };

    rsx! {
        rect {
            width: "415",
            height: "fill",
            content: "flex",
            padding: "8 10",
            background: theme::BG_PANE_RIGHT,
            border: "0 0 0 1 solid rgb(52, 57, 74)",
            direction: "vertical",
            spacing: "8",

            rect {
                width: "fill",
                direction: "horizontal",
                main_align: "space-between",
                cross_align: "center",
                spacing: "6",

                rect {
                    direction: "horizontal",
                    cross_align: "center",
                    spacing: "6",
                    label {
                        font_size: "13",
                        font_weight: "bold",
                        color: theme::ACCENT_BLUE,
                        max_lines: "1",
                        "Schema Stack"
                    }
                    label {
                        font_size: "11",
                        font_weight: if is_dirty { "bold" } else { "normal" },
                        color: if is_dirty { theme::ACCENT_AMBER } else { theme::TEXT_MUTED },
                        max_lines: "1",
                        "{schema_name}{dirty_badge}"
                    }
                }

                if stack_len > 1 {
                    ActionButton {
                        label: "Root".to_owned(),
                        style: ButtonStyle::Secondary,
                        compact: Some(true),
                        onpress: move |_| state.focus_schema_level(0),
                    }
                }
            }

            label {
                font_size: "10",
                color: theme::TEXT_DIM,
                max_lines: "1",
                "Hover a level or child to highlight in file · Edits apply live (Ctrl+S saves)"
            }

            ScrollView {
                width: "fill",
                height: "flex(1)",
                rect {
                    width: "fill",
                    direction: "vertical",
                    spacing: "6",

                    for (idx, step) in stack.iter().enumerate() {
                        {
                            let is_active = idx + 1 == stack_len;
                            let is_expanded = is_active || expanded_levels.contains(&idx);
                            let next_edge = stack.get(idx + 1).and_then(|s| s.incoming_edge.clone());
                            rsx! {
                                SchemaStackCard {
                                    key: "{idx}-{step_key(step)}",
                                    level_idx: idx,
                                    step: step.clone(),
                                    is_active,
                                    is_expanded,
                                    next_edge,
                                    all_file_rows: all_file_rows.clone(),
                                    resolved: resolved.clone(),
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn step_key(step: &SchemaStackStep) -> String {
    match &step.target {
        SchemaLevelTarget::Type {
            module_id,
            type_name,
        } => format!("T:{}:{}", module_id.0, type_name),
        SchemaLevelTarget::LeafField {
            parent_module_id,
            parent_type_name,
            is_mini_chunk,
            id,
            ..
        } => format!("L:{}:{parent_type_name}:{is_mini_chunk}:{id}", parent_module_id.0),
        SchemaLevelTarget::UnassignedFileNode {
            tree_path,
            id,
            is_mini_chunk,
            ..
        } => format!("U:{tree_path}:{is_mini_chunk}:{id}"),
    }
}

#[component]
fn SchemaStackCard(
    level_idx: usize,
    step: SchemaStackStep,
    is_active: bool,
    is_expanded: bool,
    next_edge: Option<SchemaParentEdge>,
    all_file_rows: Vec<FlatTreeRow>,
    resolved: ResolvedSchema,
) -> Element {
    let mut state = use_context::<EditorState>();
    let card_hover = step_hover_target(&step);
    let match_count = card_hover
        .as_ref()
        .map(|t| count_file_matches(&resolved, &all_file_rows, t));
    let first_match_path = if step.file_tree_path.is_none() {
        card_hover.as_ref().and_then(|t| {
            all_file_rows
                .iter()
                .find(|r| crate::schema_nav::matches_hover_target(&resolved, r, t))
                .map(|r| r.tree_path.clone())
        })
    } else {
        None
    };

    let (kind_badge, title, badge_color) = match &step.target {
        SchemaLevelTarget::Type {
            module_id,
            type_name,
        } => {
            let kind = type_kind_badge(&resolved, *module_id, type_name);
            let disp = resolved.format_type_display(*module_id, type_name);
            let color = match kind {
                "Enum" => theme::ACCENT_AMBER,
                "ByteBlock" => theme::ACCENT_GREEN,
                "MiniChunks" => theme::ACCENT_AMBER,
                _ => theme::ACCENT_BLUE,
            };
            (kind, disp, color)
        }
        SchemaLevelTarget::LeafField {
            is_mini_chunk,
            id,
            field_name,
            data_type,
            ..
        } => {
            let id_str = if *is_mini_chunk {
                format!("mc {:02X}", id)
            } else {
                format!("0x{id:04X}")
            };
            let ty_str = data_type
                .as_ref()
                .map(DataType::label)
                .unwrap_or_else(|| "ignore".to_owned());
            (
                "Leaf",
                format!("{field_name} ({id_str}: {ty_str})"),
                theme::ACCENT_GREEN,
            )
        }
        SchemaLevelTarget::UnassignedFileNode {
            id, is_mini_chunk, ..
        } => {
            let id_str = if *is_mini_chunk {
                format!("mc {:02X}", id)
            } else {
                format!("0x{id:04X}")
            };
            ("Unassigned", format!("Unassigned {id_str}"), theme::ACCENT_AMBER)
        }
    };

    let card_bg = if is_active {
        theme::BG_CARD
    } else {
        theme::BG_STACK_PARENT
    };
    let card_border = if is_active {
        theme::BORDER_STACK_ACTIVE
    } else {
        "1 solid rgb(48, 53, 68)"
    };
    let toggle_glyph = if is_active {
        "●"
    } else if is_expanded {
        "[-]"
    } else {
        "[+]"
    };
    let next_hint = next_edge
        .as_ref()
        .map(|edge| format!("→ {}", edge.short_name()));
    let match_txt = match_count.map(|c| format!("{c} in file"));
    let match_color = match match_count {
        Some(0) => theme::TEXT_DIM,
        Some(_) => theme::ACCENT_GREEN,
        None => theme::TEXT_DIM,
    };

    let enter_hover = card_hover.clone();
    let leave_hover_clear = card_hover.clone();

    rsx! {
        rect {
            width: "fill",
            padding: if is_expanded { "8 10" } else { "6 10" },
            corner_radius: "6",
            background: "{card_bg}",
            border: "{card_border}",
            direction: "vertical",
            spacing: "7",
            onpointerenter: move |_| {
                if enter_hover.is_some() {
                    state.set_hovered_schema_target(enter_hover.clone());
                }
            },
            onpointerleave: move |_| {
                if leave_hover_clear.is_some() {
                    state.set_hovered_schema_target(None);
                }
            },

            // Card header row (fixed height, clipped, no flex inside horizontal row)
            rect {
                width: "fill",
                height: "24",
                direction: "horizontal",
                cross_align: "center",
                spacing: "6",
                overflow: "clip",
                onpointerdown: move |_| {
                    if !is_active {
                        state.toggle_stack_level_expanded(level_idx);
                    }
                },

                label {
                    width: "22",
                    font_size: "10",
                    font_family: "monospace",
                    font_weight: "bold",
                    color: if is_active { theme::ACCENT_BLUE } else { theme::TEXT_MUTED },
                    max_lines: "1",
                    "{toggle_glyph}"
                }

                rect {
                    padding: "1 5",
                    corner_radius: "3",
                    background: theme::BG_INSET,
                    border: "1 solid rgb(52, 57, 74)",
                    label {
                        font_size: "10",
                        font_weight: "bold",
                        color: "{badge_color}",
                        max_lines: "1",
                        "{kind_badge}"
                    }
                }

                label {
                    font_size: "12",
                    font_weight: "bold",
                    color: theme::TEXT_PRIMARY,
                    max_lines: "1",
                    "{title}"
                }

                if !is_expanded {
                    if let Some(hint) = &next_hint {
                        label {
                            font_size: "11",
                            color: theme::ACCENT_BLUE,
                            max_lines: "1",
                            "{hint}"
                        }
                    }
                }

                if let Some(mtxt) = &match_txt {
                    label {
                        font_size: "10",
                        font_family: "monospace",
                        color: "{match_color}",
                        max_lines: "1",
                        "· {mtxt}"
                    }
                }
            }

            // Expanded body
            if is_expanded {
                // Incoming edge subtitle + quick jump / focus actions
                if !is_active || step.incoming_edge.is_some() || first_match_path.is_some() {
                    rect {
                        width: "fill",
                        direction: "horizontal",
                        cross_align: "center",
                        spacing: "6",

                        if let Some(edge) = &step.incoming_edge {
                            label {
                                font_size: "11",
                                color: theme::TEXT_MUTED,
                                max_lines: "1",
                                "via {edge.display_label()}"
                            }
                        }

                        if !is_active {
                            ActionButton {
                                label: "Focus Here".to_owned(),
                                style: ButtonStyle::Secondary,
                                compact: Some(true),
                                onpress: move |_| state.focus_schema_level(level_idx),
                            }
                        }

                        if let Some(jump_path) = first_match_path {
                            ActionButton {
                                label: "Select in File".to_owned(),
                                style: ButtonStyle::Secondary,
                                compact: Some(true),
                                onpress: move |_| state.select_node(jump_path.clone()),
                            }
                        }
                    }
                }

                // Active card: show live auto-applying Field Editor for the incoming field/leaf
                if is_active {
                    if let Some(edge) = &step.incoming_edge {
                        LiveIncomingFieldSection {
                            edge: edge.clone(),
                            resolved: resolved.clone(),
                        }
                    }
                }

                // Target-specific body (TypeDef children/settings/ByteBlock, or Unassigned)
                match &step.target {
                    SchemaLevelTarget::Type { module_id, type_name } => rsx! {
                        TypeLevelBody {
                            level_idx,
                            module_id: *module_id,
                            type_name: type_name.clone(),
                            is_active,
                            next_edge: next_edge.clone(),
                            parent_hover: card_hover.clone(),
                            all_file_rows: all_file_rows.clone(),
                            resolved: resolved.clone(),
                        }
                    },
                    SchemaLevelTarget::LeafField { .. } => rsx! {},
                    SchemaLevelTarget::UnassignedFileNode {
                        id,
                        is_mini_chunk,
                        is_container,
                        looks_like_mini_chunks,
                        parent_module_id,
                        parent_type_name,
                        ..
                    } => rsx! {
                        UnassignedNodeBody {
                            id: *id,
                            is_mini_chunk: *is_mini_chunk,
                            is_container: *is_container,
                            looks_like_mini_chunks: *looks_like_mini_chunks,
                            parent_module_id: *parent_module_id,
                            parent_type_name: parent_type_name.clone(),
                            resolved: resolved.clone(),
                        }
                    },
                }
            }
        }
    }
}

#[component]
fn TypeLevelBody(
    level_idx: usize,
    module_id: SchemaId,
    type_name: String,
    is_active: bool,
    next_edge: Option<SchemaParentEdge>,
    parent_hover: Option<HoveredSchemaTarget>,
    all_file_rows: Vec<FlatTreeRow>,
    resolved: ResolvedSchema,
) -> Element {
    let mut state = use_context::<EditorState>();
    let is_local = module_id == SchemaId::ROOT;
    let module = resolved.module(module_id);
    let typedef = module.schema.types.get(&type_name).cloned();

    let children = list_level_children(&resolved, module_id, &type_name, &all_file_rows);
    let imported_source = if !is_local {
        module.source.clone()
    } else {
        None
    };
    let imported_alias = module
        .module_name
        .clone()
        .unwrap_or_else(|| module.schema.name.clone());

    rsx! {
        if !is_local {
            rect {
                width: "fill",
                padding: "6 8",
                corner_radius: "4",
                background: theme::BG_SUB_CARD,
                border: "1 solid rgb(120, 95, 45)",
                direction: "vertical",
                spacing: "5",
                label {
                    font_size: "11",
                    color: theme::ACCENT_AMBER,
                    "Read-only imported type from `{imported_alias}`."
                }
                if let Some(src_ref) = imported_source {
                    rect {
                        direction: "horizontal",
                        ActionButton {
                            label: format!("Open `{imported_alias}` Schema"),
                            style: ButtonStyle::Secondary,
                            compact: Some(true),
                            onpress: move |_| state.request_load_schema_ref(src_ref.clone()),
                        }
                    }
                }
            }
        }

        match typedef {
            Some(TypeDef::Chunks(cc)) => rsx! {
                if is_local {
                    LiveContainerToggles {
                        type_name: type_name.clone(),
                        is_mini_chunks: false,
                        mode: cc.mode,
                        ignore_unknown: cc.ignore_unknown,
                    }
                }
                DefinedChildrenList {
                    level_idx,
                    heading: format!("Defined Child Chunks ({})", children.len()),
                    items: children,
                    active_edge: next_edge,
                    parent_hover,
                }
                if is_active && is_local {
                    AddChildFieldForm {
                        parent_type_name: type_name.clone(),
                        is_mini_chunks: false,
                    }
                }
            },
            Some(TypeDef::MiniChunks(mc)) => rsx! {
                if is_local {
                    LiveContainerToggles {
                        type_name: type_name.clone(),
                        is_mini_chunks: true,
                        mode: mc.mode,
                        ignore_unknown: mc.ignore_unknown,
                    }
                }
                DefinedChildrenList {
                    level_idx,
                    heading: format!("Defined Mini-Chunks ({})", children.len()),
                    items: children,
                    active_edge: next_edge,
                    parent_hover,
                }
                if is_active && is_local {
                    AddChildFieldForm {
                        parent_type_name: type_name.clone(),
                        is_mini_chunks: true,
                    }
                }
            },
            Some(TypeDef::ChunkEnum(_)) => rsx! {
                DefinedChildrenList {
                    level_idx,
                    heading: format!("Enum Variants ({}) — click to inspect", children.len()),
                    items: children,
                    active_edge: next_edge,
                    parent_hover,
                }
            },
            Some(TypeDef::ChunkStreamSeq(_)) => rsx! {
                DefinedChildrenList {
                    level_idx,
                    heading: format!("Stream Sequence Fields ({})", children.len()),
                    items: children,
                    active_edge: next_edge,
                    parent_hover,
                }
            },
            Some(TypeDef::ByteBlock(bb)) => rsx! {
                LiveByteBlockSection {
                    type_name: type_name.clone(),
                    is_local,
                    byte_block: bb,
                }
            },
            None => rsx! {
                label {
                    font_size: "11",
                    color: theme::ACCENT_RED,
                    "Type `{type_name}` not found in schema."
                }
            },
        }
    }
}

#[component]
fn LiveContainerToggles(
    type_name: String,
    is_mini_chunks: bool,
    mode: OrderMode,
    ignore_unknown: bool,
) -> Element {
    let mut state = use_context::<EditorState>();
    let t_map = type_name.clone();
    let t_seq = type_name.clone();
    let t_strict = type_name.clone();
    let t_ignore = type_name;

    rsx! {
        rect {
            width: "fill",
            padding: "6 8",
            corner_radius: "4",
            background: theme::BG_SUB_CARD,
            direction: "vertical",
            spacing: "5",

            rect {
                width: "fill",
                direction: "horizontal",
                cross_align: "center",
                spacing: "6",
                label { width: "74", font_size: "11", color: theme::TEXT_MUTED, "Order" }
                SegmentedChip {
                    label: "Map".to_owned(),
                    selected: mode == OrderMode::Map,
                    compact: Some(true),
                    onpress: move |_| {
                        let t = t_map.clone();
                        state.mutate_schema(format!("Set `{t}` order = map"), move |s, _| {
                            set_type_order_mode(s, &t, is_mini_chunks, OrderMode::Map);
                            Ok(())
                        });
                    },
                }
                SegmentedChip {
                    label: "Sequence".to_owned(),
                    selected: mode == OrderMode::Sequence,
                    compact: Some(true),
                    onpress: move |_| {
                        let t = t_seq.clone();
                        state.mutate_schema(format!("Set `{t}` order = sequence"), move |s, _| {
                            set_type_order_mode(s, &t, is_mini_chunks, OrderMode::Sequence);
                            Ok(())
                        });
                    },
                }
            }

            rect {
                width: "fill",
                direction: "horizontal",
                cross_align: "center",
                spacing: "6",
                label { width: "74", font_size: "11", color: theme::TEXT_MUTED, "Unknown" }
                SegmentedChip {
                    label: "Strict".to_owned(),
                    selected: !ignore_unknown,
                    compact: Some(true),
                    onpress: move |_| {
                        let t = t_strict.clone();
                        state.mutate_schema(format!("Set `{t}` ignore_unknown = false"), move |s, _| {
                            set_type_ignore_unknown(s, &t, is_mini_chunks, false);
                            Ok(())
                        });
                    },
                }
                SegmentedChip {
                    label: "Ignore".to_owned(),
                    selected: ignore_unknown,
                    compact: Some(true),
                    onpress: move |_| {
                        let t = t_ignore.clone();
                        state.mutate_schema(format!("Set `{t}` ignore_unknown = true"), move |s, _| {
                            set_type_ignore_unknown(s, &t, is_mini_chunks, true);
                            Ok(())
                        });
                    },
                }
            }
        }
    }
}

fn set_type_order_mode(schema: &mut Schema, type_name: &str, is_mini: bool, mode: OrderMode) {
    match schema.types.get_mut(type_name) {
        Some(TypeDef::Chunks(c)) if !is_mini => c.mode = mode,
        Some(TypeDef::MiniChunks(m)) if is_mini => m.mode = mode,
        _ => {}
    }
}

fn set_type_ignore_unknown(schema: &mut Schema, type_name: &str, is_mini: bool, ignore: bool) {
    match schema.types.get_mut(type_name) {
        Some(TypeDef::Chunks(c)) if !is_mini => c.ignore_unknown = ignore,
        Some(TypeDef::MiniChunks(m)) if is_mini => m.ignore_unknown = ignore,
        _ => {}
    }
}

#[component]
fn DefinedChildrenList(
    level_idx: usize,
    heading: String,
    items: Vec<SchemaChildItem>,
    active_edge: Option<SchemaParentEdge>,
    parent_hover: Option<HoveredSchemaTarget>,
) -> Element {
    let mut state = use_context::<EditorState>();

    rsx! {
        rect {
            width: "fill",
            direction: "vertical",
            spacing: "4",

            GroupCaption { text: heading }

            if items.is_empty() {
                label {
                    font_size: "11",
                    color: theme::TEXT_DIM,
                    "No child fields defined yet."
                }
            } else {
                rect {
                    width: "fill",
                    padding: "4",
                    corner_radius: "4",
                    background: theme::BG_INSET,
                    border: "1 solid rgb(44, 48, 62)",
                    direction: "vertical",
                    spacing: "2",

                    for item in items {
                        {
                            let is_current = active_edge.as_ref() == Some(&item.edge);
                            let bg = if is_current {
                                theme::BG_ROW_SELECTED
                            } else {
                                theme::BG_ROW
                            };
                            let border = if is_current {
                                theme::BORDER_CHIP_ACTIVE
                            } else {
                                "1 solid transparent"
                            };
                            let count_color = if item.file_match_count > 0 {
                                theme::ACCENT_GREEN
                            } else {
                                theme::TEXT_DIM
                            };
                            let name_color = if item.file_match_count > 0 {
                                theme::TEXT_PRIMARY
                            } else {
                                theme::TEXT_MUTED
                            };
                            let child_hover = item.hover_target.clone();
                            let restore_hover = parent_hover.clone();
                            let click_item = item.clone();
                            let count_label = format!("{}x", item.file_match_count);
                            rsx! {
                                rect {
                                    width: "fill",
                                    height: "26",
                                    padding: "0 6",
                                    corner_radius: "3",
                                    background: "{bg}",
                                    border: "{border}",
                                    direction: "horizontal",
                                    cross_align: "center",
                                    spacing: "5",
                                    overflow: "clip",
                                    onpointerenter: move |e: PointerEvent| {
                                        e.stop_propagation();
                                        state.set_hovered_schema_target(Some(child_hover.clone()));
                                    },
                                    onpointerleave: move |_| {
                                        state.set_hovered_schema_target(restore_hover.clone());
                                    },
                                    onpointerdown: move |_| {
                                        state.select_schema_child(level_idx, click_item.clone());
                                    },

                                    label {
                                        width: "52",
                                        font_size: "10",
                                        font_family: "monospace",
                                        color: theme::ACCENT_BLUE,
                                        max_lines: "1",
                                        "{item.id_label}"
                                    }
                                    label {
                                        width: "28",
                                        font_size: "10",
                                        font_family: "monospace",
                                        color: theme::ACCENT_AMBER,
                                        max_lines: "1",
                                        "{item.cardinality_badge}"
                                    }
                                    label {
                                        width: "215",
                                        font_size: "11",
                                        font_weight: if is_current { "bold" } else { "normal" },
                                        color: "{name_color}",
                                        max_lines: "1",
                                        "{item.field_name}: {item.type_summary}"
                                    }
                                    label {
                                        width: "34",
                                        font_size: "10",
                                        font_family: "monospace",
                                        color: "{count_color}",
                                        max_lines: "1",
                                        "{count_label}"
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
fn AddChildFieldForm(parent_type_name: String, is_mini_chunks: bool) -> Element {
    let mut state = use_context::<EditorState>();
    let mut id_input = use_signal(String::new);
    let mut name_input = use_signal(String::new);

    let id_placeholder = if is_mini_chunks { "0x01" } else { "0x0400" };
    let parent_clone = parent_type_name.clone();

    rsx! {
        rect {
            width: "fill",
            padding: "6 8",
            corner_radius: "4",
            background: theme::BG_SUB_CARD,
            direction: "vertical",
            spacing: "5",

            GroupCaption {
                text: if is_mini_chunks {
                    "DEFINE NEW MINI-CHUNK IN SCHEMA".to_owned()
                } else {
                    "DEFINE NEW CHUNK IN SCHEMA".to_owned()
                }
            }

            rect {
                width: "fill",
                direction: "horizontal",
                cross_align: "center",
                spacing: "6",

                StyledInput {
                    width: Some("76".to_owned()),
                    value: id_input.read().clone(),
                    placeholder: Some(id_placeholder.to_owned()),
                    onchange: move |txt| id_input.set(txt),
                }
                StyledInput {
                    width: Some("165".to_owned()),
                    value: name_input.read().clone(),
                    placeholder: Some("field_name".to_owned()),
                    onchange: move |txt| name_input.set(txt),
                }
                ActionButton {
                    label: "+ Add".to_owned(),
                    style: ButtonStyle::Secondary,
                    compact: Some(true),
                    onpress: move |_| {
                        let raw_id = id_input.read().trim().to_owned();
                        let parsed_id = parse_hex_or_dec_u32(&raw_id);
                        let Some(id_val) = parsed_id else {
                            state.set_status(format!("Invalid ID `{raw_id}` (use hex like 0x0400 or decimal)"));
                            return;
                        };
                        let raw_name = name_input.read().trim().to_owned();
                        let fname = if raw_name.is_empty() {
                            if is_mini_chunks {
                                format!("mc_{:02x}", id_val as u8)
                            } else {
                                format!("chunk_{id_val:04x}")
                            }
                        } else {
                            raw_name
                        };
                        let parent_t = parent_clone.clone();
                        let ok = if is_mini_chunks {
                            let Ok(mc_id) = u8::try_from(id_val) else {
                                state.set_status("Mini-chunk ID must be 0..=255 (0x00..=0xFF)".to_owned());
                                return;
                            };
                            let pref = ParentContainerRef::MiniChunksType {
                                type_name: parent_t.clone(),
                            };
                            let f_copy = fname.clone();
                            state.mutate_schema(
                                format!("Added mini-chunk `{fname}` (0x{mc_id:02X}) to `{parent_t}`"),
                                move |s, _| {
                                    upsert_mini_chunk_field(
                                        s,
                                        &pref,
                                        mc_id,
                                        &f_copy,
                                        Cardinality::Optional,
                                        DataType::RawBytes,
                                        None,
                                    )
                                },
                            )
                        } else {
                            let pref = ParentContainerRef::ChunksType {
                                type_name: parent_t.clone(),
                            };
                            let f_copy = fname.clone();
                            state.mutate_schema(
                                format!("Added chunk `{fname}` (0x{id_val:04X}) to `{parent_t}`"),
                                move |s, _| {
                                    upsert_chunk_field(
                                        s,
                                        &pref,
                                        id_val,
                                        &f_copy,
                                        Cardinality::Optional,
                                        ChunkPayloadSpec::Data(DataType::RawBytes),
                                        None,
                                    )
                                },
                            )
                        };
                        if ok {
                            id_input.set(String::new());
                            name_input.set(String::new());
                        }
                    },
                }
            }
        }
    }
}

fn parse_hex_or_dec_u32(s: &str) -> Option<u32> {
    let trimmed = s.trim();
    if let Some(hex) = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
    {
        u32::from_str_radix(hex, 16).ok()
    } else {
        trimmed.parse::<u32>().ok()
    }
}

#[component]
fn UnassignedNodeBody(
    id: u32,
    is_mini_chunk: bool,
    is_container: bool,
    looks_like_mini_chunks: bool,
    parent_module_id: Option<SchemaId>,
    parent_type_name: Option<String>,
    resolved: ResolvedSchema,
) -> Element {
    let mut state = use_context::<EditorState>();

    let Some((pmod, ptype)) = parent_module_id.zip(parent_type_name) else {
        return rsx! {
            label {
                font_size: "11",
                color: theme::ACCENT_AMBER,
                "Parent chunk is not mapped to a schema container yet. Define the parent level above first."
            }
        };
    };

    if pmod != SchemaId::ROOT {
        let module = resolved.module(pmod);
        let alias = module
            .module_name
            .clone()
            .unwrap_or_else(|| module.schema.name.clone());
        let src_opt = module.source.clone();
        return rsx! {
            label {
                font_size: "11",
                color: theme::ACCENT_AMBER,
                "Parent `{ptype}` belongs to imported schema `{alias}` (read-only)."
            }
            if let Some(src_ref) = src_opt {
                ActionButton {
                    label: format!("Open `{alias}` Schema"),
                    style: ButtonStyle::Secondary,
                    compact: Some(true),
                    onpress: move |_| state.request_load_schema_ref(src_ref.clone()),
                }
            }
        };
    }

    let parent_def = resolved.module(pmod).schema.types.get(&ptype);
    let can_define = if is_mini_chunk {
        matches!(parent_def, Some(TypeDef::MiniChunks(_)))
    } else {
        matches!(parent_def, Some(TypeDef::Chunks(_) | TypeDef::ChunkStreamSeq(_)))
    };

    if !can_define {
        return rsx! {
            label {
                font_size: "11",
                color: theme::ACCENT_AMBER,
                "Parent `{ptype}` is not an editable chunk/mini-chunk container."
            }
        };
    }

    let ptype_btn = ptype.clone();
    let default_name = if is_mini_chunk {
        format!("mc_{:02x}", id as u8)
    } else {
        format!("chunk_{id:04x}")
    };

    rsx! {
        rect {
            width: "fill",
            padding: "8",
            corner_radius: "4",
            background: theme::BG_SUB_CARD,
            direction: "vertical",
            spacing: "6",

            label {
                font_size: "11",
                color: theme::TEXT_MUTED,
                if is_mini_chunk {
                    "Mini-chunk 0x{id:02X} is not yet defined in `{ptype}`."
                } else {
                    "Chunk 0x{id:04X} is not yet defined in `{ptype}`."
                }
            }

            rect {
                direction: "horizontal",
                ActionButton {
                    label: format!("+ Define in `{ptype_btn}`"),
                    style: ButtonStyle::Primary,
                    onpress: move |_| {
                        let parent_name = ptype_btn.clone();
                        let fname = default_name.clone();
                        if is_mini_chunk {
                            let pref = ParentContainerRef::MiniChunksType {
                                type_name: parent_name.clone(),
                            };
                            state.mutate_schema(
                                format!("Defined mini-chunk `{fname}` (0x{:02X})", id as u8),
                                move |s, _| {
                                    upsert_mini_chunk_field(
                                        s,
                                        &pref,
                                        id as u8,
                                        &fname,
                                        Cardinality::Optional,
                                        DataType::RawBytes,
                                        None,
                                    )
                                },
                            );
                        } else {
                            let pref = ParentContainerRef::ChunksType {
                                type_name: parent_name.clone(),
                            };
                            let payload = if is_container {
                                ChunkPayloadSpec::Children(format!(
                                    "{}Container",
                                    to_pascal_case(&fname)
                                ))
                            } else if looks_like_mini_chunks {
                                ChunkPayloadSpec::MiniChunks(format!(
                                    "{}MiniChunks",
                                    to_pascal_case(&fname)
                                ))
                            } else {
                                ChunkPayloadSpec::Data(DataType::RawBytes)
                            };
                            state.mutate_schema(
                                format!("Defined chunk `{fname}` (0x{id:04X})"),
                                move |s, _| {
                                    upsert_chunk_field(
                                        s,
                                        &pref,
                                        id,
                                        &fname,
                                        Cardinality::Optional,
                                        payload,
                                        None,
                                    )
                                },
                            );
                        }
                    },
                }
            }
        }
    }
}

/// Live auto-applying editor for the field definition (`ChunkFieldDef` or `MiniChunkFieldDef`)
/// on the parent container that points to the active stack level. Edits commit directly to the
/// in-memory schema without requiring a separate "Save Field" button.
#[component]
fn LiveIncomingFieldSection(edge: SchemaParentEdge, resolved: ResolvedSchema) -> Element {
    let mut state = use_context::<EditorState>();

    // Determine whether this incoming edge is a local editable chunk/mini-chunk ID field.
    enum EditableField {
        Chunk {
            parent_type: String,
            chunk_id: u32,
            def: ChunkFieldDef,
        },
        MiniChunk {
            parent_type: String,
            mc_id: u8,
            def: MiniChunkFieldDef,
        },
    }

    let editable = match &edge {
        SchemaParentEdge::ChunkField {
            parent_module_id,
            parent_type_name,
            chunk_id: Some(cid),
            ..
        } if *parent_module_id == SchemaId::ROOT => {
            let fields = match resolved.root_schema().types.get(parent_type_name) {
                Some(TypeDef::Chunks(c)) => Some(c.fields.as_slice()),
                Some(TypeDef::ChunkStreamSeq(s)) => Some(s.fields.as_slice()),
                _ => None,
            };
            fields
                .and_then(|fs| {
                    fs.iter().find(|f| {
                        matches!(f.source, ChunkFieldSource::Id { id, .. } if id == *cid)
                    })
                })
                .map(|def| EditableField::Chunk {
                    parent_type: parent_type_name.clone(),
                    chunk_id: *cid,
                    def: def.clone(),
                })
        }
        SchemaParentEdge::MiniChunkField {
            parent_module_id,
            parent_type_name,
            mc_id,
            ..
        } if *parent_module_id == SchemaId::ROOT => {
            if let Some(TypeDef::MiniChunks(mc)) =
                resolved.root_schema().types.get(parent_type_name)
            {
                mc.fields
                    .iter()
                    .find(|f| f.id == *mc_id)
                    .map(|def| EditableField::MiniChunk {
                        parent_type: parent_type_name.clone(),
                        mc_id: *mc_id,
                        def: def.clone(),
                    })
            } else {
                None
            }
        }
        _ => None,
    };

    let Some(editable) = editable else {
        return rsx! {};
    };

    let (identity_key, is_mini, parent_type, field_id, schema_name_val, schema_card, schema_pkind, schema_ty_str, schema_wf) =
        match &editable {
            EditableField::Chunk {
                parent_type,
                chunk_id,
                def,
            } => {
                let (pkind, tystr) = match &def.source {
                    ChunkFieldSource::Id { payload, .. } => match payload {
                        ChunkPayloadSpec::Data(dt) => ("data", dt.label()),
                        ChunkPayloadSpec::MiniChunks(t) => ("mini_chunks", t.clone()),
                        ChunkPayloadSpec::Children(t) => ("children", t.clone()),
                        ChunkPayloadSpec::Ignore => ("ignore", String::new()),
                    },
                    ChunkFieldSource::Stream { item_type } => ("children", item_type.clone()),
                };
                (
                    format!("C:{parent_type}:{chunk_id}"),
                    false,
                    parent_type.clone(),
                    *chunk_id,
                    def.name.clone(),
                    cardinality_to_key(&def.cardinality).to_owned(),
                    pkind.to_owned(),
                    tystr,
                    def.wire_format,
                )
            }
            EditableField::MiniChunk {
                parent_type,
                mc_id,
                def,
            } => (
                format!("M:{parent_type}:{mc_id}"),
                true,
                parent_type.clone(),
                *mc_id as u32,
                def.name.clone(),
                cardinality_to_key(&def.cardinality).to_owned(),
                "data".to_owned(),
                def.ty.label(),
                def.wire_format,
            ),
        };

    let mut synced_key = use_signal(String::new);
    let mut name_draft = use_signal(String::new);
    let mut type_draft = use_signal(String::new);
    let mut card_draft = use_signal(|| "required".to_owned());
    let mut pkind_draft = use_signal(|| "data".to_owned());
    let mut wf_draft = use_signal(|| None::<WireFormat>);
    let mut draft_hint = use_signal(|| None::<String>);

    if *synced_key.read() != identity_key {
        synced_key.set(identity_key);
        name_draft.set(schema_name_val);
        type_draft.set(schema_ty_str);
        card_draft.set(schema_card);
        pkind_draft.set(schema_pkind);
        wf_draft.set(schema_wf);
        draft_hint.set(None);
    }

    let apply_live = {
        let parent_t = parent_type.clone();
        move |next_name: String,
              next_card: String,
              next_pkind: String,
              next_ty: String,
              next_wf: Option<WireFormat>| {
            let clean_name = next_name.trim().to_owned();
            if clean_name.is_empty() {
                draft_hint.set(Some("Enter a field name to apply live.".to_owned()));
                return;
            }
            let card = key_to_cardinality(&next_card);
            let norm_wf = if is_mini || next_pkind == "data" {
                normalize_wire_format_for_type(&next_ty, next_wf)
            } else {
                None
            };
            wf_draft.set(norm_wf);

            if is_mini {
                match DataType::parse_str(next_ty.trim()) {
                    Ok(dt) => {
                        draft_hint.set(None);
                        let pref = ParentContainerRef::MiniChunksType {
                            type_name: parent_t.clone(),
                        };
                        let name_msg = clean_name.clone();
                        state.mutate_schema(
                            format!("Updated mini-chunk `{name_msg}` (0x{:02X})", field_id as u8),
                            move |s, _| {
                                upsert_mini_chunk_field(
                                    s,
                                    &pref,
                                    field_id as u8,
                                    &clean_name,
                                    card,
                                    dt,
                                    norm_wf,
                                )
                            },
                        );
                    }
                    Err(e) => {
                        draft_hint.set(Some(format!("Incomplete type: {e}")));
                    }
                }
            } else {
                let payload = match next_pkind.as_str() {
                    "children" => {
                        let t = next_ty.trim();
                        if t.is_empty() {
                            draft_hint.set(Some("Enter a child container type name.".to_owned()));
                            return;
                        }
                        ChunkPayloadSpec::Children(t.to_owned())
                    }
                    "mini_chunks" => {
                        let t = next_ty.trim();
                        if t.is_empty() {
                            draft_hint.set(Some("Enter a MiniChunks type name.".to_owned()));
                            return;
                        }
                        ChunkPayloadSpec::MiniChunks(t.to_owned())
                    }
                    "ignore" => ChunkPayloadSpec::Ignore,
                    _ => match DataType::parse_str(next_ty.trim()) {
                        Ok(dt) => ChunkPayloadSpec::Data(dt),
                        Err(e) => {
                            draft_hint.set(Some(format!("Incomplete type: {e}")));
                            return;
                        }
                    },
                };
                draft_hint.set(None);
                let pref = ParentContainerRef::ChunksType {
                    type_name: parent_t.clone(),
                };
                let name_msg = clean_name.clone();
                state.mutate_schema(
                    format!("Updated chunk `{name_msg}` (0x{field_id:04X})"),
                    move |s, _| {
                        upsert_chunk_field(s, &pref, field_id, &clean_name, card, payload, norm_wf)
                    },
                );
            }
        }
    };

    let cur_pkind = pkind_draft.read().clone();
    let cur_card = card_draft.read().clone();
    let cur_ty = type_draft.read().clone();
    let cur_wf = *wf_draft.read();
    let compat_wfs = if is_mini || cur_pkind == "data" {
        compatible_wire_formats(&cur_ty)
    } else {
        &[]
    };
    let parent_del = parent_type.clone();

    rsx! {
        rect {
            width: "fill",
            padding: "8",
            corner_radius: "5",
            background: theme::BG_SUB_CARD,
            border: "1 solid rgb(50, 56, 74)",
            direction: "vertical",
            spacing: "7",

            rect {
                width: "fill",
                direction: "horizontal",
                main_align: "space-between",
                cross_align: "center",
                GroupCaption {
                    text: if is_mini {
                        format!("FIELD IN `{parent_type}` (MC 0x{:02X})", field_id as u8)
                    } else {
                        format!("FIELD IN `{parent_type}` (0x{field_id:04X})")
                    }
                }
                ActionButton {
                    label: "Delete Field".to_owned(),
                    style: ButtonStyle::Danger,
                    compact: Some(true),
                    onpress: move |_| {
                        let pt = parent_del.clone();
                        if is_mini {
                            let pref = ParentContainerRef::MiniChunksType { type_name: pt };
                            state.mutate_schema(
                                format!("Deleted mini-chunk field 0x{:02X}", field_id as u8),
                                move |s, _| {
                                    delete_mini_chunk_field(s, &pref, field_id as u8).map(|_| ())
                                },
                            );
                        } else {
                            let pref = ParentContainerRef::ChunksType { type_name: pt };
                            state.mutate_schema(
                                format!("Deleted chunk field 0x{field_id:04X}"),
                                move |s, _| delete_chunk_field(s, &pref, field_id).map(|_| ()),
                            );
                        }
                    },
                }
            }

            // Field Name (live auto-apply)
            rect {
                width: "fill",
                direction: "horizontal",
                cross_align: "center",
                spacing: "8",
                label { width: "74", font_size: "11", color: theme::TEXT_MUTED, "Field Name" }
                StyledInput {
                    width: Some("270".to_owned()),
                    value: name_draft.read().clone(),
                    placeholder: Some("snake_case_name".to_owned()),
                    onchange: {
                        let mut apply = apply_live.clone();
                        move |txt: String| {
                            name_draft.set(txt.clone());
                            apply(
                                txt,
                                card_draft.read().clone(),
                                pkind_draft.read().clone(),
                                type_draft.read().clone(),
                                *wf_draft.read(),
                            );
                        }
                    },
                }
            }

            // Cardinality radio chips
            rect {
                width: "fill",
                direction: "horizontal",
                cross_align: "center",
                spacing: "5",
                label { width: "74", font_size: "11", color: theme::TEXT_MUTED, "Cardinality" }
                for (ckey, clabel) in [
                    ("required", "Required"),
                    ("optional", "Optional"),
                    ("repeated", "Repeated"),
                    ("default", "Default"),
                ] {
                    {
                        let mut apply = apply_live.clone();
                        rsx! {
                            SegmentedChip {
                                key: "{ckey}",
                                label: clabel.to_owned(),
                                selected: cur_card == ckey,
                                compact: Some(true),
                                onpress: move |_| {
                                    card_draft.set(ckey.to_owned());
                                    apply(
                                        name_draft.read().clone(),
                                        ckey.to_owned(),
                                        pkind_draft.read().clone(),
                                        type_draft.read().clone(),
                                        *wf_draft.read(),
                                    );
                                },
                            }
                        }
                    }
                }
            }

            // Payload Kind radio chips (for 32-bit chunks)
            if !is_mini {
                rect {
                    width: "fill",
                    direction: "horizontal",
                    cross_align: "center",
                    spacing: "5",
                    label { width: "74", font_size: "11", color: theme::TEXT_MUTED, "Payload" }
                    for (pkey, plabel) in [
                        ("data", "Data"),
                        ("children", "Children"),
                        ("mini_chunks", "MiniChunks"),
                        ("ignore", "Ignore"),
                    ] {
                        {
                            let mut apply = apply_live.clone();
                            rsx! {
                                SegmentedChip {
                                    key: "{pkey}",
                                    label: plabel.to_owned(),
                                    selected: cur_pkind == pkey,
                                    compact: Some(true),
                                    onpress: move |_| {
                                        pkind_draft.set(pkey.to_owned());
                                        let next_ty = match pkey {
                                            "children" => {
                                                let cur = type_draft.read().clone();
                                                if cur.is_empty() || DataType::parse_str(&cur).is_ok_and(|d| !matches!(d, DataType::Named(_))) {
                                                    format!("{}Container", to_pascal_case(&name_draft.read()))
                                                } else {
                                                    cur
                                                }
                                            }
                                            "mini_chunks" => {
                                                let cur = type_draft.read().clone();
                                                if cur.is_empty() || DataType::parse_str(&cur).is_ok_and(|d| !matches!(d, DataType::Named(_))) {
                                                    format!("{}MiniChunks", to_pascal_case(&name_draft.read()))
                                                } else {
                                                    cur
                                                }
                                            }
                                            "ignore" => String::new(),
                                            _ => {
                                                let cur = type_draft.read().clone();
                                                if cur.is_empty() { "raw_bytes".to_owned() } else { cur }
                                            }
                                        };
                                        type_draft.set(next_ty.clone());
                                        apply(
                                            name_draft.read().clone(),
                                            card_draft.read().clone(),
                                            pkey.to_owned(),
                                            next_ty,
                                            *wf_draft.read(),
                                        );
                                    },
                                }
                            }
                        }
                    }
                }
            }

            // Type input + Data presets
            if is_mini || cur_pkind != "ignore" {
                rect {
                    width: "fill",
                    direction: "horizontal",
                    cross_align: "center",
                    spacing: "8",
                    label { width: "74", font_size: "11", color: theme::TEXT_MUTED, "Type" }
                    StyledInput {
                        width: Some("270".to_owned()),
                        value: type_draft.read().clone(),
                        placeholder: Some("u32, c_string, Vec<f32>, MyStruct...".to_owned()),
                        onchange: {
                            let mut apply = apply_live.clone();
                            move |txt: String| {
                                type_draft.set(txt.clone());
                                apply(
                                    name_draft.read().clone(),
                                    card_draft.read().clone(),
                                    pkind_draft.read().clone(),
                                    txt,
                                    *wf_draft.read(),
                                );
                            }
                        },
                    }
                }

                if is_mini || cur_pkind == "data" {
                    rect {
                        width: "fill",
                        direction: "horizontal",
                        cross_align: "center",
                        spacing: "4",
                        for preset in ["u32", "f32", "bool", "usize", "c_string", "vec3", "raw_bytes"] {
                            {
                                let mut apply = apply_live.clone();
                                rsx! {
                                    SegmentedChip {
                                        key: "{preset}",
                                        label: preset.to_owned(),
                                        selected: cur_ty.trim() == preset,
                                        compact: Some(true),
                                        onpress: move |_| {
                                            type_draft.set(preset.to_owned());
                                            apply(
                                                name_draft.read().clone(),
                                                card_draft.read().clone(),
                                                pkind_draft.read().clone(),
                                                preset.to_owned(),
                                                *wf_draft.read(),
                                            );
                                        },
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // Context-aware Wire Format selector (only rendered when compatible)
            if !compat_wfs.is_empty() {
                rect {
                    width: "fill",
                    direction: "horizontal",
                    cross_align: "center",
                    spacing: "5",
                    label { width: "74", font_size: "11", color: theme::TEXT_MUTED, "Wire Format" }
                    for &(wf, wf_label) in compat_wfs {
                        {
                            let mut apply = apply_live.clone();
                            let effective = normalize_wire_format_for_type(&cur_ty, cur_wf);
                            rsx! {
                                SegmentedChip {
                                    key: "{wf_label}",
                                    label: wf_label.to_owned(),
                                    selected: effective == Some(wf),
                                    compact: Some(true),
                                    onpress: move |_| {
                                        wf_draft.set(Some(wf));
                                        apply(
                                            name_draft.read().clone(),
                                            card_draft.read().clone(),
                                            pkind_draft.read().clone(),
                                            type_draft.read().clone(),
                                            Some(wf),
                                        );
                                    },
                                }
                            }
                        }
                    }
                }
            }

            if let Some(hint) = draft_hint.read().as_ref() {
                label {
                    font_size: "11",
                    color: theme::ACCENT_AMBER,
                    "{hint}"
                }
            }
        }
    }
}

fn cardinality_to_key(card: &Cardinality) -> &'static str {
    match card {
        Cardinality::Required => "required",
        Cardinality::Optional => "optional",
        Cardinality::Repeated => "repeated",
        Cardinality::Default | Cardinality::DefaultExpr(_) => "default",
    }
}

fn key_to_cardinality(key: &str) -> Cardinality {
    match key {
        "optional" => Cardinality::Optional,
        "repeated" => Cardinality::Repeated,
        "default" => Cardinality::Default,
        _ => Cardinality::Required,
    }
}

#[component]
fn LiveByteBlockSection(
    type_name: String,
    is_local: bool,
    byte_block: ByteBlockDef,
) -> Element {
    let mut state = use_context::<EditorState>();
    let mut synced_bb = use_signal(String::new);
    let mut bb_pad_input = use_signal(String::new);
    let mut new_fname = use_signal(String::new);
    let mut new_fty = use_signal(|| "u32".to_owned());
    let mut new_fwf = use_signal(|| None::<WireFormat>);
    let mut new_fpad = use_signal(String::new);

    let sync_sig = format!("{type_name}:{:?}", byte_block.pad_after);
    if *synced_bb.read() != sync_sig {
        synced_bb.set(sync_sig);
        bb_pad_input.set(byte_block.pad_after.map(|p| p.to_string()).unwrap_or_default());
    }

    let cur_new_ty = new_fty.read().clone();
    let new_compat_wfs = compatible_wire_formats(&cur_new_ty);
    let pad_bb_name = type_name.clone();
    let append_bb_name = type_name.clone();

    rsx! {
        rect {
            width: "fill",
            direction: "vertical",
            spacing: "6",

            rect {
                width: "fill",
                direction: "horizontal",
                main_align: "space-between",
                cross_align: "center",
                GroupCaption { text: format!("BYTEBLOCK FIELDS ({})", byte_block.fields.len()) }

                if is_local {
                    rect {
                        direction: "horizontal",
                        cross_align: "center",
                        spacing: "5",
                        label { font_size: "11", color: theme::TEXT_MUTED, "pad_after (B):" }
                        StyledInput {
                            width: Some("56".to_owned()),
                            value: bb_pad_input.read().clone(),
                            placeholder: Some("0".to_owned()),
                            onchange: move |txt: String| {
                                bb_pad_input.set(txt.clone());
                                let trimmed = txt.trim();
                                let parsed = if trimmed.is_empty() {
                                    Some(None)
                                } else {
                                    trimmed.parse::<usize>().ok().map(Some)
                                };
                                if let Some(pad_val) = parsed {
                                    let target = pad_bb_name.clone();
                                    state.mutate_schema(
                                        format!("Updated `{target}` pad_after = {pad_val:?}"),
                                        move |s, _| set_byte_block_pad_after(s, &target, pad_val),
                                    );
                                }
                            },
                        }
                    }
                } else if let Some(pad) = byte_block.pad_after {
                    label { font_size: "11", color: theme::TEXT_DIM, "pad_after: {pad}B" }
                }
            }

            if byte_block.fields.is_empty() {
                label {
                    font_size: "11",
                    color: theme::TEXT_DIM,
                    "No byte fields defined yet."
                }
            } else {
                rect {
                    width: "fill",
                    padding: "4",
                    corner_radius: "4",
                    background: theme::BG_INSET,
                    border: "1 solid rgb(44, 48, 62)",
                    direction: "vertical",
                    spacing: "2",

                    for (idx, field) in byte_block.fields.iter().enumerate() {
                        {
                            let wf_txt = field
                                .wire_format
                                .map(|w| format!(" [{}]", w.rust_type_name()))
                                .unwrap_or_default();
                            let pad_txt = field
                                .pad_after
                                .map(|p| format!(" +{p}B"))
                                .unwrap_or_default();
                            let del_bb = type_name.clone();
                            rsx! {
                                rect {
                                    width: "fill",
                                    height: "25",
                                    padding: "0 6",
                                    corner_radius: "3",
                                    background: theme::BG_ROW,
                                    direction: "horizontal",
                                    cross_align: "center",
                                    spacing: "6",
                                    overflow: "clip",

                                    label {
                                        width: if is_local { "305" } else { "345" },
                                        font_size: "11",
                                        font_family: "monospace",
                                        color: theme::TEXT_PRIMARY,
                                        max_lines: "1",
                                        "{idx}. {field.name}: {field.ty.label()}{wf_txt}{pad_txt}"
                                    }

                                    if is_local {
                                        ActionButton {
                                            label: "Del".to_owned(),
                                            style: ButtonStyle::Danger,
                                            compact: Some(true),
                                            onpress: move |_| {
                                                let bb_t = del_bb.clone();
                                                state.mutate_schema(
                                                    format!("Removed field #{idx} from `{bb_t}`"),
                                                    move |s, _| {
                                                        if let Some(TypeDef::ByteBlock(bb)) = s.types.get_mut(&bb_t)
                                                            && idx < bb.fields.len()
                                                        {
                                                            bb.fields.remove(idx);
                                                            Ok(())
                                                        } else {
                                                            Err("Field index out of bounds".to_owned())
                                                        }
                                                    },
                                                );
                                            },
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            if is_local {
                rect {
                    width: "fill",
                    padding: "6 8",
                    corner_radius: "4",
                    background: theme::BG_SUB_CARD,
                    direction: "vertical",
                    spacing: "5",

                    GroupCaption { text: "APPEND BYTEBLOCK FIELD".to_owned() }

                    rect {
                        width: "fill",
                        direction: "horizontal",
                        cross_align: "center",
                        spacing: "6",
                        StyledInput {
                            width: Some("128".to_owned()),
                            value: new_fname.read().clone(),
                            placeholder: Some("field_name".to_owned()),
                            onchange: move |txt| new_fname.set(txt),
                        }
                        StyledInput {
                            width: Some("112".to_owned()),
                            value: new_fty.read().clone(),
                            placeholder: Some("u32 / vec3".to_owned()),
                            onchange: move |txt: String| {
                                let norm = normalize_wire_format_for_type(&txt, *new_fwf.read());
                                new_fwf.set(norm);
                                new_fty.set(txt);
                            },
                        }
                        StyledInput {
                            width: Some("42".to_owned()),
                            value: new_fpad.read().clone(),
                            placeholder: Some("pad".to_owned()),
                            onchange: move |txt| new_fpad.set(txt),
                        }
                        ActionButton {
                            label: "+ Add".to_owned(),
                            style: ButtonStyle::Primary,
                            compact: Some(true),
                            onpress: move |_| {
                                let fname = new_fname.read().trim().to_owned();
                                if fname.is_empty() {
                                    state.set_status("Enter a ByteBlock field name.".to_owned());
                                    return;
                                }
                                let ty_str = new_fty.read().trim().to_owned();
                                let Ok(dt) = DataType::parse_str(&ty_str) else {
                                    state.set_status(format!("Invalid DataType `{ty_str}`"));
                                    return;
                                };
                                let wf = normalize_wire_format_for_type(&ty_str, *new_fwf.read());
                                let pad = new_fpad.read().trim().parse::<usize>().ok().filter(|&p| p > 0);
                                let bb_t = append_bb_name.clone();
                                let fname_msg = fname.clone();
                                let ok = state.mutate_schema(
                                    format!("Appended `{fname_msg}` to `{bb_t}`"),
                                    move |s, _| {
                                        let Some(TypeDef::ByteBlock(bb)) = s.types.get_mut(&bb_t) else {
                                            return Err(format!("ByteBlock `{bb_t}` not found"));
                                        };
                                        bb.fields.push(ByteFieldDef {
                                            name: fname,
                                            description: None,
                                            ty: dt,
                                            wire_format: wf,
                                            pad_after: pad,
                                        });
                                        Ok(())
                                    },
                                );
                                if ok {
                                    new_fname.set(String::new());
                                    new_fpad.set(String::new());
                                }
                            },
                        }
                    }

                    rect {
                        width: "fill",
                        direction: "horizontal",
                        cross_align: "center",
                        spacing: "4",
                        for preset in ["u32", "i32", "f32", "bool", "usize", "vec3", "c_string"] {
                            SegmentedChip {
                                key: "bb-{preset}",
                                label: preset.to_owned(),
                                selected: cur_new_ty.trim() == preset,
                                compact: Some(true),
                                onpress: move |_| {
                                    let norm = normalize_wire_format_for_type(preset, *new_fwf.read());
                                    new_fwf.set(norm);
                                    new_fty.set(preset.to_owned());
                                },
                            }
                        }
                    }

                    if !new_compat_wfs.is_empty() {
                        rect {
                            width: "fill",
                            direction: "horizontal",
                            cross_align: "center",
                            spacing: "5",
                            label { font_size: "11", color: theme::TEXT_MUTED, "Wire:" }
                            for &(wf, wf_label) in new_compat_wfs {
                                {
                                    let effective = normalize_wire_format_for_type(&cur_new_ty, *new_fwf.read());
                                    rsx! {
                                        SegmentedChip {
                                            key: "bbwf-{wf_label}",
                                            label: wf_label.to_owned(),
                                            selected: effective == Some(wf),
                                            compact: Some(true),
                                            onpress: move |_| new_fwf.set(Some(wf)),
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compatible_wire_formats_filters_by_data_type() {
        assert_eq!(compatible_wire_formats("bool").len(), 2);
        assert_eq!(compatible_wire_formats("usize").len(), 1);
        assert_eq!(compatible_wire_formats("Vec<usize>").len(), 1);
        assert!(compatible_wire_formats("u32").is_empty());
        assert!(compatible_wire_formats("f32").is_empty());
        assert!(compatible_wire_formats("c_string").is_empty());
        assert!(compatible_wire_formats("Vec<VertexV2>").is_empty());
    }

    #[test]
    fn test_normalize_wire_format_preserves_valid_and_clears_invalid() {
        assert_eq!(
            normalize_wire_format_for_type("bool", Some(WireFormat::U8Bool)),
            Some(WireFormat::U8Bool)
        );
        assert_eq!(
            normalize_wire_format_for_type("bool", Some(WireFormat::U32Usize)),
            Some(WireFormat::U32Bool)
        );
        assert_eq!(
            normalize_wire_format_for_type("u32", Some(WireFormat::U32Bool)),
            None
        );
        assert_eq!(
            normalize_wire_format_for_type("usize", None),
            Some(WireFormat::U32Usize)
        );
    }
}
