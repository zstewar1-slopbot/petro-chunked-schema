//! Schema evaluator, byte-block decoder, and span recorder.

use std::collections::BTreeSet;
use std::ffi::CStr;

use petro_chunked::byteblock::TakeBytes;

use crate::resolve::{ResolvedSchema, SchemaId};
use crate::schema::{
    ByteBlockDef, Cardinality, ChunkContainerDef, ChunkFieldDef, ChunkFieldSource,
    ChunkPayloadSpec, DataType, MiniChunkContainerDef, OrderMode, TypeDef, WireFormat,
};

use super::LoadOptions;
use super::model::{
    ByteSpanAnnotation, DynamicChunk, DynamicChunkContent, DynamicMiniChunk, DynamicValue,
    SchemaDiagnostic,
};
use super::parser::parse_raw_mini_chunks;

struct DecodeCtx<'a> {
    resolved: &'a ResolvedSchema,
    module_id: SchemaId,
    spans: &'a mut Vec<ByteSpanAnnotation>,
    color_counter: &'a mut usize,
    options: &'a LoadOptions,
}

impl<'a> DecodeCtx<'a> {
    fn in_module(&mut self, module_id: SchemaId) -> DecodeCtx<'_> {
        DecodeCtx {
            resolved: self.resolved,
            module_id,
            spans: self.spans,
            color_counter: self.color_counter,
            options: self.options,
        }
    }
}

pub(crate) fn evaluate_resolved_schema(
    resolved: &ResolvedSchema,
    chunks: &mut [DynamicChunk],
    options: &LoadOptions,
    diagnostics: &mut Vec<SchemaDiagnostic>,
) {
    let (root_mod_id, root_name, root_def) = resolved.root_type();
    match root_def {
        TypeDef::Chunks(container) => {
            evaluate_chunk_container(
                resolved,
                root_mod_id,
                container,
                chunks,
                "root",
                0,
                options,
                diagnostics,
            );
        }
        TypeDef::ChunkEnum(_) | TypeDef::ChunkStreamSeq(_) => {
            if chunks.is_empty() {
                diagnostics.push(SchemaDiagnostic {
                    tree_path: "root".to_owned(),
                    file_offset: 0,
                    message: format!("Empty file: expected root type `{root_name}`"),
                });
                return;
            }
            let consumed = try_match_stream_item_at(
                resolved,
                root_mod_id,
                "",
                root_name,
                chunks,
                0,
                options,
                diagnostics,
            );
            if consumed == 0 {
                let first_id = chunks[0].id;
                push_chunk_err(
                    &mut chunks[0],
                    diagnostics,
                    format!("Chunk 0x{first_id:08x} does not match root type `{root_name}`"),
                );
            } else {
                for leftover in &mut chunks[consumed..] {
                    push_chunk_err(
                        leftover,
                        diagnostics,
                        format!(
                            "Unexpected trailing chunk 0x{:08x} after root `{root_name}`",
                            leftover.id
                        ),
                    );
                }
            }
        }
        TypeDef::ByteBlock(_) | TypeDef::MiniChunks(_) => {
            diagnostics.push(SchemaDiagnostic {
                tree_path: "root".to_owned(),
                file_offset: 0,
                message: format!(
                    "Root type `{root_name}` must be `chunks`, `chunk_enum`, or `chunk_stream_seq`"
                ),
            });
        }
    }
}

fn push_chunk_err(
    chunk: &mut DynamicChunk,
    diagnostics: &mut Vec<SchemaDiagnostic>,
    msg: impl Into<String>,
) {
    let m = msg.into();
    chunk.errors.push(m.clone());
    diagnostics.push(SchemaDiagnostic {
        tree_path: chunk.tree_path.clone(),
        file_offset: chunk.header_offset,
        message: m,
    });
}

