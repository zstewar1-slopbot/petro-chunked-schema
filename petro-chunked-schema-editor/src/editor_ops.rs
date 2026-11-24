//! Tree navigation and interactive schema mutation helpers for `petro-chunked-schema-editor`.
//!
//! Bridges the dynamically parsed [`DynamicFile`] tree and the editable [`Schema`] definition so
//! that users can click any chunk or mini-chunk in the tree, inspect its bytes, and interactively
//! assign names, cardinalities, container ordering (`map` vs `sequence`), and `ByteBlock` fields.

use std::collections::BTreeSet;

use petro_chunked_schema::dynamic::{
    ByteSpanAnnotation, DynamicChunk, DynamicChunkContent, DynamicFile, DynamicMiniChunk,
    DynamicValue,
};
use petro_chunked_schema::schema::{
    ByteBlockDef, ByteFieldDef, Cardinality, ChunkContainerDef, ChunkFieldDef, ChunkFieldSource,
    ChunkPayloadSpec, DataType, MiniChunkContainerDef, MiniChunkFieldDef, OrderMode, Schema,
    TypeDef, WireFormat,
};

/// A flattened row in the left-pane chunk & mini-chunk tree view.
#[derive(Debug, Clone)]
pub struct FlatTreeRow {
    /// Unique tree path (e.g. `"0"`, `"0/1"`, `"0/1/mc2"`).
    pub tree_path: String,
    /// Indentation depth (`0` for root chunks).
    pub depth: usize,
    /// True if this node is an 8-bit mini-chunk; false if it is a 32-bit chunk.
    pub is_mini_chunk: bool,
    /// Formatted ID label (`"0x00000400"` for chunks, `"mc 0x01 (1)"` for mini-chunks).
    pub id_label: String,
    /// Assigned schema field name, if matched.
    pub field_name: Option<String>,
    /// Assigned schema type or payload summary label, if matched.
    pub type_label: Option<String>,
    /// Payload size in bytes.
    pub size: usize,
    /// Number of child chunks or mini-chunks under this node.
    pub child_count: usize,
    /// True if this node has children and is currently expanded in the tree view.
    pub is_expanded: bool,
    /// True if this node or its decoding produced any schema errors.
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
        flatten_chunk(chunk, 0, collapsed_paths, max_rows, &mut rows);
    }
    rows
}

