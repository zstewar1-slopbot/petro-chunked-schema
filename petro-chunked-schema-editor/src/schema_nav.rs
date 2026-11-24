//! Schema-stack navigation, absent-definition discovery, and hover-synchronization logic for
//! `petro-chunked-schema-editor`.
//!
//! Translates the active [`ResolvedSchema`] and optional [`DynamicFile`] into a non-indenting
//! stack of ancestor schema levels ([`SchemaStackStep`]), enumerates all defined child fields and
//! enum variants at each level (including optional or unrepresented ones with `0` file matches),
//! and matches hovered right-rail levels or fields against left-rail [`FlatTreeRow`] items.

use std::collections::BTreeSet;

use petro_chunked_schema::dynamic::{DynamicChunk, DynamicChunkContent, DynamicFile};
use petro_chunked_schema::resolve::{ResolvedSchema, SchemaId};
use petro_chunked_schema::schema::{
    Cardinality, ChunkFieldDef, ChunkFieldSource, ChunkPayloadSpec, DataType, TypeDef,
};

use crate::editor_ops::FlatTreeRow;

/// Edge from a parent schema level to one of its child fields or enum variants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchemaParentEdge {
    /// Field inside a `TypeDef::Chunks` or `TypeDef::ChunkStreamSeq`.
    ChunkField {
        /// Module owning the parent container type.
        parent_module_id: SchemaId,
        /// Local type name of the parent container.
        parent_type_name: String,
        /// Field name inside the parent container.
        field_name: String,
        /// Matched 32-bit chunk ID, or `None` for a `ChunkFieldSource::Stream` delegation.
        chunk_id: Option<u32>,
    },
    /// Field inside a `TypeDef::MiniChunks`.
    MiniChunkField {
        /// Module owning the parent `MiniChunks` type.
        parent_module_id: SchemaId,
        /// Local type name of the parent `MiniChunks` container.
        parent_type_name: String,
        /// Field name inside the parent mini-chunk container.
        field_name: String,
        /// 8-bit mini-chunk ID.
        mc_id: u8,
    },
    /// Variant inside a `TypeDef::ChunkEnum`.
    EnumVariant {
        /// Module owning the parent `ChunkEnum` type.
        parent_module_id: SchemaId,
        /// Local type name of the parent `ChunkEnum`.
        parent_type_name: String,
        /// UpperCamelCase variant name.
        variant_name: String,
        /// Matched 32-bit chunk ID, or `None` for a `ChunkFieldSource::Stream` variant.
        chunk_id: Option<u32>,
    },
}

impl SchemaParentEdge {
    /// Short field or variant name for display in collapsed parent headers.
    pub fn short_name(&self) -> &str {
        match self {
            Self::ChunkField { field_name, .. } | Self::MiniChunkField { field_name, .. } => {
                field_name
            }
            Self::EnumVariant { variant_name, .. } => variant_name,
        }
    }

    /// Concise display label including chunk/mini-chunk ID or `stream`.
    pub fn display_label(&self) -> String {
        match self {
            Self::ChunkField {
                field_name,
                chunk_id: Some(id),
                ..
            } => format!("{field_name} (0x{id:04X})"),
            Self::ChunkField {
                field_name,
                chunk_id: None,
                ..
            } => format!("{field_name} (stream)"),
            Self::MiniChunkField {
                field_name, mc_id, ..
            } => format!("{field_name} (mc {mc_id:02X})"),
            Self::EnumVariant {
                variant_name,
                chunk_id: Some(id),
                ..
            } => format!("{variant_name} (0x{id:04X})"),
            Self::EnumVariant {
                variant_name,
                chunk_id: None,
                ..
            } => format!("{variant_name} (stream)"),
        }
    }

    /// Converts this edge into a [`HoveredSchemaTarget`] for left-rail highlighting.
    pub fn hover_target(&self) -> HoveredSchemaTarget {
        match self {
            Self::ChunkField {
                parent_module_id,
                parent_type_name,
                field_name,
                chunk_id,
            } => HoveredSchemaTarget::Field {
                parent_module_id: *parent_module_id,
                parent_type_name: parent_type_name.clone(),
                field_name: field_name.clone(),
                chunk_id: *chunk_id,
                is_mini_chunk: false,
            },
            Self::MiniChunkField {
                parent_module_id,
                parent_type_name,
                field_name,
                mc_id,
            } => HoveredSchemaTarget::Field {
                parent_module_id: *parent_module_id,
                parent_type_name: parent_type_name.clone(),
                field_name: field_name.clone(),
                chunk_id: Some(*mc_id as u32),
                is_mini_chunk: true,
            },
            Self::EnumVariant {
                parent_module_id,
                parent_type_name,
                variant_name,
                chunk_id,
            } => HoveredSchemaTarget::Field {
                parent_module_id: *parent_module_id,
                parent_type_name: parent_type_name.clone(),
                field_name: variant_name.clone(),
                chunk_id: *chunk_id,
                is_mini_chunk: false,
            },
        }
    }
}

/// Entity represented by a single card in the right-rail schema stack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchemaLevelTarget {
    /// A named [`TypeDef`] in `module_id` (such as `AloModel`, `Object`, `Mesh`, `MeshInfo`).
    Type {
        /// Owning module in the [`ResolvedSchema`].
        module_id: SchemaId,
        /// Local type key in that module's `schema.types`.
        type_name: String,
    },
    /// A primitive/inline/ignored field that does not target a named [`TypeDef`].
    LeafField {
        /// Module owning the parent container.
        parent_module_id: SchemaId,
        /// Local type name of the parent container.
        parent_type_name: String,
        /// `true` for 8-bit mini-chunks; `false` for 32-bit chunks.
        is_mini_chunk: bool,
        /// 32-bit chunk ID or widened 8-bit mini-chunk ID.
        id: u32,
        /// Defined field or variant name.
        field_name: String,
        /// Payload data type, or `None` for `ChunkPayloadSpec::Ignore`.
        data_type: Option<DataType>,
    },
    /// A file node whose chunk/mini-chunk ID is not yet defined in its parent schema container.
    UnassignedFileNode {
        /// Tree path of the unassigned file node.
        tree_path: String,
        /// 32-bit chunk ID or widened 8-bit mini-chunk ID.
        id: u32,
        /// `true` if this node is an 8-bit mini-chunk.
        is_mini_chunk: bool,
        /// `true` if this is a 32-bit container chunk (`0x8000_0000` flag set).
        is_container: bool,
        /// `true` if this leaf chunk's bytes can be parsed as mini-chunks.
        looks_like_mini_chunks: bool,
        /// Owning parent container module, if the parent itself is mapped to a schema container.
        parent_module_id: Option<SchemaId>,
        /// Owning parent container type name, if the parent itself is mapped to a schema container.
        parent_type_name: Option<String>,
    },
}

