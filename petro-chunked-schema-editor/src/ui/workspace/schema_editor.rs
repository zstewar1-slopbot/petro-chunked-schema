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
    count_file_matches, list_level_children, matches_hover_target, step_hover_target,
    type_kind_badge,
};
use crate::ui::context::EditorState;
use crate::ui::theme;
use crate::ui::widgets::{
    ButtonStyle, action_button, group_caption, segmented_chip, styled_input, styled_input_live,
};

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
#[derive(PartialEq)]
pub struct SchemaEditorPane;

impl Component for SchemaEditorPane {
    fn render(&self) -> impl IntoElement {
        let mut state = EditorState::current();
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

        let cards: Vec<Element> = stack
            .iter()
            .enumerate()
            .map(|(idx, step)| {
                let is_active = idx + 1 == stack_len;
                let is_expanded = is_active || expanded_levels.contains(&idx);
                let next_edge = stack.get(idx + 1).and_then(|s| s.incoming_edge.clone());
                SchemaStackCard {
                    level_idx: idx,
                    step: step.clone(),
                    is_active,
                    is_expanded,
                    next_edge,
                    all_file_rows: all_file_rows.clone(),
                    resolved: resolved.clone(),
                }
                .into()
            })
            .collect();

        rect()
            .width(Size::px(415.0))
            .height(Size::fill())
            .content(Content::Flex)
            .padding((8.0, 10.0))
            .background(theme::BG_PANE_RIGHT)
            .border(theme::border_left(theme::BORDER_SUBTLE))
            .vertical()
            .spacing(8.0)
            .child(
                rect()
                    .width(Size::fill())
                    .horizontal()
                    .main_align(Alignment::SpaceBetween)
                    .cross_align(Alignment::Center)
                    .spacing(6.0)
                    .child(
                        rect()
                            .horizontal()
                            .cross_align(Alignment::Center)
                            .spacing(6.0)
                            .child(
                                label()
                                    .font_size(13.0)
                                    .font_weight(FontWeight::BOLD)
                                    .color(theme::ACCENT_BLUE)
                                    .max_lines(1)
                                    .text("Schema Stack"),
                            )
                            .child(
                                label()
                                    .font_size(11.0)
                                    .font_weight(if is_dirty {
                                        FontWeight::BOLD
                                    } else {
                                        FontWeight::NORMAL
                                    })
                                    .color(if is_dirty {
                                        theme::ACCENT_AMBER
                                    } else {
                                        theme::TEXT_MUTED
                                    })
                                    .max_lines(1)
                                    .text(format!("{schema_name}{dirty_badge}")),
                            ),
                    )
                    .maybe_child((stack_len > 1).then(|| {
                        action_button("Root", ButtonStyle::Secondary, true, move || {
                            state.focus_schema_level(0)
                        })
                    })),
            )
            .child(
                label()
                    .font_size(10.0)
                    .color(theme::TEXT_DIM)
                    .max_lines(1)
                    .text("Hover a level or child to highlight in file · Edits apply live (Ctrl+S saves)"),
            )
            .child(
                ScrollView::new()
                    .width(Size::fill())
                    .height(Size::flex(1.0))
                    .child(rect().width(Size::fill()).vertical().spacing(6.0).children(cards)),
            )
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

#[derive(PartialEq)]
struct SchemaStackCard {
    level_idx: usize,
    step: SchemaStackStep,
    is_active: bool,
    is_expanded: bool,
    next_edge: Option<SchemaParentEdge>,
    all_file_rows: Vec<FlatTreeRow>,
    resolved: ResolvedSchema,
}

impl Component for SchemaStackCard {
    fn render_key(&self) -> DiffKey {
        DiffKey::from(&format!("{}-{}", self.level_idx, step_key(&self.step)))
    }

    fn render(&self) -> impl IntoElement {
        let mut state = EditorState::current();
        let card_hover = step_hover_target(&self.step);
        let match_count = card_hover
            .as_ref()
            .map(|t| count_file_matches(&self.resolved, &self.all_file_rows, t));
        let first_match_path = if self.step.file_tree_path.is_none() {
            card_hover.as_ref().and_then(|t| {
                self.all_file_rows
                    .iter()
                    .find(|r| matches_hover_target(&self.resolved, r, t))
                    .map(|r| r.tree_path.clone())
            })
        } else {
            None
        };

        let (kind_badge, title, badge_color) = match &self.step.target {
            SchemaLevelTarget::Type {
                module_id,
                type_name,
            } => {
                let kind = type_kind_badge(&self.resolved, *module_id, type_name);
                let disp = self.resolved.format_type_display(*module_id, type_name);
                let color = match kind {
                    "Enum" | "MiniChunks" => theme::ACCENT_AMBER,
                    "ByteBlock" => theme::ACCENT_GREEN,
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

        let card_bg = if self.is_active {
            theme::BG_CARD
        } else {
            theme::BG_STACK_PARENT
        };
        let card_border = if self.is_active {
            theme::border_all(theme::BORDER_STACK_ACTIVE)
        } else {
            theme::border_all(theme::BORDER_CARD)
        };
        let toggle_glyph = if self.is_active {
            "●"
        } else if self.is_expanded {
            "[-]"
        } else {
            "[+]"
        };
        let next_hint = self
            .next_edge
            .as_ref()
            .map(|edge| format!("→ {}", edge.short_name()));
        let match_txt = match_count.map(|c| format!("· {c} in file"));
        let match_color = match match_count {
            Some(0) | None => theme::TEXT_DIM,
            Some(_) => theme::ACCENT_GREEN,
        };

        let enter_hover = card_hover.clone();
        let leave_has_hover = card_hover.is_some();
        let mut st_enter = state;
        let mut st_leave = state;
        let level_idx = self.level_idx;
        let is_active = self.is_active;

        let mut card = rect()
            .width(Size::fill())
            .padding(if self.is_expanded {
                (8.0, 10.0)
            } else {
                (6.0, 10.0)
            })
            .corner_radius(6.0)
            .background(card_bg)
            .border(card_border)
            .vertical()
            .spacing(7.0)
            .on_pointer_enter(move |_| {
                if enter_hover.is_some() {
                    st_enter.set_hovered_schema_target(enter_hover.clone());
                }
            })
            .on_pointer_leave(move |_| {
                if leave_has_hover {
                    st_leave.set_hovered_schema_target(None);
                }
            })
            .child(
                rect()
                    .width(Size::fill())
                    .height(Size::px(24.0))
                    .horizontal()
                    .cross_align(Alignment::Center)
                    .spacing(6.0)
                    .overflow(Overflow::Clip)
                    .on_pointer_down(move |_| {
                        if !is_active {
                            state.toggle_stack_level_expanded(level_idx);
                        }
                    })
                    .child(
                        label()
                            .width(Size::px(22.0))
                            .font_size(10.0)
                            .font_family("monospace")
                            .font_weight(FontWeight::BOLD)
                            .color(if self.is_active {
                                theme::ACCENT_BLUE
                            } else {
                                theme::TEXT_MUTED
                            })
                            .max_lines(1)
                            .text(toggle_glyph),
                    )
                    .child(
                        rect()
                            .padding((1.0, 5.0))
                            .corner_radius(3.0)
                            .background(theme::BG_INSET)
                            .border(theme::border_all(theme::BORDER_SUBTLE))
                            .child(
                                label()
                                    .font_size(10.0)
                                    .font_weight(FontWeight::BOLD)
                                    .color(badge_color)
                                    .max_lines(1)
                                    .text(kind_badge),
                            ),
                    )
                    .child(
                        label()
                            .font_size(12.0)
                            .font_weight(FontWeight::BOLD)
                            .color(theme::TEXT_PRIMARY)
                            .max_lines(1)
                            .text(title),
                    )
                    .maybe_child((!self.is_expanded).then_some(()).and(next_hint).map(|hint| {
                        label()
                            .font_size(11.0)
                            .color(theme::ACCENT_BLUE)
                            .max_lines(1)
                            .text(hint)
                    }))
                    .maybe_child(match_txt.map(|mtxt| {
                        label()
                            .font_size(10.0)
                            .font_family("monospace")
                            .color(match_color)
                            .max_lines(1)
                            .text(mtxt)
                    })),
            );

        if self.is_expanded {
            if !self.is_active || self.step.incoming_edge.is_some() || first_match_path.is_some() {
                let mut st_focus = state;
                let mut st_jump = state;
                card = card.child(
                    rect()
                        .width(Size::fill())
                        .horizontal()
                        .cross_align(Alignment::Center)
                        .spacing(6.0)
                        .maybe_child(self.step.incoming_edge.as_ref().map(|edge| {
                            label()
                                .font_size(11.0)
                                .color(theme::TEXT_MUTED)
                                .max_lines(1)
                                .text(format!("via {}", edge.display_label()))
                        }))
                        .maybe_child((!self.is_active).then(|| {
                            action_button("Focus Here", ButtonStyle::Secondary, true, move || {
                                st_focus.focus_schema_level(level_idx)
                            })
                        }))
                        .maybe_child(first_match_path.map(|jump_path| {
                            action_button("Select in File", ButtonStyle::Secondary, true, move || {
                                st_jump.select_node(jump_path.clone())
                            })
                        })),
                );
            }

            if self.is_active
                && let Some(edge) = &self.step.incoming_edge
            {
                card = card.child(LiveIncomingFieldSection {
                    edge: edge.clone(),
                    resolved: self.resolved.clone(),
                });
            }

            match &self.step.target {
                SchemaLevelTarget::Type {
                    module_id,
                    type_name,
                } => {
                    card = card.child(TypeLevelBody {
                        level_idx,
                        module_id: *module_id,
                        type_name: type_name.clone(),
                        is_active: self.is_active,
                        next_edge: self.next_edge.clone(),
                        parent_hover: card_hover,
                        all_file_rows: self.all_file_rows.clone(),
                        resolved: self.resolved.clone(),
                    });
                }
                SchemaLevelTarget::LeafField { .. } => {}
                SchemaLevelTarget::UnassignedFileNode {
                    id,
                    is_mini_chunk,
                    is_container,
                    looks_like_mini_chunks,
                    parent_module_id,
                    parent_type_name,
                    ..
                } => {
                    card = card.child(render_unassigned_node_body(
                        *id,
                        *is_mini_chunk,
                        *is_container,
                        *looks_like_mini_chunks,
                        *parent_module_id,
                        parent_type_name.as_deref(),
                        &self.resolved,
                        state,
                    ));
                }
            }
        }

        card
    }
}

#[derive(PartialEq)]
struct TypeLevelBody {
    level_idx: usize,
    module_id: SchemaId,
    type_name: String,
    is_active: bool,
    next_edge: Option<SchemaParentEdge>,
    parent_hover: Option<HoveredSchemaTarget>,
    all_file_rows: Vec<FlatTreeRow>,
    resolved: ResolvedSchema,
}

impl Component for TypeLevelBody {
    fn render(&self) -> impl IntoElement {
        let mut state = EditorState::current();
        let is_local = self.module_id == SchemaId::ROOT;
        let module = self.resolved.module(self.module_id);
        let typedef = module.schema.types.get(&self.type_name).cloned();

        let children = list_level_children(
            &self.resolved,
            self.module_id,
            &self.type_name,
            &self.all_file_rows,
        );
        let imported_source = if !is_local {
            module.source.clone()
        } else {
            None
        };
        let imported_alias = module
            .module_name
            .clone()
            .unwrap_or_else(|| module.schema.name.clone());

        let mut body = rect().width(Size::fill()).vertical().spacing(6.0);

        if !is_local {
            body = body.child(
                rect()
                    .width(Size::fill())
                    .padding((6.0, 8.0))
                    .corner_radius(4.0)
                    .background(theme::BG_SUB_CARD)
                    .border(theme::border_all(theme::BORDER_AMBER))
                    .vertical()
                    .spacing(5.0)
                    .child(
                        label()
                            .font_size(11.0)
                            .color(theme::ACCENT_AMBER)
                            .text(format!("Read-only imported type from `{imported_alias}`.")),
                    )
                    .maybe_child(imported_source.map(|src_ref| {
                        rect().horizontal().child(action_button(
                            format!("Open `{imported_alias}` Schema"),
                            ButtonStyle::Secondary,
                            true,
                            move || state.request_load_schema_ref(src_ref.clone()),
                        ))
                    })),
            );
        }

        match typedef {
            Some(TypeDef::Chunks(cc)) => {
                if is_local {
                    body = body.child(render_live_container_toggles(
                        &self.type_name,
                        false,
                        cc.mode,
                        cc.ignore_unknown,
                        state,
                    ));
                }
                body = body.child(render_defined_children_list(
                    self.level_idx,
                    format!("Defined Child Chunks ({})", children.len()),
                    &children,
                    self.next_edge.as_ref(),
                    self.parent_hover.as_ref(),
                    state,
                ));
                if self.is_active && is_local {
                    body = body.child(AddChildFieldForm {
                        parent_type_name: self.type_name.clone(),
                        is_mini_chunks: false,
                    });
                }
            }
            Some(TypeDef::MiniChunks(mc)) => {
                if is_local {
                    body = body.child(render_live_container_toggles(
                        &self.type_name,
                        true,
                        mc.mode,
                        mc.ignore_unknown,
                        state,
                    ));
                }
                body = body.child(render_defined_children_list(
                    self.level_idx,
                    format!("Defined Mini-Chunks ({})", children.len()),
                    &children,
                    self.next_edge.as_ref(),
                    self.parent_hover.as_ref(),
                    state,
                ));
                if self.is_active && is_local {
                    body = body.child(AddChildFieldForm {
                        parent_type_name: self.type_name.clone(),
                        is_mini_chunks: true,
                    });
                }
            }
            Some(TypeDef::ChunkEnum(_)) => {
                body = body.child(render_defined_children_list(
                    self.level_idx,
                    format!("Enum Variants ({}) — click to inspect", children.len()),
                    &children,
                    self.next_edge.as_ref(),
                    self.parent_hover.as_ref(),
                    state,
                ));
            }
            Some(TypeDef::ChunkStreamSeq(_)) => {
                body = body.child(render_defined_children_list(
                    self.level_idx,
                    format!("Stream Sequence Fields ({})", children.len()),
                    &children,
                    self.next_edge.as_ref(),
                    self.parent_hover.as_ref(),
                    state,
                ));
            }
            Some(TypeDef::ByteBlock(bb)) => {
                body = body.child(LiveByteBlockSection {
                    type_name: self.type_name.clone(),
                    is_local,
                    byte_block: bb,
                });
            }
            None => {
                body = body.child(
                    label()
                        .font_size(11.0)
                        .color(theme::ACCENT_RED)
                        .text(format!("Type `{}` not found in schema.", self.type_name)),
                );
            }
        }

        body
    }
}

fn render_live_container_toggles(
    type_name: &str,
    is_mini_chunks: bool,
    mode: OrderMode,
    ignore_unknown: bool,
    state: EditorState,
) -> Element {
    let t_map = type_name.to_owned();
    let t_seq = type_name.to_owned();
    let t_strict = type_name.to_owned();
    let t_ignore = type_name.to_owned();
    let mut s1 = state;
    let mut s2 = state;
    let mut s3 = state;
    let mut s4 = state;

    rect()
        .width(Size::fill())
        .padding((6.0, 8.0))
        .corner_radius(4.0)
        .background(theme::BG_SUB_CARD)
        .vertical()
        .spacing(5.0)
        .child(
            rect()
                .width(Size::fill())
                .horizontal()
                .cross_align(Alignment::Center)
                .spacing(6.0)
                .child(
                    label()
                        .width(Size::px(74.0))
                        .font_size(11.0)
                        .color(theme::TEXT_MUTED)
                        .text("Order"),
                )
                .child(segmented_chip("Map", mode == OrderMode::Map, true, move || {
                    let t = t_map.clone();
                    s1.mutate_schema(format!("Set `{t}` order = map"), move |s, _| {
                        set_type_order_mode(s, &t, is_mini_chunks, OrderMode::Map);
                        Ok(())
                    });
                }))
                .child(segmented_chip(
                    "Sequence",
                    mode == OrderMode::Sequence,
                    true,
                    move || {
                        let t = t_seq.clone();
                        s2.mutate_schema(format!("Set `{t}` order = sequence"), move |s, _| {
                            set_type_order_mode(s, &t, is_mini_chunks, OrderMode::Sequence);
                            Ok(())
                        });
                    },
                )),
        )
        .child(
            rect()
                .width(Size::fill())
                .horizontal()
                .cross_align(Alignment::Center)
                .spacing(6.0)
                .child(
                    label()
                        .width(Size::px(74.0))
                        .font_size(11.0)
                        .color(theme::TEXT_MUTED)
                        .text("Unknown"),
                )
                .child(segmented_chip("Strict", !ignore_unknown, true, move || {
                    let t = t_strict.clone();
                    s3.mutate_schema(format!("Set `{t}` ignore_unknown = false"), move |s, _| {
                        set_type_ignore_unknown(s, &t, is_mini_chunks, false);
                        Ok(())
                    });
                }))
                .child(segmented_chip("Ignore", ignore_unknown, true, move || {
                    let t = t_ignore.clone();
                    s4.mutate_schema(format!("Set `{t}` ignore_unknown = true"), move |s, _| {
                        set_type_ignore_unknown(s, &t, is_mini_chunks, true);
                        Ok(())
                    });
                })),
        )
        .into()
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

fn render_defined_children_list(
    level_idx: usize,
    heading: String,
    items: &[SchemaChildItem],
    active_edge: Option<&SchemaParentEdge>,
    parent_hover: Option<&HoveredSchemaTarget>,
    state: EditorState,
) -> Element {
    let rows: Vec<Element> = items
        .iter()
        .map(|item| {
            let is_current = active_edge == Some(&item.edge);
            let bg = if is_current {
                theme::BG_ROW_SELECTED
            } else {
                theme::BG_ROW
            };
            let border = if is_current {
                theme::border_all(theme::BORDER_CHIP_ACTIVE)
            } else {
                theme::border_all(Color::TRANSPARENT)
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
            let restore_hover = parent_hover.cloned();
            let click_item = item.clone();
            let count_label = format!("{}x", item.file_match_count);
            let mut st_enter = state;
            let mut st_leave = state;
            let mut st_click = state;

            rect()
                .width(Size::fill())
                .height(Size::px(26.0))
                .padding((0.0, 6.0))
                .corner_radius(3.0)
                .background(bg)
                .border(border)
                .horizontal()
                .cross_align(Alignment::Center)
                .spacing(5.0)
                .overflow(Overflow::Clip)
                .on_pointer_enter(move |e: Event<PointerEventData>| {
                    e.stop_propagation();
                    st_enter.set_hovered_schema_target(Some(child_hover.clone()));
                })
                .on_pointer_leave(move |_| {
                    st_leave.set_hovered_schema_target(restore_hover.clone());
                })
                .on_pointer_down(move |e: Event<PointerEventData>| {
                    e.stop_propagation();
                    st_click.select_schema_child(level_idx, click_item.clone());
                })
                .child(
                    label()
                        .width(Size::px(52.0))
                        .font_size(10.0)
                        .font_family("monospace")
                        .color(theme::ACCENT_BLUE)
                        .max_lines(1)
                        .text(item.id_label.clone()),
                )
                .child(
                    label()
                        .width(Size::px(28.0))
                        .font_size(10.0)
                        .font_family("monospace")
                        .color(theme::ACCENT_AMBER)
                        .max_lines(1)
                        .text(item.cardinality_badge.clone()),
                )
                .child(
                    label()
                        .width(Size::px(215.0))
                        .font_size(11.0)
                        .font_weight(if is_current {
                            FontWeight::BOLD
                        } else {
                            FontWeight::NORMAL
                        })
                        .color(name_color)
                        .max_lines(1)
                        .text(format!("{}: {}", item.field_name, item.type_summary)),
                )
                .child(
                    label()
                        .width(Size::px(34.0))
                        .font_size(10.0)
                        .font_family("monospace")
                        .color(count_color)
                        .max_lines(1)
                        .text(count_label),
                )
                .into()
        })
        .collect();

    rect()
        .width(Size::fill())
        .vertical()
        .spacing(4.0)
        .child(group_caption(heading))
        .child(if items.is_empty() {
            label()
                .font_size(11.0)
                .color(theme::TEXT_DIM)
                .text("No child fields defined yet.")
                .into_element()
        } else {
            rect()
                .width(Size::fill())
                .padding(4.0)
                .corner_radius(4.0)
                .background(theme::BG_INSET)
                .border(theme::border_all(theme::BORDER_CARD))
                .vertical()
                .spacing(2.0)
                .children(rows)
                .into_element()
        })
        .into()
}

#[derive(PartialEq)]
struct AddChildFieldForm {
    parent_type_name: String,
    is_mini_chunks: bool,
}

impl Component for AddChildFieldForm {
    fn render(&self) -> impl IntoElement {
        let mut state = EditorState::current();
        let mut id_input = use_state(String::new);
        let mut name_input = use_state(String::new);

        let id_placeholder = if self.is_mini_chunks { "0x01" } else { "0x0400" };
        let parent_clone = self.parent_type_name.clone();
        let is_mini = self.is_mini_chunks;

        rect()
            .width(Size::fill())
            .padding((6.0, 8.0))
            .corner_radius(4.0)
            .background(theme::BG_SUB_CARD)
            .vertical()
            .spacing(5.0)
            .child(group_caption(if is_mini {
                "DEFINE NEW MINI-CHUNK IN SCHEMA"
            } else {
                "DEFINE NEW CHUNK IN SCHEMA"
            }))
            .child(
                rect()
                    .width(Size::fill())
                    .horizontal()
                    .cross_align(Alignment::Center)
                    .spacing(6.0)
                    .child(styled_input(Size::px(76.0), id_input, id_placeholder))
                    .child(styled_input(Size::px(165.0), name_input, "field_name"))
                    .child(action_button("+ Add", ButtonStyle::Secondary, true, move || {
                        let raw_id = id_input.read().trim().to_owned();
                        let Some(id_val) = parse_hex_or_dec_u32(&raw_id) else {
                            state.set_status(format!(
                                "Invalid ID `{raw_id}` (use hex like 0x0400 or decimal)"
                            ));
                            return;
                        };
                        let raw_name = name_input.read().trim().to_owned();
                        let fname = if raw_name.is_empty() {
                            if is_mini {
                                format!("mc_{:02x}", id_val as u8)
                            } else {
                                format!("chunk_{id_val:04x}")
                            }
                        } else {
                            raw_name
                        };
                        let parent_t = parent_clone.clone();
                        let ok = if is_mini {
                            let Ok(mc_id) = u8::try_from(id_val) else {
                                state.set_status(
                                    "Mini-chunk ID must be 0..=255 (0x00..=0xFF)".to_owned(),
                                );
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
                    })),
            )
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

#[allow(clippy::too_many_arguments)]
fn render_unassigned_node_body(
    id: u32,
    is_mini_chunk: bool,
    is_container: bool,
    looks_like_mini_chunks: bool,
    parent_module_id: Option<SchemaId>,
    parent_type_name: Option<&str>,
    resolved: &ResolvedSchema,
    mut state: EditorState,
) -> Element {
    let Some((pmod, ptype)) = parent_module_id.zip(parent_type_name) else {
        return label()
            .font_size(11.0)
            .color(theme::ACCENT_AMBER)
            .text("Parent chunk is not mapped to a schema container yet. Define the parent level above first.")
            .into();
    };

    if pmod != SchemaId::ROOT {
        let module = resolved.module(pmod);
        let alias = module
            .module_name
            .clone()
            .unwrap_or_else(|| module.schema.name.clone());
        let src_opt = module.source.clone();
        return rect()
            .vertical()
            .spacing(5.0)
            .child(
                label()
                    .font_size(11.0)
                    .color(theme::ACCENT_AMBER)
                    .text(format!(
                        "Parent `{ptype}` belongs to imported schema `{alias}` (read-only)."
                    )),
            )
            .maybe_child(src_opt.map(|src_ref| {
                action_button(
                    format!("Open `{alias}` Schema"),
                    ButtonStyle::Secondary,
                    true,
                    move || state.request_load_schema_ref(src_ref.clone()),
                )
            }))
            .into();
    }

    let parent_def = resolved.module(pmod).schema.types.get(ptype);
    let can_define = if is_mini_chunk {
        matches!(parent_def, Some(TypeDef::MiniChunks(_)))
    } else {
        matches!(parent_def, Some(TypeDef::Chunks(_) | TypeDef::ChunkStreamSeq(_)))
    };

    if !can_define {
        return label()
            .font_size(11.0)
            .color(theme::ACCENT_AMBER)
            .text(format!(
                "Parent `{ptype}` is not an editable chunk/mini-chunk container."
            ))
            .into();
    }

    let ptype_btn = ptype.to_owned();
    let default_name = if is_mini_chunk {
        format!("mc_{:02x}", id as u8)
    } else {
        format!("chunk_{id:04x}")
    };

    rect()
        .width(Size::fill())
        .padding(8.0)
        .corner_radius(4.0)
        .background(theme::BG_SUB_CARD)
        .vertical()
        .spacing(6.0)
        .child(
            label()
                .font_size(11.0)
                .color(theme::TEXT_MUTED)
                .text(if is_mini_chunk {
                    format!("Mini-chunk 0x{id:02X} is not yet defined in `{ptype}`.")
                } else {
                    format!("Chunk 0x{id:04X} is not yet defined in `{ptype}`.")
                }),
        )
        .child(rect().horizontal().child(action_button(
            format!("+ Define in `{ptype_btn}`"),
            ButtonStyle::Primary,
            false,
            move || {
                let parent_name = ptype_btn.clone();
                let fname = default_name.clone();
                if is_mini_chunk {
                    let pref = ParentContainerRef::MiniChunksType {
                        type_name: parent_name,
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
                        type_name: parent_name,
                    };
                    let payload = if is_container {
                        ChunkPayloadSpec::Children(format!("{}Container", to_pascal_case(&fname)))
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
        )))
        .into()
}

/// Live auto-applying editor for the incoming field definition (`ChunkFieldDef` or
/// `MiniChunkFieldDef`) on the parent container.
#[derive(PartialEq)]
struct LiveIncomingFieldSection {
    edge: SchemaParentEdge,
    resolved: ResolvedSchema,
}

impl Component for LiveIncomingFieldSection {
    fn render(&self) -> impl IntoElement {
        let mut state = EditorState::current();

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

        let editable = match &self.edge {
            SchemaParentEdge::ChunkField {
                parent_module_id,
                parent_type_name,
                chunk_id: Some(cid),
                ..
            } if *parent_module_id == SchemaId::ROOT => {
                let fields = match self.resolved.root_schema().types.get(parent_type_name) {
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
                    self.resolved.root_schema().types.get(parent_type_name)
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
            return rect();
        };

        let (
            identity_key,
            is_mini,
            parent_type,
            field_id,
            schema_name_val,
            schema_card,
            schema_pkind,
            schema_ty_str,
            schema_wf,
        ) = match &editable {
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

        let mut synced_key = use_state(String::new);
        let mut name_draft = use_state(String::new);
        let mut type_draft = use_state(String::new);
        let mut card_draft = use_state(|| "required".to_owned());
        let mut pkind_draft = use_state(|| "data".to_owned());
        let mut wf_draft = use_state(|| None::<WireFormat>);
        let mut draft_hint = use_state(|| None::<String>);

        if *synced_key.peek() != identity_key {
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
                            upsert_chunk_field(
                                s,
                                &pref,
                                field_id,
                                &clean_name,
                                card,
                                payload,
                                norm_wf,
                            )
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
        let mut st_del = state;

        let card_chips: Vec<Element> = [
            ("required", "Required"),
            ("optional", "Optional"),
            ("repeated", "Repeated"),
            ("default", "Default"),
        ]
        .into_iter()
        .map(|(ckey, clabel)| {
            let mut apply = apply_live.clone();
            let mut cd = card_draft;
            segmented_chip(clabel, cur_card == ckey, true, move || {
                cd.set(ckey.to_owned());
                apply(
                    name_draft.read().clone(),
                    ckey.to_owned(),
                    pkind_draft.read().clone(),
                    type_draft.read().clone(),
                    *wf_draft.read(),
                );
            })
        })
        .collect();

        let payload_chips: Vec<Element> = [
            ("data", "Data"),
            ("children", "Children"),
            ("mini_chunks", "MiniChunks"),
            ("ignore", "Ignore"),
        ]
        .into_iter()
        .map(|(pkey, plabel)| {
            let mut apply = apply_live.clone();
            let mut pk = pkind_draft;
            let mut td = type_draft;
            segmented_chip(plabel, cur_pkind == pkey, true, move || {
                pk.set(pkey.to_owned());
                let next_ty = match pkey {
                    "children" => {
                        let cur = td.read().clone();
                        if cur.is_empty()
                            || DataType::parse_str(&cur)
                                .is_ok_and(|d| !matches!(d, DataType::Named(_)))
                        {
                            format!("{}Container", to_pascal_case(&name_draft.read()))
                        } else {
                            cur
                        }
                    }
                    "mini_chunks" => {
                        let cur = td.read().clone();
                        if cur.is_empty()
                            || DataType::parse_str(&cur)
                                .is_ok_and(|d| !matches!(d, DataType::Named(_)))
                        {
                            format!("{}MiniChunks", to_pascal_case(&name_draft.read()))
                        } else {
                            cur
                        }
                    }
                    "ignore" => String::new(),
                    _ => {
                        let cur = td.read().clone();
                        if cur.is_empty() {
                            "raw_bytes".to_owned()
                        } else {
                            cur
                        }
                    }
                };
                td.set(next_ty.clone());
                apply(
                    name_draft.read().clone(),
                    card_draft.read().clone(),
                    pkey.to_owned(),
                    next_ty,
                    *wf_draft.read(),
                );
            })
        })
        .collect();

        let preset_chips: Vec<Element> = ["u32", "f32", "bool", "usize", "c_string", "vec3", "raw_bytes"]
            .into_iter()
            .map(|preset| {
                let mut apply = apply_live.clone();
                let mut td = type_draft;
                segmented_chip(preset, cur_ty.trim() == preset, true, move || {
                    td.set(preset.to_owned());
                    apply(
                        name_draft.read().clone(),
                        card_draft.read().clone(),
                        pkind_draft.read().clone(),
                        preset.to_owned(),
                        *wf_draft.read(),
                    );
                })
            })
            .collect();

        let wf_chips: Vec<Element> = compat_wfs
            .iter()
            .map(|&(wf, wf_label)| {
                let mut apply = apply_live.clone();
                let mut wfd = wf_draft;
                let effective = normalize_wire_format_for_type(&cur_ty, cur_wf);
                segmented_chip(wf_label, effective == Some(wf), true, move || {
                    wfd.set(Some(wf));
                    apply(
                        name_draft.read().clone(),
                        card_draft.read().clone(),
                        pkind_draft.read().clone(),
                        type_draft.read().clone(),
                        Some(wf),
                    );
                })
            })
            .collect();

        let mut apply_name = apply_live.clone();
        let mut apply_type = apply_live;

        let mut box_el = rect()
            .width(Size::fill())
            .padding(8.0)
            .corner_radius(5.0)
            .background(theme::BG_SUB_CARD)
            .border(theme::border_all(theme::BORDER_SUBTLE))
            .vertical()
            .spacing(7.0)
            .child(
                rect()
                    .width(Size::fill())
                    .horizontal()
                    .main_align(Alignment::SpaceBetween)
                    .cross_align(Alignment::Center)
                    .child(group_caption(if is_mini {
                        format!("FIELD IN `{parent_type}` (MC 0x{:02X})", field_id as u8)
                    } else {
                        format!("FIELD IN `{parent_type}` (0x{field_id:04X})")
                    }))
                    .child(action_button("Delete Field", ButtonStyle::Danger, true, move || {
                        let pt = parent_del.clone();
                        if is_mini {
                            let pref = ParentContainerRef::MiniChunksType { type_name: pt };
                            st_del.mutate_schema(
                                format!("Deleted mini-chunk field 0x{:02X}", field_id as u8),
                                move |s, _| {
                                    delete_mini_chunk_field(s, &pref, field_id as u8).map(|_| ())
                                },
                            );
                        } else {
                            let pref = ParentContainerRef::ChunksType { type_name: pt };
                            st_del.mutate_schema(
                                format!("Deleted chunk field 0x{field_id:04X}"),
                                move |s, _| delete_chunk_field(s, &pref, field_id).map(|_| ()),
                            );
                        }
                    })),
            )
            .child(
                rect()
                    .width(Size::fill())
                    .horizontal()
                    .cross_align(Alignment::Center)
                    .spacing(8.0)
                    .child(
                        label()
                            .width(Size::px(74.0))
                            .font_size(11.0)
                            .color(theme::TEXT_MUTED)
                            .text("Field Name"),
                    )
                    .child(styled_input_live(
                        Size::px(270.0),
                        name_draft,
                        "snake_case_name",
                        move |txt| {
                            apply_name(
                                txt,
                                card_draft.read().clone(),
                                pkind_draft.read().clone(),
                                type_draft.read().clone(),
                                *wf_draft.read(),
                            );
                        },
                    )),
            )
            .child(
                rect()
                    .width(Size::fill())
                    .horizontal()
                    .cross_align(Alignment::Center)
                    .spacing(5.0)
                    .child(
                        label()
                            .width(Size::px(74.0))
                            .font_size(11.0)
                            .color(theme::TEXT_MUTED)
                            .text("Cardinality"),
                    )
                    .children(card_chips),
            );

        if !is_mini {
            box_el = box_el.child(
                rect()
                    .width(Size::fill())
                    .horizontal()
                    .cross_align(Alignment::Center)
                    .spacing(5.0)
                    .child(
                        label()
                            .width(Size::px(74.0))
                            .font_size(11.0)
                            .color(theme::TEXT_MUTED)
                            .text("Payload"),
                    )
                    .children(payload_chips),
            );
        }

        if is_mini || cur_pkind != "ignore" {
            box_el = box_el.child(
                rect()
                    .width(Size::fill())
                    .horizontal()
                    .cross_align(Alignment::Center)
                    .spacing(8.0)
                    .child(
                        label()
                            .width(Size::px(74.0))
                            .font_size(11.0)
                            .color(theme::TEXT_MUTED)
                            .text("Type"),
                    )
                    .child(styled_input_live(
                        Size::px(270.0),
                        type_draft,
                        "u32, c_string, Vec<f32>, MyStruct...",
                        move |txt| {
                            apply_type(
                                name_draft.read().clone(),
                                card_draft.read().clone(),
                                pkind_draft.read().clone(),
                                txt,
                                *wf_draft.read(),
                            );
                        },
                    )),
            );

            if is_mini || cur_pkind == "data" {
                box_el = box_el.child(
                    rect()
                        .width(Size::fill())
                        .horizontal()
                        .cross_align(Alignment::Center)
                        .spacing(4.0)
                        .children(preset_chips),
                );
            }
        }

        if !compat_wfs.is_empty() {
            box_el = box_el.child(
                rect()
                    .width(Size::fill())
                    .horizontal()
                    .cross_align(Alignment::Center)
                    .spacing(5.0)
                    .child(
                        label()
                            .width(Size::px(74.0))
                            .font_size(11.0)
                            .color(theme::TEXT_MUTED)
                            .text("Wire Format"),
                    )
                    .children(wf_chips),
            );
        }

        if let Some(hint) = draft_hint.read().as_ref() {
            box_el = box_el.child(
                label()
                    .font_size(11.0)
                    .color(theme::ACCENT_AMBER)
                    .text(hint.clone()),
            );
        }

        box_el
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

#[derive(PartialEq)]
struct LiveByteBlockSection {
    type_name: String,
    is_local: bool,
    byte_block: ByteBlockDef,
}

impl Component for LiveByteBlockSection {
    fn render(&self) -> impl IntoElement {
        let mut state = EditorState::current();
        let mut synced_bb = use_state(String::new);
        let mut bb_pad_input = use_state(String::new);
        let mut new_fname = use_state(String::new);
        let new_fty = use_state(|| "u32".to_owned());
        let mut new_fwf = use_state(|| None::<WireFormat>);
        let mut new_fpad = use_state(String::new);

        let sync_sig = format!("{}:{:?}", self.type_name, self.byte_block.pad_after);
        if *synced_bb.peek() != sync_sig {
            synced_bb.set(sync_sig);
            bb_pad_input.set(
                self.byte_block
                    .pad_after
                    .map(|p| p.to_string())
                    .unwrap_or_default(),
            );
        }

        let cur_new_ty = new_fty.read().clone();
        let new_compat_wfs = compatible_wire_formats(&cur_new_ty);
        let pad_bb_name = self.type_name.clone();
        let append_bb_name = self.type_name.clone();
        let is_local = self.is_local;

        let field_rows: Vec<Element> = self
            .byte_block
            .fields
            .iter()
            .enumerate()
            .map(|(idx, field)| {
                let wf_txt = field
                    .wire_format
                    .map(|w| format!(" [{}]", w.rust_type_name()))
                    .unwrap_or_default();
                let pad_txt = field
                    .pad_after
                    .map(|p| format!(" +{p}B"))
                    .unwrap_or_default();
                let del_bb = self.type_name.clone();
                let mut st = state;

                rect()
                    .width(Size::fill())
                    .height(Size::px(25.0))
                    .padding((0.0, 6.0))
                    .corner_radius(3.0)
                    .background(theme::BG_ROW)
                    .horizontal()
                    .cross_align(Alignment::Center)
                    .spacing(6.0)
                    .overflow(Overflow::Clip)
                    .child(
                        label()
                            .width(Size::px(if is_local { 305.0 } else { 345.0 }))
                            .font_size(11.0)
                            .font_family("monospace")
                            .color(theme::TEXT_PRIMARY)
                            .max_lines(1)
                            .text(format!(
                                "{idx}. {}: {}{wf_txt}{pad_txt}",
                                field.name,
                                field.ty.label()
                            )),
                    )
                    .maybe_child(is_local.then(|| {
                        action_button("Del", ButtonStyle::Danger, true, move || {
                            let bb_t = del_bb.clone();
                            st.mutate_schema(
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
                        })
                    }))
                    .into()
            })
            .collect();

        let preset_chips: Vec<Element> = ["u32", "i32", "f32", "bool", "usize", "vec3", "c_string"]
            .into_iter()
            .map(|preset| {
                let mut fty = new_fty;
                let mut fwf = new_fwf;
                segmented_chip(preset, cur_new_ty.trim() == preset, true, move || {
                    let norm = normalize_wire_format_for_type(preset, *fwf.read());
                    fwf.set(norm);
                    fty.set(preset.to_owned());
                })
            })
            .collect();

        let wf_chips: Vec<Element> = new_compat_wfs
            .iter()
            .map(|&(wf, wf_label)| {
                let mut fwf = new_fwf;
                let effective = normalize_wire_format_for_type(&cur_new_ty, *new_fwf.read());
                segmented_chip(wf_label, effective == Some(wf), true, move || {
                    fwf.set(Some(wf));
                })
            })
            .collect();

        let mut st_pad = state;
        let pad_header: Option<Element> = if is_local {
            Some(
                rect()
                    .horizontal()
                    .cross_align(Alignment::Center)
                    .spacing(5.0)
                    .child(
                        label()
                            .font_size(11.0)
                            .color(theme::TEXT_MUTED)
                            .text("pad_after (B):"),
                    )
                    .child(styled_input_live(
                        Size::px(56.0),
                        bb_pad_input,
                        "0",
                        move |txt| {
                            let trimmed = txt.trim();
                            let parsed = if trimmed.is_empty() {
                                Some(None)
                            } else {
                                trimmed.parse::<usize>().ok().map(Some)
                            };
                            if let Some(pad_val) = parsed {
                                let target = pad_bb_name.clone();
                                st_pad.mutate_schema(
                                    format!("Updated `{target}` pad_after = {pad_val:?}"),
                                    move |s, _| set_byte_block_pad_after(s, &target, pad_val),
                                );
                            }
                        },
                    ))
                    .into(),
            )
        } else {
            self.byte_block.pad_after.map(|pad| {
                label()
                    .font_size(11.0)
                    .color(theme::TEXT_DIM)
                    .text(format!("pad_after: {pad}B"))
                    .into()
            })
        };

        let mut section = rect()
            .width(Size::fill())
            .vertical()
            .spacing(6.0)
            .child(
                rect()
                    .width(Size::fill())
                    .horizontal()
                    .main_align(Alignment::SpaceBetween)
                    .cross_align(Alignment::Center)
                    .child(group_caption(format!(
                        "BYTEBLOCK FIELDS ({})",
                        self.byte_block.fields.len()
                    )))
                    .maybe_child(pad_header),
            )
            .child(if self.byte_block.fields.is_empty() {
                label()
                    .font_size(11.0)
                    .color(theme::TEXT_DIM)
                    .text("No byte fields defined yet.")
                    .into_element()
            } else {
                rect()
                    .width(Size::fill())
                    .padding(4.0)
                    .corner_radius(4.0)
                    .background(theme::BG_INSET)
                    .border(theme::border_all(theme::BORDER_CARD))
                    .vertical()
                    .spacing(2.0)
                    .children(field_rows)
                    .into_element()
            });

        if is_local {
            section = section.child(
                rect()
                    .width(Size::fill())
                    .padding((6.0, 8.0))
                    .corner_radius(4.0)
                    .background(theme::BG_SUB_CARD)
                    .vertical()
                    .spacing(5.0)
                    .child(group_caption("APPEND BYTEBLOCK FIELD"))
                    .child(
                        rect()
                            .width(Size::fill())
                            .horizontal()
                            .cross_align(Alignment::Center)
                            .spacing(6.0)
                            .child(styled_input(Size::px(128.0), new_fname, "field_name"))
                            .child(styled_input_live(
                                Size::px(112.0),
                                new_fty,
                                "u32 / vec3",
                                move |txt| {
                                    let norm = normalize_wire_format_for_type(&txt, *new_fwf.read());
                                    new_fwf.set(norm);
                                },
                            ))
                            .child(styled_input(Size::px(42.0), new_fpad, "pad"))
                            .child(action_button("+ Add", ButtonStyle::Primary, true, move || {
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
                                let pad = new_fpad
                                    .read()
                                    .trim()
                                    .parse::<usize>()
                                    .ok()
                                    .filter(|&p| p > 0);
                                let bb_t = append_bb_name.clone();
                                let fname_msg = fname.clone();
                                let ok = state.mutate_schema(
                                    format!("Appended `{fname_msg}` to `{bb_t}`"),
                                    move |s, _| {
                                        let Some(TypeDef::ByteBlock(bb)) = s.types.get_mut(&bb_t)
                                        else {
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
                            })),
                    )
                    .child(
                        rect()
                            .width(Size::fill())
                            .horizontal()
                            .cross_align(Alignment::Center)
                            .spacing(4.0)
                            .children(preset_chips),
                    )
                    .maybe_child((!new_compat_wfs.is_empty()).then(|| {
                        rect()
                            .width(Size::fill())
                            .horizontal()
                            .cross_align(Alignment::Center)
                            .spacing(5.0)
                            .child(
                                label()
                                    .font_size(11.0)
                                    .color(theme::TEXT_MUTED)
                                    .text("Wire:"),
                            )
                            .children(wf_chips)
                    })),
            );
        }

        section
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
