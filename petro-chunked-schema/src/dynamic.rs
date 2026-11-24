//! Dynamic value representation, byte-span annotations, multi-format byte inspection, and schema
//! evaluation for Petroglyph chunked binary files.
//!
//! This module provides a dynamic AST (analogous to `serde_json::Value` for chunked files) that can:
//! 1. Parse any raw chunked binary buffer without a schema (auto-detecting container chunks vs. leaf
//!    mini-chunk blocks vs. plain byte blocks, with support for forcing specific chunk paths to be
//!    treated as plain byte data).
//! 2. Evaluate a [`Schema`] against a raw chunked binary buffer, annotating every matched chunk,
//!    mini-chunk, and byte-block field with its schema field name, type name, decoded [`DynamicValue`],
//!    color-coded [`ByteSpanAnnotation`]s (for highlighting assigned vs. unassigned bytes in a hex
//!    editor), and collecting [`SchemaDiagnostic`]s for any mismatches.

use std::collections::BTreeSet;
use std::ffi::CStr;

use petro_chunked::byteblock::TakeBytes;
use petro_chunked::{ChunkContent, iter_chunks, iter_mini_chunks};
use serde::{Deserialize, Serialize};

use crate::schema::{
    ByteBlockDef, Cardinality, ChunkContainerDef, ChunkFieldDef, ChunkFieldSource,
    ChunkPayloadSpec, DataType, MiniChunkContainerDef, OrderMode, Schema, TypeDef, WireFormat,
};

/// A dynamically decoded value from a byte block, mini-chunk, or chunk.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DynamicValue {
    U8(u8),
    I8(i8),
    U16(u16),
    I16(i16),
    U32(u32),
    I32(i32),
    U64(u64),
    I64(i64),
    F32(f32),
    F64(f64),
    Bool(bool),
    Usize(usize),
    String(String),
    Array(Vec<DynamicValue>),
    Tuple(Vec<DynamicValue>),
    Struct {
        type_name: Option<String>,
        fields: Vec<(String, DynamicValue)>,
    },
    Bytes(Vec<u8>),
}

impl DynamicValue {
    /// Produces a compact single-line summary string for UI display.
    pub fn short_summary(&self) -> String {
        match self {
            Self::U8(v) => format!("{v} (0x{v:02x})"),
            Self::I8(v) => format!("{v}"),
            Self::U16(v) => format!("{v} (0x{v:04x})"),
            Self::I16(v) => format!("{v}"),
            Self::U32(v) => format!("{v} (0x{v:08x})"),
            Self::I32(v) => format!("{v}"),
            Self::U64(v) => format!("{v} (0x{v:016x})"),
            Self::I64(v) => format!("{v}"),
            Self::F32(v) => format!("{v:.4}"),
            Self::F64(v) => format!("{v:.4}"),
            Self::Bool(v) => format!("{v}"),
            Self::Usize(v) => format!("{v}"),
            Self::String(s) => format!("{s:?}"),
            Self::Array(items) => {
                if items.len() <= 4 {
                    let parts: Vec<String> = items.iter().map(|i| i.short_summary()).collect();
                    format!("[{}]", parts.join(", "))
                } else {
                    format!("[...; {} items]", items.len())
                }
            }
            Self::Tuple(items) => {
                let parts: Vec<String> = items.iter().map(|i| i.short_summary()).collect();
                format!("({})", parts.join(", "))
            }
            Self::Struct { type_name, fields } => {
                let prefix = type_name.as_deref().unwrap_or("struct");
                if fields.len() <= 3 {
                    let parts: Vec<String> = fields
                        .iter()
                        .map(|(k, v)| format!("{k}: {}", v.short_summary()))
                        .collect();
                    format!("{prefix} {{ {} }}", parts.join(", "))
                } else {
                    format!("{prefix} {{ ... ({} fields) }}", fields.len())
                }
            }
            Self::Bytes(b) => format!("<{} bytes>", b.len()),
        }
    }
}

/// Annotation for a contiguous range of bytes within a chunk or mini-chunk's data payload that has
/// been assigned to a structural field in the schema.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ByteSpanAnnotation {
    /// Start byte offset relative to the chunk or mini-chunk payload buffer.
    pub start: usize,
    /// End byte offset (exclusive) relative to the chunk or mini-chunk payload buffer.
    pub end: usize,
    /// Dotted/indexed field path (e.g. `"submesh_count"`, `"bounds_min"`, `"<pad_after>"`).
    pub field_path: String,
    /// Human-readable type label (e.g. `"usize (U32Usize)"`, `"Vec3"`).
    pub type_label: String,
    /// Human-readable decoded value summary.
    pub value_summary: String,
    /// Palette index used by the hex editor to color-code this span.
    pub color_index: usize,
    /// True if this span represents padding bytes rather than a named field.
    pub is_padding: bool,
}