#[allow(clippy::too_many_arguments)]
fn evaluate_chunk_container(
    resolved: &ResolvedSchema,
    module_id: SchemaId,
    container: &ChunkContainerDef,
    chunks: &mut [DynamicChunk],
    container_path: &str,
    container_offset: usize,
    options: &LoadOptions,
    diagnostics: &mut Vec<SchemaDiagnostic>,
) {
    match container.mode {
        OrderMode::Sequence => {
            let mut cursor = 0usize;
            for field in &container.fields {
                let mut count = 0usize;
                while cursor < chunks.len() {
                    let consumed = try_match_field_at(
                        resolved,
                        module_id,
                        field,
                        chunks,
                        cursor,
                        options,
                        diagnostics,
                    );
                    if consumed == 0 {
                        break;
                    }
                    cursor += consumed;
                    count += 1;
                    if !matches!(field.cardinality, Cardinality::Repeated) {
                        break;
                    }
                }
                if matches!(field.cardinality, Cardinality::Required) && count == 0 {
                    diagnostics.push(SchemaDiagnostic {
                        tree_path: container_path.to_owned(),
                        file_offset: container_offset,
                        message: format!(
                            "Missing required chunk field `{}` in sequence container",
                            field.name
                        ),
                    });
                }
            }
            if !container.ignore_unknown {
                for leftover in &mut chunks[cursor..] {
                    push_chunk_err(
                        leftover,
                        diagnostics,
                        format!("Unexpected chunk 0x{:08x} after sequence", leftover.id),
                    );
                }
            }
        }
        OrderMode::Map => {
            let mut field_counts = vec![0usize; container.fields.len()];
            let mut cursor = 0usize;
            while cursor < chunks.len() {
                let mut matched_consumed = 0usize;
                for (fidx, field) in container.fields.iter().enumerate() {
                    let consumed = try_match_field_at(
                        resolved,
                        module_id,
                        field,
                        chunks,
                        cursor,
                        options,
                        diagnostics,
                    );
                    if consumed > 0 {
                        field_counts[fidx] += 1;
                        if field_counts[fidx] > 1
                            && !matches!(field.cardinality, Cardinality::Repeated)
                        {
                            let cid = chunks[cursor].id;
                            push_chunk_err(
                                &mut chunks[cursor],
                                diagnostics,
                                format!(
                                    "Duplicate chunk 0x{cid:08x} for non-repeated field `{}`",
                                    field.name
                                ),
                            );
                        }
                        matched_consumed = consumed;
                        break;
                    }
                }
                if matched_consumed > 0 {
                    cursor += matched_consumed;
                } else {
                    if !container.ignore_unknown {
                        let cid = chunks[cursor].id;
                        push_chunk_err(
                            &mut chunks[cursor],
                            diagnostics,
                            format!("Unexpected chunk 0x{cid:08x} in map container"),
                        );
                    }
                    cursor += 1;
                }
            }
            for (fidx, field) in container.fields.iter().enumerate() {
                if matches!(field.cardinality, Cardinality::Required) && field_counts[fidx] == 0 {
                    diagnostics.push(SchemaDiagnostic {
                        tree_path: container_path.to_owned(),
                        file_offset: container_offset,
                        message: format!(
                            "Missing required chunk field `{}` in map container",
                            field.name
                        ),
                    });
                }
            }
        }
    }
}

/// Attempts to match `field` starting at `chunks[cursor]`. Returns the number of chunks consumed
/// (`0` if `chunks[cursor]` does not match the start of `field`).
fn try_match_field_at(
    resolved: &ResolvedSchema,
    module_id: SchemaId,
    field: &ChunkFieldDef,
    chunks: &mut [DynamicChunk],
    cursor: usize,
    options: &LoadOptions,
    diagnostics: &mut Vec<SchemaDiagnostic>,
) -> usize {
    match &field.source {
        ChunkFieldSource::Id { id, payload } => {
            if chunks[cursor].id != *id {
                return 0;
            }
            apply_payload_to_chunk(
                resolved,
                module_id,
                &field.name,
                payload,
                field.wire_format,
                &mut chunks[cursor],
                options,
                diagnostics,
            );
            1
        }
        ChunkFieldSource::Stream { item_type } => try_match_stream_item_at(
            resolved,
            module_id,
            &field.name,
            item_type,
            chunks,
            cursor,
            options,
            diagnostics,
        ),
    }
}

#[allow(clippy::too_many_arguments)]
fn try_match_stream_item_at(
    resolved: &ResolvedSchema,
    from_mod: SchemaId,
    field_name: &str,
    item_type: &str,
    chunks: &mut [DynamicChunk],
    cursor: usize,
    options: &LoadOptions,
    diagnostics: &mut Vec<SchemaDiagnostic>,
) -> usize {
    let Some((target_mod, target_name, typedef)) = resolved.resolve_type(from_mod, item_type)
    else {
        return 0;
    };

    match typedef {
        TypeDef::ChunkEnum(enum_def) => {
            let item_disp = resolved.format_type_display(target_mod, target_name);
            for variant in &enum_def.variants {
                let label = if field_name.is_empty() {
                    variant.name.clone()
                } else {
                    format!("{field_name} ({item_disp}::{})", variant.name)
                };
                match &variant.source {
                    ChunkFieldSource::Id { id, payload } => {
                        if chunks[cursor].id == *id {
                            apply_payload_to_chunk(
                                resolved,
                                target_mod,
                                &label,
                                payload,
                                variant.wire_format,
                                &mut chunks[cursor],
                                options,
                                diagnostics,
                            );
                            return 1;
                        }
                    }
                    ChunkFieldSource::Stream {
                        item_type: nested_item,
                    } => {
                        let c = try_match_stream_item_at(
                            resolved,
                            target_mod,
                            &label,
                            nested_item,
                            chunks,
                            cursor,
                            options,
                            diagnostics,
                        );
                        if c > 0 {
                            return c;
                        }
                    }
                }
            }
            0
        }
        TypeDef::ChunkStreamSeq(seq) => try_match_stream_seq_fields(
            resolved,
            target_mod,
            field_name,
            item_type,
            &seq.fields,
            chunks,
            cursor,
            options,
            diagnostics,
        ),
        TypeDef::Chunks(seq_def) => {
            if seq_def.mode != OrderMode::Sequence {
                push_chunk_err(
                    &mut chunks[cursor],
                    diagnostics,
                    format!(
                        "Chunks type `{item_type}` used in stream must use `mode: sequence`, not `mode: map`"
                    ),
                );
                return 0;
            }
            try_match_stream_seq_fields(
                resolved,
                target_mod,
                field_name,
                item_type,
                &seq_def.fields,
                chunks,
                cursor,
                options,
                diagnostics,
            )
        }
        _ => 0,
    }
}

