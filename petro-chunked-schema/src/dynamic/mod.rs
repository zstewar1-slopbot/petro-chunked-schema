//! Dynamic AST loading, byte-span annotation, multi-format byte inspection, and schema
//! evaluation for Petroglyph chunked binary files.
//!
//! Loading proceeds in two stages:
//! 1. **Raw parsing** (`parser`) builds a [`DynamicFile`] chunk/mini-chunk tree from raw bytes,
//!    auto-detecting leaf mini-chunk payloads unless listed in
//!    [`LoadOptions::force_plain_data_paths`].
//! 2. **Schema overlay** (`eval`) evaluates an optional [`ResolvedSchema`] over that raw tree,
//!    annotating matched nodes with field names, types, [`DynamicValue`]s, color-coded
//!    [`ByteSpanAnnotation`]s, and [`SchemaDiagnostic`]s.

use std::collections::BTreeSet;

use crate::error::DynamicLoadError;
use crate::resolve::ResolvedSchema;

mod eval;
mod inspector;
mod model;
mod parser;

pub use eval::decode_data_type_with_spans;
pub use inspector::{BytePreviewEntry, inspect_bytes_at_offset};
pub use model::{
    ByteSpanAnnotation, DynamicChunk, DynamicChunkContent, DynamicFile, DynamicMiniChunk,
    DynamicValue, SchemaDiagnostic,
};
pub use parser::looks_like_mini_chunks;

/// Options controlling how a binary buffer is loaded into a [`DynamicFile`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadOptions {
    /// Tree paths (e.g. `"0/2"`) of leaf chunks forced to decode as plain byte data even if their
    /// payload resembles or is typed as mini-chunks.
    pub force_plain_data_paths: BTreeSet<String>,
    /// Maximum number of [`ByteSpanAnnotation`]s recorded per chunk or mini-chunk.
    pub max_spans: usize,
    /// When `true`, large fixed-size unvalidated `Vec<T>` lists are summarized into a single span
    /// after decoding the first 16 elements while still checking exact byte-length divisibility.
    pub fast_skip_large_lists: bool,
}

impl Default for LoadOptions {
    fn default() -> Self {
        Self {
            force_plain_data_paths: BTreeSet::new(),
            max_spans: 512,
            fast_skip_large_lists: false,
        }
    }
}

impl LoadOptions {
    /// Preset tailored for batch validation: disables span recording (`max_spans = 0`) and enables
    /// `fast_skip_large_lists` while keeping full structural and byte-length validation.
    pub fn for_validation() -> Self {
        Self {
            force_plain_data_paths: BTreeSet::new(),
            max_spans: 0,
            fast_skip_large_lists: true,
        }
    }

    /// Preset tailored for interactive inspection: caps spans at 512 and summarizes large fixed-size
    /// lists after 16 items while respecting per-file `force_plain_data_paths` overrides.
    pub fn for_ui(force_plain_data_paths: BTreeSet<String>) -> Self {
        Self {
            force_plain_data_paths,
            max_spans: 512,
            fast_skip_large_lists: true,
        }
    }
}

/// Loads a chunked binary buffer into a [`DynamicFile`], optionally evaluating and annotating it
/// against a [`ResolvedSchema`].
pub fn load_dynamic_file(
    bytes: &[u8],
    schema: Option<&ResolvedSchema>,
    options: &LoadOptions,
) -> Result<DynamicFile, DynamicLoadError> {
    let mut diagnostics = Vec::new();
    let mut raw_chunks = parser::parse_raw_chunk_list(bytes, 0, "", options)?;

    if let Some(resolved) = schema {
        eval::evaluate_resolved_schema(resolved, &mut raw_chunks, options, &mut diagnostics);
    }

    Ok(DynamicFile {
        total_size: bytes.len(),
        chunks: raw_chunks,
        diagnostics,
    })
}

/// Validates `bytes` against `schema`, returning `Ok(DynamicFile)` if zero diagnostics occurred or
/// `Err(Vec<SchemaDiagnostic>)` if any structural or decoding errors were found.
pub fn validate_bytes(
    schema: &ResolvedSchema,
    bytes: &[u8],
) -> Result<DynamicFile, Vec<SchemaDiagnostic>> {
    match load_dynamic_file(bytes, Some(schema), &LoadOptions::for_validation()) {
        Ok(file) if file.diagnostics.is_empty() => Ok(file),
        Ok(file) => Err(file.diagnostics),
        Err(fatal) => Err(vec![SchemaDiagnostic {
            tree_path: "root".to_owned(),
            file_offset: 0,
            message: fatal.to_string(),
        }]),
    }
}
