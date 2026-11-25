//! Tree navigation and interactive schema mutation helpers for `petro-chunked-schema-editor`.
//!
//! Bridges the dynamically parsed [`DynamicFile`] tree and the editable [`Schema`] definition so
//! users can select any chunk or mini-chunk in the tree, inspect its bytes, and assign names,
//! cardinalities, container ordering (`map` vs `sequence`), and `ByteBlock` fields while keeping
//! type definitions ordered parents-before-children.

use std::collections::BTreeSet;

use petro_chunked_schema::dynamic::{
    ByteSpanAnnotation, DynamicChunk, DynamicChunkContent, DynamicFile, DynamicMiniChunk,
    DynamicValue,
};
use petro_chunked_schema::resolve::{ResolvedSchema, SchemaId};
use petro_chunked_schema::schema::{
    ByteBlockDef, ByteFieldDef, Cardinality, ChunkContainerDef, ChunkFieldDef, ChunkFieldSource,
    ChunkPayloadSpec, DataType, MiniChunkContainerDef, MiniChunkFieldDef, OrderMode, Schema,
    TypeDef, WireFormat,
};
use petro_chunked_schema::source::SchemaRef;

/// Flattened row in the left-pane chunk and mini-chunk tree view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlatTreeRow {
    /// Unique tree path (e.g. `"0"`, `"0/1"`, `"0/1/mc2"`).
    pub tree_path: String,
    /// Indentation depth (`0` for root chunks).
    pub depth: usize,
    /// `true` if this node is an 8-bit mini-chunk; `false` if it is a 32-bit chunk.
    pub is_mini_chunk: bool,
    /// Raw 32-bit chunk ID (or 8-bit mini-chunk ID widened to `u32`).
    pub raw_id: u32,
    /// Formatted ID label (`"0x0400"` for chunks, `"mc 01 (0x01)"` for mini-chunks).
    pub id_label: String,
    /// Assigned schema field name, if matched.
    pub field_name: Option<String>,
    /// Assigned schema type or payload summary label, if matched.
    pub type_label: Option<String>,
    /// Module ID of the container whose field matched this node, if matched.
    pub field_owner_module_id: Option<SchemaId>,
    /// Module ID owning [`Self::schema_type_ref`], if this node targets a named type.
    pub schema_module_id: Option<SchemaId>,
    /// Local type name inside [`Self::schema_module_id`], if this node targets a named type.
    pub schema_type_ref: Option<String>,
    /// `(SchemaId, type_name)` of the immediate parent chunk in the file tree (`None` at root).
    pub parent_type_ref: Option<(SchemaId, String)>,
    /// Payload size in bytes.
    pub size: usize,
    /// Number of direct child chunks or mini-chunks under this node.
    pub child_count: usize,
    /// `true` if this node has children and is currently expanded.
    pub is_expanded: bool,
    /// `true` if this node or its decoding produced any schema errors.
    pub has_error: bool,
}

/// Flattens a [`DynamicFile`] into visible rows for the tree view, respecting `collapsed_paths`.
pub fn build_flat_tree(
    file: &DynamicFile,
    collapsed_paths: &BTreeSet<String>,
    max_rows: usize,
) -> Vec<FlatTreeRow> {
    let mut rows = Vec::new();
    for chunk in &file.chunks {
        if rows.len() >= max_rows {
            break;
        }
        flatten_chunk(chunk, 0, None, collapsed_paths, max_rows, &mut rows);
    }
    rows
}

fn flatten_chunk(
    chunk: &DynamicChunk,
    depth: usize,
    parent_type_ref: Option<&(SchemaId, String)>,
    collapsed_paths: &BTreeSet<String>,
    max_rows: usize,
    rows: &mut Vec<FlatTreeRow>,
) {
    if rows.len() >= max_rows {
        return;
    }
    let child_count = match &chunk.content {
        DynamicChunkContent::Children(children) => children.len(),
        DynamicChunkContent::MiniChunks { mini_chunks, .. } => mini_chunks.len(),
        DynamicChunkContent::Data { .. } => 0,
    };
    let is_expanded = !collapsed_paths.contains(&chunk.tree_path);

    rows.push(FlatTreeRow {
        tree_path: chunk.tree_path.clone(),
        depth,
        is_mini_chunk: false,
        raw_id: chunk.id,
        id_label: format!("0x{:04X}", chunk.id),
        field_name: chunk.schema_field_name.clone(),
        type_label: chunk.schema_type_name.clone(),
        field_owner_module_id: chunk.field_owner_module_id,
        schema_module_id: chunk.schema_module_id,
        schema_type_ref: chunk.schema_type_ref.clone(),
        parent_type_ref: parent_type_ref.cloned(),
        size: chunk.size,
        child_count,
        is_expanded,
        has_error: !chunk.errors.is_empty(),
    });

    if !is_expanded {
        return;
    }

    let own_type_ref = chunk
        .schema_module_id
        .zip(chunk.schema_type_ref.clone());

    match &chunk.content {
        DynamicChunkContent::Children(children) => {
            for child in children {
                if rows.len() >= max_rows {
                    break;
                }
                flatten_chunk(
                    child,
                    depth + 1,
                    own_type_ref.as_ref(),
                    collapsed_paths,
                    max_rows,
                    rows,
                );
            }
        }
        DynamicChunkContent::MiniChunks { mini_chunks, .. } => {
            for mc in mini_chunks {
                if rows.len() >= max_rows {
                    break;
                }
                rows.push(FlatTreeRow {
                    tree_path: mc.tree_path.clone(),
                    depth: depth + 1,
                    is_mini_chunk: true,
                    raw_id: mc.id as u32,
                    id_label: format!("mc {:02} (0x{:02X})", mc.id, mc.id),
                    field_name: mc.schema_field_name.clone(),
                    type_label: mc.schema_type_label.clone(),
                    field_owner_module_id: mc.field_owner_module_id,
                    schema_module_id: mc.schema_module_id,
                    schema_type_ref: mc.schema_type_ref.clone(),
                    parent_type_ref: own_type_ref.clone(),
                    size: mc.raw_bytes.len(),
                    child_count: 0,
                    is_expanded: false,
                    has_error: !mc.errors.is_empty(),
                });
            }
        }
        DynamicChunkContent::Data { .. } => {}
    }
}

/// Identifies which container in the [`ResolvedSchema`] owns the currently selected node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParentContainerRef {
    /// Top-level root container when `schema.root` is a local `TypeDef::Chunks`.
    Root,
    /// Local `TypeDef::Chunks` or `TypeDef::ChunkStreamSeq` in `schema.types`.
    ChunksType {
        /// Local type key inside `schema.types`.
        type_name: String,
    },
    /// Local `TypeDef::MiniChunks` container in `schema.types`.
    MiniChunksType {
        /// Local type key inside `schema.types`.
        type_name: String,
    },
    /// Local `TypeDef::ChunkEnum` (variants edited in YAML rather than the chunk form).
    ChunkEnumType {
        /// Local type key inside `schema.types`.
        type_name: String,
    },
    /// Container owned by an imported schema module (read-only from the importing schema).
    Imported {
        /// Module ID in the resolved schema graph.
        module_id: SchemaId,
        /// Module alias / display prefix.
        alias: String,
        /// Source URI or path of the imported schema so the user can open it directly.
        source: Option<SchemaRef>,
        /// Type name inside the imported module.
        type_name: String,
    },
    /// Parent chunk does not yet have a container type assigned in the schema.
    UnassignedParentChunk {
        /// Tree path of the unassigned parent chunk.
        parent_tree_path: String,
        /// 32-bit chunk ID of the unassigned parent chunk.
        parent_chunk_id: u32,
    },
}

