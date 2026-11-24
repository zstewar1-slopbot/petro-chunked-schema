//! Right pane: interactive schema editing cards for parent containers, chunk/mini-chunk fields,
//! and `ByteBlock` struct layouts.

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

/// Right-hand pane hosting interactive schema editing controls for the selected chunk or
/// mini-chunk.
#[component]
pub fn SchemaEditorPane(node: Option<SelectedNodeDetails>) -> Element {
    rsx! {
        rect {
            width: "390",
            height: "fill",
            padding: "8 10",
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
                height: "fill",
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
            padding: "8",
            background: theme::BG_CARD,
            direction: "vertical",
            spacing: "6",

            label {
                font_size: "12",
                font_weight: "bold",
                color: theme::ACCENT_BLUE,
                "1. Parent Container Relationship"
            }

            if let Some((p_title, p_mode, p_ignore)) = editable_summary {
                {
                    let p_ref_mode = parent_ref.clone();
                    let p_ref_ign = parent_ref.clone();
                    rsx! {
                        label { font_size: "11", "Container: {p_title}" }
                        rect {
                            direction: "horizontal",
                            spacing: "6",
                            Button {
                                onpress: move |_| {
                                    let next_mode = if p_mode == OrderMode::Map {
                                        OrderMode::Sequence
                                    } else {
                                        OrderMode::Map
                                    };
                                    let target_ref = p_ref_mode.clone();
                                    state.mutate_schema(
                                        format!("Set container ordering mode to {next_mode:?}"),
                                        move |s, _| {
                                            set_container_mode(s, &target_ref, next_mode);
                                            Ok(())
                                        },
                                    );
                                },
                                label { font_size: "11", "Order Mode: {p_mode:?} (Toggle)" }
                            }
                            Button {
                                onpress: move |_| {
                                    let next_ign = !p_ignore;
                                    let target_ref = p_ref_ign.clone();
                                    state.mutate_schema(
                                        format!("Set ignore_unknown = {next_ign}"),
                                        move |s, _| {
                                            set_container_ignore_unknown(s, &target_ref, next_ign);
                                            Ok(())
                                        },
                                    );
                                },
                                label { font_size: "11", "Ignore Unknown: {p_ignore}" }
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
                                font_size: "11",
                                color: theme::ACCENT_AMBER,
                                "Defined in imported schema `{alias}` (`{type_name}`), which is read-only from the current schema."
                            }
                            if let Some(src_ref) = source_opt {
                                Button {
                                    onpress: move |_| state.load_schema_ref(src_ref.clone()),
                                    label { font_size: "11", "Open Imported Schema ({alias})" }
                                }
                            }
                        }
                    }
                    ParentContainerRef::ChunkEnumType { type_name } => rsx! {
                        label {
                            font_size: "11",
                            color: theme::ACCENT_AMBER,
                            "Dispatched by `chunk_enum` `{type_name}` (edit variants in YAML)."
                        }
                    },
                    ParentContainerRef::ChunksType { type_name } => rsx! {
                        label {
                            font_size: "11",
                            color: theme::TEXT_MUTED,
                            "Stream sequence `{type_name}` (strict field declaration order)."
                        }
                    },
                    _ => rsx! {
                        label {
                            font_size: "11",
                            color: theme::ACCENT_AMBER,
                            "Parent chunk has no container type assigned yet. Select the parent chunk first and assign it a `Children` or `MiniChunks` container."
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

    rsx! {
        rect {
            width: "fill",
            padding: "8",
            background: theme::BG_CARD,
            direction: "vertical",
            spacing: "6",

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

            label { font_size: "11", "Field Name:" }
            Input {
                value: edit_field_name.read().clone(),
                placeholder: "field_name",
                onchange: move |txt| edit_field_name.set(txt),
            }

            label {
                font_size: "11",
                "Cardinality (currently: {edit_cardinality.read():?}):"
            }
            rect {
                direction: "horizontal",
                spacing: "4",
                Button {
                    onpress: move |_| edit_cardinality.set(Cardinality::Required),
                    label { font_size: "11", "Required" }
                }
                Button {
                    onpress: move |_| edit_cardinality.set(Cardinality::Optional),
                    label { font_size: "11", "Optional" }
                }
                Button {
                    onpress: move |_| edit_cardinality.set(Cardinality::Repeated),
                    label { font_size: "11", "Repeated" }
                }
                Button {
                    onpress: move |_| edit_cardinality.set(Cardinality::Default),
                    label { font_size: "11", "Default" }
                }
            }

            if !is_mc {
                label {
                    font_size: "11",
                    "Chunk Payload Kind (currently: {edit_chunk_payload_kind.read()}):"
                }
                rect {
                    direction: "horizontal",
                    spacing: "4",
                    Button {
                        onpress: move |_| edit_chunk_payload_kind.set("data".to_owned()),
                        label { font_size: "11", "Data" }
                    }
                    Button {
                        onpress: move |_| {
                            edit_chunk_payload_kind.set("mini_chunks".to_owned());
                            let fname = edit_field_name.read().clone();
                            edit_payload_type_str.set(format!("{}MiniChunks", to_pascal_case(&fname)));
                        },
                        label { font_size: "11", "MiniChunks" }
                    }
                    Button {
                        onpress: move |_| {
                            edit_chunk_payload_kind.set("children".to_owned());
                            let fname = edit_field_name.read().clone();
                            edit_payload_type_str.set(format!("{}Children", to_pascal_case(&fname)));
                        },
                        label { font_size: "11", "Children" }
                    }
                    Button {
                        onpress: move |_| edit_chunk_payload_kind.set("ignore".to_owned()),
                        label { font_size: "11", "Ignore" }
                    }
                }
            }

            label {
                font_size: "11",
                "Type / Target Struct Name (e.g. u32, f32, c_string, vec3, Vec<u16>, MyBlock):"
            }
            Input {
                value: edit_payload_type_str.read().clone(),
                placeholder: "u32 or MyByteBlock",
                onchange: move |txt| edit_payload_type_str.set(txt),
            }

            rect {
                direction: "horizontal",
                spacing: "4",
                for preset in ["u8", "u16", "u32", "f32", "c_string", "vec3"] {
                    Button {
                        onpress: move |_| {
                            edit_chunk_payload_kind.set("data".to_owned());
                            edit_payload_type_str.set(preset.to_owned());
                        },
                        label { font_size: "10", "{preset}" }
                    }
                }
            }

            label {
                font_size: "11",
                "Wire Format (currently: {edit_wire_format.read():?}):"
            }
            rect {
                direction: "horizontal",
                spacing: "4",
                Button {
                    onpress: move |_| edit_wire_format.set(None),
                    label { font_size: "10", "None" }
                }
                Button {
                    onpress: move |_| edit_wire_format.set(Some(WireFormat::U8Bool)),
                    label { font_size: "10", "U8Bool" }
                }
                Button {
                    onpress: move |_| edit_wire_format.set(Some(WireFormat::U32Bool)),
                    label { font_size: "10", "U32Bool" }
                }
                Button {
                    onpress: move |_| edit_wire_format.set(Some(WireFormat::U32Usize)),
                    label { font_size: "10", "U32Usize" }
                }
                Button {
                    onpress: move |_| edit_wire_format.set(Some(WireFormat::U32UsizeVec)),
                    label { font_size: "10", "U32UsizeVec" }
                }
            }

            if is_editable {
                rect {
                    direction: "horizontal",
                    spacing: "6",
                    Button {
                        onpress: move |_| {
                            let fname = edit_field_name.read().clone();
                            let card = edit_cardinality.read().clone();
                            let wf = *edit_wire_format.read();
                            let ty_str = edit_payload_type_str.read().clone();
                            let pkind = edit_chunk_payload_kind.read().clone();
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
                        label { font_weight: "bold", "Save Field to Schema" }
                    }

                    if has_existing {
                        Button {
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
                            label { color: theme::ACCENT_RED, "Delete Field" }
                        }
                    }
                }
            } else {
                label {
                    font_size: "11",
                    color: theme::TEXT_MUTED,
                    "Read-only in current schema (assign parent container or open the imported schema to edit)."
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

    rsx! {
        rect {
            width: "fill",
            padding: "8",
            background: theme::BG_CARD,
            direction: "vertical",
            spacing: "6",

            label {
                font_size: "12",
                font_weight: "bold",
                color: theme::ACCENT_AMBER,
                "3. ByteBlock Struct `{bb_name}` ({bb_def.fields.len()} fields, pad_after={bb_pad_val}B)"
            }

            if !is_local {
                label {
                    font_size: "11",
                    color: theme::TEXT_MUTED,
                    "Defined in imported schema `{owner_label}` (read-only)."
                }
            }

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
                            padding: "2 4",
                            background: theme::BG_INSET,
                            direction: "horizontal",
                            spacing: "6",
                            cross_align: "center",

                            if is_local {
                                Button {
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
                                    label { font_size: "10", "Del" }
                                }
                            }
                            label {
                                font_size: "11",
                                font_family: "monospace",
                                "{f_summary}"
                            }
                        }
                    }
                }
            }

            if is_local {
                rect {
                    direction: "horizontal",
                    spacing: "6",
                    cross_align: "center",
                    label { font_size: "11", "Trailing pad_after (B):" }
                    Input {
                        value: struct_pad_input.read().clone(),
                        placeholder: "0",
                        onchange: move |txt| struct_pad_input.set(txt),
                    }
                    Button {
                        onpress: move |_| {
                            let target_bb = bb_pad_save.clone();
                            let pad_opt: Option<usize> = struct_pad_input.read().trim().parse().ok();
                            state.mutate_schema(
                                format!("Updated pad_after on `{target_bb}`"),
                                move |s, _| set_byte_block_pad_after(s, &target_bb, pad_opt),
                            );
                        },
                        label { font_size: "11", "Set Pad" }
                    }
                }

                label { font_size: "11", "Append Field (name / type / pad_after):" }
                Input {
                    value: new_bb_field_name.read().clone(),
                    placeholder: "field_name",
                    onchange: move |txt| new_bb_field_name.set(txt),
                }
                Input {
                    value: new_bb_field_type.read().clone(),
                    placeholder: "u32, f32, vec3, [u8; 4]...",
                    onchange: move |txt| new_bb_field_type.set(txt),
                }
                Input {
                    value: new_bb_field_pad.read().clone(),
                    placeholder: "0 (pad_after bytes)",
                    onchange: move |txt| new_bb_field_pad.set(txt),
                }
                Button {
                    onpress: move |_| {
                        let fname = new_bb_field_name.read().trim().to_owned();
                        let fty_str = new_bb_field_type.read().trim().to_owned();
                        let pad: Option<usize> =
                            new_bb_field_pad.read().trim().parse().ok().filter(|&p| p > 0);
                        let target_bb = bb_add.clone();

                        state.mutate_schema(
                            format!("Appended field `{fname}` to `{target_bb}`"),
                            move |s, _| {
                                if fname.is_empty() {
                                    return Err("Field name cannot be empty".to_owned());
                                }
                                let dt = DataType::parse_str(&fty_str)
                                    .map_err(|e| format!("Invalid DataType `{fty_str}`: {e}"))?;
                                let wf = dt.default_wire_format();
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
                    label { "+ Append Field to {bb_name}" }
                }
            }
        }
    }
}