fn flatten_chunk(
    chunk: &DynamicChunk,
    depth: usize,
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
        id_label: format!("0x{:04X}", chunk.id),
        field_name: chunk.schema_field_name.clone(),
        type_label: chunk.schema_type_name.clone(),
        size: chunk.size,
        child_count,
        is_expanded,
        has_error: !chunk.errors.is_empty(),
    });

    if !is_expanded {
        return;
    }

    match &chunk.content {
        DynamicChunkContent::Children(children) => {
            for child in children {
                if rows.len() >= max_rows {
                    break;
                }
                flatten_chunk(child, depth + 1, collapsed_paths, max_rows, rows);
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
                    id_label: format!("mc {:02} (0x{:02X})", mc.id, mc.id),
                    field_name: mc.schema_field_name.clone(),
                    type_label: mc.schema_type_label.clone(),
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

/// Identifies which container in the [`Schema`] holds the currently selected node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParentContainerRef {
    /// The top-level `schema.root` chunk container.
    Root,
    /// A named `TypeDef::Chunks` or `TypeDef::ChunkStreamSeq` container in `schema.types`.
    ChunksType {
        type_name: String,
    },
    /// A named `TypeDef::MiniChunks` container in `schema.types`.
    MiniChunksType {
        type_name: String,
    },
    /// The parent chunk does not yet have a container type assigned in the schema.
    UnassignedParentChunk {
        parent_tree_path: String,
        parent_chunk_id: u32,
    },
}

/// Resolved details about the currently selected chunk or mini-chunk in the UI.
#[derive(Debug, Clone)]
pub struct SelectedNodeDetails {
    pub tree_path: String,
    pub is_mini_chunk: bool,
    /// 32-bit chunk ID (or 8-bit mini-chunk ID widened to `u32`).
    pub id: u32,
    pub is_container: bool,
    pub looks_like_mini_chunks: bool,
    pub header_offset: usize,
    pub payload_offset: usize,
    pub size: usize,
    pub schema_field_name: Option<String>,
    pub schema_type_label: Option<String>,
    /// Raw payload bytes available for the hex editor (empty for container chunks with child chunks).
    pub raw_bytes: Vec<u8>,
    /// Byte span annotations within `raw_bytes`.
    pub spans: Vec<ByteSpanAnnotation>,
    /// Decoded value of this payload, if any.
    pub decoded: Option<DynamicValue>,
    /// Errors recorded specifically on this node.
    pub errors: Vec<String>,
    /// Where this node lives in the schema hierarchy.
    pub parent_container: ParentContainerRef,
    /// If this node's payload is a named `TypeDef::ByteBlock`, its type name in `schema.types`.
    pub active_byte_block_type: Option<String>,
}

/// Looks up the node at `tree_path` in `file` and resolves its [`SelectedNodeDetails`] against `schema`.
pub fn resolve_selected_node(
    file: &DynamicFile,
    schema: &Schema,
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

    for (seg_idx, &seg) in segments.iter().enumerate().skip(1) {
        if let Some(mc_idx_str) = seg.strip_prefix("mc") {
            let mc_idx: usize = mc_idx_str.parse().ok()?;
            if let DynamicChunkContent::MiniChunks { mini_chunks, .. } = &current_chunk.content {
                let mc = mini_chunks.get(mc_idx)?;
                return Some(build_mini_chunk_details(mc, current_chunk, schema));
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
            let _ = seg_idx;
        }
    }

    Some(build_chunk_details(current_chunk, parent_chunk, schema))
}

fn build_mini_chunk_details(
    mc: &DynamicMiniChunk,
    parent_chunk: &DynamicChunk,
    schema: &Schema,
) -> SelectedNodeDetails {
    let parent_container = if let Some(type_name) = &parent_chunk.schema_type_name
        && matches!(schema.types.get(type_name), Some(TypeDef::MiniChunks(_)))
    {
        ParentContainerRef::MiniChunksType {
            type_name: type_name.clone(),
        }
    } else {
        ParentContainerRef::UnassignedParentChunk {
            parent_tree_path: parent_chunk.tree_path.clone(),
            parent_chunk_id: parent_chunk.id,
        }
    };

    let active_byte_block_type = match & parent_container {
        ParentContainerRef::MiniChunksType { type_name } => {
            if let Some(TypeDef::MiniChunks(mc_def)) = schema.types.get(type_name) {
                mc_def
                    .fields
                    .iter()
                    .find(|f| f.id == mc.id)
                    .and_then(|f| match &f.ty {
                        DataType::Named(n)
                            if matches!(schema.types.get(n), Some(TypeDef::ByteBlock(_))) =>
                        {
                            Some(n.clone())
                        }
                        DataType::List(inner) => match &**inner {
                            DataType::Named(n)
                                if matches!(schema.types.get(n), Some(TypeDef::ByteBlock(_))) =>
                            {
                                Some(n.clone())
                            }
                            _ => None,
                        },
                        _ => None,
                    })
            } else {
                None
            }
        }
        _ => None,
    };

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
        parent_container,
        active_byte_block_type,
    }
}

fn build_chunk_details(
    chunk: &DynamicChunk,
    parent_chunk: Option<&DynamicChunk>,
    schema: &Schema,
) -> SelectedNodeDetails {
    let parent_container = match parent_chunk {
        None => ParentContainerRef::Root,
        Some(p) => {
            if let Some(tname) = &p.schema_type_name
                && matches!(
                    schema.types.get(tname),
                    Some(TypeDef::Chunks(_) | TypeDef::ChunkStreamSeq(_))
                )
            {
                ParentContainerRef::ChunksType {
                    type_name: tname.clone(),
                }
            } else {
                ParentContainerRef::UnassignedParentChunk {
                    parent_tree_path: p.tree_path.clone(),
                    parent_chunk_id: p.id,
                }
            }
        }
    };

    let (raw_bytes, spans, decoded) = match &chunk.content {
        DynamicChunkContent::Children(_) => (Vec::new(), Vec::new(), None),
        DynamicChunkContent::MiniChunks { raw_bytes, .. } => {
            (raw_bytes.clone(), Vec::new(), None)
        }
        DynamicChunkContent::Data {
            raw_bytes,
            decoded,
            spans,
        } => (raw_bytes.clone(), spans.clone(), decoded.clone()),
    };

    // Check if this chunk's payload references a named ByteBlock in schema.types
    let active_byte_block_type = chunk.schema_type_name.as_ref().and_then(|label| {
        let candidate = label
            .strip_prefix("Vec<")
            .and_then(|s| s.strip_suffix('>'))
            .unwrap_or(label.as_str());
        if matches!(schema.types.get(candidate), Some(TypeDef::ByteBlock(_))) {
            Some(candidate.to_owned())
        } else {
            None
        }
    });

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
        parent_container,
        active_byte_block_type,
    }
}

/// Converts a snake_case field name into a UpperCamelCase type name.
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

/// Inspects the existing [`ChunkFieldDef`] for `chunk_id` inside `parent_ref`, if one is defined.
pub fn find_existing_chunk_field<'a>(
    schema: &'a Schema,
    parent_ref: &ParentContainerRef,
    chunk_id: u32,
) -> Option<&'a ChunkFieldDef> {
    let container = match parent_ref {
        ParentContainerRef::Root => Some(&schema.root),
        ParentContainerRef::ChunksType { type_name } => match schema.types.get(type_name) {
            Some(TypeDef::Chunks(c) | TypeDef::ChunkStreamSeq(c)) => Some(c),
            _ => None,
        },
        _ => None,
    }?;
    container.fields.iter().find(|f| {
        matches!(
            f.source,
            ChunkFieldSource::Id { id, .. } if id == chunk_id
        )
    })
}