/// Summary row for an immediate child chunk or mini-chunk displayed in the center overview
/// panel when a parent container chunk (or mini-chunk block) is selected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChildSummaryRow {
    /// Zero-based index among the container's immediate children.
    pub index: usize,
    /// Unique tree path for clicking through to inspect this child.
    pub tree_path: String,
    /// Formatted ID label (`"0x0400"` for chunks, `"mc 01"` for mini-chunks).
    pub id_label: String,
    /// Structural kind summary (`"Container (N)"`, `"MiniChunks (N)"`, `"Data"`, or `"MiniChunk"`).
    pub kind_label: String,
    /// Assigned schema field name, if matched.
    pub field_name: Option<String>,
    /// Assigned schema type label, if matched.
    pub type_label: Option<String>,
    /// Child payload size in bytes.
    pub size: usize,
    /// Absolute file byte offset of the child's header.
    pub header_offset: usize,
    /// Compact preview string of any decoded leaf value.
    pub value_summary: Option<String>,
    /// `true` if validation errors exist on this child.
    pub has_error: bool,
}

/// Resolved details about the currently selected chunk or mini-chunk in the UI.
#[derive(Debug, Clone, PartialEq)]
pub struct SelectedNodeDetails {
    /// Tree path identifying this node.
    pub tree_path: String,
    /// `true` if this node is an 8-bit mini-chunk.
    pub is_mini_chunk: bool,
    /// 32-bit chunk ID (or 8-bit mini-chunk ID widened to `u32`).
    pub id: u32,
    /// `true` if this is a 32-bit container chunk holding child chunks.
    pub is_container: bool,
    /// `true` if this leaf chunk's payload can be segmented as mini-chunks.
    pub looks_like_mini_chunks: bool,
    /// Absolute byte offset of the chunk/mini-chunk header.
    pub header_offset: usize,
    /// Absolute byte offset of the payload data.
    pub payload_offset: usize,
    /// Payload length in bytes.
    pub size: usize,
    /// Matched schema field name, if any.
    pub schema_field_name: Option<String>,
    /// Matched schema type label, if any.
    pub schema_type_label: Option<String>,
    /// Raw payload bytes available for the hex editor.
    pub raw_bytes: Vec<u8>,
    /// Byte span annotations recorded within `raw_bytes`.
    pub spans: Vec<ByteSpanAnnotation>,
    /// Dynamically decoded value of this payload, if matched.
    pub decoded: Option<DynamicValue>,
    /// Errors recorded specifically on this node.
    pub errors: Vec<String>,
    /// Immediate child chunks or mini-chunks when inspecting a container or mini-chunk block.
    pub direct_children: Vec<ChildSummaryRow>,
    /// Container owning this node in the schema hierarchy.
    pub parent_container: ParentContainerRef,
    /// `(SchemaId, type_name)` when this payload maps to a named `TypeDef::ByteBlock`.
    pub active_byte_block: Option<(SchemaId, String)>,
}


/// Looks up the node at `tree_path` in `file` and resolves its [`SelectedNodeDetails`] against
/// `resolved`.
pub fn resolve_selected_node(
    file: &DynamicFile,
    resolved: &ResolvedSchema,
    tree_path: &str,
) -> Option<SelectedNodeDetails> {
    if tree_path.is_empty() {
        return None;
    }
    let segments: Vec<&str> = tree_path.split('/').collect();
    if segments.is_empty() {
        return None;
    }

    let root_idx: usize = segments[0].parse().ok()?;
    let mut current_chunk = file.chunks.get(root_idx)?;
    let mut parent_chunk: Option<&DynamicChunk> = None;

    for &seg in segments.iter().skip(1) {
        if let Some(mc_idx_str) = seg.strip_prefix("mc") {
            let mc_idx: usize = mc_idx_str.parse().ok()?;
            if let DynamicChunkContent::MiniChunks { mini_chunks, .. } = &current_chunk.content {
                let mc = mini_chunks.get(mc_idx)?;
                return Some(build_mini_chunk_details(mc, current_chunk, resolved));
            }
            return None;
        } else {
            let child_idx: usize = seg.parse().ok()?;
            if let DynamicChunkContent::Children(children) = &current_chunk.content {
                parent_chunk = Some(current_chunk);
                current_chunk = children.get(child_idx)?;
            } else {
                return None;
            }
        }
    }

    Some(build_chunk_details(current_chunk, parent_chunk, resolved))
}

fn imported_parent_ref(
    resolved: &ResolvedSchema,
    module_id: SchemaId,
    type_name: &str,
) -> ParentContainerRef {
    let module = resolved.module(module_id);
    ParentContainerRef::Imported {
        module_id,
        alias: module
            .module_name
            .clone()
            .unwrap_or_else(|| module.schema.name.clone()),
        source: module.source.clone(),
        type_name: type_name.to_owned(),
    }
}

fn resolve_byte_block_ref(
    resolved: &ResolvedSchema,
    mod_id: Option<SchemaId>,
    type_ref: Option<&str>,
) -> Option<(SchemaId, String)> {
    let mod_id = mod_id?;
    let tref = type_ref?;
    if matches!(
        resolved.module(mod_id).schema.types.get(tref),
        Some(TypeDef::ByteBlock(_))
    ) {
        Some((mod_id, tref.to_owned()))
    } else {
        None
    }
}

fn build_mini_chunk_details(
    mc: &DynamicMiniChunk,
    parent_chunk: &DynamicChunk,
    resolved: &ResolvedSchema,
) -> SelectedNodeDetails {
    let parent_container = if let (Some(mod_id), Some(tname)) =
        (parent_chunk.schema_module_id, parent_chunk.schema_type_ref.as_deref())
        && matches!(
            resolved.module(mod_id).schema.types.get(tname),
            Some(TypeDef::MiniChunks(_))
        ) {
        if mod_id == SchemaId::ROOT {
            ParentContainerRef::MiniChunksType {
                type_name: tname.to_owned(),
            }
        } else {
            imported_parent_ref(resolved, mod_id, tname)
        }
    } else {
        ParentContainerRef::UnassignedParentChunk {
            parent_tree_path: parent_chunk.tree_path.clone(),
            parent_chunk_id: parent_chunk.id,
        }
    };

    let active_byte_block =
        resolve_byte_block_ref(resolved, mc.schema_module_id, mc.schema_type_ref.as_deref());

    SelectedNodeDetails {
        tree_path: mc.tree_path.clone(),
        is_mini_chunk: true,
        id: mc.id as u32,
        is_container: false,
        looks_like_mini_chunks: false,
        header_offset: mc.header_offset,
        payload_offset: mc.payload_offset,
        size: mc.raw_bytes.len(),
        schema_field_name: mc.schema_field_name.clone(),
        schema_type_label: mc.schema_type_label.clone(),
        raw_bytes: mc.raw_bytes.clone(),
        spans: mc.spans.clone(),
        decoded: mc.decoded.clone(),
        errors: mc.errors.clone(),
        direct_children: Vec::new(),
        parent_container,
        active_byte_block,
    }
}