/// A validation or decoding diagnostic produced while evaluating a [`Schema`] against a file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SchemaDiagnostic {
    /// Hierarchical tree path of the chunk or mini-chunk where the issue occurred (e.g. `"root/0[0x0400]/2[0x10000]"`).
    pub tree_path: String,
    /// Absolute byte offset in the file where the issue occurred.
    pub file_offset: usize,
    /// Human-readable diagnostic message.
    pub message: String,
}

/// A dynamically parsed 8-bit mini-chunk inside a leaf data chunk.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DynamicMiniChunk {
    /// Tree path identifier for UI selection (e.g. `"0/1/mc0"`).
    pub tree_path: String,
    /// 8-bit mini-chunk ID.
    pub id: u8,
    /// Absolute file byte offset of the 2-byte mini-chunk header.
    pub header_offset: usize,
    /// Absolute file byte offset of the mini-chunk payload.
    pub payload_offset: usize,
    /// Raw payload bytes of this mini-chunk.
    pub raw_bytes: Vec<u8>,
    /// Schema field name assigned to this mini-chunk, if matched.
    pub schema_field_name: Option<String>,
    /// Schema type label assigned to this mini-chunk, if matched.
    pub schema_type_label: Option<String>,
    /// Decoded value of this mini-chunk according to the schema, if matched.
    pub decoded: Option<DynamicValue>,
    /// Byte span annotations within `raw_bytes`.
    pub spans: Vec<ByteSpanAnnotation>,
    /// Any validation/decoding errors specific to this mini-chunk.
    pub errors: Vec<String>,
}

/// Content of a [`DynamicChunk`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DynamicChunkContent {
    /// Container chunk (`0x8000_0000` flag bit set) holding child 32-bit chunks.
    Children(Vec<DynamicChunk>),
    /// Leaf data chunk interpreted as a sequence/map of 8-bit mini-chunks.
    MiniChunks {
        raw_bytes: Vec<u8>,
        mini_chunks: Vec<DynamicMiniChunk>,
    },
    /// Leaf data chunk interpreted as a raw byte block.
    Data {
        raw_bytes: Vec<u8>,
        decoded: Option<DynamicValue>,
        spans: Vec<ByteSpanAnnotation>,
    },
}

/// A dynamically parsed 32-bit chunk in a chunked binary file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DynamicChunk {
    /// Unique tree path for UI selection and `force_plain_data` overrides (e.g. `"0"`, `"0/1"`).
    pub tree_path: String,
    /// 32-bit chunk ID.
    pub id: u32,
    /// True if the chunk header's container bit (`0x8000_0000`) was set.
    pub is_container: bool,
    /// Absolute file byte offset of the 8-byte chunk header.
    pub header_offset: usize,
    /// Absolute file byte offset of the chunk payload.
    pub payload_offset: usize,
    /// Size of the chunk payload in bytes.
    pub size: usize,
    /// Schema field name matched to this chunk, if any.
    pub schema_field_name: Option<String>,
    /// Schema type or container name matched to this chunk, if any.
    pub schema_type_name: Option<String>,
    /// True if this leaf chunk's bytes could be parsed as mini-chunks (so the user can toggle
    /// forcing plain data vs. mini-chunks in the UI).
    pub looks_like_mini_chunks: bool,
    /// Parsed content of this chunk.
    pub content: DynamicChunkContent,
    /// Validation or decoding errors on this chunk.
    pub errors: Vec<String>,
}

/// Top-level result of loading a chunked binary file into dynamic values.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DynamicFile {
    /// Total file size in bytes.
    pub total_size: usize,
    /// Top-level 32-bit chunks in the file.
    pub chunks: Vec<DynamicChunk>,
    /// All schema validation/decoding diagnostics collected across the file.
    pub diagnostics: Vec<SchemaDiagnostic>,
}

/// Options controlling how a binary buffer is loaded into a [`DynamicFile`].
#[derive(Debug, Clone, Default)]
pub struct LoadOptions {
    /// Tree paths (e.g. `"0/2"`) of leaf chunks that should be forced to decode as plain byte data
    /// even if their payload looks like valid mini-chunks (or is marked as mini-chunks).
    pub force_plain_data_paths: BTreeSet<String>,
}

