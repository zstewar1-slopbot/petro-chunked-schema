//! Scanner for ASCII and null-terminated (ASCIIZ) string payloads across a [`DynamicFile`].

use serde::{Deserialize, Serialize};

use super::model::{DynamicChunk, DynamicChunkContent, DynamicFile};

/// Classification of an ASCII or ASCIIZ match discovered by [`find_strings`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum StringMatchKind {
    /// Entire chunk or mini-chunk payload is printable ASCII followed by one trailing `0x00`.
    WholeAsciiz,
    /// Entire chunk or mini-chunk payload is printable ASCII followed by two or more trailing `0x00` padding bytes.
    WholeAsciizPadded,
    /// Entire chunk or mini-chunk payload is printable ASCII with no trailing NUL terminator.
    WholeAscii,
    /// NUL-terminated printable ASCII run embedded inside a larger non-all-ASCII payload.
    EmbeddedAsciiz,
    /// Non-NUL-terminated printable ASCII run embedded inside a larger non-all-ASCII payload.
    EmbeddedAscii,
}

impl StringMatchKind {
    /// Returns `true` if this match spans the entire chunk or mini-chunk payload.
    pub fn is_whole_block(self) -> bool {
        matches!(
            self,
            Self::WholeAsciiz | Self::WholeAsciizPadded | Self::WholeAscii
        )
    }

    /// Returns `true` if this match ends with one or more `0x00` NUL bytes.
    pub fn is_nul_terminated(self) -> bool {
        matches!(
            self,
            Self::WholeAsciiz | Self::WholeAsciizPadded | Self::EmbeddedAsciiz
        )
    }

    /// Returns a compact badge label suitable for tables and CLI output.
    pub fn short_label(self) -> &'static str {
        match self {
            Self::WholeAsciiz => "ASCIIZ",
            Self::WholeAsciizPadded => "ASCIIZ+0",
            Self::WholeAscii => "ASCII",
            Self::EmbeddedAsciiz => "EMB-Z",
            Self::EmbeddedAscii => "EMB",
        }
    }
}

/// Configuration controlling how [`find_strings`] scans leaf chunks and mini-chunks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FindStringsOptions {
    /// Minimum character count for whole-block matches (clamped to at least `1`; zero-padded
    /// whole blocks always require at least `2` characters to suppress 32-bit small integers).
    pub min_whole_len: usize,
    /// Minimum character count for embedded runs inside larger binary blocks (clamped to at least `1`).
    pub min_embedded_len: usize,
    /// Include NUL-terminated ASCIIZ matches (`WholeAsciiz`, `WholeAsciizPadded`, `EmbeddedAsciiz`).
    pub include_asciiz: bool,
    /// Include non-NUL-terminated ASCII matches (`WholeAscii`, `EmbeddedAscii`).
    pub include_ascii: bool,
    /// When `true`, also scan inside non-all-ASCII leaf payloads for embedded ASCII/ASCIIZ runs.
    pub include_embedded: bool,
    /// Allow `\t`, `\n`, and `\r` control whitespace alongside printable ASCII (`0x20..=0x7E`).
    pub allow_whitespace_controls: bool,
    /// Maximum number of results returned (`0` disables the cap).
    pub max_results: usize,
}

impl Default for FindStringsOptions {
    fn default() -> Self {
        Self {
            min_whole_len: 2,
            min_embedded_len: 4,
            include_asciiz: true,
            include_ascii: true,
            include_embedded: false,
            allow_whitespace_controls: true,
            max_results: 2000,
        }
    }
}

/// A single ASCII or ASCIIZ match returned by [`find_strings`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FoundString {
    /// Hierarchical tree path of the matched chunk or mini-chunk (e.g. `"0/2"`, `"0/1/mc0"`).
    pub tree_path: String,
    /// `true` if the match came from an 8-bit mini-chunk rather than a 32-bit chunk.
    pub is_mini_chunk: bool,
    /// 32-bit chunk ID or zero-extended 8-bit mini-chunk ID.
    pub node_id: u32,
    /// Schema field name matched to this node, if any.
    pub schema_field_name: Option<String>,
    /// Schema type or container label matched to this node, if any.
    pub schema_type_label: Option<String>,
    /// Byte offset relative to the chunk or mini-chunk payload buffer (`0` for whole-block matches).
    pub payload_offset: usize,
    /// Absolute byte offset in the file (`node.payload_offset + payload_offset`).
    pub file_offset: usize,
    /// Total matched byte span in the payload (including trailing NUL byte(s) when present).
    pub byte_len: usize,
    /// Classification of the ASCII/ASCIIZ match.
    pub kind: StringMatchKind,
    /// Decoded ASCII text without trailing NUL bytes.
    pub text: String,
}

/// Scans all leaf chunks and mini-chunks in `file` for whole-block (and optionally embedded)
/// printable ASCII and NUL-terminated ASCIIZ strings.
pub fn find_strings(file: &DynamicFile, options: &FindStringsOptions) -> Vec<FoundString> {
    let mut out = Vec::new();
    scan_chunk_list(&file.chunks, options, &mut out);
    out
}