fn build_chunk_details(
    chunk: &DynamicChunk,
    parent_chunk: Option<&DynamicChunk>,
    resolved: &ResolvedSchema,
) -> SelectedNodeDetails {
    let parent_container = match parent_chunk {
        None => {
            let (root_mod, root_name, root_def) = resolved.root_type();
            if let Some(owner_mod) = chunk.field_owner_module_id
                && owner_mod != SchemaId::ROOT
            {
                let owner_schema = &resolved.module(owner_mod).schema;
                imported_parent_ref(resolved, owner_mod, &owner_schema.root)
            } else if root_mod != SchemaId::ROOT {
                imported_parent_ref(resolved, root_mod, root_name)
            } else {
                match root_def {
                    TypeDef::Chunks(_) => ParentContainerRef::Root,
                    TypeDef::ChunkStreamSeq(_) => ParentContainerRef::ChunksType {
                        type_name: root_name.to_owned(),
                    },
                    TypeDef::ChunkEnum(_) => ParentContainerRef::ChunkEnumType {
                        type_name: root_name.to_owned(),
                    },
                    _ => ParentContainerRef::Root,
                }
            }
        }
        Some(p) => {
            if let (Some(mod_id), Some(tname)) =
                (p.schema_module_id, p.schema_type_ref.as_deref())
                && matches!(
                    resolved.module(mod_id).schema.types.get(tname),
                    Some(TypeDef::Chunks(_) | TypeDef::ChunkStreamSeq(_))
                )
            {
                if mod_id == SchemaId::ROOT {
                    ParentContainerRef::ChunksType {
                        type_name: tname.to_owned(),
                    }
                } else {
                    imported_parent_ref(resolved, mod_id, tname)
                }
            } else {
                ParentContainerRef::UnassignedParentChunk {
                    parent_tree_path: p.tree_path.clone(),
                    parent_chunk_id: p.id,
                }
            }
        }
    };

    let (raw_bytes, spans, decoded, direct_children) = match &chunk.content {
        DynamicChunkContent::Children(children) => {
            let summaries = children
                .iter()
                .enumerate()
                .map(|(index, c)| {
                    let (kind_label, value_summary) = match &c.content {
                        DynamicChunkContent::Children(ch) => {
                            (format!("Container ({})", ch.len()), None)
                        }
                        DynamicChunkContent::MiniChunks { mini_chunks, .. } => {
                            (format!("MiniChunks ({})", mini_chunks.len()), None)
                        }
                        DynamicChunkContent::Data { decoded, .. } => (
                            "Data".to_owned(),
                            decoded.as_ref().map(DynamicValue::short_summary),
                        ),
                    };
                    ChildSummaryRow {
                        index,
                        tree_path: c.tree_path.clone(),
                        id_label: format!("0x{:04X}", c.id),
                        kind_label,
                        field_name: c.schema_field_name.clone(),
                        type_label: c.schema_type_name.clone(),
                        size: c.size,
                        header_offset: c.header_offset,
                        value_summary,
                        has_error: !c.errors.is_empty(),
                    }
                })
                .collect();
            (Vec::new(), Vec::new(), None, summaries)
        }
        DynamicChunkContent::MiniChunks {
            raw_bytes,
            mini_chunks,
        } => {
            let summaries = mini_chunks
                .iter()
                .enumerate()
                .map(|(index, mc)| ChildSummaryRow {
                    index,
                    tree_path: mc.tree_path.clone(),
                    id_label: format!("mc {:02X}", mc.id),
                    kind_label: "MiniChunk".to_owned(),
                    field_name: mc.schema_field_name.clone(),
                    type_label: mc.schema_type_label.clone(),
                    size: mc.raw_bytes.len(),
                    header_offset: mc.header_offset,
                    value_summary: mc.decoded.as_ref().map(DynamicValue::short_summary),
                    has_error: !mc.errors.is_empty(),
                })
                .collect();
            (raw_bytes.clone(), Vec::new(), None, summaries)
        }
        DynamicChunkContent::Data {
            raw_bytes,
            decoded,
            spans,
        } => (
            raw_bytes.clone(),
            spans.clone(),
            decoded.clone(),
            Vec::new(),
        ),
    };

    let active_byte_block = resolve_byte_block_ref(
        resolved,
        chunk.schema_module_id,
        chunk.schema_type_ref.as_deref(),
    );

    SelectedNodeDetails {
        tree_path: chunk.tree_path.clone(),
        is_mini_chunk: false,
        id: chunk.id,
        is_container: chunk.is_container,
        looks_like_mini_chunks: chunk.looks_like_mini_chunks,
        header_offset: chunk.header_offset,
        payload_offset: chunk.payload_offset,
        size: chunk.size,
        schema_field_name: chunk.schema_field_name.clone(),
        schema_type_label: chunk.schema_type_name.clone(),
        raw_bytes,
        spans,
        decoded,
        errors: chunk.errors.clone(),
        direct_children,
        parent_container,
        active_byte_block,
    }
}

/// Converts a snake_case field name into an UpperCamelCase type name.
pub fn to_pascal_case(s: &str) -> String {
    let mut out = String::new();
    for part in s.split(|c: char| !c.is_alphanumeric()) {
        if part.is_empty() {
            continue;
        }
        let mut chars = part.chars();
        if let Some(first) = chars.next() {
            out.extend(first.to_uppercase());
            out.push_str(chars.as_str());
        }
    }
    if out.is_empty() {
        "CustomType".to_owned()
    } else {
        out
    }
}

/// Looks up the existing [`ChunkFieldDef`] for `chunk_id` inside `parent_ref`, if defined.
pub fn find_existing_chunk_field<'a>(
    resolved: &'a ResolvedSchema,
    parent_ref: &ParentContainerRef,
    chunk_id: u32,
) -> Option<&'a ChunkFieldDef> {
    let (mod_id, type_name) = match parent_ref {
        ParentContainerRef::Root => {
            let root_name = &resolved.root_schema().root;
            (SchemaId::ROOT, root_name.as_str())
        }
        ParentContainerRef::ChunksType { type_name } => (SchemaId::ROOT, type_name.as_str()),
        ParentContainerRef::Imported {
            module_id,
            type_name,
            ..
        } => (*module_id, type_name.as_str()),
        _ => return None,
    };

    let fields: &[ChunkFieldDef] = match resolved.module(mod_id).schema.types.get(type_name) {
        Some(TypeDef::Chunks(c)) => &c.fields,
        Some(TypeDef::ChunkStreamSeq(s)) => &s.fields,
        _ => return None,
    };

    fields.iter().find(|f| {
        matches!(
            f.source,
            ChunkFieldSource::Id { id, .. } if id == chunk_id
        )
    })
}

/// Looks up the existing [`MiniChunkFieldDef`] for `mc_id` inside `parent_ref`, if defined.
pub fn find_existing_mini_chunk_field<'a>(
    resolved: &'a ResolvedSchema,
    parent_ref: &ParentContainerRef,
    mc_id: u8,
) -> Option<&'a MiniChunkFieldDef> {
    let (mod_id, type_name) = match parent_ref {
        ParentContainerRef::MiniChunksType { type_name } => (SchemaId::ROOT, type_name.as_str()),
        ParentContainerRef::Imported {
            module_id,
            type_name,
            ..
        } => (*module_id, type_name.as_str()),
        _ => return None,
    };
    let Some(TypeDef::MiniChunks(mc_container)) =
        resolved.module(mod_id).schema.types.get(type_name)
    else {
        return None;
    };
    mc_container.fields.iter().find(|f| f.id == mc_id)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LocalTargetKind {
    Chunks,
    MiniChunks,
    ByteBlock,
}

fn make_empty_stub(kind: LocalTargetKind) -> TypeDef {
    match kind {
        LocalTargetKind::Chunks => TypeDef::Chunks(ChunkContainerDef {
            description: None,
            mode: OrderMode::Map,
            ignore_unknown: true,
            fields: Vec::new(),
        }),
        LocalTargetKind::MiniChunks => TypeDef::MiniChunks(MiniChunkContainerDef {
            description: None,
            mode: OrderMode::Map,
            ignore_unknown: true,
            fields: Vec::new(),
        }),
        LocalTargetKind::ByteBlock => TypeDef::ByteBlock(ByteBlockDef {
            description: None,
            fields: Vec::new(),
            pad_after: None,
        }),
    }
}