/// One level in the right-rail schema stack, ordered from the schema root (`index 0`) down to the
/// currently focused level.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaStackStep {
    /// Parent field or enum variant that selected this level (`None` for root).
    pub incoming_edge: Option<SchemaParentEdge>,
    /// Schema type, primitive leaf field, or unassigned file node represented at this level.
    pub target: SchemaLevelTarget,
    /// Corresponding tree path in the open file, if this step was reached via a concrete node.
    pub file_tree_path: Option<String>,
}

/// One selectable child field or variant displayed inside an expanded schema level card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaChildItem {
    /// Edge describing this child in the parent type.
    pub edge: SchemaParentEdge,
    /// Target level entered when the user clicks this child.
    pub target: SchemaLevelTarget,
    /// Short ID or source tag (`"0x0402"`, `"mc 01"`, `"stream"`).
    pub id_label: String,
    /// Field or enum variant name.
    pub field_name: String,
    /// Compact target type label (`"MeshInfo"`, `"c_string"`, `"Object"`, `"ignore"`).
    pub type_summary: String,
    /// Short cardinality badge (`"req"`, `"opt"`, `"rep"`, `"def"`, `"var"`).
    pub cardinality_badge: String,
    /// Number of matching nodes in the currently open file (`0` if absent or no file open).
    pub file_match_count: usize,
    /// Tree path of the first matching node in the open file, if any.
    pub first_file_tree_path: Option<String>,
    /// Hover target used when the pointer enters this child row.
    pub hover_target: HoveredSchemaTarget,
}

/// Schema element currently hovered in the right rail, used to highlight all matching rows in the
/// left-rail chunk tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HoveredSchemaTarget {
    /// Hovering a named schema type card (e.g. `Mesh`, `Object`, `MeshInfo`, `AloModel`).
    Type {
        /// Module owning the hovered type.
        module_id: SchemaId,
        /// Local type key in that module.
        type_name: String,
    },
    /// Hovering a specific field or enum variant row (or leaf field card).
    Field {
        /// Module owning the parent container or enum.
        parent_module_id: SchemaId,
        /// Local type name of the parent container or enum.
        parent_type_name: String,
        /// Field or variant name.
        field_name: String,
        /// Matched chunk/mini-chunk ID, or `None` for a `Stream` field.
        chunk_id: Option<u32>,
        /// `true` for 8-bit mini-chunks; `false` for 32-bit chunks.
        is_mini_chunk: bool,
    },
}

/// Builds a single-element stack containing only the schema's resolved root type.
pub fn build_root_stack(resolved: &ResolvedSchema) -> Vec<SchemaStackStep> {
    let (root_mod, root_name, _) = resolved.root_type();
    vec![SchemaStackStep {
        incoming_edge: None,
        target: SchemaLevelTarget::Type {
            module_id: root_mod,
            type_name: root_name.to_owned(),
        },
        file_tree_path: None,
    }]
}

/// Resolves the full ancestor [`SchemaStackStep`] chain from `resolved`'s root down to the file
/// node at `tree_path`, inserting intermediate `ChunkEnum` and `ChunkStreamSeq` levels.
pub fn resolve_schema_stack_for_file_node(
    file: &DynamicFile,
    resolved: &ResolvedSchema,
    tree_path: &str,
) -> Vec<SchemaStackStep> {
    let mut stack = build_root_stack(resolved);
    if tree_path.is_empty() {
        return stack;
    }

    let segments: Vec<&str> = tree_path.split('/').collect();
    let Ok(root_idx) = segments[0].parse::<usize>() else {
        return stack;
    };
    let Some(mut cur_chunk) = file.chunks.get(root_idx) else {
        return stack;
    };

    push_chunk_steps(&mut stack, resolved, cur_chunk);

    for &seg in segments.iter().skip(1) {
        if let Some(mc_idx_str) = seg.strip_prefix("mc") {
            let Ok(mc_idx) = mc_idx_str.parse::<usize>() else {
                break;
            };
            if let DynamicChunkContent::MiniChunks { mini_chunks, .. } = &cur_chunk.content
                && let Some(mc) = mini_chunks.get(mc_idx)
            {
                push_mini_chunk_step(&mut stack, resolved, mc.id, &mc.tree_path);
            }
            break;
        } else {
            let Ok(child_idx) = seg.parse::<usize>() else {
                break;
            };
            if let DynamicChunkContent::Children(children) = &cur_chunk.content
                && let Some(child) = children.get(child_idx)
            {
                cur_chunk = child;
                push_chunk_steps(&mut stack, resolved, cur_chunk);
            } else {
                break;
            }
        }
    }

    stack
}