/// Inspects the existing [`MiniChunkFieldDef`] for `mc_id` inside `parent_ref`, if one is defined.
pub fn find_existing_mini_chunk_field<'a>(
    schema: &'a Schema,
    parent_ref: &ParentContainerRef,
    mc_id: u8,
) -> Option<&'a MiniChunkFieldDef> {
    let ParentContainerRef::MiniChunksType { type_name } = parent_ref else {
        return None;
    };
    let Some(TypeDef::MiniChunks(mc_container)) = schema.types.get(type_name) else {
        return None;
    };
    mc_container.fields.iter().find(|f| f.id == mc_id)
}

/// Assigns or updates a 32-bit chunk field definition in `parent_ref`.
///
/// If `payload_kind` is `"children"` or `"mini_chunks"` or `"byte_block"` and the referenced
/// named type does not yet exist in `schema.types`, it is automatically created so the schema
/// stays valid and ready to edit.
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

    // Ensure any referenced child container / mini-chunk container / byte block type exists
    match &payload {
        ChunkPayloadSpec::Children(type_name) => {
            schema
                .types
                .entry(type_name.clone())
                .or_insert_with(|| {
                    TypeDef::Chunks(ChunkContainerDef {
                        description: None,
                        mode: OrderMode::Map,
                        ignore_unknown: true,
                        fields: Vec::new(),
                    })
                });
        }
        ChunkPayloadSpec::MiniChunks(type_name) => {
            schema
                .types
                .entry(type_name.clone())
                .or_insert_with(|| {
                    TypeDef::MiniChunks(MiniChunkContainerDef {
                        description: None,
                        mode: OrderMode::Map,
                        ignore_unknown: true,
                        fields: Vec::new(),
                    })
                });
        }
        ChunkPayloadSpec::Data(DataType::Named(type_name)) => {
            schema
                .types
                .entry(type_name.clone())
                .or_insert_with(|| {
                    TypeDef::ByteBlock(ByteBlockDef {
                        description: None,
                        fields: Vec::new(),
                        pad_after: None,
                    })
                });
        }
        ChunkPayloadSpec::Data(DataType::List(inner)) => {
            if let DataType::Named(type_name) = &**inner {
                schema
                    .types
                    .entry(type_name.clone())
                    .or_insert_with(|| {
                        TypeDef::ByteBlock(ByteBlockDef {
                            description: None,
                            fields: Vec::new(),
                            pad_after: None,
                        })
                    });
            }
        }
        _ => {}
    }

    let container = match parent_ref {
        ParentContainerRef::Root => &mut schema.root,
        ParentContainerRef::ChunksType { type_name } => match schema.types.get_mut(type_name) {
            Some(TypeDef::Chunks(c) | TypeDef::ChunkStreamSeq(c)) => c,
            _ => return Err(format!("Parent container type `{type_name}` not found")),
        },
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

    if let Some(existing) = container.fields.iter_mut().find(|f| {
        matches!(
            f.source,
            ChunkFieldSource::Id { id, .. } if id == chunk_id
        )
    }) {
        existing.name = clean_name.to_owned();
        existing.cardinality = cardinality;
        existing.source = ChunkFieldSource::Id {
            id: chunk_id,
            payload,
        };
        existing.wire_format = wire_format;
    } else {
        container.fields.push(ChunkFieldDef {
            name: clean_name.to_owned(),
            description: None,
            cardinality,
            source: ChunkFieldSource::Id {
                id: chunk_id,
                payload,
            },
            wire_format,
        });
    }

    Ok(())
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

    if let DataType::Named(type_name) = &ty {
        schema
            .types
            .entry(type_name.clone())
            .or_insert_with(|| {
                TypeDef::ByteBlock(ByteBlockDef {
                    description: None,
                    fields: Vec::new(),
                    pad_after: None,
                })
            });
    }

    let ParentContainerRef::MiniChunksType { type_name } = parent_ref else {
        return Err("Assign a `MiniChunks` container type to the parent chunk first".to_owned());
    };
    let Some(TypeDef::MiniChunks(mc_container)) = schema.types.get_mut(type_name) else {
        return Err(format!("MiniChunks type `{type_name}` not found in schema"));
    };

    if let Some(existing) = mc_container.fields.iter_mut().find(|f| f.id == mc_id) {
        existing.name = clean_name.to_owned();
        existing.cardinality = cardinality;
        existing.ty = ty;
        existing.wire_format = wire_format;
    } else {
        mc_container.fields.push(MiniChunkFieldDef {
            id: mc_id,
            name: clean_name.to_owned(),
            description: None,
            cardinality,
            ty,
            wire_format,
        });
    }

    Ok(())
}