fn extract_data_type_local_target(dt: &DataType) -> Option<(LocalTargetKind, &str)> {
    match dt {
        DataType::Named(type_name) if !type_name.contains("::") => {
            Some((LocalTargetKind::ByteBlock, type_name.as_str()))
        }
        DataType::List(inner) | DataType::Array { elem: inner, .. } => {
            extract_data_type_local_target(inner)
        }
        _ => None,
    }
}

fn extract_payload_local_target(payload: &ChunkPayloadSpec) -> Option<(LocalTargetKind, &str)> {
    match payload {
        ChunkPayloadSpec::Children(type_name) if !type_name.contains("::") => {
            Some((LocalTargetKind::Chunks, type_name.as_str()))
        }
        ChunkPayloadSpec::MiniChunks(type_name) if !type_name.contains("::") => {
            Some((LocalTargetKind::MiniChunks, type_name.as_str()))
        }
        ChunkPayloadSpec::Data(dt) => extract_data_type_local_target(dt),
        _ => None,
    }
}

fn count_data_type_refs(dt: &DataType, type_name: &str) -> usize {
    match dt {
        DataType::Named(n) => usize::from(n == type_name),
        DataType::Array { elem, .. } | DataType::List(elem) => count_data_type_refs(elem, type_name),
        DataType::Tuple(elems) => elems.iter().map(|e| count_data_type_refs(e, type_name)).sum(),
        _ => 0,
    }
}

fn count_chunk_source_refs(source: &ChunkFieldSource, type_name: &str) -> usize {
    match source {
        ChunkFieldSource::Id { payload, .. } => match payload {
            ChunkPayloadSpec::Children(t) | ChunkPayloadSpec::MiniChunks(t) => {
                usize::from(t == type_name)
            }
            ChunkPayloadSpec::Data(dt) => count_data_type_refs(dt, type_name),
            ChunkPayloadSpec::Ignore => 0,
        },
        ChunkFieldSource::Stream { item_type } => usize::from(item_type == type_name),
    }
}

fn count_local_type_refs(schema: &Schema, type_name: &str) -> usize {
    let mut total = usize::from(schema.root == type_name);
    for typedef in schema.types.values() {
        let delta: usize = match typedef {
            TypeDef::Chunks(c) => c
                .fields
                .iter()
                .map(|f| count_chunk_source_refs(&f.source, type_name))
                .sum(),
            TypeDef::ChunkStreamSeq(s) => s
                .fields
                .iter()
                .map(|f| count_chunk_source_refs(&f.source, type_name))
                .sum(),
            TypeDef::ChunkEnum(e) => e
                .variants
                .iter()
                .map(|v| count_chunk_source_refs(&v.source, type_name))
                .sum(),
            TypeDef::MiniChunks(m) => m
                .fields
                .iter()
                .map(|f| count_data_type_refs(&f.ty, type_name))
                .sum(),
            TypeDef::ByteBlock(bb) => bb
                .fields
                .iter()
                .map(|f| count_data_type_refs(&f.ty, type_name))
                .sum(),
        };
        total += delta;
    }
    total
}

fn is_empty_stub_type(def: &TypeDef) -> bool {
    match def {
        TypeDef::Chunks(c) => c.fields.is_empty() && c.description.is_none(),
        TypeDef::MiniChunks(m) => m.fields.is_empty() && m.description.is_none(),
        TypeDef::ByteBlock(b) => {
            b.fields.is_empty() && b.description.is_none() && b.pad_after.is_none()
        }
        TypeDef::ChunkStreamSeq(s) => s.fields.is_empty() && s.description.is_none(),
        TypeDef::ChunkEnum(e) => e.variants.is_empty() && e.description.is_none(),
    }
}

/// Removes unreferenced empty stub types from `schema.types` (never removing `schema.root` or any
/// type with fields, variants, descriptions, or `pad_after`), preserving definition order.
pub fn prune_unreferenced_empty_types(schema: &mut Schema) -> usize {
    let mut removed = 0usize;
    loop {
        let to_remove: Vec<String> = schema
            .types
            .iter()
            .filter(|(k, def)| {
                k.as_str() != schema.root
                    && is_empty_stub_type(def)
                    && count_local_type_refs(schema, k) == 0
            })
            .map(|(k, _)| k.clone())
            .collect();
        if to_remove.is_empty() {
            break;
        }
        for key in to_remove {
            if schema.types.shift_remove(&key).is_some() {
                removed += 1;
            }
        }
    }
    removed
}

fn ensure_payload_target_type(
    schema: &mut Schema,
    after_key: Option<&str>,
    payload: &ChunkPayloadSpec,
) {
    if let Some((kind, type_name)) = extract_payload_local_target(payload) {
        schema.insert_type_after(after_key, type_name.to_owned(), make_empty_stub(kind));
    }
}

fn try_reconcile_sole_owned_target(
    schema: &mut Schema,
    old_target: Option<(LocalTargetKind, String)>,
    new_target: Option<(LocalTargetKind, String)>,
) -> Result<(), String> {
    let (Some((old_kind, old_name)), Some((new_kind, new_name))) = (old_target, new_target) else {
        return Ok(());
    };
    if old_name == schema.root || count_local_type_refs(schema, &old_name) != 1 {
        return Ok(());
    }
    if old_kind == new_kind && old_name != new_name && !schema.types.contains_key(&new_name) {
        rename_local_type(schema, &old_name, &new_name)?;
    } else if old_kind != new_kind
        && old_name == new_name
        && schema.types.get(&old_name).is_some_and(is_empty_stub_type)
    {
        schema.types.insert(old_name, make_empty_stub(new_kind));
    }
    Ok(())
}

fn resolve_editable_chunk_fields<'a>(
    schema: &'a mut Schema,
    parent_ref: &ParentContainerRef,
) -> Result<(String, &'a mut Vec<ChunkFieldDef>), String> {
    let type_name = match parent_ref {
        ParentContainerRef::Root => schema.root.clone(),
        ParentContainerRef::ChunksType { type_name } => type_name.clone(),
        ParentContainerRef::ChunkEnumType { type_name } => {
            return Err(format!(
                "Root/parent `{type_name}` is a `chunk_enum`; edit its variants in the YAML modal"
            ));
        }
        ParentContainerRef::Imported { alias, .. } => {
            return Err(format!(
                "Container belongs to imported schema `{alias}` (open that schema directly to edit)"
            ));
        }
        ParentContainerRef::UnassignedParentChunk {
            parent_chunk_id, ..
        } => {
            return Err(format!(
                "Assign a `Children` container type to parent chunk 0x{parent_chunk_id:04X} first"
            ));
        }
        ParentContainerRef::MiniChunksType { .. } => {
            return Err("Cannot add a 32-bit chunk to a mini-chunk container".to_owned());
        }
    };

    let fields = match schema.types.get_mut(&type_name) {
        Some(TypeDef::Chunks(c)) => &mut c.fields,
        Some(TypeDef::ChunkStreamSeq(s)) => &mut s.fields,
        _ => return Err(format!("Parent container type `{type_name}` not found")),
    };
    Ok((type_name, fields))
}