/// Re-synchronizes `current_stack` after an in-memory schema edit.
///
/// If the bottom step tracks a concrete file node (or `current_stack` is empty), re-resolves from
/// `file` + `selected_tree_path`. If the user is inspecting a schema-only child (absent from the
/// open file), replays the stack's parent edges against the updated `resolved` schema so the user
/// stays on the same level while editing.
pub fn refresh_schema_stack(
    resolved: &ResolvedSchema,
    current_stack: &[SchemaStackStep],
    file: Option<&DynamicFile>,
    selected_tree_path: &str,
) -> Vec<SchemaStackStep> {
    if current_stack.is_empty() {
        return match file {
            Some(df) if !selected_tree_path.is_empty() => {
                resolve_schema_stack_for_file_node(df, resolved, selected_tree_path)
            }
            _ => build_root_stack(resolved),
        };
    }

    if let Some(df) = file
        && let Some(last) = current_stack.last()
        && let Some(path) = &last.file_tree_path
    {
        return resolve_schema_stack_for_file_node(df, resolved, path);
    }

    let mut refreshed = build_root_stack(resolved);
    for old_step in current_stack.iter().skip(1) {
        let Some(edge) = &old_step.incoming_edge else {
            break;
        };
        let Some((cur_mod, cur_type)) = current_step_type(&refreshed) else {
            break;
        };
        let children = list_level_children(resolved, cur_mod, cur_type, &[]);
        let matched = children.into_iter().find(|child| match (&child.edge, edge) {
            (
                SchemaParentEdge::ChunkField {
                    chunk_id: Some(a), ..
                },
                SchemaParentEdge::ChunkField {
                    chunk_id: Some(b), ..
                },
            ) => a == b,
            (
                SchemaParentEdge::ChunkField {
                    chunk_id: None,
                    field_name: a,
                    ..
                },
                SchemaParentEdge::ChunkField {
                    chunk_id: None,
                    field_name: b,
                    ..
                },
            ) => a == b,
            (
                SchemaParentEdge::MiniChunkField { mc_id: a, .. },
                SchemaParentEdge::MiniChunkField { mc_id: b, .. },
            ) => a == b,
            (
                SchemaParentEdge::EnumVariant {
                    variant_name: a, ..
                },
                SchemaParentEdge::EnumVariant {
                    variant_name: b, ..
                },
            ) => a == b,
            _ => false,
        });

        let Some(child) = matched else {
            break;
        };
        refreshed.push(SchemaStackStep {
            incoming_edge: Some(child.edge),
            target: child.target,
            file_tree_path: None,
        });
    }

    refreshed
}

fn current_step_type(stack: &[SchemaStackStep]) -> Option<(SchemaId, &str)> {
    match &stack.last()?.target {
        SchemaLevelTarget::Type {
            module_id,
            type_name,
        } => Some((*module_id, type_name.as_str())),
        _ => None,
    }
}

fn push_chunk_steps(
    stack: &mut Vec<SchemaStackStep>,
    resolved: &ResolvedSchema,
    chunk: &DynamicChunk,
) {
    let parent_type = current_step_type(stack).map(|(m, n)| (m, n.to_owned()));
    if let Some((cur_mod, cur_type)) = parent_type {
        let mut visited = BTreeSet::new();
        if let Some(mut steps) = find_chunk_id_path(
            resolved,
            cur_mod,
            &cur_type,
            chunk.id,
            &chunk.tree_path,
            &mut visited,
        ) {
            stack.append(&mut steps);
            return;
        }
        stack.push(SchemaStackStep {
            incoming_edge: None,
            target: SchemaLevelTarget::UnassignedFileNode {
                tree_path: chunk.tree_path.clone(),
                id: chunk.id,
                is_mini_chunk: false,
                is_container: chunk.is_container,
                looks_like_mini_chunks: chunk.looks_like_mini_chunks,
                parent_module_id: Some(cur_mod),
                parent_type_name: Some(cur_type),
            },
            file_tree_path: Some(chunk.tree_path.clone()),
        });
    } else {
        stack.push(SchemaStackStep {
            incoming_edge: None,
            target: SchemaLevelTarget::UnassignedFileNode {
                tree_path: chunk.tree_path.clone(),
                id: chunk.id,
                is_mini_chunk: false,
                is_container: chunk.is_container,
                looks_like_mini_chunks: chunk.looks_like_mini_chunks,
                parent_module_id: None,
                parent_type_name: None,
            },
            file_tree_path: Some(chunk.tree_path.clone()),
        });
    }
}

fn push_mini_chunk_step(
    stack: &mut Vec<SchemaStackStep>,
    resolved: &ResolvedSchema,
    mc_id: u8,
    tree_path: &str,
) {
    let parent_type = current_step_type(stack).map(|(m, n)| (m, n.to_owned()));
    if let Some((cur_mod, cur_type)) = parent_type {
        if let Some(TypeDef::MiniChunks(mc_def)) =
            resolved.module(cur_mod).schema.types.get(&cur_type)
            && let Some(field) = mc_def.fields.iter().find(|f| f.id == mc_id)
        {
            let edge = SchemaParentEdge::MiniChunkField {
                parent_module_id: cur_mod,
                parent_type_name: cur_type.clone(),
                field_name: field.name.clone(),
                mc_id,
            };
            let target = target_for_data_type(
                resolved,
                cur_mod,
                &cur_type,
                true,
                mc_id as u32,
                &field.name,
                &field.ty,
            );
            stack.push(SchemaStackStep {
                incoming_edge: Some(edge),
                target,
                file_tree_path: Some(tree_path.to_owned()),
            });
            return;
        }
        stack.push(SchemaStackStep {
            incoming_edge: None,
            target: SchemaLevelTarget::UnassignedFileNode {
                tree_path: tree_path.to_owned(),
                id: mc_id as u32,
                is_mini_chunk: true,
                is_container: false,
                looks_like_mini_chunks: false,
                parent_module_id: Some(cur_mod),
                parent_type_name: Some(cur_type),
            },
            file_tree_path: Some(tree_path.to_owned()),
        });
    } else {
        stack.push(SchemaStackStep {
            incoming_edge: None,
            target: SchemaLevelTarget::UnassignedFileNode {
                tree_path: tree_path.to_owned(),
                id: mc_id as u32,
                is_mini_chunk: true,
                is_container: false,
                looks_like_mini_chunks: false,
                parent_module_id: None,
                parent_type_name: None,
            },
            file_tree_path: Some(tree_path.to_owned()),
        });
    }
}

