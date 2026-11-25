//! Scanner for ASCII and null-terminated (ASCIIZ) string payloads across a [`DynamicFile`].

use serde::{Deserialize, Serialize};

use super::model::{ByteSpanAnnotation, DynamicChunk, DynamicChunkContent, DynamicFile};

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
    /// When `true` (default), scan inside non-all-ASCII leaf payloads for NUL-terminated
    /// printable ASCII runs (`EmbeddedAsciiz`).
    pub include_embedded_asciiz: bool,
    /// When `true`, also scan inside non-all-ASCII leaf payloads for non-NUL-terminated ASCII runs
    /// (`EmbeddedAscii`).
    pub include_embedded_ascii: bool,
    /// When `true` (default), skip byte ranges already covered by non-padding, non-`RawBytes`,
    /// non-string (`CString`/`Path`) schema spans so numeric buffers (e.g. vertex or matrix arrays)
    /// and preceding `f32`/`u32` fields do not produce false-positive string matches.
    pub skip_typed_non_string_spans: bool,
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
            include_embedded_asciiz: true,
            include_embedded_ascii: false,
            skip_typed_non_string_spans: true,
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
    /// Field path of the [`ByteSpanAnnotation`] covering `payload_offset` (e.g. `"name"`,
    /// `"header.texture"`, `"TestStruct.<pad_after>"`), if any.
    pub span_field_path: Option<String>,
    /// `true` if this string is covered by an assigned non-padding, non-`RawBytes` schema span
    /// (or node-level schema assignment when no spans were recorded). Strings inside `<pad_after>`,
    /// `RawBytes`, or unmapped tail bytes report `false`.
    pub is_schema_assigned: bool,
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
                    scan_payload(&meta, &mc.raw_bytes, &mc.spans, options, out);
                }
            }
            DynamicChunkContent::Data {
                raw_bytes, spans, ..
            } => {
                let meta = NodeMeta {
                    tree_path: &chunk.tree_path,
                    is_mini_chunk: false,
                    node_id: chunk.id,
                    schema_field_name: chunk.schema_field_name.as_deref(),
                    schema_type_label: chunk.schema_type_name.as_deref(),
                    base_file_offset: chunk.payload_offset,
                };
                scan_payload(&meta, raw_bytes, spans, options, out);
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
        span_field_path: Option<String>,
        is_schema_assigned: bool,
    ) -> FoundString {
        FoundString {
            tree_path: self.tree_path.to_owned(),
            is_mini_chunk: self.is_mini_chunk,
            node_id: self.node_id,
            schema_field_name: self.schema_field_name.map(str::to_owned),
            schema_type_label: self.schema_type_label.map(str::to_owned),
            span_field_path,
            is_schema_assigned,
            payload_offset,
            file_offset: self.base_file_offset + payload_offset,
            byte_len,
            kind,
            text: text.to_owned(),
        }
    }
}

fn is_typed_non_string_span(span: &ByteSpanAnnotation) -> bool {
    !span.is_padding && !matches!(span.type_label.as_str(), "RawBytes" | "CString" | "Path")
}

fn build_skip_ranges(spans: &[ByteSpanAnnotation], payload_len: usize) -> Vec<(usize, usize)> {
    let mut ranges: Vec<(usize, usize)> = spans
        .iter()
        .filter(|span| is_typed_non_string_span(span))
        .filter_map(|span| {
            let start = span.start.min(payload_len);
            let end = span.end.min(payload_len);
            (start < end).then_some((start, end))
        })
        .collect();
    ranges.sort_unstable_by_key(|&(start, end)| (start, end));

    let mut merged: Vec<(usize, usize)> = Vec::with_capacity(ranges.len());
    for (start, end) in ranges {
        if let Some(last) = merged.last_mut()
            && start <= last.1
        {
            last.1 = last.1.max(end);
        } else {
            merged.push((start, end));
        }
    }
    merged
}