fn scan_chunk_list(
    chunks: &[DynamicChunk],
    options: &FindStringsOptions,
    out: &mut Vec<FoundString>,
) {
    for chunk in chunks {
        if options.max_results > 0 && out.len() >= options.max_results {
            return;
        }
        match &chunk.content {
            DynamicChunkContent::Children(children) => {
                scan_chunk_list(children, options, out);
            }
            DynamicChunkContent::MiniChunks { mini_chunks, .. } => {
                for mc in mini_chunks {
                    if options.max_results > 0 && out.len() >= options.max_results {
                        return;
                    }
                    let meta = NodeMeta {
                        tree_path: &mc.tree_path,
                        is_mini_chunk: true,
                        node_id: u32::from(mc.id),
                        schema_field_name: mc.schema_field_name.as_deref(),
                        schema_type_label: mc.schema_type_label.as_deref(),
                        base_file_offset: mc.payload_offset,
                    };
                    scan_payload(&meta, &mc.raw_bytes, options, out);
                }
            }
            DynamicChunkContent::Data { raw_bytes, .. } => {
                let meta = NodeMeta {
                    tree_path: &chunk.tree_path,
                    is_mini_chunk: false,
                    node_id: chunk.id,
                    schema_field_name: chunk.schema_field_name.as_deref(),
                    schema_type_label: chunk.schema_type_name.as_deref(),
                    base_file_offset: chunk.payload_offset,
                };
                scan_payload(&meta, raw_bytes, options, out);
            }
        }
    }
}

struct NodeMeta<'a> {
    tree_path: &'a str,
    is_mini_chunk: bool,
    node_id: u32,
    schema_field_name: Option<&'a str>,
    schema_type_label: Option<&'a str>,
    base_file_offset: usize,
}

impl NodeMeta<'_> {
    fn to_found(
        &self,
        payload_offset: usize,
        byte_len: usize,
        kind: StringMatchKind,
        text: &str,
    ) -> FoundString {
        FoundString {
            tree_path: self.tree_path.to_owned(),
            is_mini_chunk: self.is_mini_chunk,
            node_id: self.node_id,
            schema_field_name: self.schema_field_name.map(str::to_owned),
            schema_type_label: self.schema_type_label.map(str::to_owned),
            payload_offset,
            file_offset: self.base_file_offset + payload_offset,
            byte_len,
            kind,
            text: text.to_owned(),
        }
    }
}

fn scan_payload(
    meta: &NodeMeta<'_>,
    bytes: &[u8],
    options: &FindStringsOptions,
    out: &mut Vec<FoundString>,
) {
    if bytes.is_empty() || (options.max_results > 0 && out.len() >= options.max_results) {
        return;
    }

    if let Some((kind, text)) = classify_whole_block(bytes, options) {
        if kind_matches_filter(kind, options) {
            out.push(meta.to_found(0, bytes.len(), kind, text));
        }
        return;
    }

    if !options.include_embedded {
        return;
    }

    let min_emb_len = options.min_embedded_len.max(1);
    let mut i = 0;
    while i < bytes.len() {
        if options.max_results > 0 && out.len() >= options.max_results {
            return;
        }
        if !is_ascii_text_byte(bytes[i], options.allow_whitespace_controls) {
            i += 1;
            continue;
        }
        let start = i;
        while i < bytes.len() && is_ascii_text_byte(bytes[i], options.allow_whitespace_controls) {
            i += 1;
        }
        let end = i;
        let run = &bytes[start..end];
        let is_z = bytes.get(end) == Some(&0);

        if run.len() >= min_emb_len
            && has_visible_printable(run)
            && let Ok(text) = std::str::from_utf8(run)
        {
            let (kind, byte_len) = if is_z {
                (StringMatchKind::EmbeddedAsciiz, run.len() + 1)
            } else {
                (StringMatchKind::EmbeddedAscii, run.len())
            };
            if kind_matches_filter(kind, options) {
                out.push(meta.to_found(start, byte_len, kind, text));
            }
        }

        if is_z {
            i += 1;
        }
    }
}

fn classify_whole_block<'a>(
    bytes: &'a [u8],
    options: &FindStringsOptions,
) -> Option<(StringMatchKind, &'a str)> {
    if bytes.is_empty() {
        return None;
    }

    let min_len = options.min_whole_len.max(1);
    if let Some(nul_pos) = bytes.iter().position(|&b| b == 0) {
        if !bytes[nul_pos..].iter().all(|&b| b == 0) {
            return None;
        }
        let prefix = &bytes[..nul_pos];
        let trailing_nuls = bytes.len() - nul_pos;
        // Require at least 2 chars on multi-NUL padded buffers so little-endian small integers
        // like `65u32 = [0x41, 0x00, 0x00, 0x00]` are not flagged even when `min_whole_len == 1`.
        let (kind, req_len) = if trailing_nuls == 1 {
            (StringMatchKind::WholeAsciiz, min_len)
        } else {
            (StringMatchKind::WholeAsciizPadded, min_len.max(2))
        };
        if prefix.len() >= req_len
            && prefix
                .iter()
                .all(|&b| is_ascii_text_byte(b, options.allow_whitespace_controls))
            && has_visible_printable(prefix)
            && let Ok(text) = std::str::from_utf8(prefix)
        {
            return Some((kind, text));
        }
        None
    } else if bytes.len() >= min_len
        && bytes
            .iter()
            .all(|&b| is_ascii_text_byte(b, options.allow_whitespace_controls))
        && has_visible_printable(bytes)
        && let Ok(text) = std::str::from_utf8(bytes)
    {
        Some((StringMatchKind::WholeAscii, text))
    } else {
        None
    }
}

fn kind_matches_filter(kind: StringMatchKind, options: &FindStringsOptions) -> bool {
    (kind.is_nul_terminated() && options.include_asciiz)
        || (!kind.is_nul_terminated() && options.include_ascii)
}

fn is_ascii_text_byte(b: u8, allow_ws: bool) -> bool {
    matches!(b, 0x20..=0x7E) || (allow_ws && matches!(b, b'\t' | b'\n' | b'\r'))
}

fn has_visible_printable(slice: &[u8]) -> bool {
    slice.iter().any(|&b| matches!(b, 0x21..=0x7E))
}