fn find_chunk_id_path(
    resolved: &ResolvedSchema,
    mod_id: SchemaId,
    type_name: &str,
    target_id: u32,
    file_tree_path: &str,
    visited: &mut BTreeSet<(SchemaId, String)>,
) -> Option<Vec<SchemaStackStep>> {
    if !visited.insert((mod_id, type_name.to_owned())) {
        return None;
    }

    let typedef = resolved.module(mod_id).schema.types.get(type_name)?;

    let result = match typedef {
        TypeDef::Chunks(cc) => find_in_chunk_fields(
            resolved,
            mod_id,
            type_name,
            &cc.fields,
            target_id,
            file_tree_path,
            visited,
        ),
        TypeDef::ChunkStreamSeq(seq) => find_in_chunk_fields(
            resolved,
            mod_id,
            type_name,
            &seq.fields,
            target_id,
            file_tree_path,
            visited,
        ),
        TypeDef::ChunkEnum(ce) => {
            // Check direct chunk ID variants first so direct hits take priority.
            for variant in &ce.variants {
                if let ChunkFieldSource::Id { id, payload } = &variant.source
                    && *id == target_id
                {
                    let edge = SchemaParentEdge::EnumVariant {
                        parent_module_id: mod_id,
                        parent_type_name: type_name.to_owned(),
                        variant_name: variant.name.clone(),
                        chunk_id: Some(*id),
                    };
                    let target = target_for_chunk_payload(
                        resolved,
                        mod_id,
                        type_name,
                        *id,
                        &variant.name,
                        payload,
                    );
                    return Some(vec![SchemaStackStep {
                        incoming_edge: Some(edge),
                        target,
                        file_tree_path: Some(file_tree_path.to_owned()),
                    }]);
                }
            }
            // Recurse through stream variants.
            for variant in &ce.variants {
                if let ChunkFieldSource::Stream { item_type } = &variant.source
                    && let Some((next_mod, next_name, _)) = resolved.resolve_type(mod_id, item_type)
                    && let Some(mut tail) = find_chunk_id_path(
                        resolved,
                        next_mod,
                        next_name,
                        target_id,
                        file_tree_path,
                        visited,
                    )
                {
                    let mut steps = Vec::with_capacity(tail.len() + 1);
                    steps.push(SchemaStackStep {
                        incoming_edge: Some(SchemaParentEdge::EnumVariant {
                            parent_module_id: mod_id,
                            parent_type_name: type_name.to_owned(),
                            variant_name: variant.name.clone(),
                            chunk_id: None,
                        }),
                        target: SchemaLevelTarget::Type {
                            module_id: next_mod,
                            type_name: next_name.to_owned(),
                        },
                        file_tree_path: None,
                    });
                    steps.append(&mut tail);
                    return Some(steps);
                }
            }
            None
        }
        TypeDef::MiniChunks(_) | TypeDef::ByteBlock(_) => None,
    };

    visited.remove(&(mod_id, type_name.to_owned()));
    result
}

fn find_in_chunk_fields(
    resolved: &ResolvedSchema,
    mod_id: SchemaId,
    type_name: &str,
    fields: &[ChunkFieldDef],
    target_id: u32,
    file_tree_path: &str,
    visited: &mut BTreeSet<(SchemaId, String)>,
) -> Option<Vec<SchemaStackStep>> {
    // Check direct ID matches first.
    for field in fields {
        if let ChunkFieldSource::Id { id, payload } = &field.source
            && *id == target_id
        {
            let edge = SchemaParentEdge::ChunkField {
                parent_module_id: mod_id,
                parent_type_name: type_name.to_owned(),
                field_name: field.name.clone(),
                chunk_id: Some(*id),
            };
            let target =
                target_for_chunk_payload(resolved, mod_id, type_name, *id, &field.name, payload);
            return Some(vec![SchemaStackStep {
                incoming_edge: Some(edge),
                target,
                file_tree_path: Some(file_tree_path.to_owned()),
            }]);
        }
    }

    // Then recurse into stream-delegated fields.
    for field in fields {
        if let ChunkFieldSource::Stream { item_type } = &field.source
            && let Some((next_mod, next_name, _)) = resolved.resolve_type(mod_id, item_type)
            && let Some(mut tail) = find_chunk_id_path(
                resolved,
                next_mod,
                next_name,
                target_id,
                file_tree_path,
                visited,
            )
        {
            let mut steps = Vec::with_capacity(tail.len() + 1);
            steps.push(SchemaStackStep {
                incoming_edge: Some(SchemaParentEdge::ChunkField {
                    parent_module_id: mod_id,
                    parent_type_name: type_name.to_owned(),
                    field_name: field.name.clone(),
                    chunk_id: None,
                }),
                target: SchemaLevelTarget::Type {
                    module_id: next_mod,
                    type_name: next_name.to_owned(),
                },
                file_tree_path: None,
            });
            steps.append(&mut tail);
            return Some(steps);
        }
    }

    None
}

fn target_for_chunk_payload(
    resolved: &ResolvedSchema,
    parent_mod: SchemaId,
    parent_type: &str,
    chunk_id: u32,
    field_name: &str,
    payload: &ChunkPayloadSpec,
) -> SchemaLevelTarget {
    match payload {
        ChunkPayloadSpec::Children(tref) | ChunkPayloadSpec::MiniChunks(tref) => {
            if let Some((tmod, tname, _)) = resolved.resolve_type(parent_mod, tref) {
                SchemaLevelTarget::Type {
                    module_id: tmod,
                    type_name: tname.to_owned(),
                }
            } else {
                SchemaLevelTarget::LeafField {
                    parent_module_id: parent_mod,
                    parent_type_name: parent_type.to_owned(),
                    is_mini_chunk: false,
                    id: chunk_id,
                    field_name: field_name.to_owned(),
                    data_type: Some(DataType::Named(tref.clone())),
                }
            }
        }
        ChunkPayloadSpec::Data(dt) => target_for_data_type(
            resolved,
            parent_mod,
            parent_type,
            false,
            chunk_id,
            field_name,
            dt,
        ),
        ChunkPayloadSpec::Ignore => SchemaLevelTarget::LeafField {
            parent_module_id: parent_mod,
            parent_type_name: parent_type.to_owned(),
            is_mini_chunk: false,
            id: chunk_id,
            field_name: field_name.to_owned(),
            data_type: None,
        },
    }
}