/// Heuristic check: returns true if `bytes` is non-empty and its entire length can be segmented
/// cleanly into 8-bit mini-chunks (`[id: u8, len: u8, payload: [u8; len]]*`).
pub fn looks_like_mini_chunks(bytes: &[u8]) -> bool {
    if bytes.len() < 2 {
        return false;
    }
    let mut pos = 0usize;
    let mut count = 0usize;
    while pos < bytes.len() {
        if pos + 2 > bytes.len() {
            return false;
        }
        let len = bytes[pos + 1] as usize;
        pos += 2;
        if pos + len > bytes.len() {
            return false;
        }
        pos += len;
        count += 1;
    }
    count > 0 && pos == bytes.len()
}

/// Loads a chunked binary file into a [`DynamicFile`], optionally evaluating and annotating it
/// against a [`Schema`].
pub fn load_dynamic_file(
    bytes: &[u8],
    schema: Option<&Schema>,
    options: &LoadOptions,
) -> Result<DynamicFile, String> {
    let mut diagnostics = Vec::new();
    let mut raw_chunks = parse_raw_chunk_list(bytes, 0, "", options, &mut diagnostics)?;

    if let Some(schema) = schema {
        evaluate_chunk_container(
            schema,
            &schema.root,
            &mut raw_chunks,
            "root",
            0,
            options,
            &mut diagnostics,
        );
    }

    Ok(DynamicFile {
        total_size: bytes.len(),
        chunks: raw_chunks,
        diagnostics,
    })
}

/// Validates `bytes` against `schema`, returning `Ok(DynamicFile)` if there are zero diagnostics or
/// `Err(Vec<SchemaDiagnostic>)` if any validation or decoding errors occurred.
pub fn validate_bytes(schema: &Schema, bytes: &[u8]) -> Result<DynamicFile, Vec<SchemaDiagnostic>> {
    match load_dynamic_file(bytes, Some(schema), &LoadOptions::default()) {
        Ok(file) if file.diagnostics.is_empty() => Ok(file),
        Ok(file) => Err(file.diagnostics),
        Err(fatal) => Err(vec![SchemaDiagnostic {
            tree_path: "root".to_owned(),
            file_offset: 0,
            message: fatal,
        }]),
    }
}

fn parse_raw_chunk_list(
    bytes: &[u8],
    base_offset: usize,
    parent_path: &str,
    options: &LoadOptions,
    diagnostics: &mut Vec<SchemaDiagnostic>,
) -> Result<Vec<DynamicChunk>, String> {
    let mut out = Vec::new();
    let mut cursor = 0usize;
    let mut idx = 0usize;

    let iter = iter_chunks(bytes);
    for chunk_res in iter {
        let header_offset = base_offset + cursor;
        let chunk = chunk_res.map_err(|e| format!("Failed to read chunk at offset {header_offset}: {e}"))?;
        let payload_offset = header_offset + 8;
        let tree_path = if parent_path.is_empty() {
            format!("{idx}")
        } else {
            format!("{parent_path}/{idx}")
        };

        let (is_container, size, content, looks_mc) = match chunk.content() {
            ChunkContent::Chunks(_) => {
                let child_len = compute_chunk_payload_len(bytes, cursor);
                let child_slice = &bytes[cursor + 8..cursor + 8 + child_len];
                let children = parse_raw_chunk_list(
                    child_slice,
                    payload_offset,
                    &tree_path,
                    options,
                    diagnostics,
                )?;
                (
                    true,
                    child_len,
                    DynamicChunkContent::Children(children),
                    false,
                )
            }
            ChunkContent::Data(data) => {
                let raw = data.as_data();
                let size = raw.len();
                let looks_mc = looks_like_mini_chunks(raw);
                let forced_plain = options.force_plain_data_paths.contains(&tree_path);

                let content = if looks_mc && !forced_plain {
                    let mcs = parse_raw_mini_chunks(raw, payload_offset, &tree_path);
                    DynamicChunkContent::MiniChunks {
                        raw_bytes: raw.to_vec(),
                        mini_chunks: mcs,
                    }
                } else {
                    DynamicChunkContent::Data {
                        raw_bytes: raw.to_vec(),
                        decoded: None,
                        spans: Vec::new(),
                    }
                };
                (false, size, content, looks_mc)
            }
        };

        out.push(DynamicChunk {
            tree_path,
            id: chunk.id(),
            is_container,
            header_offset,
            payload_offset,
            size,
            schema_field_name: None,
            schema_type_name: None,
            looks_like_mini_chunks: looks_mc,
            content,
            errors: Vec::new(),
        });

        cursor += 8 + size;
        idx += 1;
    }

    Ok(out)
}