/// Assigns or updates a 32-bit chunk field definition in `parent_ref`.
///
/// Newly created child types (`Children`, `MiniChunks`, `ByteBlock`) are inserted directly after the
/// parent type in `schema.types` to preserve parents-before-children order.
pub fn upsert_chunk_field(
    schema: &mut Schema,
    parent_ref: &ParentContainerRef,
    chunk_id: u32,
    field_name: &str,
    cardinality: Cardinality,
    payload: ChunkPayloadSpec,
    wire_format: Option<WireFormat>,
) -> Result<(), String> {
    let clean_name = field_name.trim();
    if clean_name.is_empty() {
        return Err("Field name cannot be empty".to_owned());
    }

    let (old_target, parent_type_hint) = {
        let (tname, fields) = resolve_editable_chunk_fields(schema, parent_ref)?;
        let prev = fields
            .iter()
            .find(|f| matches!(f.source, ChunkFieldSource::Id { id, .. } if id == chunk_id))
            .and_then(|existing| match &existing.source {
                ChunkFieldSource::Id { payload, .. } => {
                    extract_payload_local_target(payload).map(|(k, n)| (k, n.to_owned()))
                }
                ChunkFieldSource::Stream { .. } => None,
            });
        (prev, tname)
    };

    let new_target = extract_payload_local_target(&payload).map(|(k, n)| (k, n.to_owned()));
    try_reconcile_sole_owned_target(schema, old_target, new_target)?;

    let parent_type_name = if let ParentContainerRef::ChunksType { type_name } = parent_ref {
        if schema.types.contains_key(type_name) {
            parent_type_hint
        } else {
            schema.root.clone()
        }
    } else {
        parent_type_hint
    };

    let (resolved_parent, fields) = {
        let mut active_ref = parent_ref.clone();
        if let ParentContainerRef::ChunksType { .. } = active_ref
            && !matches!(
                schema.types.get(&parent_type_name),
                Some(TypeDef::Chunks(_) | TypeDef::ChunkStreamSeq(_))
            )
        {
            active_ref = ParentContainerRef::Root;
        }
        resolve_editable_chunk_fields(schema, &active_ref)?
    };

    if let Some(existing) = fields.iter_mut().find(|f| {
        matches!(
            f.source,
            ChunkFieldSource::Id { id, .. } if id == chunk_id
        )
    }) {
        existing.name = clean_name.to_owned();
        existing.cardinality = cardinality;
        existing.source = ChunkFieldSource::Id {
            id: chunk_id,
            payload: payload.clone(),
        };
        existing.wire_format = wire_format;
    } else {
        fields.push(ChunkFieldDef {
            name: clean_name.to_owned(),
            description: None,
            cardinality,
            source: ChunkFieldSource::Id {
                id: chunk_id,
                payload: payload.clone(),
            },
            wire_format,
        });
    }

    ensure_payload_target_type(schema, Some(&resolved_parent), &payload);
    prune_unreferenced_empty_types(schema);
    Ok(())
}

/// Removes the [`ChunkFieldDef`] matching `chunk_id` from `parent_ref`, returning the removed field
/// name.
pub fn delete_chunk_field(
    schema: &mut Schema,
    parent_ref: &ParentContainerRef,
    chunk_id: u32,
) -> Result<String, String> {
    let removed = {
        let (_, fields) = resolve_editable_chunk_fields(schema, parent_ref)?;
        let Some(idx) = fields.iter().position(|f| {
            matches!(
                f.source,
                ChunkFieldSource::Id { id, .. } if id == chunk_id
            )
        }) else {
            return Err(format!("No field found for chunk 0x{chunk_id:04X}"));
        };
        fields.remove(idx).name
    };
    prune_unreferenced_empty_types(schema);
    Ok(removed)
}

/// Assigns or updates an 8-bit mini-chunk field definition in `parent_ref`.
pub fn upsert_mini_chunk_field(
    schema: &mut Schema,
    parent_ref: &ParentContainerRef,
    mc_id: u8,
    field_name: &str,
    cardinality: Cardinality,
    ty: DataType,
    wire_format: Option<WireFormat>,
) -> Result<(), String> {
    let clean_name = field_name.trim();
    if clean_name.is_empty() {
        return Err("Mini-chunk field name cannot be empty".to_owned());
    }

    let type_name = match parent_ref {
        ParentContainerRef::MiniChunksType { type_name } => type_name.clone(),
        ParentContainerRef::Imported { alias, .. } => {
            return Err(format!(
                "MiniChunks container belongs to imported schema `{alias}` (open that schema directly to edit)"
            ));
        }
        _ => return Err("Assign a `MiniChunks` container type to the parent chunk first".to_owned()),
    };

    let old_target = {
        let Some(TypeDef::MiniChunks(mc_container)) = schema.types.get(&type_name) else {
            return Err(format!("MiniChunks type `{type_name}` not found in schema"));
        };
        mc_container
            .fields
            .iter()
            .find(|f| f.id == mc_id)
            .and_then(|existing| {
                extract_data_type_local_target(&existing.ty).map(|(k, n)| (k, n.to_owned()))
            })
    };

    let new_target = extract_data_type_local_target(&ty).map(|(k, n)| (k, n.to_owned()));
    try_reconcile_sole_owned_target(schema, old_target, new_target)?;

    let Some(TypeDef::MiniChunks(mc_container)) = schema.types.get_mut(&type_name) else {
        return Err(format!("MiniChunks type `{type_name}` not found in schema"));
    };

    if let Some(existing) = mc_container.fields.iter_mut().find(|f| f.id == mc_id) {
        existing.name = clean_name.to_owned();
        existing.cardinality = cardinality;
        existing.ty = ty.clone();
        existing.wire_format = wire_format;
    } else {
        mc_container.fields.push(MiniChunkFieldDef {
            id: mc_id,
            name: clean_name.to_owned(),
            description: None,
            cardinality,
            ty: ty.clone(),
            wire_format,
        });
    }

    if let Some((LocalTargetKind::ByteBlock, bb_name)) = extract_data_type_local_target(&ty) {
        schema.insert_type_after(
            Some(&type_name),
            bb_name.to_owned(),
            make_empty_stub(LocalTargetKind::ByteBlock),
        );
    }

    prune_unreferenced_empty_types(schema);
    Ok(())
}

/// Removes the [`MiniChunkFieldDef`] matching `mc_id` from `parent_ref`, returning the removed
/// field name.
pub fn delete_mini_chunk_field(
    schema: &mut Schema,
    parent_ref: &ParentContainerRef,
    mc_id: u8,
) -> Result<String, String> {
    let removed = {
        let ParentContainerRef::MiniChunksType { type_name } = parent_ref else {
            return Err(
                "Mini-chunk field does not belong to an editable local MiniChunks type".to_owned(),
            );
        };
        let Some(TypeDef::MiniChunks(mc_container)) = schema.types.get_mut(type_name) else {
            return Err(format!("MiniChunks type `{type_name}` not found in schema"));
        };
        let Some(idx) = mc_container.fields.iter().position(|f| f.id == mc_id) else {
            return Err(format!("No field found for mini-chunk 0x{mc_id:02X}"));
        };
        mc_container.fields.remove(idx).name
    };
    prune_unreferenced_empty_types(schema);
    Ok(removed)
}

/// Parses a comma-, semicolon-, or whitespace-separated extension string into normalized
/// lowercase file extensions without leading dots, deduplicated in first-seen order.
pub fn parse_file_extensions(raw: &str) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for tok in raw.split(|c: char| c == ',' || c == ';' || c.is_whitespace()) {
        let cleaned = tok.trim().trim_start_matches('.').to_ascii_lowercase();
        if !cleaned.is_empty() && seen.insert(cleaned.clone()) {
            out.push(cleaned);
        }
    }
    out
}