/// Ensures that the selected leaf chunk or mini-chunk is backed by a named [`TypeDef::ByteBlock`]
/// in `schema.types`, creating one if necessary, and appends a new field (`field_name`, `ty`, `wire_format`)
/// to that `ByteBlock`.
pub fn append_field_to_node_byte_block(
    schema: &mut Schema,
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

    let block_type_name = if let Some(existing_bb) = &node.active_byte_block_type {
        existing_bb.clone()
    } else {
        // Determine a sensible default struct name and link this node to it
        let base_field_name = node
            .schema_field_name
            .clone()
            .unwrap_or_else(|| {
                if node.is_mini_chunk {
                    format!("mc_{:02x}", node.id)
                } else {
                    format!("chunk_{:04x}", node.id)
                }
            });
        let new_bb_name = format!("{}Block", to_pascal_case(&base_field_name));

        if node.is_mini_chunk {
            let existing_card = find_existing_mini_chunk_field(
                schema,
                &node.parent_container,
                node.id as u8,
            )
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
                find_existing_chunk_field(schema, &node.parent_container, node.id)
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
    };

    let entry = schema
        .types
        .entry(block_type_name.clone())
        .or_insert_with(|| {
            TypeDef::ByteBlock(ByteBlockDef {
                description: None,
                fields: Vec::new(),
                pad_after: None,
            })
        });

    let TypeDef::ByteBlock(bb) = entry else {
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

    fn make_sample_chunk_buffer() -> Vec<u8> {
        // Container chunk 0x0200 holding leaf chunk 0x0201 (8 bytes of data: u32=3, f32=1.5)
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
    fn test_interactive_schema_construction_workflow() {
        let bytes = make_sample_chunk_buffer();
        let mut schema = Schema::new_empty("TestSkeletonFormat");
        let opts = LoadOptions::default();

        // 1. Load with blank schema
        let df0 = load_dynamic_file(&bytes, Some(&schema), &opts).unwrap();
        let rows = build_flat_tree(&df0, &BTreeSet::new(), 100);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].tree_path, "0");
        assert_eq!(rows[1].tree_path, "0/0");

        // 2. Select root chunk "0" (0x0200) and assign it a Children container `SkeletonContainer`
        let root_node = resolve_selected_node(&df0, &schema, "0").unwrap();
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
        let df1 = load_dynamic_file(&bytes, Some(&schema), &opts).unwrap();
        let child_node = resolve_selected_node(&df1, &schema, "0/0").unwrap();
        assert_eq!(child_node.id, 0x0201);
        assert_eq!(
            child_node.parent_container,
            ParentContainerRef::ChunksType {
                type_name: "SkeletonContainer".to_owned()
            }
        );

        // 4. Use "+ Add to ByteBlock" twice on child chunk 0x0201 (just like clicking in the Byte Inspector!)
        let bb_name = append_field_to_node_byte_block(
            &mut schema,
            &child_node,
            "bone_count",
            DataType::Usize,
            Some(WireFormat::U32Usize),
            None,
        )
        .unwrap();
        assert_eq!(bb_name, "Chunk0201Block");

        let df2 = load_dynamic_file(&bytes, Some(&schema), &opts).unwrap();
        let child_node2 = resolve_selected_node(&df2, &schema, "0/0").unwrap();
        assert_eq!(
            child_node2.active_byte_block_type.as_deref(),
            Some("Chunk0201Block")
        );
        append_field_to_node_byte_block(
            &mut schema,
            &child_node2,
            "scale",
            DataType::F32,
            None,
            None,
        )
        .unwrap();

        // 5. Re-evaluate: all 8 bytes of 0x0201 should now be covered by 2 spans with 0 diagnostics!
        let df3 = load_dynamic_file(&bytes, Some(&schema), &opts).unwrap();
        assert!(df3.diagnostics.is_empty(), "diags: {:?}", df3.diagnostics);
        let child_node3 = resolve_selected_node(&df3, &schema, "0/0").unwrap();
        assert_eq!(child_node3.spans.len(), 2);
        assert_eq!(child_node3.spans[0].start, 0);
        assert_eq!(child_node3.spans[0].end, 4);
        assert_eq!(child_node3.spans[0].field_path, "chunk_0201.bone_count");
        assert_eq!(child_node3.spans[1].start, 4);
        assert_eq!(child_node3.spans[1].end, 8);
        assert_eq!(child_node3.spans[1].field_path, "chunk_0201.scale");
    }
}