fn compute_chunk_payload_len(parent_bytes: &[u8], cursor: usize) -> usize {
    if cursor + 8 > parent_bytes.len() {
        return 0;
    }
    let raw_size = u32::from_le_bytes([
        parent_bytes[cursor + 4],
        parent_bytes[cursor + 5],
        parent_bytes[cursor + 6],
        parent_bytes[cursor + 7],
    ]);
    (raw_size & 0x7fff_ffff) as usize
}

fn parse_raw_mini_chunks(
    bytes: &[u8],
    base_offset: usize,
    parent_path: &str,
) -> Vec<DynamicMiniChunk> {
    let mut out = Vec::new();
    let mut cursor = 0usize;
    let mut idx = 0usize;
    for mc_res in iter_mini_chunks(bytes) {
        let Ok(mc) = mc_res else {
            break;
        };
        let header_offset = base_offset + cursor;
        let payload_offset = header_offset + 2;
        let raw = mc.content().to_vec();
        let len = raw.len();
        out.push(DynamicMiniChunk {
            tree_path: format!("{parent_path}/mc{idx}"),
            id: mc.id(),
            header_offset,
            payload_offset,
            raw_bytes: raw,
            schema_field_name: None,
            schema_type_label: None,
            decoded: None,
            spans: Vec::new(),
            errors: Vec::new(),
        });
        cursor += 2 + len;
        idx += 1;
    }
    out
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

fn evaluate_chunk_container(
    schema: &Schema,
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
                loop {
                    if cursor >= chunks.len() {
                        break;
                    }
                    let consumed = try_match_field_at(
                        schema,
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
                        schema,
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
    schema: &Schema,
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
                schema,
                &field.name,
                payload,
                field.wire_format,
                &mut chunks[cursor],
                options,
                diagnostics,
            );
            1
        }
        ChunkFieldSource::Stream { item_type } => {
            try_match_stream_item_at(schema, &field.name, item_type, chunks, cursor, options, diagnostics)
        }
    }
}