/// Renames a local `TypeDef` key in `schema.types`, preserving definition order and updating
/// `schema.root` as well as all local type references across containers, enums, mini-chunks,
/// and byte blocks.
pub fn rename_local_type(
    schema: &mut Schema,
    old_name: &str,
    new_name: &str,
) -> Result<(), String> {
    let clean_new = new_name.trim();
    if clean_new.is_empty() {
        return Err("Type name cannot be empty".to_owned());
    }
    if clean_new.contains("::") {
        return Err("Local type name cannot contain `::`".to_owned());
    }
    if old_name == clean_new {
        return Ok(());
    }
    if !schema.types.contains_key(old_name) {
        return Err(format!("Local type `{old_name}` not found in schema"));
    }
    if schema.types.contains_key(clean_new) {
        return Err(format!("Type `{clean_new}` already exists in schema"));
    }

    let old_types = std::mem::take(&mut schema.types);
    for (k, v) in old_types {
        let next_k = if k == old_name {
            clean_new.to_owned()
        } else {
            k
        };
        schema.types.insert(next_k, v);
    }

    if schema.root == old_name {
        schema.root = clean_new.to_owned();
    }

    for typedef in schema.types.values_mut() {
        match typedef {
            TypeDef::Chunks(c) => {
                for f in &mut c.fields {
                    rename_chunk_source_ref(&mut f.source, old_name, clean_new);
                }
            }
            TypeDef::ChunkStreamSeq(s) => {
                for f in &mut s.fields {
                    rename_chunk_source_ref(&mut f.source, old_name, clean_new);
                }
            }
            TypeDef::ChunkEnum(e) => {
                for v in &mut e.variants {
                    rename_chunk_source_ref(&mut v.source, old_name, clean_new);
                }
            }
            TypeDef::MiniChunks(m) => {
                for f in &mut m.fields {
                    rename_data_type_ref(&mut f.ty, old_name, clean_new);
                }
            }
            TypeDef::ByteBlock(bb) => {
                for f in &mut bb.fields {
                    rename_data_type_ref(&mut f.ty, old_name, clean_new);
                }
            }
        }
    }

    Ok(())
}

fn rename_chunk_source_ref(source: &mut ChunkFieldSource, old_name: &str, new_name: &str) {
    match source {
        ChunkFieldSource::Id { payload, .. } => match payload {
            ChunkPayloadSpec::Children(t) | ChunkPayloadSpec::MiniChunks(t) => {
                if t == old_name {
                    *t = new_name.to_owned();
                }
            }
            ChunkPayloadSpec::Data(dt) => rename_data_type_ref(dt, old_name, new_name),
            ChunkPayloadSpec::Ignore => {}
        },
        ChunkFieldSource::Stream { item_type } => {
            if item_type == old_name {
                *item_type = new_name.to_owned();
            }
        }
    }
}

fn rename_data_type_ref(dt: &mut DataType, old_name: &str, new_name: &str) {
    match dt {
        DataType::Named(n) => {
            if n == old_name {
                *n = new_name.to_owned();
            }
        }
        DataType::Array { elem, .. } | DataType::List(elem) => {
            rename_data_type_ref(elem, old_name, new_name);
        }
        DataType::Tuple(elems) => {
            for e in elems {
                rename_data_type_ref(e, old_name, new_name);
            }
        }
        _ => {}
    }
}

/// Updates top-level schema format metadata (`name`, `description`, `file_extensions`).
///
/// When `schema.root` still matches the previous `schema.name` (e.g. a newly created
/// `CustomFormat` schema) and the new name does not conflict with an existing type, the root
/// definition is co-renamed cleanly via [`rename_local_type`].
pub fn update_schema_metadata(
    schema: &mut Schema,
    name: &str,
    description: &str,
    file_extensions: Vec<String>,
) -> Result<(), String> {
    let clean_name = name.trim();
    if clean_name.is_empty() {
        return Err("Format name cannot be empty".to_owned());
    }
    let old_name = schema.name.clone();
    schema.name = clean_name.to_owned();
    schema.description = {
        let trimmed = description.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_owned())
    };
    schema.file_extensions = file_extensions;

    if schema.root == old_name
        && old_name != clean_name
        && !clean_name.contains("::")
        && !schema.types.contains_key(clean_name)
    {
        rename_local_type(schema, &old_name, clean_name)?;
    }

    Ok(())
}

/// Updates trailing `pad_after` on a named `TypeDef::ByteBlock` in `schema.types`.
pub fn set_byte_block_pad_after(
    schema: &mut Schema,
    bb_name: &str,
    pad_after: Option<usize>,
) -> Result<(), String> {
    let Some(TypeDef::ByteBlock(bb)) = schema.types.get_mut(bb_name) else {
        return Err(format!("ByteBlock `{bb_name}` not found in schema"));
    };
    bb.pad_after = pad_after.filter(|&p| p > 0);
    Ok(())
}

