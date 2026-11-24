//! Right pane: interactive schema editing cards for parent containers, chunk/mini-chunk fields,
//! and `ByteBlock` struct layouts using normalized typography, radio-style selector chips, and
//! context-aware wire-format controls.

use freya::prelude::*;
use petro_chunked_schema::resolve::SchemaId;
use petro_chunked_schema::schema::{
    ByteFieldDef, Cardinality, ChunkFieldSource, ChunkPayloadSpec, DataType, OrderMode, TypeDef,
    WireFormat,
};

use crate::editor_ops::{
    ParentContainerRef, SelectedNodeDetails, delete_chunk_field, delete_mini_chunk_field,
    find_existing_chunk_field, find_existing_mini_chunk_field, resolve_selected_node,
    set_byte_block_pad_after, to_pascal_case, upsert_chunk_field, upsert_mini_chunk_field,
};
use crate::ui::context::EditorState;
use crate::ui::theme;
use crate::ui::widgets::{ActionButton, ButtonStyle, SegmentedChip, StyledInput};

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

/// Right-hand pane hosting interactive schema editing controls for the selected chunk or
/// mini-chunk.
#[component]
pub fn SchemaEditorPane(node: Option<SelectedNodeDetails>) -> Element {
    rsx! {
        rect {
            width: "390",
            height: "fill",
            content: "flex",
            padding: "10",
            background: theme::BG_PANE_RIGHT,
            direction: "vertical",
            spacing: "10",

            label {
                font_size: "13",
                font_weight: "bold",
                color: theme::ACCENT_BLUE,
                "Interactive Schema Definition"
            }

            ScrollView {
                width: "fill",
                height: "flex(1)",
                rect {
                    width: "fill",
                    direction: "vertical",
                    spacing: "10",

                    if let Some(node) = node {
                        ContainerSettingsCard { parent_ref: node.parent_container.clone() }
                        FieldAssignmentCard { node: node.clone() }
                        if let Some((mod_id, bb_name)) = node.active_byte_block {
                            ByteBlockEditorCard {
                                owner_module: mod_id,
                                bb_name,
                            }
                        }
                    } else {
                        label {
                            font_size: "12",
                            color: theme::TEXT_MUTED,
                            "Select a chunk or mini-chunk in the tree to configure its schema."
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn ContainerSettingsCard(parent_ref: ParentContainerRef) -> Element {
    let mut state = use_context::<EditorState>();
    let schema = state.schema.read().clone();

    let editable_summary = match &parent_ref {
        ParentContainerRef::Root => match schema.root_type() {
            Some(TypeDef::Chunks(c)) => {
                Some((format!("root (`{}`)", schema.root), c.mode, c.ignore_unknown))
            }
            _ => None,
        },
        ParentContainerRef::ChunksType { type_name } => match schema.types.get(type_name) {
            Some(TypeDef::Chunks(c)) => {
                Some((format!("Chunks `{type_name}`"), c.mode, c.ignore_unknown))
            }
            Some(TypeDef::ChunkStreamSeq(_)) => None,
            _ => None,
        },
        ParentContainerRef::MiniChunksType { type_name } => match schema.types.get(type_name) {
            Some(TypeDef::MiniChunks(c)) => Some((
                format!("MiniChunks `{type_name}`"),
                c.mode,
                c.ignore_unknown,
            )),
            _ => None,
        },
        ParentContainerRef::ChunkEnumType { .. }
        | ParentContainerRef::Imported { .. }
        | ParentContainerRef::UnassignedParentChunk { .. } => None,
    };

    rsx! {
        rect {
            width: "fill",
            padding: "10",
            corner_radius: "4",
            background: theme::BG_CARD,
            direction: "vertical",
            spacing: "8",

            label {
                font_size: "12",
                font_weight: "bold",
                color: theme::ACCENT_BLUE,
                "1. Parent Container"
            }

            if let Some((p_title, p_mode, p_ignore)) = editable_summary {
                {
                    let p_ref_map = parent_ref.clone();
                    let p_ref_seq = parent_ref.clone();
                    let p_ref_strict = parent_ref.clone();
                    let p_ref_ignore = parent_ref.clone();
                    rsx! {
                        label {
                            font_size: "12",
                            color: theme::TEXT_PRIMARY,
                            "{p_title}"
                        }
                        rect {
                            width: "fill",
                            direction: "horizontal",
                            spacing: "8",
                            cross_align: "center",
                            label {
                                width: "110",
                                font_size: "12",
                                color: theme::TEXT_MUTED,
                                "Order Mode"
                            }
                            SegmentedChip {
                                label: "Map".to_owned(),
                                selected: p_mode == OrderMode::Map,
                                onpress: move |_| {
                                    let target_ref = p_ref_map.clone();
                                    state.mutate_schema(
                                        "Set container ordering mode to Map".to_owned(),
                                        move |s, _| {
                                            set_container_mode(s, &target_ref, OrderMode::Map);
                                            Ok(())
                                        },
                                    );
                                },
                            }
                            SegmentedChip {
                                label: "Sequence".to_owned(),
                                selected: p_mode == OrderMode::Sequence,
                                onpress: move |_| {
                                    let target_ref = p_ref_seq.clone();
                                    state.mutate_schema(
                                        "Set container ordering mode to Sequence".to_owned(),
                                        move |s, _| {
                                            set_container_mode(s, &target_ref, OrderMode::Sequence);
                                            Ok(())
                                        },
                                    );
                                },
                            }
                        }

                        rect {
                            width: "fill",
                            direction: "horizontal",
                            spacing: "8",
                            cross_align: "center",
                            label {
                                width: "110",
                                font_size: "12",
                                color: theme::TEXT_MUTED,
                                "Unknown Chunks"
                            }
                            SegmentedChip {
                                label: "Strict".to_owned(),
                                selected: !p_ignore,
                                onpress: move |_| {
                                    let target_ref = p_ref_strict.clone();
                                    state.mutate_schema(
                                        "Set ignore_unknown = false".to_owned(),
                                        move |s, _| {
                                            set_container_ignore_unknown(s, &target_ref, false);
                                            Ok(())
                                        },
                                    );
                                },
                            }
                            SegmentedChip {
                                label: "Ignore".to_owned(),
                                selected: p_ignore,
                                onpress: move |_| {
                                    let target_ref = p_ref_ignore.clone();
                                    state.mutate_schema(
                                        "Set ignore_unknown = true".to_owned(),
                                        move |s, _| {
                                            set_container_ignore_unknown(s, &target_ref, true);
                                            Ok(())
                                        },
                                    );
                                },
                            }
                        }
                    }
                }
            } else {
                match &parent_ref {
                    ParentContainerRef::Imported { alias, source, type_name, .. } => {
                        let source_opt = source.clone();
                        rsx! {
                            label {
                                font_size: "12",
                                color: theme::ACCENT_AMBER,
                                "Defined in imported schema `{alias}` (`{type_name}`), which is read-only from this file."
                            }
                            if let Some(src_ref) = source_opt {
                                rect {
                                    direction: "horizontal",
                                    ActionButton {
                                        label: format!("Open `{alias}` Schema"),
                                        style: ButtonStyle::Secondary,
                                        onpress: move |_| state.load_schema_ref(src_ref.clone()),
                                    }
                                }
                            }
                        }
                    }
                    ParentContainerRef::ChunkEnumType { type_name } => rsx! {
                        label {
                            font_size: "12",
                            color: theme::ACCENT_AMBER,
                            "Dispatched by `chunk_enum` `{type_name}` (edit variants in YAML)."
                        }
                    },
                    ParentContainerRef::ChunksType { type_name } => rsx! {
                        label {
                            font_size: "12",
                            color: theme::TEXT_MUTED,
                            "Stream sequence `{type_name}` (strict field declaration order)."
                        }
                    },
                    _ => rsx! {
                        label {
                            font_size: "12",
                            color: theme::ACCENT_AMBER,
                            "Parent chunk has no container type assigned yet. Assign a `Children` or `MiniChunks` container to its parent first."
                        }
                    },
                }
            }
        }
    }
}

fn set_container_mode(s: &mut petro_chunked_schema::Schema, pref: &ParentContainerRef, mode: OrderMode) {
    match pref {
        ParentContainerRef::Root => {
            if let Some(TypeDef::Chunks(c)) = s.root_type_mut() {
                c.mode = mode;
            }
        }
        ParentContainerRef::ChunksType { type_name } => {
            if let Some(TypeDef::Chunks(c)) = s.types.get_mut(type_name) {
                c.mode = mode;
            }
        }
        ParentContainerRef::MiniChunksType { type_name } => {
            if let Some(TypeDef::MiniChunks(c)) = s.types.get_mut(type_name) {
                c.mode = mode;
            }
        }
        _ => {}
    }
}

fn set_container_ignore_unknown(
    s: &mut petro_chunked_schema::Schema,
    pref: &ParentContainerRef,
    ignore_unknown: bool,
) {
    match pref {
        ParentContainerRef::Root => {
            if let Some(TypeDef::Chunks(c)) = s.root_type_mut() {
                c.ignore_unknown = ignore_unknown;
            }
        }
        ParentContainerRef::ChunksType { type_name } => {
            if let Some(TypeDef::Chunks(c)) = s.types.get_mut(type_name) {
                c.ignore_unknown = ignore_unknown;
            }
        }
        ParentContainerRef::MiniChunksType { type_name } => {
            if let Some(TypeDef::MiniChunks(c)) = s.types.get_mut(type_name) {
                c.ignore_unknown = ignore_unknown;
            }
        }
        _ => {}
    }
}

#[component]
fn FieldAssignmentCard(node: SelectedNodeDetails) -> Element {
    let mut state = use_context::<EditorState>();

    let mut edit_field_name = use_signal(String::new);
    let mut edit_cardinality = use_signal(|| Cardinality::Required);
    let mut edit_chunk_payload_kind = use_signal(|| "data".to_owned());
    let mut edit_payload_type_str = use_signal(|| "u32".to_owned());
    let mut edit_wire_format = use_signal(|| None::<WireFormat>);

    use_effect(move || {
        let path = state.selected_tree_path.read().clone();
        let resolved = state.resolved_schema.read().clone();
        let df_opt = state.dynamic_file.read().clone();
        let Some(df) = df_opt.as_ref() else {
            return;
        };
        let Some(nd) = resolve_selected_node(df, &resolved, &path) else {
            return;
        };

        if nd.is_mini_chunk {
            if let Some(existing) =
                find_existing_mini_chunk_field(&resolved, &nd.parent_container, nd.id as u8)
            {
                edit_field_name.set(existing.name.clone());
                edit_cardinality.set(existing.cardinality.clone());
                edit_payload_type_str.set(existing.ty.label());
                edit_wire_format.set(existing.wire_format);
            } else {
                edit_field_name.set(
                    nd.schema_field_name
                        .clone()
                        .unwrap_or_else(|| format!("mc_{:02x}", nd.id)),
                );
                edit_cardinality.set(Cardinality::Required);
                let default_ty = match nd.size {
                    1 => "u8",
                    2 => "u16",
                    4 => "u32",
                    8 => "vec2",
                    12 => "vec3",
                    16 => "vec4",
                    _ => "c_string",
                };
                edit_payload_type_str.set(default_ty.to_owned());
                edit_wire_format.set(None);
            }
        } else if let Some(existing) =
            find_existing_chunk_field(&resolved, &nd.parent_container, nd.id)
        {
            edit_field_name.set(existing.name.clone());
            edit_cardinality.set(existing.cardinality.clone());
            edit_wire_format.set(existing.wire_format);
            if let ChunkFieldSource::Id { payload, .. } = &existing.source {
                match payload {
                    ChunkPayloadSpec::Data(dt) => {
                        edit_chunk_payload_kind.set("data".to_owned());
                        edit_payload_type_str.set(dt.label());
                    }
                    ChunkPayloadSpec::MiniChunks(tname) => {
                        edit_chunk_payload_kind.set("mini_chunks".to_owned());
                        edit_payload_type_str.set(tname.clone());
                    }
                    ChunkPayloadSpec::Children(tname) => {
                        edit_chunk_payload_kind.set("children".to_owned());
                        edit_payload_type_str.set(tname.clone());
                    }
                    ChunkPayloadSpec::Ignore => {
                        edit_chunk_payload_kind.set("ignore".to_owned());
                    }
                }
            }
        } else {
            let suggested_field = nd
                .schema_field_name
                .clone()
                .unwrap_or_else(|| format!("chunk_{:04x}", nd.id));
            edit_field_name.set(suggested_field.clone());
            edit_cardinality.set(Cardinality::Required);
            edit_wire_format.set(None);
            if nd.is_container {
                edit_chunk_payload_kind.set("children".to_owned());
                edit_payload_type_str
                    .set(format!("{}Container", to_pascal_case(&suggested_field)));
            } else if nd.looks_like_mini_chunks {
                edit_chunk_payload_kind.set("mini_chunks".to_owned());
                edit_payload_type_str
                    .set(format!("{}MiniChunks", to_pascal_case(&suggested_field)));
            } else {
                edit_chunk_payload_kind.set("data".to_owned());
                edit_payload_type_str.set(format!("{}Block", to_pascal_case(&suggested_field)));
            }
        }
    });

    let parent_ref = node.parent_container.clone();
    let is_editable = matches!(
        parent_ref,
        ParentContainerRef::Root
            | ParentContainerRef::ChunksType { .. }
            | ParentContainerRef::MiniChunksType { .. }
    );
    let node_id = node.id;
    let is_mc = node.is_mini_chunk;

    let has_existing = {
        let resolved = state.resolved_schema.read();
        if is_mc {
            find_existing_mini_chunk_field(&resolved, &parent_ref, node_id as u8).is_some()
        } else {
            find_existing_chunk_field(&resolved, &parent_ref, node_id).is_some()
        }
    };

    let parent_ref_save = parent_ref.clone();
    let parent_ref_del = parent_ref;

    let cur_card = edit_cardinality.read().clone();
    let cur_pkind = edit_chunk_payload_kind.read().clone();
    let cur_ty_str = edit_payload_type_str.read().clone();
    let trimmed_ty = cur_ty_str.trim().to_owned();
    let wf_opts = compatible_wire_formats(&cur_ty_str);
    let active_wf = normalize_wire_format_for_type(&cur_ty_str, *edit_wire_format.read());
    let is_data_payload = is_mc || cur_pkind == "data";

    rsx! {
        rect {
            width: "fill",
            padding: "10",
            corner_radius: "4",
            background: theme::BG_CARD,
            direction: "vertical",
            spacing: "8",

            label {
                font_size: "12",
                font_weight: "bold",
                color: theme::ACCENT_GREEN,
                if is_mc {
                    "2. Assign Mini-Chunk 0x{node_id:02X} ({node_id})"
                } else {
                    "2. Assign Chunk 0x{node_id:04X}"
                }
            }

            label { font_size: "12", color: theme::TEXT_MUTED, "Field Name" }
            StyledInput {
                value: edit_field_name.read().clone(),
                placeholder: Some("field_name".to_owned()),
                onchange: move |txt| edit_field_name.set(txt),
            }

            label { font_size: "12", color: theme::TEXT_MUTED, "Cardinality" }
            rect {
                direction: "horizontal",
                spacing: "4",
                SegmentedChip {
                    label: "Required".to_owned(),
                    selected: cur_card == Cardinality::Required,
                    compact: Some(true),
                    onpress: move |_| edit_cardinality.set(Cardinality::Required),
                }
                SegmentedChip {
                    label: "Optional".to_owned(),
                    selected: cur_card == Cardinality::Optional,
                    compact: Some(true),
                    onpress: move |_| edit_cardinality.set(Cardinality::Optional),
                }
                SegmentedChip {
                    label: "Repeated".to_owned(),
                    selected: cur_card == Cardinality::Repeated,
                    compact: Some(true),
                    onpress: move |_| edit_cardinality.set(Cardinality::Repeated),
                }
                SegmentedChip {
                    label: "Default".to_owned(),
                    selected: cur_card == Cardinality::Default,
                    compact: Some(true),
                    onpress: move |_| edit_cardinality.set(Cardinality::Default),
                }
            }

            if !is_mc {
                label { font_size: "12", color: theme::TEXT_MUTED, "Payload Kind" }
                rect {
                    direction: "horizontal",
                    spacing: "4",
                    SegmentedChip {
                        label: "Data".to_owned(),
                        selected: cur_pkind == "data",
                        compact: Some(true),
                        onpress: move |_| edit_chunk_payload_kind.set("data".to_owned()),
                    }
                    SegmentedChip {
                        label: "MiniChunks".to_owned(),
                        selected: cur_pkind == "mini_chunks",
                        compact: Some(true),
                        onpress: move |_| {
                            edit_chunk_payload_kind.set("mini_chunks".to_owned());
                            let fname = edit_field_name.read().clone();
                            edit_payload_type_str.set(format!("{}MiniChunks", to_pascal_case(&fname)));
                            edit_wire_format.set(None);
                        },
                    }
                    SegmentedChip {
                        label: "Children".to_owned(),
                        selected: cur_pkind == "children",
                        compact: Some(true),
                        onpress: move |_| {
                            edit_chunk_payload_kind.set("children".to_owned());
                            let fname = edit_field_name.read().clone();
                            edit_payload_type_str.set(format!("{}Children", to_pascal_case(&fname)));
                            edit_wire_format.set(None);
                        },
                    }
                    SegmentedChip {
                        label: "Ignore".to_owned(),
                        selected: cur_pkind == "ignore",
                        compact: Some(true),
                        onpress: move |_| {
                            edit_chunk_payload_kind.set("ignore".to_owned());
                            edit_wire_format.set(None);
                        },
                    }
                }
            }

            if !is_mc && cur_pkind == "ignore" {
                label {
                    font_size: "12",
                    color: theme::TEXT_DIM,
                    "Ignored chunks are skipped cleanly during schema validation."
                }
            } else if !is_data_payload {
                label {
                    font_size: "12",
                    color: theme::TEXT_MUTED,
                    "Target Container Type"
                }
                StyledInput {
                    value: cur_ty_str.clone(),
                    placeholder: Some("ContainerTypeName".to_owned()),
                    onchange: move |txt| edit_payload_type_str.set(txt),
                }
            } else {
                label {
                    font_size: "12",
                    color: theme::TEXT_MUTED,
                    "Type / ByteBlock Struct"
                }
                StyledInput {
                    value: cur_ty_str.clone(),
                    placeholder: Some("u32, f32, bool, vec3, or MyBlock".to_owned()),
                    onchange: move |txt| edit_payload_type_str.set(txt),
                }

                rect {
                    direction: "vertical",
                    spacing: "4",
                    rect {
                        direction: "horizontal",
                        spacing: "4",
                        for preset in ["u8", "u16", "u32", "f32", "bool", "usize"] {
                            SegmentedChip {
                                key: "{preset}",
                                label: preset.to_owned(),
                                selected: trimmed_ty == preset,
                                compact: Some(true),
                                onpress: move |_| {
                                    edit_chunk_payload_kind.set("data".to_owned());
                                    edit_payload_type_str.set(preset.to_owned());
                                    edit_wire_format.set(normalize_wire_format_for_type(preset, None));
                                },
                            }
                        }
                    }
                    rect {
                        direction: "horizontal",
                        spacing: "4",
                        for preset in ["c_string", "vec3", "Vec<usize>"] {
                            SegmentedChip {
                                key: "{preset}",
                                label: preset.to_owned(),
                                selected: trimmed_ty == preset,
                                compact: Some(true),
                                onpress: move |_| {
                                    edit_chunk_payload_kind.set("data".to_owned());
                                    edit_payload_type_str.set(preset.to_owned());
                                    edit_wire_format.set(normalize_wire_format_for_type(preset, None));
                                },
                            }
                        }
                    }
                }

                if !wf_opts.is_empty() {
                    label { font_size: "12", color: theme::TEXT_MUTED, "Wire Format" }
                    rect {
                        direction: "horizontal",
                        spacing: "4",
                        for &(wf, lbl) in wf_opts {
                            SegmentedChip {
                                key: "{lbl}",
                                label: lbl.to_owned(),
                                selected: active_wf == Some(wf),
                                compact: Some(true),
                                onpress: move |_| edit_wire_format.set(Some(wf)),
                            }
                        }
                    }
                }
            }

            if is_editable {
                rect {
                    direction: "horizontal",
                    spacing: "6",
                    ActionButton {
                        label: "Save Field".to_owned(),
                        style: ButtonStyle::Primary,
                        onpress: move |_| {
                            let fname = edit_field_name.read().clone();
                            let card = edit_cardinality.read().clone();
                            let ty_str = edit_payload_type_str.read().clone();
                            let pkind = edit_chunk_payload_kind.read().clone();
                            let wf = if is_mc || pkind == "data" {
                                normalize_wire_format_for_type(&ty_str, *edit_wire_format.read())
                            } else {
                                None
                            };
                            let pref = parent_ref_save.clone();

                            state.mutate_schema(
                                format!("Updated schema field `{fname}` for ID 0x{node_id:X}"),
                                move |s, _| {
                                    if is_mc {
                                        let dt = DataType::parse_str(&ty_str).map_err(|e| e.to_string())?;
                                        upsert_mini_chunk_field(s, &pref, node_id as u8, &fname, card, dt, wf)
                                    } else {
                                        let spec = match pkind.as_str() {
                                            "mini_chunks" => ChunkPayloadSpec::MiniChunks(ty_str.trim().to_owned()),
                                            "children" => ChunkPayloadSpec::Children(ty_str.trim().to_owned()),
                                            "ignore" => ChunkPayloadSpec::Ignore,
                                            _ => ChunkPayloadSpec::Data(
                                                DataType::parse_str(&ty_str).map_err(|e| e.to_string())?,
                                            ),
                                        };
                                        upsert_chunk_field(s, &pref, node_id, &fname, card, spec, wf)
                                    }
                                },
                            );
                        },
                    }

                    if has_existing {
                        ActionButton {
                            label: "Delete Field".to_owned(),
                            style: ButtonStyle::Danger,
                            onpress: move |_| {
                                let pref = parent_ref_del.clone();
                                state.mutate_schema(
                                    format!("Removed field for ID 0x{node_id:X}"),
                                    move |s, _| {
                                        if is_mc {
                                            delete_mini_chunk_field(s, &pref, node_id as u8).map(|_| ())
                                        } else {
                                            delete_chunk_field(s, &pref, node_id).map(|_| ())
                                        }
                                    },
                                );
                            },
                        }
                    }
                }
            } else {
                label {
                    font_size: "12",
                    color: theme::TEXT_MUTED,
                    "Read-only in this schema (edit via the parent container or open the imported schema)."
                }
            }
        }
    }
}

#[component]
fn ByteBlockEditorCard(owner_module: SchemaId, bb_name: String) -> Element {
    let mut state = use_context::<EditorState>();
    let mut new_bb_field_name = use_signal(|| "field_0".to_owned());
    let mut new_bb_field_type = use_signal(|| "u32".to_owned());
    let mut new_bb_wire_format = use_signal(|| None::<WireFormat>);
    let mut new_bb_field_pad = use_signal(|| "0".to_owned());
    let mut struct_pad_input = use_signal(|| "0".to_owned());

    let resolved = state.resolved_schema.read().clone();
    let Some(TypeDef::ByteBlock(bb_def)) =
        resolved.module(owner_module).schema.types.get(&bb_name).cloned()
    else {
        return rsx! {};
    };

    let is_local = owner_module == SchemaId::ROOT;
    let owner_label = resolved.module(owner_module).schema.name.clone();

    let bb_key = bb_name.clone();
    let bb_pad_val = bb_def.pad_after.unwrap_or(0);
    use_effect(move || {
        let _path = state.selected_tree_path.read();
        let cur_resolved = state.resolved_schema.read();
        if let Some(TypeDef::ByteBlock(b)) = cur_resolved.module(owner_module).schema.types.get(&bb_key) {
            struct_pad_input.set(b.pad_after.unwrap_or(0).to_string());
        }
    });

    let bb_pad_save = bb_name.clone();
    let bb_add = bb_name.clone();
    let new_ty_str = new_bb_field_type.read().clone();
    let bb_wf_opts = compatible_wire_formats(&new_ty_str);
    let bb_active_wf = normalize_wire_format_for_type(&new_ty_str, *new_bb_wire_format.read());

    rsx! {
        rect {
            width: "fill",
            padding: "10",
            corner_radius: "4",
            background: theme::BG_CARD,
            direction: "vertical",
            spacing: "8",

            label {
                font_size: "12",
                font_weight: "bold",
                color: theme::ACCENT_AMBER,
                "3. ByteBlock `{bb_name}` ({bb_def.fields.len()} fields, pad_after={bb_pad_val}B)"
            }

            if !is_local {
                label {
                    font_size: "12",
                    color: theme::TEXT_MUTED,
                    "Defined in imported schema `{owner_label}` (read-only)."
                }
            }

            rect {
                width: "fill",
                direction: "vertical",
                spacing: "3",
                for (f_idx, field) in bb_def.fields.iter().enumerate() {
                    {
                        let f_summary = format!(
                            "#{f_idx} {}: {}{}{}",
                            field.name,
                            field.ty.label(),
                            field
                                .wire_format
                                .map(|w| format!(" ({})", w.rust_type_name()))
                                .unwrap_or_default(),
                            field
                                .pad_after
                                .map(|p| format!(" [+{p}B pad]"))
                                .unwrap_or_default()
                        );
                        let bb_del = bb_name.clone();
                        rsx! {
                            rect {
                                width: "fill",
                                padding: "4 6",
                                corner_radius: "3",
                                background: theme::BG_INSET,
                                direction: "horizontal",
                                spacing: "6",
                                cross_align: "center",

                                if is_local {
                                    ActionButton {
                                        label: "Del".to_owned(),
                                        style: ButtonStyle::Danger,
                                        compact: Some(true),
                                        onpress: move |_| {
                                            let target_bb = bb_del.clone();
                                            state.mutate_schema(
                                                format!("Removed field #{f_idx} from `{target_bb}`"),
                                                move |s, _| {
                                                    if let Some(TypeDef::ByteBlock(b)) = s.types.get_mut(&target_bb)
                                                        && f_idx < b.fields.len()
                                                    {
                                                        b.fields.remove(f_idx);
                                                    }
                                                    Ok(())
                                                },
                                            );
                                        },
                                    }
                                }
                                label {
                                    font_size: "12",
                                    font_family: "monospace",
                                    color: theme::TEXT_PRIMARY,
                                    "{f_summary}"
                                }
                            }
                        }
                    }
                }
            }

            if is_local {
                rect {
                    width: "fill",
                    direction: "horizontal",
                    spacing: "8",
                    cross_align: "center",
                    label { font_size: "12", color: theme::TEXT_MUTED, "pad_after (B)" }
                    StyledInput {
                        width: Some("80".to_owned()),
                        value: struct_pad_input.read().clone(),
                        placeholder: Some("0".to_owned()),
                        onchange: move |txt| struct_pad_input.set(txt),
                    }
                    ActionButton {
                        label: "Set Pad".to_owned(),
                        style: ButtonStyle::Secondary,
                        compact: Some(true),
                        onpress: move |_| {
                            let target_bb = bb_pad_save.clone();
                            let pad_opt: Option<usize> = struct_pad_input.read().trim().parse().ok();
                            state.mutate_schema(
                                format!("Updated pad_after on `{target_bb}`"),
                                move |s, _| set_byte_block_pad_after(s, &target_bb, pad_opt),
                            );
                        },
                    }
                }

                label {
                    font_size: "12",
                    font_weight: "bold",
                    color: theme::TEXT_MUTED,
                    "Append Field"
                }
                StyledInput {
                    value: new_bb_field_name.read().clone(),
                    placeholder: Some("field_name".to_owned()),
                    onchange: move |txt| new_bb_field_name.set(txt),
                }
                StyledInput {
                    value: new_ty_str.clone(),
                    placeholder: Some("u32, f32, bool, vec3, [u8; 4]...".to_owned()),
                    onchange: move |txt: String| {
                        let wf = normalize_wire_format_for_type(&txt, *new_bb_wire_format.read());
                        new_bb_wire_format.set(wf);
                        new_bb_field_type.set(txt);
                    },
                }

                if !bb_wf_opts.is_empty() {
                    rect {
                        direction: "horizontal",
                        spacing: "4",
                        cross_align: "center",
                        label { font_size: "12", color: theme::TEXT_MUTED, "Wire Format" }
                        for &(wf, lbl) in bb_wf_opts {
                            SegmentedChip {
                                key: "{lbl}",
                                label: lbl.to_owned(),
                                selected: bb_active_wf == Some(wf),
                                compact: Some(true),
                                onpress: move |_| new_bb_wire_format.set(Some(wf)),
                            }
                        }
                    }
                }

                StyledInput {
                    value: new_bb_field_pad.read().clone(),
                    placeholder: Some("0 (optional pad_after bytes)".to_owned()),
                    onchange: move |txt| new_bb_field_pad.set(txt),
                }
                rect {
                    direction: "horizontal",
                    ActionButton {
                        label: "+ Append Field".to_owned(),
                        style: ButtonStyle::Primary,
                        onpress: move |_| {
                            let fname = new_bb_field_name.read().trim().to_owned();
                            let fty_str = new_bb_field_type.read().trim().to_owned();
                            let pad: Option<usize> =
                                new_bb_field_pad.read().trim().parse().ok().filter(|&p| p > 0);
                            let wf = normalize_wire_format_for_type(&fty_str, *new_bb_wire_format.read());
                            let target_bb = bb_add.clone();

                            state.mutate_schema(
                                format!("Appended field `{fname}` to `{target_bb}`"),
                                move |s, _| {
                                    if fname.is_empty() {
                                        return Err("Field name cannot be empty".to_owned());
                                    }
                                    let dt = DataType::parse_str(&fty_str)
                                        .map_err(|e| format!("Invalid DataType `{fty_str}`: {e}"))?;
                                    if let Some(TypeDef::ByteBlock(b)) = s.types.get_mut(&target_bb) {
                                        b.fields.push(ByteFieldDef {
                                            name: fname,
                                            description: None,
                                            ty: dt,
                                            wire_format: wf,
                                            pad_after: pad,
                                        });
                                        Ok(())
                                    } else {
                                        Err(format!("ByteBlock `{target_bb}` not found"))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compatible_wire_formats_filters_by_data_type() {
        assert_eq!(
            compatible_wire_formats("bool"),
            &[
                (WireFormat::U8Bool, "U8Bool (1B)"),
                (WireFormat::U32Bool, "U32Bool (4B)"),
            ]
        );
        assert_eq!(
            compatible_wire_formats("usize"),
            &[(WireFormat::U32Usize, "U32Usize (4B)")]
        );
        assert_eq!(
            compatible_wire_formats("Vec<usize>"),
            &[(WireFormat::U32UsizeVec, "U32UsizeVec")]
        );
        assert!(compatible_wire_formats("u32").is_empty());
        assert!(compatible_wire_formats("vec3").is_empty());
        assert!(compatible_wire_formats("MeshInfo").is_empty());
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
            normalize_wire_format_for_type("Vec<usize>", None),
            Some(WireFormat::U32UsizeVec)
        );
        assert_eq!(
            normalize_wire_format_for_type("f32", Some(WireFormat::U32Bool)),
            None
        );
    }
}