#[allow(clippy::too_many_arguments)]
fn try_match_stream_seq_fields(
    resolved: &ResolvedSchema,
    target_mod: SchemaId,
    field_name: &str,
    item_type: &str,
    fields: &[ChunkFieldDef],
    chunks: &mut [DynamicChunk],
    cursor: usize,
    options: &LoadOptions,
    diagnostics: &mut Vec<SchemaDiagnostic>,
) -> usize {
    let Some(first_field) = fields.first() else {
        return 0;
    };
    let mut visited = BTreeSet::new();
    if !matches_field_start(resolved, target_mod, first_field, chunks[cursor].id, &mut visited) {
        return 0;
    }

    let mut pos = cursor;
    for sub_field in fields {
        let sub_label = if field_name.is_empty() {
            sub_field.name.clone()
        } else {
            format!("{field_name}.{}", sub_field.name)
        };
        let mut sub_clone = sub_field.clone();
        sub_clone.name = sub_label;
        let mut count = 0usize;
        while pos < chunks.len() {
            let c = try_match_field_at(
                resolved,
                target_mod,
                &sub_clone,
                chunks,
                pos,
                options,
                diagnostics,
            );
            if c == 0 {
                break;
            }
            pos += c;
            count += 1;
            if !matches!(sub_field.cardinality, Cardinality::Repeated) {
                break;
            }
        }
        if matches!(sub_field.cardinality, Cardinality::Required) && count == 0 {
            diagnostics.push(SchemaDiagnostic {
                tree_path: chunks[cursor].tree_path.clone(),
                file_offset: chunks[cursor].header_offset,
                message: format!(
                    "Incomplete stream sequence `{item_type}`: missing required field `{}`",
                    sub_field.name
                ),
            });
            break;
        }
    }
    (pos - cursor).max(1)
}

fn matches_field_start(
    resolved: &ResolvedSchema,
    from_mod: SchemaId,
    field: &ChunkFieldDef,
    chunk_id: u32,
    visited: &mut BTreeSet<(SchemaId, String)>,
) -> bool {
    match &field.source {
        ChunkFieldSource::Id { id, .. } => *id == chunk_id,
        ChunkFieldSource::Stream { item_type } => {
            matches_stream_item_start(resolved, from_mod, item_type, chunk_id, visited)
        }
    }
}

fn matches_stream_item_start(
    resolved: &ResolvedSchema,
    from_mod: SchemaId,
    item_type: &str,
    chunk_id: u32,
    visited: &mut BTreeSet<(SchemaId, String)>,
) -> bool {
    let Some((target_mod, target_name, typedef)) = resolved.resolve_type(from_mod, item_type)
    else {
        return false;
    };
    let key = (target_mod, target_name.to_owned());
    if !visited.insert(key.clone()) {
        return false;
    }
    let matched = match typedef {
        TypeDef::ChunkEnum(e) => e.variants.iter().any(|v| match &v.source {
            ChunkFieldSource::Id { id, .. } => *id == chunk_id,
            ChunkFieldSource::Stream { item_type: nested } => {
                matches_stream_item_start(resolved, target_mod, nested, chunk_id, visited)
            }
        }),
        TypeDef::ChunkStreamSeq(seq) => seq
            .fields
            .first()
            .is_some_and(|f| matches_field_start(resolved, target_mod, f, chunk_id, visited)),
        TypeDef::Chunks(c) if c.mode == OrderMode::Sequence => c
            .fields
            .first()
            .is_some_and(|f| matches_field_start(resolved, target_mod, f, chunk_id, visited)),
        _ => false,
    };
    visited.remove(&key);
    matched
}

