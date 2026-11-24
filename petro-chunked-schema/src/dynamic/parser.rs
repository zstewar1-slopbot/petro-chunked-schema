//! Raw chunk and mini-chunk tree parser operating without a schema.

use petro_chunked::{ChunkContent, iter_chunks, iter_mini_chunks};

use crate::error::DynamicLoadError;

use super::LoadOptions;
use super::model::{DynamicChunk, DynamicChunkContent, DynamicMiniChunk};

/// Heuristic check: returns `true` if `bytes` is non-empty and its entire length can be segmented
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

pub(crate) fn parse_raw_chunk_list(
    bytes: &[u8],
    base_offset: usize,
    parent_path: &str,
    options: &LoadOptions,
) -> Result<Vec<DynamicChunk>, DynamicLoadError> {
    let mut out = Vec::new();
    let mut cursor = 0usize;

    for (idx, chunk_res) in iter_chunks(bytes).enumerate() {
        let header_offset = base_offset + cursor;
        let chunk = chunk_res.map_err(|e| DynamicLoadError::ChunkRead {
            offset: header_offset,
            message: e.to_string(),
        })?;
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
                let children =
                    parse_raw_chunk_list(child_slice, payload_offset, &tree_path, options)?;
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
            field_owner_module_id: None,
            schema_module_id: None,
            schema_type_ref: None,
            looks_like_mini_chunks: looks_mc,
            content,
            errors: Vec::new(),
        });

        cursor += 8 + size;
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

pub(crate) fn parse_raw_mini_chunks(
    bytes: &[u8],
    base_offset: usize,
    parent_path: &str,
) -> Vec<DynamicMiniChunk> {
    let mut out = Vec::new();
    let mut cursor = 0usize;
    for (idx, mc_res) in iter_mini_chunks(bytes).enumerate() {
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
            field_owner_module_id: None,
            schema_module_id: None,
            schema_type_ref: None,
            decoded: None,
            spans: Vec::new(),
            errors: Vec::new(),
        });
        cursor += 2 + len;
    }
    out
}