fn try_match_stream_item_at(
    schema: &Schema,
    field_name: &str,
    item_type: &str,
    chunks: &mut [DynamicChunk],
    cursor: usize,
    options: &LoadOptions,
    diagnostics: &mut Vec<SchemaDiagnostic>,
) -> usize {
    let Some(typedef) = schema.types.get(item_type) else {
        return 0;
    };
    match typedef {
        TypeDef::ChunkEnum(enum_def) => {
            for variant in &enum_def.variants {
                let label = format!("{field_name} ({item_type}::{})", variant.name);
                match &variant.source {
                    ChunkFieldSource::Id { id, payload } => {
                        if chunks[cursor].id == *id {
                            apply_payload_to_chunk(
                                schema,
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
                            schema,
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
        TypeDef::ChunkStreamSeq(seq_def) | TypeDef::Chunks(seq_def) => {
            let Some(first_field) = seq_def.fields.first() else {
                return 0;
            };
            if !chunk_matches_field_start(schema, first_field, chunks[cursor].id) {
                return 0;
            }
            // Match the sequence struct's fields in order from `chunks[cursor..]`.
            let mut pos = cursor;
            for sub_field in &seq_def.fields {
                let sub_label = format!("{field_name}.{}", sub_field.name);
                let mut sub_clone = sub_field.clone();
                sub_clone.name = sub_label;
                let mut count = 0usize;
                while pos < chunks.len() {
                    let c = try_match_field_at(
                        schema,
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
        _ => 0,
    }
}

fn chunk_matches_field_start(schema: &Schema, field: &ChunkFieldDef, chunk_id: u32) -> bool {
    match &field.source {
        ChunkFieldSource::Id { id, .. } => *id == chunk_id,
        ChunkFieldSource::Stream { item_type } => {
            let Some(typedef) = schema.types.get(item_type) else {
                return false;
            };
            match typedef {
                TypeDef::ChunkEnum(e) => e.variants.iter().any(|v| match &v.source {
                    ChunkFieldSource::Id { id, .. } => *id == chunk_id,
                    ChunkFieldSource::Stream { item_type: inner } => {
                        schema.types.get(inner).is_some()
                    }
                }),
                TypeDef::ChunkStreamSeq(s) | TypeDef::Chunks(s) => s
                    .fields
                    .first()
                    .is_some_and(|f| chunk_matches_field_start(schema, f, chunk_id)),
                _ => false,
            }
        }
    }
}

fn apply_payload_to_chunk(
    schema: &Schema,
    field_name: &str,
    payload: &ChunkPayloadSpec,
    wire_format: Option<WireFormat>,
    chunk: &mut DynamicChunk,
    options: &LoadOptions,
    diagnostics: &mut Vec<SchemaDiagnostic>,
) {
    chunk.schema_field_name = Some(field_name.to_owned());

    match payload {
        ChunkPayloadSpec::Ignore => {
            chunk.schema_type_name = Some("ignored".to_owned());
        }
        ChunkPayloadSpec::Children(type_name) => {
            chunk.schema_type_name = Some(type_name.clone());
            let Some(TypeDef::Chunks(child_container)) = schema.types.get(type_name) else {
                push_chunk_err(
                    chunk,
                    diagnostics,
                    format!("Unknown `chunks` type `{type_name}` in schema"),
                );
                return;
            };
            let tree_path = chunk.tree_path.clone();
            let offset = chunk.payload_offset;
            match &mut chunk.content {
                DynamicChunkContent::Children(children) => {
                    evaluate_chunk_container(
                        schema,
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
                            "Expected container chunk (children `{type_name}`), but chunk 0x{:08x} is a leaf data chunk",
                            chunk.id
                        ),
                    );
                }
            }
        }
        ChunkPayloadSpec::MiniChunks(type_name) => {
            chunk.schema_type_name = Some(type_name.clone());
            let Some(TypeDef::MiniChunks(mc_container)) = schema.types.get(type_name) else {
                push_chunk_err(
                    chunk,
                    diagnostics,
                    format!("Unknown `mini_chunks` type `{type_name}` in schema"),
                );
                return;
            };
            if chunk.is_container {
                push_chunk_err(
                    chunk,
                    diagnostics,
                    format!(
                        "Expected leaf mini-chunks (`{type_name}`), but chunk 0x{:08x} is a container chunk",
                        chunk.id
                    ),
                );
                return;
            }
            let forced_plain = options.force_plain_data_paths.contains(&chunk.tree_path);
            if forced_plain {
                return;
            }
            // Ensure content is parsed as MiniChunks if possible
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
                    schema,
                    mc_container,
                    mini_chunks,
                    &tree_path,
                    offset,
                    diagnostics,
                );
            }
        }
        ChunkPayloadSpec::Data(data_type) => {
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
            // If heuristic had treated a plain-data chunk as mini-chunks, switch it to Data!
            let raw_bytes = match &chunk.content {
                DynamicChunkContent::Data { raw_bytes, .. } => raw_bytes.clone(),
                DynamicChunkContent::MiniChunks { raw_bytes, .. } => raw_bytes.clone(),
                DynamicChunkContent::Children(_) => return,
            };
            let mut spans = Vec::new();
            let mut color_counter = 0usize;
            match decode_data_type_with_wire_and_spans(
                schema,
                data_type,
                wire_format,
                &raw_bytes,
                0,
                field_name,
                &mut spans,
                &mut color_counter,
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

fn evaluate_mini_chunk_container(
    schema: &Schema,
    container: &MiniChunkContainerDef,
    mini_chunks: &mut [DynamicMiniChunk],
    container_path: &str,
    container_offset: usize,
    diagnostics: &mut Vec<SchemaDiagnostic>,
) {
    match container.mode {
        OrderMode::Sequence => {
            let mut cursor = 0usize;
            for field in &container.fields {
                let mut count = 0usize;
                while cursor < mini_chunks.len() && mini_chunks[cursor].id == field.id {
                    apply_mini_chunk_field(
                        schema,
                        &field.name,
                        &field.ty,
                        field.wire_format,
                        &mut mini_chunks[cursor],
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
                        schema,
                        &field.name,
                        &field.ty,
                        field.wire_format,
                        mc,
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

fn apply_mini_chunk_field(
    schema: &Schema,
    field_name: &str,
    ty: &DataType,
    wire_format: Option<WireFormat>,
    mc: &mut DynamicMiniChunk,
    diagnostics: &mut Vec<SchemaDiagnostic>,
) {
    mc.schema_field_name = Some(field_name.to_owned());
    mc.schema_type_label = Some(ty.label());
    let mut spans = Vec::new();
    let mut color_counter = 0usize;
    match decode_data_type_with_wire_and_spans(
        schema,
        ty,
        wire_format,
        &mc.raw_bytes,
        0,
        field_name,
        &mut spans,
        &mut color_counter,
    ) {
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

/// Decodes a [`DataType`] starting at `bytes[offset..]`, recording [`ByteSpanAnnotation`]s for
/// every leaf field and padding region, and returning `(DynamicValue, new_offset)`.
pub fn decode_data_type_with_spans(
    schema: &Schema,
    ty: &DataType,
    bytes: &[u8],
    offset: usize,
    field_path: &str,
    spans: &mut Vec<ByteSpanAnnotation>,
    color_counter: &mut usize,
) -> Result<(DynamicValue, usize), String> {
    decode_data_type_with_wire_and_spans(
        schema,
        ty,
        None,
        bytes,
        offset,
        field_path,
        spans,
        color_counter,
    )
}

pub fn decode_data_type_with_wire_and_spans(
    schema: &Schema,
    ty: &DataType,
    wire_format: Option<WireFormat>,
    bytes: &[u8],
    offset: usize,
    field_path: &str,
    spans: &mut Vec<ByteSpanAnnotation>,
    color_counter: &mut usize,
) -> Result<(DynamicValue, usize), String> {
    let mut slice = bytes
        .get(offset..)
        .ok_or_else(|| format!("Offset {offset} out of bounds (len {})", bytes.len()))?;

    let mut record_leaf = |len: usize, label: &str, val: DynamicValue| -> (DynamicValue, usize) {
        if spans.len() < 512 {
            let color_index = *color_counter;
            *color_counter += 1;
            spans.push(ByteSpanAnnotation {
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
                let (v, next) = decode_data_type_with_spans(
                    schema,
                    elem,
                    bytes,
                    cur,
                    &sub_path,
                    spans,
                    color_counter,
                )?;
                items.push(v);
                cur = next;
            }
            Ok((DynamicValue::Array(items), cur))
        }
        DataType::List(elem) => {
            let mut items = Vec::new();
            let mut cur = offset;
            let mut i = 0usize;
            let fast_elem_size = fixed_unvalidated_size(schema, elem);
            while cur < bytes.len() {
                if i >= 16
                    && let Some(elem_size) = fast_elem_size
                    && elem_size > 0
                {
                    let rem_bytes = bytes.len() - cur;
                    if rem_bytes % elem_size == 0 {
                        let rem_count = rem_bytes / elem_size;
                        let total_count = i + rem_count;
                        let color_index = *color_counter;
                        *color_counter += 1;
                        spans.push(ByteSpanAnnotation {
                            start: cur,
                            end: bytes.len(),
                            field_path: format!("{field_path}[{i}..{total_count}]"),
                            type_label: format!("{} ({rem_count} items)", elem.label()),
                            value_summary: format!("{rem_count} more items ({total_count} total)"),
                            color_index,
                            is_padding: false,
                        });
                        cur = bytes.len();
                        break;
                    }
                }
                let sub_path = format!("{field_path}[{i}]");
                let (v, next) = decode_data_type_with_spans(
                    schema,
                    elem,
                    bytes,
                    cur,
                    &sub_path,
                    spans,
                    color_counter,
                )?;
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
                let (v, next) = decode_data_type_with_spans(
                    schema,
                    elem,
                    bytes,
                    cur,
                    &sub_path,
                    spans,
                    color_counter,
                )?;
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
        DataType::Named(type_name) => {
            let Some(TypeDef::ByteBlock(bb)) = schema.types.get(type_name) else {
                return Err(format!("Unknown byte_block type `{type_name}`"));
            };
            decode_byte_block_def(schema, type_name, bb, bytes, offset, field_path, spans, color_counter)
        }
    }
}

fn decode_byte_block_def(
    schema: &Schema,
    type_name: &str,
    bb: &ByteBlockDef,
    bytes: &[u8],
    offset: usize,
    parent_path: &str,
    spans: &mut Vec<ByteSpanAnnotation>,
    color_counter: &mut usize,
) -> Result<(DynamicValue, usize), String> {
    let mut cur = offset;
    let mut fields = Vec::with_capacity(bb.fields.len());

    for f in &bb.fields {
        let sub_path = if parent_path.is_empty() {
            f.name.clone()
        } else {
            format!("{parent_path}.{}", f.name)
        };
        let (val, next) = decode_data_type_with_wire_and_spans(
            schema,
            &f.ty,
            f.wire_format,
            bytes,
            cur,
            &sub_path,
            spans,
            color_counter,
        )?;
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
            spans.push(ByteSpanAnnotation {
                start: cur,
                end: cur + pad,
                field_path: format!("{sub_path}.<pad_after>"),
                type_label: format!("pad ({pad}B)"),
                value_summary: format!("{pad} padding bytes"),
                color_index: 0,
                is_padding: true,
            });
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
        spans.push(ByteSpanAnnotation {
            start: cur,
            end: cur + pad,
            field_path: format!("{type_name}.<pad_after>"),
            type_label: format!("pad ({pad}B)"),
            value_summary: format!("{pad} padding bytes"),
            color_index: 0,
            is_padding: true,
        });
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
fn fixed_unvalidated_size(schema: &Schema, ty: &DataType) -> Option<usize> {
    match ty {
        DataType::U8 | DataType::I8 => Some(1),
        DataType::U16 | DataType::I16 => Some(2),
        DataType::U32 | DataType::I32 | DataType::F32 | DataType::Usize => Some(4),
        DataType::U64 | DataType::I64 | DataType::F64 | DataType::Vec2 => Some(8),
        DataType::Vec3 => Some(12),
        DataType::Vec4 => Some(16),
        DataType::Mat3x4 => Some(48),
        DataType::Array { elem, len } => fixed_unvalidated_size(schema, elem).map(|s| s * len),
        DataType::Tuple(items) => {
            let mut total = 0usize;
            for item in items {
                total += fixed_unvalidated_size(schema, item)?;
            }
            Some(total)
        }
        DataType::Named(name) => {
            let Some(TypeDef::ByteBlock(bb)) = schema.types.get(name) else {
                return None;
            };
            let mut total = bb.pad_after.unwrap_or(0);
            for f in &bb.fields {
                total += fixed_unvalidated_size(schema, &f.ty)?;
                total += f.pad_after.unwrap_or(0);
            }
            Some(total)
        }
        DataType::Bool | DataType::CString | DataType::Path | DataType::List(_) | DataType::RawBytes => None,
    }
}

/// Preview entry produced by [`inspect_bytes_at_offset`] for the hex-editor data inspector.
#[derive(Debug, Clone, PartialEq)]
pub struct BytePreviewEntry {
    pub label: &'static str,
    pub byte_len: usize,
    pub value: String,
    pub suggested_type: Option<(DataType, Option<WireFormat>)>,
}

/// Decodes the bytes starting at `bytes[offset..]` into all common primitive, vector, boolean, and
/// string interpretations for the hex-editor's multi-format preview panel.
pub fn inspect_bytes_at_offset(bytes: &[u8], offset: usize) -> Vec<BytePreviewEntry> {
    let mut out = Vec::new();
    let Some(rem) = bytes.get(offset..) else {
        return out;
    };
    if rem.is_empty() {
        return out;
    }

    // 1-byte types
    let b0 = rem[0];
    out.push(BytePreviewEntry {
        label: "u8",
        byte_len: 1,
        value: format!("{b0} (0x{b0:02x})"),
        suggested_type: Some((DataType::U8, None)),
    });
    out.push(BytePreviewEntry {
        label: "i8",
        byte_len: 1,
        value: format!("{}", b0 as i8),
        suggested_type: Some((DataType::I8, None)),
    });
    if b0 <= 1 {
        out.push(BytePreviewEntry {
            label: "bool (U8Bool)",
            byte_len: 1,
            value: format!("{}", b0 == 1),
            suggested_type: Some((DataType::Bool, Some(WireFormat::U8Bool))),
        });
    }

    // 2-byte types
    if rem.len() >= 2 {
        let u = u16::from_le_bytes([rem[0], rem[1]]);
        let i = i16::from_le_bytes([rem[0], rem[1]]);
        out.push(BytePreviewEntry {
            label: "u16",
            byte_len: 2,
            value: format!("{u} (0x{u:04x})"),
            suggested_type: Some((DataType::U16, None)),
        });
        out.push(BytePreviewEntry {
            label: "i16",
            byte_len: 2,
            value: format!("{i}"),
            suggested_type: Some((DataType::I16, None)),
        });
    }

    // 4-byte types
    if rem.len() >= 4 {
        let arr = [rem[0], rem[1], rem[2], rem[3]];
        let u = u32::from_le_bytes(arr);
        let i = i32::from_le_bytes(arr);
        let f = f32::from_le_bytes(arr);
        out.push(BytePreviewEntry {
            label: "u32",
            byte_len: 4,
            value: format!("{u} (0x{u:08x})"),
            suggested_type: Some((DataType::U32, None)),
        });
        out.push(BytePreviewEntry {
            label: "i32",
            byte_len: 4,
            value: format!("{i}"),
            suggested_type: Some((DataType::I32, None)),
        });
        out.push(BytePreviewEntry {
            label: "f32",
            byte_len: 4,
            value: format!("{f:.6}"),
            suggested_type: Some((DataType::F32, None)),
        });
        if u <= 1 {
            out.push(BytePreviewEntry {
                label: "bool (U32Bool)",
                byte_len: 4,
                value: format!("{}", u == 1),
                suggested_type: Some((DataType::Bool, Some(WireFormat::U32Bool))),
            });
        }
        out.push(BytePreviewEntry {
            label: "usize (U32Usize)",
            byte_len: 4,
            value: format!("{u}"),
            suggested_type: Some((DataType::Usize, Some(WireFormat::U32Usize))),
        });
    }

    // 8-byte types
    if rem.len() >= 8 {
        let arr: [u8; 8] = rem[..8].try_into().unwrap();
        let u = u64::from_le_bytes(arr);
        let i = i64::from_le_bytes(arr);
        let f = f64::from_le_bytes(arr);
        out.push(BytePreviewEntry {
            label: "u64",
            byte_len: 8,
            value: format!("{u} (0x{u:016x})"),
            suggested_type: Some((DataType::U64, None)),
        });
        out.push(BytePreviewEntry {
            label: "i64",
            byte_len: 8,
            value: format!("{i}"),
            suggested_type: Some((DataType::I64, None)),
        });
        out.push(BytePreviewEntry {
            label: "f64",
            byte_len: 8,
            value: format!("{f:.6}"),
            suggested_type: Some((DataType::F64, None)),
        });
        let f0 = f32::from_le_bytes(rem[0..4].try_into().unwrap());
        let f1 = f32::from_le_bytes(rem[4..8].try_into().unwrap());
        out.push(BytePreviewEntry {
            label: "Vec2 ([f32; 2])",
            byte_len: 8,
            value: format!("[{f0:.4}, {f1:.4}]"),
            suggested_type: Some((DataType::Vec2, None)),
        });
    }

    // 12-byte Vec3
    if rem.len() >= 12 {
        let f0 = f32::from_le_bytes(rem[0..4].try_into().unwrap());
        let f1 = f32::from_le_bytes(rem[4..8].try_into().unwrap());
        let f2 = f32::from_le_bytes(rem[8..12].try_into().unwrap());
        out.push(BytePreviewEntry {
            label: "Vec3 ([f32; 3])",
            byte_len: 12,
            value: format!("[{f0:.4}, {f1:.4}, {f2:.4}]"),
            suggested_type: Some((DataType::Vec3, None)),
        });
    }

    // 16-byte Vec4
    if rem.len() >= 16 {
        let f0 = f32::from_le_bytes(rem[0..4].try_into().unwrap());
        let f1 = f32::from_le_bytes(rem[4..8].try_into().unwrap());
        let f2 = f32::from_le_bytes(rem[8..12].try_into().unwrap());
        let f3 = f32::from_le_bytes(rem[12..16].try_into().unwrap());
        out.push(BytePreviewEntry {
            label: "Vec4 ([f32; 4])",
            byte_len: 16,
            value: format!("[{f0:.4}, {f1:.4}, {f2:.4}, {f3:.4}]"),
            suggested_type: Some((DataType::Vec4, None)),
        });
    }

    // Null-terminated ASCII string preview
    if let Ok(cstr) = CStr::from_bytes_until_nul(rem)
        && let Ok(s) = cstr.to_str()
        && !s.is_empty()
        && s.chars().all(|c| !c.is_control() || c == '\t')
    {
        let len = cstr.to_bytes_with_nul().len();
        out.push(BytePreviewEntry {
            label: "CString (ASCIIZ)",
            byte_len: len,
            value: format!("{s:?}"),
            suggested_type: Some((DataType::CString, None)),
        });
    }

    out
}