#[allow(clippy::too_many_arguments)]
fn apply_payload_to_chunk(
    resolved: &ResolvedSchema,
    owner_mod: SchemaId,
    field_name: &str,
    payload: &ChunkPayloadSpec,
    wire_format: Option<WireFormat>,
    chunk: &mut DynamicChunk,
    options: &LoadOptions,
    diagnostics: &mut Vec<SchemaDiagnostic>,
) {
    chunk.schema_field_name = Some(field_name.to_owned());
    chunk.field_owner_module_id = Some(owner_mod);

    match payload {
        ChunkPayloadSpec::Ignore => {
            chunk.schema_type_name = Some("ignored".to_owned());
            chunk.schema_module_id = Some(owner_mod);
            chunk.schema_type_ref = None;
        }
        ChunkPayloadSpec::Children(type_ref) => {
            let Some((target_mod, target_name, TypeDef::Chunks(child_container))) =
                resolved.resolve_type(owner_mod, type_ref)
            else {
                chunk.schema_type_name = Some(type_ref.clone());
                push_chunk_err(
                    chunk,
                    diagnostics,
                    format!("Unknown `chunks` type `{type_ref}` in schema"),
                );
                return;
            };
            chunk.schema_type_name = Some(resolved.format_type_display(target_mod, target_name));
            chunk.schema_module_id = Some(target_mod);
            chunk.schema_type_ref = Some(target_name.to_owned());

            let tree_path = chunk.tree_path.clone();
            let offset = chunk.payload_offset;
            match &mut chunk.content {
                DynamicChunkContent::Children(children) => {
                    evaluate_chunk_container(
                        resolved,
                        target_mod,
                        child_container,
                        children,
                        &tree_path,
                        offset,
                        options,
                        diagnostics,
                    );
                }
                _ => {
                    push_chunk_err(
                        chunk,
                        diagnostics,
                        format!(
                            "Expected container chunk (children `{type_ref}`), but chunk 0x{:08x} is a leaf data chunk",
                            chunk.id
                        ),
                    );
                }
            }
        }
        ChunkPayloadSpec::MiniChunks(type_ref) => {
            let Some((target_mod, target_name, TypeDef::MiniChunks(mc_container))) =
                resolved.resolve_type(owner_mod, type_ref)
            else {
                chunk.schema_type_name = Some(type_ref.clone());
                push_chunk_err(
                    chunk,
                    diagnostics,
                    format!("Unknown `mini_chunks` type `{type_ref}` in schema"),
                );
                return;
            };
            chunk.schema_type_name = Some(resolved.format_type_display(target_mod, target_name));
            chunk.schema_module_id = Some(target_mod);
            chunk.schema_type_ref = Some(target_name.to_owned());

            if chunk.is_container {
                push_chunk_err(
                    chunk,
                    diagnostics,
                    format!(
                        "Expected leaf mini-chunks (`{type_ref}`), but chunk 0x{:08x} is a container chunk",
                        chunk.id
                    ),
                );
                return;
            }
            if options.force_plain_data_paths.contains(&chunk.tree_path) {
                return;
            }
            if let DynamicChunkContent::Data { raw_bytes, .. } = &chunk.content {
                let mcs = parse_raw_mini_chunks(raw_bytes, chunk.payload_offset, &chunk.tree_path);
                chunk.content = DynamicChunkContent::MiniChunks {
                    raw_bytes: raw_bytes.clone(),
                    mini_chunks: mcs,
                };
            }
            let tree_path = chunk.tree_path.clone();
            let offset = chunk.payload_offset;
            if let DynamicChunkContent::MiniChunks { mini_chunks, .. } = &mut chunk.content {
                evaluate_mini_chunk_container(
                    resolved,
                    target_mod,
                    mc_container,
                    mini_chunks,
                    &tree_path,
                    offset,
                    options,
                    diagnostics,
                );
            }
        }
        ChunkPayloadSpec::Data(data_type) => {
            if let Some((target_mod, target_name, _)) =
                resolved.resolve_data_type_target(owner_mod, data_type)
            {
                chunk.schema_module_id = Some(target_mod);
                chunk.schema_type_ref = Some(target_name.to_owned());
            } else {
                chunk.schema_module_id = Some(owner_mod);
                chunk.schema_type_ref = None;
            }
            chunk.schema_type_name = Some(data_type.label());

            if chunk.is_container {
                push_chunk_err(
                    chunk,
                    diagnostics,
                    format!(
                        "Expected leaf data chunk (`{}`), but chunk 0x{:08x} is a container chunk",
                        data_type.label(),
                        chunk.id
                    ),
                );
                return;
            }
            let raw_bytes = match &chunk.content {
                DynamicChunkContent::Data { raw_bytes, .. }
                | DynamicChunkContent::MiniChunks { raw_bytes, .. } => raw_bytes.clone(),
                DynamicChunkContent::Children(_) => return,
            };

            let mut spans = Vec::new();
            let mut color_counter = 0usize;
            let mut ctx = DecodeCtx {
                resolved,
                module_id: owner_mod,
                spans: &mut spans,
                color_counter: &mut color_counter,
                options,
            };
            match decode_data_type_internal(
                &mut ctx,
                data_type,
                wire_format,
                &raw_bytes,
                0,
                field_name,
            ) {
                Ok((val, consumed)) => {
                    if consumed < raw_bytes.len() {
                        push_chunk_err(
                            chunk,
                            diagnostics,
                            format!(
                                "Unused trailing bytes in chunk 0x{:08x} (`{field_name}`): consumed {consumed} of {} bytes",
                                chunk.id,
                                raw_bytes.len()
                            ),
                        );
                    }
                    chunk.content = DynamicChunkContent::Data {
                        raw_bytes,
                        decoded: Some(val),
                        spans,
                    };
                }
                Err(e) => {
                    push_chunk_err(
                        chunk,
                        diagnostics,
                        format!(
                            "Error decoding chunk 0x{:08x} (`{field_name}`) as `{}`: {e}",
                            chunk.id,
                            data_type.label()
                        ),
                    );
                    chunk.content = DynamicChunkContent::Data {
                        raw_bytes,
                        decoded: None,
                        spans,
                    };
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn evaluate_mini_chunk_container(
    resolved: &ResolvedSchema,
    module_id: SchemaId,
    container: &MiniChunkContainerDef,
    mini_chunks: &mut [DynamicMiniChunk],
    container_path: &str,
    container_offset: usize,
    options: &LoadOptions,
    diagnostics: &mut Vec<SchemaDiagnostic>,
) {
    match container.mode {
        OrderMode::Sequence => {
            let mut cursor = 0usize;
            for field in &container.fields {
                let mut count = 0usize;
                while cursor < mini_chunks.len() && mini_chunks[cursor].id == field.id {
                    apply_mini_chunk_field(
                        resolved,
                        module_id,
                        &field.name,
                        &field.ty,
                        field.wire_format,
                        &mut mini_chunks[cursor],
                        options,
                        diagnostics,
                    );
                    cursor += 1;
                    count += 1;
                    if !matches!(field.cardinality, Cardinality::Repeated) {
                        break;
                    }
                }
                if matches!(field.cardinality, Cardinality::Required) && count == 0 {
                    diagnostics.push(SchemaDiagnostic {
                        tree_path: container_path.to_owned(),
                        file_offset: container_offset,
                        message: format!(
                            "Missing required mini-chunk 0x{:02x} (`{}`) in sequence",
                            field.id, field.name
                        ),
                    });
                }
            }
            if !container.ignore_unknown {
                for leftover in &mut mini_chunks[cursor..] {
                    let msg = format!("Unexpected mini-chunk 0x{:02x} after sequence", leftover.id);
                    leftover.errors.push(msg.clone());
                    diagnostics.push(SchemaDiagnostic {
                        tree_path: leftover.tree_path.clone(),
                        file_offset: leftover.header_offset,
                        message: msg,
                    });
                }
            }
        }
        OrderMode::Map => {
            let mut counts = vec![0usize; container.fields.len()];
            for mc in mini_chunks.iter_mut() {
                if let Some((fidx, field)) = container
                    .fields
                    .iter()
                    .enumerate()
                    .find(|(_, f)| f.id == mc.id)
                {
                    counts[fidx] += 1;
                    if counts[fidx] > 1 && !matches!(field.cardinality, Cardinality::Repeated) {
                        let msg = format!(
                            "Duplicate mini-chunk 0x{:02x} for non-repeated field `{}`",
                            mc.id, field.name
                        );
                        mc.errors.push(msg.clone());
                        diagnostics.push(SchemaDiagnostic {
                            tree_path: mc.tree_path.clone(),
                            file_offset: mc.header_offset,
                            message: msg,
                        });
                    }
                    apply_mini_chunk_field(
                        resolved,
                        module_id,
                        &field.name,
                        &field.ty,
                        field.wire_format,
                        mc,
                        options,
                        diagnostics,
                    );
                } else if !container.ignore_unknown {
                    let msg = format!("Unexpected mini-chunk 0x{:02x} in map", mc.id);
                    mc.errors.push(msg.clone());
                    diagnostics.push(SchemaDiagnostic {
                        tree_path: mc.tree_path.clone(),
                        file_offset: mc.header_offset,
                        message: msg,
                    });
                }
            }
            for (fidx, field) in container.fields.iter().enumerate() {
                if matches!(field.cardinality, Cardinality::Required) && counts[fidx] == 0 {
                    diagnostics.push(SchemaDiagnostic {
                        tree_path: container_path.to_owned(),
                        file_offset: container_offset,
                        message: format!(
                            "Missing required mini-chunk 0x{:02x} (`{}`) in map",
                            field.id, field.name
                        ),
                    });
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn apply_mini_chunk_field(
    resolved: &ResolvedSchema,
    owner_mod: SchemaId,
    field_name: &str,
    ty: &DataType,
    wire_format: Option<WireFormat>,
    mc: &mut DynamicMiniChunk,
    options: &LoadOptions,
    diagnostics: &mut Vec<SchemaDiagnostic>,
) {
    mc.schema_field_name = Some(field_name.to_owned());
    mc.schema_type_label = Some(ty.label());
    mc.field_owner_module_id = Some(owner_mod);
    if let Some((target_mod, target_name, _)) = resolved.resolve_data_type_target(owner_mod, ty) {
        mc.schema_module_id = Some(target_mod);
        mc.schema_type_ref = Some(target_name.to_owned());
    } else {
        mc.schema_module_id = Some(owner_mod);
        mc.schema_type_ref = None;
    }

    let mut spans = Vec::new();
    let mut color_counter = 0usize;
    let mut ctx = DecodeCtx {
        resolved,
        module_id: owner_mod,
        spans: &mut spans,
        color_counter: &mut color_counter,
        options,
    };
    match decode_data_type_internal(&mut ctx, ty, wire_format, &mc.raw_bytes, 0, field_name) {
        Ok((val, consumed)) => {
            if consumed < mc.raw_bytes.len() {
                let msg = format!(
                    "Unused trailing bytes in mini-chunk 0x{:02x} (`{field_name}`): consumed {consumed} of {} bytes",
                    mc.id,
                    mc.raw_bytes.len()
                );
                mc.errors.push(msg.clone());
                diagnostics.push(SchemaDiagnostic {
                    tree_path: mc.tree_path.clone(),
                    file_offset: mc.header_offset,
                    message: msg,
                });
            }
            mc.decoded = Some(val);
            mc.spans = spans;
        }
        Err(e) => {
            let msg = format!(
                "Error decoding mini-chunk 0x{:02x} (`{field_name}`) as `{}`: {e}",
                mc.id,
                ty.label()
            );
            mc.errors.push(msg.clone());
            diagnostics.push(SchemaDiagnostic {
                tree_path: mc.tree_path.clone(),
                file_offset: mc.header_offset,
                message: msg,
            });
            mc.spans = spans;
        }
    }
}

/// Decodes `ty` starting at `bytes[offset..]`, recording [`ByteSpanAnnotation`]s into `spans` and
/// returning `(DynamicValue, new_offset)`.
pub fn decode_data_type_with_spans(
    schema: &ResolvedSchema,
    ty: &DataType,
    bytes: &[u8],
    offset: usize,
    field_path: &str,
    spans: &mut Vec<ByteSpanAnnotation>,
    color_counter: &mut usize,
) -> Result<(DynamicValue, usize), String> {
    let options = LoadOptions::default();
    let mut ctx = DecodeCtx {
        resolved: schema,
        module_id: SchemaId::ROOT,
        spans,
        color_counter,
        options: &options,
    };
    decode_data_type_internal(&mut ctx, ty, None, bytes, offset, field_path)
}

fn decode_data_type_internal(
    ctx: &mut DecodeCtx<'_>,
    ty: &DataType,
    wire_format: Option<WireFormat>,
    bytes: &[u8],
    offset: usize,
    field_path: &str,
) -> Result<(DynamicValue, usize), String> {
    let mut slice = bytes
        .get(offset..)
        .ok_or_else(|| format!("Offset {offset} out of bounds (len {})", bytes.len()))?;

    let max_spans = ctx.options.max_spans;
    let mut record_leaf = |len: usize, label: &str, val: DynamicValue| -> (DynamicValue, usize) {
        if ctx.spans.len() < max_spans {
            let color_index = *ctx.color_counter;
            *ctx.color_counter += 1;
            ctx.spans.push(ByteSpanAnnotation {
                start: offset,
                end: offset + len,
                field_path: field_path.to_owned(),
                type_label: label.to_owned(),
                value_summary: val.short_summary(),
                color_index,
                is_padding: false,
            });
        }
        (val, offset + len)
    };

    match ty {
        DataType::U8 => {
            let v: u8 = slice.take().map_err(|e| e.to_string())?;
            Ok(record_leaf(1, "u8", DynamicValue::U8(v)))
        }
        DataType::I8 => {
            let v: i8 = slice.take().map_err(|e| e.to_string())?;
            Ok(record_leaf(1, "i8", DynamicValue::I8(v)))
        }
        DataType::U16 => {
            let v: u16 = slice.take().map_err(|e| e.to_string())?;
            Ok(record_leaf(2, "u16", DynamicValue::U16(v)))
        }
        DataType::I16 => {
            let v: i16 = slice.take().map_err(|e| e.to_string())?;
            Ok(record_leaf(2, "i16", DynamicValue::I16(v)))
        }
        DataType::U32 => {
            let v: u32 = slice.take().map_err(|e| e.to_string())?;
            Ok(record_leaf(4, "u32", DynamicValue::U32(v)))
        }
        DataType::I32 => {
            let v: i32 = slice.take().map_err(|e| e.to_string())?;
            Ok(record_leaf(4, "i32", DynamicValue::I32(v)))
        }
        DataType::U64 => {
            let v: u64 = slice.take().map_err(|e| e.to_string())?;
            Ok(record_leaf(8, "u64", DynamicValue::U64(v)))
        }
        DataType::I64 => {
            let v: i64 = slice.take().map_err(|e| e.to_string())?;
            Ok(record_leaf(8, "i64", DynamicValue::I64(v)))
        }
        DataType::F32 => {
            let v: f32 = slice.take().map_err(|e| e.to_string())?;
            Ok(record_leaf(4, "f32", DynamicValue::F32(v)))
        }
        DataType::F64 => {
            let v: f64 = slice.take().map_err(|e| e.to_string())?;
            Ok(record_leaf(8, "f64", DynamicValue::F64(v)))
        }
        DataType::Bool => {
            let wf = wire_format.unwrap_or(WireFormat::U32Bool);
            match wf {
                WireFormat::U8Bool => {
                    let raw: u8 = slice.take().map_err(|e| e.to_string())?;
                    let b = match raw {
                        0 => false,
                        1 => true,
                        _ => return Err(format!("Expected U8Bool (0 or 1), found {raw}")),
                    };
                    Ok(record_leaf(1, "bool (U8Bool)", DynamicValue::Bool(b)))
                }
                _ => {
                    let raw: u32 = slice.take().map_err(|e| e.to_string())?;
                    let b = match raw {
                        0 => false,
                        1 => true,
                        _ => return Err(format!("Expected U32Bool (0 or 1), found {raw}")),
                    };
                    Ok(record_leaf(4, "bool (U32Bool)", DynamicValue::Bool(b)))
                }
            }
        }
        DataType::Usize => {
            let raw: u32 = slice.take().map_err(|e| e.to_string())?;
            Ok(record_leaf(
                4,
                "usize (U32Usize)",
                DynamicValue::Usize(raw as usize),
            ))
        }
        DataType::CString | DataType::Path => {
            let cstr = CStr::from_bytes_until_nul(slice).map_err(|e| e.to_string())?;
            let byte_len = cstr.to_bytes_with_nul().len();
            let s = cstr.to_str().map_err(|e| e.to_string())?.to_owned();
            let label = if matches!(ty, DataType::Path) {
                "Path"
            } else {
                "CString"
            };
            Ok(record_leaf(byte_len, label, DynamicValue::String(s)))
        }
        DataType::Vec2 => {
            let v: [f32; 2] = slice.take().map_err(|e| e.to_string())?;
            Ok(record_leaf(
                8,
                "Vec2",
                DynamicValue::Array(v.into_iter().map(DynamicValue::F32).collect()),
            ))
        }
        DataType::Vec3 => {
            let v: [f32; 3] = slice.take().map_err(|e| e.to_string())?;
            Ok(record_leaf(
                12,
                "Vec3",
                DynamicValue::Array(v.into_iter().map(DynamicValue::F32).collect()),
            ))
        }
        DataType::Vec4 => {
            let v: [f32; 4] = slice.take().map_err(|e| e.to_string())?;
            Ok(record_leaf(
                16,
                "Vec4",
                DynamicValue::Array(v.into_iter().map(DynamicValue::F32).collect()),
            ))
        }
        DataType::Mat3x4 => {
            let rows: [[f32; 4]; 3] = slice.take().map_err(|e| e.to_string())?;
            let val = DynamicValue::Array(
                rows.into_iter()
                    .map(|r| DynamicValue::Array(r.into_iter().map(DynamicValue::F32).collect()))
                    .collect(),
            );
            Ok(record_leaf(48, "Mat3x4", val))
        }
        DataType::Array { elem, len } => {
            let mut items = Vec::with_capacity(*len);
            let mut cur = offset;
            for i in 0..*len {
                let sub_path = format!("{field_path}[{i}]");
                let (v, next) = decode_data_type_internal(ctx, elem, None, bytes, cur, &sub_path)?;
                items.push(v);
                cur = next;
            }
            Ok((DynamicValue::Array(items), cur))
        }
        DataType::List(elem) => {
            let mut items = Vec::new();
            let mut cur = offset;
            let mut i = 0usize;
            let fast_elem_size = if ctx.options.fast_skip_large_lists {
                fixed_unvalidated_size(ctx.resolved, ctx.module_id, elem)
            } else {
                None
            };
            while cur < bytes.len() {
                if ctx.options.fast_skip_large_lists
                    && i >= 16
                    && let Some(elem_size) = fast_elem_size
                    && elem_size > 0
                {
                    let rem_bytes = bytes.len() - cur;
                    if rem_bytes.is_multiple_of(elem_size) {
                        let rem_count = rem_bytes / elem_size;
                        let total_count = i + rem_count;
                        if ctx.spans.len() < max_spans {
                            let color_index = *ctx.color_counter;
                            *ctx.color_counter += 1;
                            ctx.spans.push(ByteSpanAnnotation {
                                start: cur,
                                end: bytes.len(),
                                field_path: format!("{field_path}[{i}..{total_count}]"),
                                type_label: format!("{} ({rem_count} items)", elem.label()),
                                value_summary: format!(
                                    "{rem_count} more items ({total_count} total)"
                                ),
                                color_index,
                                is_padding: false,
                            });
                        }
                        cur = bytes.len();
                        break;
                    }
                }
                let sub_path = format!("{field_path}[{i}]");
                let (v, next) = decode_data_type_internal(ctx, elem, None, bytes, cur, &sub_path)?;
                if next == cur {
                    break;
                }
                items.push(v);
                cur = next;
                i += 1;
            }
            Ok((DynamicValue::Array(items), cur))
        }
        DataType::Tuple(elems) => {
            let mut items = Vec::with_capacity(elems.len());
            let mut cur = offset;
            for (i, elem) in elems.iter().enumerate() {
                let sub_path = format!("{field_path}.{i}");
                let (v, next) = decode_data_type_internal(ctx, elem, None, bytes, cur, &sub_path)?;
                items.push(v);
                cur = next;
            }
            Ok((DynamicValue::Tuple(items), cur))
        }
        DataType::RawBytes => {
            let rem = slice.to_vec();
            let len = rem.len();
            Ok(record_leaf(len, "RawBytes", DynamicValue::Bytes(rem)))
        }
        DataType::Named(type_ref) => {
            let Some((target_mod, target_name, TypeDef::ByteBlock(bb))) =
                ctx.resolved.resolve_type(ctx.module_id, type_ref)
            else {
                return Err(format!("Unknown byte_block type `{type_ref}`"));
            };
            let bb = bb.clone();
            let target_name = target_name.to_owned();
            let mut sub_ctx = ctx.in_module(target_mod);
            decode_byte_block_def(&mut sub_ctx, &target_name, &bb, bytes, offset, field_path)
        }
    }
}

fn decode_byte_block_def(
    ctx: &mut DecodeCtx<'_>,
    type_name: &str,
    bb: &ByteBlockDef,
    bytes: &[u8],
    offset: usize,
    parent_path: &str,
) -> Result<(DynamicValue, usize), String> {
    let mut cur = offset;
    let mut fields = Vec::with_capacity(bb.fields.len());

    for f in &bb.fields {
        let sub_path = if parent_path.is_empty() {
            f.name.clone()
        } else {
            format!("{parent_path}.{}", f.name)
        };
        let (val, next) =
            decode_data_type_internal(ctx, &f.ty, f.wire_format, bytes, cur, &sub_path)?;
        cur = next;
        fields.push((f.name.clone(), val));

        if let Some(pad) = f.pad_after
            && pad > 0
        {
            if cur + pad > bytes.len() {
                return Err(format!(
                    "Not enough bytes for pad_after = {pad} after field `{}`",
                    f.name
                ));
            }
            if ctx.spans.len() < ctx.options.max_spans {
                ctx.spans.push(ByteSpanAnnotation {
                    start: cur,
                    end: cur + pad,
                    field_path: format!("{sub_path}.<pad_after>"),
                    type_label: format!("pad ({pad}B)"),
                    value_summary: format!("{pad} padding bytes"),
                    color_index: 0,
                    is_padding: true,
                });
            }
            cur += pad;
        }
    }

    if let Some(pad) = bb.pad_after
        && pad > 0
    {
        if cur + pad > bytes.len() {
            return Err(format!(
                "Not enough bytes for container pad_after = {pad} on `{type_name}`"
            ));
        }
        if ctx.spans.len() < ctx.options.max_spans {
            ctx.spans.push(ByteSpanAnnotation {
                start: cur,
                end: cur + pad,
                field_path: format!("{type_name}.<pad_after>"),
                type_label: format!("pad ({pad}B)"),
                value_summary: format!("{pad} padding bytes"),
                color_index: 0,
                is_padding: true,
            });
        }
        cur += pad;
    }

    Ok((
        DynamicValue::Struct {
            type_name: Some(type_name.to_owned()),
            fields,
        },
        cur,
    ))
}

/// If `ty` has a fixed byte size and requires no per-value runtime validation (i.e. contains no
/// booleans or null-terminated strings), returns `Some(size_in_bytes)`.
fn fixed_unvalidated_size(
    resolved: &ResolvedSchema,
    module_id: SchemaId,
    ty: &DataType,
) -> Option<usize> {
    let mut visited = BTreeSet::new();
    fixed_unvalidated_size_inner(resolved, module_id, ty, &mut visited)
}

fn fixed_unvalidated_size_inner(
    resolved: &ResolvedSchema,
    module_id: SchemaId,
    ty: &DataType,
    visited: &mut BTreeSet<(SchemaId, String)>,
) -> Option<usize> {
    match ty {
        DataType::U8 | DataType::I8 => Some(1),
        DataType::U16 | DataType::I16 => Some(2),
        DataType::U32 | DataType::I32 | DataType::F32 | DataType::Usize => Some(4),
        DataType::U64 | DataType::I64 | DataType::F64 | DataType::Vec2 => Some(8),
        DataType::Vec3 => Some(12),
        DataType::Vec4 => Some(16),
        DataType::Mat3x4 => Some(48),
        DataType::Array { elem, len } => {
            fixed_unvalidated_size_inner(resolved, module_id, elem, visited).map(|s| s * len)
        }
        DataType::Tuple(items) => {
            let mut total = 0usize;
            for item in items {
                total += fixed_unvalidated_size_inner(resolved, module_id, item, visited)?;
            }
            Some(total)
        }
        DataType::Named(name) => {
            let Some((target_mod, target_name, TypeDef::ByteBlock(bb))) =
                resolved.resolve_type(module_id, name)
            else {
                return None;
            };
            let key = (target_mod, target_name.to_owned());
            if !visited.insert(key.clone()) {
                return None;
            }
            let mut total = bb.pad_after.unwrap_or(0);
            for f in &bb.fields {
                total += fixed_unvalidated_size_inner(resolved, target_mod, &f.ty, visited)?;
                total += f.pad_after.unwrap_or(0);
            }
            visited.remove(&key);
            Some(total)
        }
        DataType::Bool
        | DataType::CString
        | DataType::Path
        | DataType::List(_)
        | DataType::RawBytes => None,
    }
}