fn target_for_data_type(
    resolved: &ResolvedSchema,
    parent_mod: SchemaId,
    parent_type: &str,
    is_mini_chunk: bool,
    id: u32,
    field_name: &str,
    dt: &DataType,
) -> SchemaLevelTarget {
    if let Some((tmod, tname, _)) = resolved.resolve_data_type_target(parent_mod, dt) {
        SchemaLevelTarget::Type {
            module_id: tmod,
            type_name: tname.to_owned(),
        }
    } else {
        SchemaLevelTarget::LeafField {
            parent_module_id: parent_mod,
            parent_type_name: parent_type.to_owned(),
            is_mini_chunk,
            id,
            field_name: field_name.to_owned(),
            data_type: Some(dt.clone()),
        }
    }
}

/// Lists all defined child fields or enum variants of `(module_id, type_name)` — including
/// optional or unrepresented definitions with `0` occurrences in `all_file_rows`.
pub fn list_level_children(
    resolved: &ResolvedSchema,
    module_id: SchemaId,
    type_name: &str,
    all_file_rows: &[FlatTreeRow],
) -> Vec<SchemaChildItem> {
    let Some(typedef) = resolved.module(module_id).schema.types.get(type_name) else {
        return Vec::new();
    };

    match typedef {
        TypeDef::Chunks(cc) => {
            chunk_fields_to_items(resolved, module_id, type_name, &cc.fields, all_file_rows)
        }
        TypeDef::ChunkStreamSeq(seq) => {
            chunk_fields_to_items(resolved, module_id, type_name, &seq.fields, all_file_rows)
        }
        TypeDef::ChunkEnum(ce) => ce
            .variants
            .iter()
            .map(|variant| {
                let (id_label, chunk_id, target, type_summary) = match &variant.source {
                    ChunkFieldSource::Id { id, payload } => (
                        format!("0x{id:04X}"),
                        Some(*id),
                        target_for_chunk_payload(
                            resolved,
                            module_id,
                            type_name,
                            *id,
                            &variant.name,
                            payload,
                        ),
                        format_payload_summary(payload),
                    ),
                    ChunkFieldSource::Stream { item_type } => {
                        let target = if let Some((tmod, tname, _)) =
                            resolved.resolve_type(module_id, item_type)
                        {
                            SchemaLevelTarget::Type {
                                module_id: tmod,
                                type_name: tname.to_owned(),
                            }
                        } else {
                            SchemaLevelTarget::LeafField {
                                parent_module_id: module_id,
                                parent_type_name: type_name.to_owned(),
                                is_mini_chunk: false,
                                id: 0,
                                field_name: variant.name.clone(),
                                data_type: Some(DataType::Named(item_type.clone())),
                            }
                        };
                        ("stream".to_owned(), None, target, item_type.clone())
                    }
                };
                let edge = SchemaParentEdge::EnumVariant {
                    parent_module_id: module_id,
                    parent_type_name: type_name.to_owned(),
                    variant_name: variant.name.clone(),
                    chunk_id,
                };
                let hover_target = edge.hover_target();
                let (file_match_count, first_file_tree_path) =
                    match_stats(resolved, all_file_rows, &hover_target);
                SchemaChildItem {
                    edge,
                    target,
                    id_label,
                    field_name: variant.name.clone(),
                    type_summary,
                    cardinality_badge: "var".to_owned(),
                    file_match_count,
                    first_file_tree_path,
                    hover_target,
                }
            })
            .collect(),
        TypeDef::MiniChunks(mc_def) => mc_def
            .fields
            .iter()
            .map(|field| {
                let edge = SchemaParentEdge::MiniChunkField {
                    parent_module_id: module_id,
                    parent_type_name: type_name.to_owned(),
                    field_name: field.name.clone(),
                    mc_id: field.id,
                };
                let target = target_for_data_type(
                    resolved,
                    module_id,
                    type_name,
                    true,
                    field.id as u32,
                    &field.name,
                    &field.ty,
                );
                let hover_target = edge.hover_target();
                let (file_match_count, first_file_tree_path) =
                    match_stats(resolved, all_file_rows, &hover_target);
                SchemaChildItem {
                    edge,
                    target,
                    id_label: format!("mc {:02X}", field.id),
                    field_name: field.name.clone(),
                    type_summary: field.ty.label(),
                    cardinality_badge: cardinality_short_badge(&field.cardinality).to_owned(),
                    file_match_count,
                    first_file_tree_path,
                    hover_target,
                }
            })
            .collect(),
        TypeDef::ByteBlock(_) => Vec::new(),
    }
}