/// Ensures that the selected leaf chunk or mini-chunk is backed by a local [`TypeDef::ByteBlock`]
/// in `schema.types`, creating one if necessary, and appends a new field to that `ByteBlock`.
pub fn append_field_to_node_byte_block(
    schema: &mut Schema,
    resolved: &ResolvedSchema,
    node: &SelectedNodeDetails,
    field_name: &str,
    ty: DataType,
    wire_format: Option<WireFormat>,
    pad_after: Option<usize>,
) -> Result<String, String> {
    let clean_field = field_name.trim();
    if clean_field.is_empty() {
        return Err("Field name cannot be empty".to_owned());
    }

    let block_type_name = match &node.active_byte_block {
        Some((SchemaId::ROOT, existing_bb)) => existing_bb.clone(),
        Some((mod_id, _)) => {
            let mod_name = &resolved.module(*mod_id).schema.name;
            return Err(format!(
                "ByteBlock belongs to imported schema `{mod_name}` (open that schema directly to edit)"
            ));
        }
        None => {
            let base_field_name = node.schema_field_name.clone().unwrap_or_else(|| {
                if node.is_mini_chunk {
                    format!("mc_{:02x}", node.id)
                } else {
                    format!("chunk_{:04x}", node.id)
                }
            });
            let new_bb_name = format!("{}Block", to_pascal_case(&base_field_name));

            if node.is_mini_chunk {
                let existing_card =
                    find_existing_mini_chunk_field(resolved, &node.parent_container, node.id as u8)
                        .map(|f| f.cardinality.clone())
                        .unwrap_or(Cardinality::Required);
                upsert_mini_chunk_field(
                    schema,
                    &node.parent_container,
                    node.id as u8,
                    &base_field_name,
                    existing_card,
                    DataType::Named(new_bb_name.clone()),
                    None,
                )?;
            } else {
                let existing_card =
                    find_existing_chunk_field(resolved, &node.parent_container, node.id)
                        .map(|f| f.cardinality.clone())
                        .unwrap_or(Cardinality::Required);
                upsert_chunk_field(
                    schema,
                    &node.parent_container,
                    node.id,
                    &base_field_name,
                    existing_card,
                    ChunkPayloadSpec::Data(DataType::Named(new_bb_name.clone())),
                    None,
                )?;
            }
            new_bb_name
        }
    };

    schema.insert_type_after(
        None,
        block_type_name.clone(),
        TypeDef::ByteBlock(ByteBlockDef {
            description: None,
            fields: Vec::new(),
            pad_after: None,
        }),
    );

    let Some(TypeDef::ByteBlock(bb)) = schema.types.get_mut(&block_type_name) else {
        return Err(format!("Type `{block_type_name}` is not a ByteBlock"));
    };

    bb.fields.push(ByteFieldDef {
        name: clean_field.to_owned(),
        description: None,
        ty,
        wire_format,
        pad_after: pad_after.filter(|&p| p > 0),
    });

    Ok(block_type_name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use petro_chunked_schema::dynamic::{LoadOptions, load_dynamic_file};
    use petro_chunked_schema::resolve::SchemaLoader;

    fn make_chunk(id: u32, is_container: bool, payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(8 + payload.len());
        out.extend_from_slice(&id.to_le_bytes());
        let mut size = payload.len() as u32;
        if is_container {
            size |= 0x8000_0000;
        }
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(payload);
        out
    }

    fn make_sample_chunk_buffer() -> Vec<u8> {
        let mut leaf_payload = Vec::new();
        leaf_payload.extend_from_slice(&3u32.to_le_bytes());
        leaf_payload.extend_from_slice(&1.5f32.to_le_bytes());

        let mut container_payload = Vec::new();
        container_payload.extend_from_slice(&0x0201u32.to_le_bytes());
        container_payload.extend_from_slice(&(leaf_payload.len() as u32).to_le_bytes());
        container_payload.extend_from_slice(&leaf_payload);

        let mut file_bytes = Vec::new();
        file_bytes.extend_from_slice(&0x0200u32.to_le_bytes());
        file_bytes.extend_from_slice(
            &((container_payload.len() as u32) | 0x8000_0000).to_le_bytes(),
        );
        file_bytes.extend_from_slice(&container_payload);
        file_bytes
    }

    #[test]
    fn test_interactive_schema_construction_deletion_and_ordering() {
        let bytes = make_sample_chunk_buffer();
        let mut schema = Schema::new_empty("TestSkeletonFormat");
        let opts = LoadOptions::default();

        // 1. Load with blank schema
        let res0 = SchemaLoader::resolve(schema.clone(), None).unwrap();
        let df0 = load_dynamic_file(&bytes, Some(&res0), &opts).unwrap();
        let rows = build_flat_tree(&df0, &BTreeSet::new(), 100);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].tree_path, "0");
        assert_eq!(rows[1].tree_path, "0/0");

        // 2. Select root chunk "0" (0x0200) and assign it a Children container `SkeletonContainer`
        let root_node = resolve_selected_node(&df0, &res0, "0").unwrap();
        assert_eq!(root_node.id, 0x0200);
        assert_eq!(root_node.parent_container, ParentContainerRef::Root);
        upsert_chunk_field(
            &mut schema,
            &root_node.parent_container,
            root_node.id,
            "skeleton",
            Cardinality::Required,
            ChunkPayloadSpec::Children("SkeletonContainer".to_owned()),
            None,
        )
        .unwrap();

        // 3. Re-evaluate and select child chunk "0/0" (0x0201)
        let res1 = SchemaLoader::resolve(schema.clone(), None).unwrap();
        let df1 = load_dynamic_file(&bytes, Some(&res1), &opts).unwrap();
        let child_node = resolve_selected_node(&df1, &res1, "0/0").unwrap();
        assert_eq!(child_node.id, 0x0201);
        assert_eq!(
            child_node.parent_container,
            ParentContainerRef::ChunksType {
                type_name: "SkeletonContainer".to_owned()
            }
        );

        // 4. Add two ByteBlock fields + verify parents-before-children insertion order
        let bb_name = append_field_to_node_byte_block(
            &mut schema,
            &res1,
            &child_node,
            "bone_count",
            DataType::Usize,
            Some(WireFormat::U32Usize),
            None,
        )
        .unwrap();
        assert_eq!(bb_name, "Chunk0201Block");

        let res2 = SchemaLoader::resolve(schema.clone(), None).unwrap();
        let df2 = load_dynamic_file(&bytes, Some(&res2), &opts).unwrap();
        let child_node2 = resolve_selected_node(&df2, &res2, "0/0").unwrap();
        assert_eq!(
            child_node2.active_byte_block,
            Some((SchemaId::ROOT, "Chunk0201Block".to_owned()))
        );

        append_field_to_node_byte_block(
            &mut schema,
            &res2,
            &child_node2,
            "scale",
            DataType::F32,
            None,
            None,
        )
        .unwrap();

        // Check parents-before-children ordering in schema.types
        let type_order: Vec<&str> = schema.types.keys().map(String::as_str).collect();
        assert_eq!(
            type_order,
            vec!["TestSkeletonFormat", "SkeletonContainer", "Chunk0201Block"]
        );

        // 5. Re-evaluate: all 8 bytes covered cleanly
        let res3 = SchemaLoader::resolve(schema.clone(), None).unwrap();
        let df3 = load_dynamic_file(&bytes, Some(&res3), &opts).unwrap();
        assert!(df3.diagnostics.is_empty(), "diags: {:?}", df3.diagnostics);
        let child_node3 = resolve_selected_node(&df3, &res3, "0/0").unwrap();
        assert_eq!(child_node3.spans.len(), 2);

        // 6. Test pad_after & deletion helpers
        set_byte_block_pad_after(&mut schema, "Chunk0201Block", Some(4)).unwrap();
        assert!(matches!(
            schema.types.get("Chunk0201Block"),
            Some(TypeDef::ByteBlock(bb)) if bb.pad_after == Some(4)
        ));
        let removed = delete_chunk_field(&mut schema, &child_node3.parent_container, 0x0201).unwrap();
        assert_eq!(removed, "chunk_0201");
    }

    #[test]
    fn test_imported_nodes_are_marked_read_only() {
        let resolved = SchemaLoader::load(&SchemaRef::Builtin("Alo".to_owned())).unwrap();
        let mut skel_hdr = Vec::from(0u32.to_le_bytes());
        skel_hdr.extend_from_slice(&[0u8; 124]);
        let skel = make_chunk(0x0201, false, &skel_hdr);
        let model_bytes = make_chunk(0x0200, true, &skel);

        let df = load_dynamic_file(&model_bytes, Some(&resolved), &LoadOptions::default()).unwrap();
        let node = resolve_selected_node(&df, &resolved, "0/0").unwrap();
        assert!(matches!(
            node.parent_container,
            ParentContainerRef::Imported { .. }
        ));
        assert_ne!(
            node.active_byte_block.as_ref().map(|(id, _)| *id),
            Some(SchemaId::ROOT)
        );
        assert!(node.active_byte_block.is_some());
    }

    #[test]
    fn test_container_direct_children_overview() {
        let resolved = SchemaLoader::load(&SchemaRef::Builtin("Alo".to_owned())).unwrap();
        let mut skel_hdr = Vec::from(0u32.to_le_bytes());
        skel_hdr.extend_from_slice(&[0u8; 124]);
        let skel = make_chunk(0x0201, false, &skel_hdr);
        let model_bytes = make_chunk(0x0200, true, &skel);

        let df = load_dynamic_file(&model_bytes, Some(&resolved), &LoadOptions::default()).unwrap();
        let parent = resolve_selected_node(&df, &resolved, "0").unwrap();
        assert!(parent.is_container);
        assert_eq!(parent.direct_children.len(), 1);
        let child = &parent.direct_children[0];
        assert_eq!(child.index, 0);
        assert_eq!(child.tree_path, "0/0");
        assert_eq!(child.id_label, "0x0201");
        assert_eq!(child.kind_label, "Data");
        assert_eq!(child.field_name.as_deref(), Some("header"));
        assert_eq!(child.type_label.as_deref(), Some("SkeletonHeader"));
        assert_eq!(child.size, 128);
        assert!(!child.has_error);
    }

    #[test]
    fn test_parse_file_extensions_normalizes_and_dedups() {
        assert_eq!(
            parse_file_extensions(".TED, .map; ted  ALO ."),
            vec!["ted".to_owned(), "map".to_owned(), "alo".to_owned()]
        );
        assert!(parse_file_extensions("  , ; . ").is_empty());
    }

    #[test]
    fn test_update_schema_metadata_and_root_co_rename() {
        let mut schema = Schema::new_empty("CustomFormat");
        upsert_chunk_field(
            &mut schema,
            &ParentContainerRef::Root,
            0x0001,
            "header",
            Cardinality::Required,
            ChunkPayloadSpec::Children("HeaderBox".to_owned()),
            None,
        )
        .unwrap();

        // Starter schema where root == name co-renames root and keeps parents-before-children order.
        update_schema_metadata(
            &mut schema,
            "TedMap",
            "Terrain map",
            parse_file_extensions(".TED, map"),
        )
        .unwrap();
        assert_eq!(schema.name, "TedMap");
        assert_eq!(schema.root, "TedMap");
        assert_eq!(schema.description.as_deref(), Some("Terrain map"));
        assert_eq!(schema.file_extensions, vec!["ted".to_owned(), "map".to_owned()]);
        let keys: Vec<&str> = schema.types.keys().map(String::as_str).collect();
        assert_eq!(keys, vec!["TedMap", "HeaderBox"]);

        // Schema where name != root keeps existing root name on metadata rename.
        schema.root = "TedRoot".to_owned();
        let old = schema.types.shift_remove("TedMap").unwrap();
        schema.types.shift_insert(0, "TedRoot".to_owned(), old);
        update_schema_metadata(&mut schema, "TedV2", "", vec!["ted".to_owned()]).unwrap();
        assert_eq!(schema.name, "TedV2");
        assert_eq!(schema.root, "TedRoot");
        assert_eq!(schema.description, None);

        // Explicit rename_local_type updates root and local references.
        rename_local_type(&mut schema, "HeaderBox", "MapHeader").unwrap();
        let Some(TypeDef::Chunks(root_cc)) = schema.types.get("TedRoot") else {
            panic!("expected root container");
        };
        assert!(matches!(
            &root_cc.fields[0].source,
            ChunkFieldSource::Id { payload: ChunkPayloadSpec::Children(t), .. } if t == "MapHeader"
        ));
        rename_local_type(&mut schema, "TedRoot", "TedMapRoot").unwrap();
        assert_eq!(schema.root, "TedMapRoot");
        assert!(schema.types.contains_key("TedMapRoot"));
    }

    #[test]
    fn test_live_typing_type_name_renames_in_place_without_orphans() {
        let mut schema = Schema::new_empty("TedMap");

        // 1. Define chunk 0x0000 with auto-generated MiniChunks("Chunk0000MiniChunks"), and populate
        //    a field inside it.
        upsert_chunk_field(
            &mut schema,
            &ParentContainerRef::Root,
            0x0000,
            "map_info",
            Cardinality::Required,
            ChunkPayloadSpec::MiniChunks("Chunk0000MiniChunks".to_owned()),
            None,
        )
        .unwrap();
        upsert_mini_chunk_field(
            &mut schema,
            &ParentContainerRef::MiniChunksType {
                type_name: "Chunk0000MiniChunks".to_owned(),
            },
            0,
            "map_format_version",
            Cardinality::Optional,
            DataType::U32,
            None,
        )
        .unwrap();

        // 2. Simulate live keystroke-by-keystroke typing in the `Type` box: "M" .. "MapInfo".
        for prefix in ["M", "Ma", "Map", "MapI", "MapIn", "MapInf", "MapInfo"] {
            upsert_chunk_field(
                &mut schema,
                &ParentContainerRef::Root,
                0x0000,
                "map_info",
                Cardinality::Required,
                ChunkPayloadSpec::MiniChunks(prefix.to_owned()),
                None,
            )
            .unwrap();
        }
        let keys: Vec<&str> = schema.types.keys().map(String::as_str).collect();
        assert_eq!(keys, vec!["TedMap", "MapInfo"]);
        let Some(TypeDef::MiniChunks(mc_info)) = schema.types.get("MapInfo") else {
            panic!("expected MapInfo mini_chunks container");
        };
        assert_eq!(mc_info.fields.len(), 1);
        assert_eq!(mc_info.fields[0].name, "map_format_version");

        // 3. Typing prefixes of `c_string` on a Data chunk leaves zero orphan ByteBlocks.
        upsert_chunk_field(
            &mut schema,
            &ParentContainerRef::Root,
            0x0002,
            "label",
            Cardinality::Optional,
            ChunkPayloadSpec::Data(DataType::RawBytes),
            None,
        )
        .unwrap();
        for prefix in ["c", "c_", "c_s", "c_strin"] {
            upsert_chunk_field(
                &mut schema,
                &ParentContainerRef::Root,
                0x0002,
                "label",
                Cardinality::Optional,
                ChunkPayloadSpec::Data(DataType::Named(prefix.to_owned())),
                None,
            )
            .unwrap();
        }
        upsert_chunk_field(
            &mut schema,
            &ParentContainerRef::Root,
            0x0002,
            "label",
            Cardinality::Optional,
            ChunkPayloadSpec::Data(DataType::CString),
            None,
        )
        .unwrap();
        let keys_after_cstr: Vec<&str> = schema.types.keys().map(String::as_str).collect();
        assert_eq!(keys_after_cstr, vec!["TedMap", "MapInfo"]);

        // 4. Shared type protection: when two chunks reference `"MapInfo"`, changing the second
        //    to `"MapExtra"` keeps `"MapInfo"` intact and creates `"MapExtra"`.
        upsert_chunk_field(
            &mut schema,
            &ParentContainerRef::Root,
            0x0003,
            "dup_info",
            Cardinality::Optional,
            ChunkPayloadSpec::MiniChunks("MapInfo".to_owned()),
            None,
        )
        .unwrap();
        upsert_chunk_field(
            &mut schema,
            &ParentContainerRef::Root,
            0x0003,
            "dup_info",
            Cardinality::Optional,
            ChunkPayloadSpec::MiniChunks("MapExtra".to_owned()),
            None,
        )
        .unwrap();
        assert!(schema.types.contains_key("MapInfo"));
        assert!(schema.types.contains_key("MapExtra"));

        // 5. Deleting a field prunes its unreferenced empty stub, and same-name kind conversion
        //    replaces the empty stub cleanly.
        delete_chunk_field(&mut schema, &ParentContainerRef::Root, 0x0003).unwrap();
        assert!(!schema.types.contains_key("MapExtra"));

        upsert_chunk_field(
            &mut schema,
            &ParentContainerRef::Root,
            0x0004,
            "temp",
            Cardinality::Optional,
            ChunkPayloadSpec::MiniChunks("TempNode".to_owned()),
            None,
        )
        .unwrap();
        upsert_chunk_field(
            &mut schema,
            &ParentContainerRef::Root,
            0x0004,
            "temp",
            Cardinality::Optional,
            ChunkPayloadSpec::Children("TempNode".to_owned()),
            None,
        )
        .unwrap();
        assert!(matches!(schema.types.get("TempNode"), Some(TypeDef::Chunks(_))));
        upsert_chunk_field(
            &mut schema,
            &ParentContainerRef::Root,
            0x0004,
            "temp",
            Cardinality::Optional,
            ChunkPayloadSpec::Data(DataType::U32),
            None,
        )
        .unwrap();
        assert!(!schema.types.contains_key("TempNode"));
    }
}