fn resolve_span_assignment(
    spans: &[ByteSpanAnnotation],
    schema_field_name: Option<&str>,
    payload_offset: usize,
) -> (Option<String>, bool) {
    if spans.is_empty() {
        return (None, schema_field_name.is_some());
    }
    if let Some(span) = spans
        .iter()
        .find(|s| s.start <= payload_offset && payload_offset < s.end)
    {
        let path = (!span.field_path.is_empty()).then(|| span.field_path.clone());
        let assigned = !span.is_padding && span.type_label != "RawBytes";
        (path, assigned)
    } else {
        (None, false)
    }
}

fn scan_payload(
    meta: &NodeMeta<'_>,
    bytes: &[u8],
    spans: &[ByteSpanAnnotation],
    options: &FindStringsOptions,
    out: &mut Vec<FoundString>,
) {
    if bytes.is_empty() || (options.max_results > 0 && out.len() >= options.max_results) {
        return;
    }

    let skip_ranges = if options.skip_typed_non_string_spans {
        build_skip_ranges(spans, bytes.len())
    } else {
        Vec::new()
    };
    if skip_ranges.as_slice() == [(0, bytes.len())] {
        return;
    }

    if let Some((kind, text)) = classify_whole_block(bytes, options) {
        if kind_matches_filter(kind, options) {
            let (span_field_path, is_schema_assigned) =
                resolve_span_assignment(spans, meta.schema_field_name, 0);
            out.push(meta.to_found(
                0,
                bytes.len(),
                kind,
                text,
                span_field_path,
                is_schema_assigned,
            ));
        }
        return;
    }

    if !options.include_embedded_asciiz && !options.include_embedded_ascii {
        return;
    }

    let min_emb_len = options.min_embedded_len.max(1);
    let mut i = 0usize;
    let mut skip_idx = 0usize;
    while i < bytes.len() {
        if options.max_results > 0 && out.len() >= options.max_results {
            return;
        }
        while skip_idx < skip_ranges.len() && skip_ranges[skip_idx].1 <= i {
            skip_idx += 1;
        }
        if skip_idx < skip_ranges.len() && skip_ranges[skip_idx].0 <= i {
            i = skip_ranges[skip_idx].1;
            skip_idx += 1;
            continue;
        }
        let limit = skip_ranges
            .get(skip_idx)
            .map_or(bytes.len(), |r| r.0)
            .min(bytes.len());

        while i < limit {
            if options.max_results > 0 && out.len() >= options.max_results {
                return;
            }
            if !is_ascii_text_byte(bytes[i], options.allow_whitespace_controls) {
                i += 1;
                continue;
            }
            let start = i;
            while i < limit && is_ascii_text_byte(bytes[i], options.allow_whitespace_controls) {
                i += 1;
            }
            let end = i;
            let run = &bytes[start..end];
            // Never treat byte 0 of a subsequent typed span at `limit` as a NUL terminator.
            let is_z = end < limit && bytes[end] == 0;

            if run.len() >= min_emb_len
                && has_visible_printable(run)
                && let Ok(text) = std::str::from_utf8(run)
            {
                let (kind, byte_len, allowed) = if is_z {
                    (
                        StringMatchKind::EmbeddedAsciiz,
                        run.len() + 1,
                        options.include_embedded_asciiz && options.include_asciiz,
                    )
                } else {
                    (
                        StringMatchKind::EmbeddedAscii,
                        run.len(),
                        options.include_embedded_ascii && options.include_ascii,
                    )
                };
                if allowed {
                    let (span_field_path, is_schema_assigned) =
                        resolve_span_assignment(spans, meta.schema_field_name, start);
                    out.push(meta.to_found(
                        start,
                        byte_len,
                        kind,
                        text,
                        span_field_path,
                        is_schema_assigned,
                    ));
                }
            }

            if is_z {
                i += 1;
            }
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