fn chunk_fields_to_items(
    resolved: &ResolvedSchema,
    module_id: SchemaId,
    type_name: &str,
    fields: &[ChunkFieldDef],
    all_file_rows: &[FlatTreeRow],
) -> Vec<SchemaChildItem> {
    fields
        .iter()
        .map(|field| {
            let (id_label, chunk_id, target, type_summary) = match &field.source {
                ChunkFieldSource::Id { id, payload } => (
                    format!("0x{id:04X}"),
                    Some(*id),
                    target_for_chunk_payload(
                        resolved,
                        module_id,
                        type_name,
                        *id,
                        &field.name,
                        payload,
                    ),
                    format_payload_summary(payload),
                ),
                ChunkFieldSource::Stream { item_type } => {
                    let target =
                        if let Some((tmod, tname, _)) = resolved.resolve_type(module_id, item_type)
                        {
                            SchemaLevelTarget::Type {
                                module_id: tmod,
                                type_name: tname.to_owned(),
                            }
                        } else {
                            SchemaLevelTarget::LeafField {
                                parent_module_id: module_id,
                                parent_type_name: type_name.to_owned(),
                                is_mini_chunk: false,
                                id: 0,
                                field_name: field.name.clone(),
                                data_type: Some(DataType::Named(item_type.clone())),
                            }
                        };
                    ("stream".to_owned(), None, target, item_type.clone())
                }
            };
            let edge = SchemaParentEdge::ChunkField {
                parent_module_id: module_id,
                parent_type_name: type_name.to_owned(),
                field_name: field.name.clone(),
                chunk_id,
            };
            let hover_target = edge.hover_target();
            let (file_match_count, first_file_tree_path) =
                match_stats(resolved, all_file_rows, &hover_target);
            SchemaChildItem {
                edge,
                target,
                id_label,
                field_name: field.name.clone(),
                type_summary,
                cardinality_badge: cardinality_short_badge(&field.cardinality).to_owned(),
                file_match_count,
                first_file_tree_path,
                hover_target,
            }
        })
        .collect()
}

fn format_payload_summary(payload: &ChunkPayloadSpec) -> String {
    match payload {
        ChunkPayloadSpec::Data(dt) => dt.label(),
        ChunkPayloadSpec::MiniChunks(name) => name.clone(),
        ChunkPayloadSpec::Children(name) => name.clone(),
        ChunkPayloadSpec::Ignore => "ignore".to_owned(),
    }
}

fn cardinality_short_badge(card: &Cardinality) -> &'static str {
    match card {
        Cardinality::Required => "req",
        Cardinality::Optional => "opt",
        Cardinality::Repeated => "rep",
        Cardinality::Default | Cardinality::DefaultExpr(_) => "def",
    }
}

/// Returns the [`HoveredSchemaTarget`] associated with a stack step, if it represents a defined
/// schema type or field.
pub fn step_hover_target(step: &SchemaStackStep) -> Option<HoveredSchemaTarget> {
    match &step.target {
        SchemaLevelTarget::Type {
            module_id,
            type_name,
        } => Some(HoveredSchemaTarget::Type {
            module_id: *module_id,
            type_name: type_name.clone(),
        }),
        SchemaLevelTarget::LeafField {
            parent_module_id,
            parent_type_name,
            is_mini_chunk,
            id,
            field_name,
            ..
        } => Some(HoveredSchemaTarget::Field {
            parent_module_id: *parent_module_id,
            parent_type_name: parent_type_name.clone(),
            field_name: field_name.clone(),
            chunk_id: Some(*id),
            is_mini_chunk: *is_mini_chunk,
        }),
        SchemaLevelTarget::UnassignedFileNode { .. } => None,
    }
}

/// Short kind badge for a named type in `resolved`.
pub fn type_kind_badge(
    resolved: &ResolvedSchema,
    module_id: SchemaId,
    type_name: &str,
) -> &'static str {
    match resolved.module(module_id).schema.types.get(type_name) {
        Some(TypeDef::Chunks(_)) => "Chunks",
        Some(TypeDef::MiniChunks(_)) => "MiniChunks",
        Some(TypeDef::ByteBlock(_)) => "ByteBlock",
        Some(TypeDef::ChunkEnum(_)) => "Enum",
        Some(TypeDef::ChunkStreamSeq(_)) => "StreamSeq",
        None => "Type",
    }
}

/// Returns `true` if `row` in the left-rail chunk tree corresponds to `target`.
pub fn matches_hover_target(
    resolved: &ResolvedSchema,
    row: &FlatTreeRow,
    target: &HoveredSchemaTarget,
) -> bool {
    match target {
        HoveredSchemaTarget::Type {
            module_id,
            type_name,
        } => matches_type_target(resolved, row, *module_id, type_name),
        HoveredSchemaTarget::Field {
            parent_module_id,
            parent_type_name,
            field_name,
            chunk_id,
            is_mini_chunk,
        } => matches_field_target(
            resolved,
            row,
            *parent_module_id,
            parent_type_name,
            field_name,
            *chunk_id,
            *is_mini_chunk,
        ),
    }
}

fn matches_type_target(
    resolved: &ResolvedSchema,
    row: &FlatTreeRow,
    module_id: SchemaId,
    type_name: &str,
) -> bool {
    // 1. Direct target type match (handles `Chunks`, `MiniChunks`, `ByteBlock`, `Vec<ByteBlock>`).
    if row.schema_module_id == Some(module_id) && row.schema_type_ref.as_deref() == Some(type_name)
    {
        return true;
    }

    // 2. Root container match: hovering the root schema card highlights all matched top-level rows.
    let (root_mod, root_name, _) = resolved.root_type();
    if module_id == root_mod
        && type_name == root_name
        && row.depth == 0
        && !row.is_mini_chunk
        && row.field_name.is_some()
    {
        return true;
    }

    // 3. Stream / enum / stream-referenced types (e.g. `Object`, `model::AloModel`):
    //    match any 32-bit chunk consumed by a direct ID field/variant reachable through streams.
    if row.is_mini_chunk || row.field_name.is_none() {
        return false;
    }
    let Some(typedef) = resolved.module(module_id).schema.types.get(type_name) else {
        return false;
    };
    let is_stream_kind = matches!(typedef, TypeDef::ChunkEnum(_) | TypeDef::ChunkStreamSeq(_))
        || resolved
            .referenced_stream_types()
            .contains(&(module_id, type_name.to_owned()));
    if !is_stream_kind {
        return false;
    }

    let mut owned_ids = BTreeSet::new();
    let mut visited = BTreeSet::new();
    collect_stream_chunk_ids(resolved, module_id, type_name, &mut owned_ids, &mut visited);

    row.field_owner_module_id
        .is_some_and(|owner| owned_ids.contains(&(owner, row.raw_id)))
}

fn matches_field_target(
    resolved: &ResolvedSchema,
    row: &FlatTreeRow,
    parent_module_id: SchemaId,
    parent_type_name: &str,
    field_name: &str,
    chunk_id: Option<u32>,
    is_mini_chunk: bool,
) -> bool {
    if row.is_mini_chunk != is_mini_chunk {
        return false;
    }

    // Mini-chunk field match: exact parent MiniChunks container + 8-bit ID.
    if is_mini_chunk {
        let Some(id) = chunk_id else {
            return false;
        };
        return row.raw_id == id
            && row.field_name.is_some()
            && row.parent_type_ref.as_ref().is_some_and(|(m, n)| {
                *m == parent_module_id && n == parent_type_name
            });
    }

    // Stream field/variant (no single chunk ID): delegate to its target stream item type.
    let Some(id) = chunk_id else {
        if let Some((target_mod, target_name)) =
            resolve_stream_field_target(resolved, parent_module_id, parent_type_name, field_name)
        {
            return matches_type_target(resolved, row, target_mod, target_name);
        }
        return false;
    };

    if row.raw_id != id || row.field_owner_module_id != Some(parent_module_id) {
        return false;
    }

    let Some(row_field) = row.field_name.as_deref() else {
        return false;
    };

    // If the parent is a concrete chunk container, verify parent_type_ref (or depth == 0 for root).
    let (root_mod, root_name, _) = resolved.root_type();
    if let Some((pm, pn)) = &row.parent_type_ref {
        if *pm == parent_module_id && pn == parent_type_name {
            return true;
        }
        // Otherwise, parent_type_name may be a ChunkEnum / ChunkStreamSeq reached via a stream
        // inside `(pm, pn)` — check formatted field label matches `field_name`.
        return field_label_matches(row_field, field_name);
    }

    // Depth 0: either parent is the root type directly, or parent is a stream/enum invoked at root.
    if parent_module_id == root_mod && parent_type_name == root_name {
        return true;
    }
    field_label_matches(row_field, field_name)
}

fn field_label_matches(row_label: &str, name: &str) -> bool {
    if row_label == name {
        return true;
    }
    if row_label.ends_with(&format!("::{name})")) {
        return true;
    }
    if row_label.ends_with(&format!(".{name}")) {
        return true;
    }
    if row_label.starts_with(&format!("{name} (")) || row_label.starts_with(&format!("{name}.")) {
        return true;
    }
    false
}

fn resolve_stream_field_target<'a>(
    resolved: &'a ResolvedSchema,
    parent_mod: SchemaId,
    parent_type: &str,
    field_name: &str,
) -> Option<(SchemaId, &'a str)> {
    let typedef = resolved.module(parent_mod).schema.types.get(parent_type)?;
    let item_type: &str = match typedef {
        TypeDef::Chunks(cc) => cc.fields.iter().find_map(|f| match &f.source {
            ChunkFieldSource::Stream { item_type } if f.name == field_name => {
                Some(item_type.as_str())
            }
            _ => None,
        })?,
        TypeDef::ChunkStreamSeq(seq) => seq.fields.iter().find_map(|f| match &f.source {
            ChunkFieldSource::Stream { item_type } if f.name == field_name => {
                Some(item_type.as_str())
            }
            _ => None,
        })?,
        TypeDef::ChunkEnum(ce) => ce.variants.iter().find_map(|v| match &v.source {
            ChunkFieldSource::Stream { item_type } if v.name == field_name => {
                Some(item_type.as_str())
            }
            _ => None,
        })?,
        _ => return None,
    };
    let (target_mod, target_name, _) = resolved.resolve_type(parent_mod, item_type)?;
    Some((target_mod, target_name))
}

fn collect_stream_chunk_ids(
    resolved: &ResolvedSchema,
    mod_id: SchemaId,
    type_name: &str,
    out: &mut BTreeSet<(SchemaId, u32)>,
    visited: &mut BTreeSet<(SchemaId, String)>,
) {
    if !visited.insert((mod_id, type_name.to_owned())) {
        return;
    }
    let Some(typedef) = resolved.module(mod_id).schema.types.get(type_name) else {
        return;
    };

    match typedef {
        TypeDef::Chunks(cc) => {
            for f in &cc.fields {
                collect_source_ids(resolved, mod_id, &f.source, out, visited);
            }
        }
        TypeDef::ChunkStreamSeq(seq) => {
            for f in &seq.fields {
                collect_source_ids(resolved, mod_id, &f.source, out, visited);
            }
        }
        TypeDef::ChunkEnum(ce) => {
            for v in &ce.variants {
                collect_source_ids(resolved, mod_id, &v.source, out, visited);
            }
        }
        TypeDef::MiniChunks(_) | TypeDef::ByteBlock(_) => {}
    }
}

fn collect_source_ids(
    resolved: &ResolvedSchema,
    mod_id: SchemaId,
    source: &ChunkFieldSource,
    out: &mut BTreeSet<(SchemaId, u32)>,
    visited: &mut BTreeSet<(SchemaId, String)>,
) {
    match source {
        ChunkFieldSource::Id { id, .. } => {
            out.insert((mod_id, *id));
        }
        ChunkFieldSource::Stream { item_type } => {
            if let Some((next_mod, next_name, _)) = resolved.resolve_type(mod_id, item_type) {
                collect_stream_chunk_ids(resolved, next_mod, next_name, out, visited);
            }
        }
    }
}

/// Counts how many rows in `all_file_rows` match `target`.
pub fn count_file_matches(
    resolved: &ResolvedSchema,
    all_file_rows: &[FlatTreeRow],
    target: &HoveredSchemaTarget,
) -> usize {
    all_file_rows
        .iter()
        .filter(|row| matches_hover_target(resolved, row, target))
        .count()
}

fn match_stats(
    resolved: &ResolvedSchema,
    all_file_rows: &[FlatTreeRow],
    target: &HoveredSchemaTarget,
) -> (usize, Option<String>) {
    let mut count = 0usize;
    let mut first = None;
    for row in all_file_rows {
        if matches_hover_target(resolved, row, target) {
            if first.is_none() {
                first = Some(row.tree_path.clone());
            }
            count += 1;
        }
    }
    (count, first)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor_ops::build_flat_tree;
    use petro_chunked_schema::dynamic::{LoadOptions, load_dynamic_file};
    use petro_chunked_schema::resolve::SchemaLoader;
    use petro_chunked_schema::source::SchemaRef;

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

    fn make_two_mesh_alo_model() -> Vec<u8> {
        let mut skel_hdr = Vec::from(0u32.to_le_bytes());
        skel_hdr.extend_from_slice(&[0u8; 124]);
        let skel = make_chunk(0x0200, true, &make_chunk(0x0201, false, &skel_hdr));

        let mesh_info_bytes = [0u8; 40];
        let mut mesh1_body = Vec::new();
        mesh1_body.extend(make_chunk(0x0401, false, b"Hull\0"));
        mesh1_body.extend(make_chunk(0x0402, false, &mesh_info_bytes));
        let mesh1 = make_chunk(0x0400, true, &mesh1_body);

        let mut mesh2_body = Vec::new();
        mesh2_body.extend(make_chunk(0x0401, false, b"Wing\0"));
        mesh2_body.extend(make_chunk(0x0402, false, &mesh_info_bytes));
        let mesh2 = make_chunk(0x0400, true, &mesh2_body);

        let mut file = Vec::new();
        file.extend(skel);
        file.extend(mesh1);
        file.extend(mesh2);
        file
    }

    #[test]
    fn test_schema_stack_resolves_intermediate_enum_and_absent_children() {
        let resolved = SchemaLoader::load(&SchemaRef::Builtin("AloModel".to_owned())).unwrap();
        let bytes = make_two_mesh_alo_model();
        let df = load_dynamic_file(&bytes, Some(&resolved), &LoadOptions::default()).unwrap();
        let all_rows = build_flat_tree(&df, &BTreeSet::new(), 500);

        // Select mesh1's `info` chunk ("1/1"):
        // Expected stack: AloModel -> Object -> Mesh -> MeshInfo
        let stack = resolve_schema_stack_for_file_node(&df, &resolved, "1/1");
        let names: Vec<String> = stack
            .iter()
            .map(|s| match &s.target {
                SchemaLevelTarget::Type { type_name, .. } => type_name.clone(),
                SchemaLevelTarget::LeafField { field_name, .. } => field_name.clone(),
                SchemaLevelTarget::UnassignedFileNode { id, .. } => format!("0x{id:04X}"),
            })
            .collect();
        assert_eq!(names, vec!["AloModel", "Object", "Mesh", "MeshInfo"]);

        // Expand `AloModel`: shows `connections` even though it is absent (`0` matches in file).
        let root_children = list_level_children(&resolved, SchemaId::ROOT, "AloModel", &all_rows);
        let conn = root_children
            .iter()
            .find(|c| c.field_name == "connections")
            .expect("connections field should be listed");
        assert_eq!(conn.file_match_count, 0);
        assert!(conn.first_file_tree_path.is_none());

        let objs = root_children
            .iter()
            .find(|c| c.field_name == "objects")
            .expect("objects stream field should be listed");
        assert_eq!(objs.file_match_count, 2);

        // Expand `Object`: shows `Light` with 0 matches and `Mesh` with 2 matches.
        let obj_children = list_level_children(&resolved, SchemaId::ROOT, "Object", &all_rows);
        let mesh_item = obj_children.iter().find(|c| c.field_name == "Mesh").unwrap();
        assert_eq!(mesh_item.file_match_count, 2);
        let light_item = obj_children
            .iter()
            .find(|c| c.field_name == "Light")
            .expect("Light variant should be listed even when absent");
        assert_eq!(light_item.file_match_count, 0);

        // Hover matching highlights BOTH MeshInfo chunks across mesh1 and mesh2.
        let hover_mesh_info = HoveredSchemaTarget::Type {
            module_id: SchemaId::ROOT,
            type_name: "MeshInfo".to_owned(),
        };
        assert_eq!(count_file_matches(&resolved, &all_rows, &hover_mesh_info), 2);

        // Hover matching on `Mesh`'s `name` leaf field highlights BOTH `0x0401` chunks.
        let hover_mesh_name = HoveredSchemaTarget::Field {
            parent_module_id: SchemaId::ROOT,
            parent_type_name: "Mesh".to_owned(),
            field_name: "name".to_owned(),
            chunk_id: Some(0x0401),
            is_mini_chunk: false,
        };
        assert_eq!(count_file_matches(&resolved, &all_rows, &hover_mesh_name), 2);
    }

    #[test]
    fn test_imported_alo_schema_stack_and_hover_sync() {
        let resolved = SchemaLoader::load(&SchemaRef::Builtin("Alo".to_owned())).unwrap();
        let bytes = make_two_mesh_alo_model();
        let df = load_dynamic_file(&bytes, Some(&resolved), &LoadOptions::default()).unwrap();
        let all_rows = build_flat_tree(&df, &BTreeSet::new(), 500);

        // Stack traverses cross-module import: AloFile -> model::AloModel -> model::Object -> model::Mesh -> model::MeshInfo
        let stack = resolve_schema_stack_for_file_node(&df, &resolved, "1/1");
        let display_names: Vec<String> = stack
            .iter()
            .map(|s| match &s.target {
                SchemaLevelTarget::Type {
                    module_id,
                    type_name,
                } => resolved.format_type_display(*module_id, type_name),
                _ => "other".to_owned(),
            })
            .collect();
        assert_eq!(
            display_names,
            vec![
                "AloFile",
                "model::AloModel",
                "model::Object",
                "model::Mesh",
                "model::MeshInfo",
            ]
        );

        // Hovering `model::Mesh` highlights both Mesh chunks even across module boundaries.
        let mesh_step = &stack[3];
        let hover_mesh = step_hover_target(mesh_step).unwrap();
        assert_eq!(count_file_matches(&resolved, &all_rows, &hover_mesh), 2);
    }
}
