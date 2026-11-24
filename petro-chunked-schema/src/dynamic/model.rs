//! Dynamic AST, byte-span annotation, and diagnostic types for chunked binary files.

use serde::{Deserialize, Serialize};

use crate::resolve::SchemaId;

/// Dynamically decoded value from a byte block, mini-chunk, or leaf data chunk.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DynamicValue {
    /// Unsigned 8-bit integer.
    U8(u8),
    /// Signed 8-bit integer.
    I8(i8),
    /// Unsigned 16-bit integer.
    U16(u16),
    /// Signed 16-bit integer.
    I16(i16),
    /// Unsigned 32-bit integer.
    U32(u32),
    /// Signed 32-bit integer.
    I32(i32),
    /// Unsigned 64-bit integer.
    U64(u64),
    /// Signed 64-bit integer.
    I64(i64),
    /// 32-bit IEEE 754 floating-point scalar.
    F32(f32),
    /// 64-bit IEEE 754 floating-point scalar.
    F64(f64),
    /// Boolean decoded from a wire integer (`U8Bool` or `U32Bool`).
    Bool(bool),
    /// Host `usize` decoded from a 32-bit wire integer (`U32Usize`).
    Usize(usize),
    /// Null-terminated UTF-8 string or filesystem path.
    String(String),
    /// Fixed-length or variable-length sequence of values.
    Array(Vec<DynamicValue>),
    /// Heterogeneous tuple of values.
    Tuple(Vec<DynamicValue>),
    /// Named or anonymous byte-block struct containing ordered `(field_name, value)` pairs.
    Struct {
        /// Optional schema type name of this struct.
        type_name: Option<String>,
        /// Decoded fields in wire order.
        fields: Vec<(String, DynamicValue)>,
    },
    /// Raw uninterpreted byte payload.
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

/// Annotation for a contiguous range of bytes within a chunk or mini-chunk's payload that has been
/// assigned to a structural field in a schema.
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
    /// Palette index used by hex viewers to color-code this span.
    pub color_index: usize,
    /// `true` if this span represents padding bytes rather than a named field.
    pub is_padding: bool,
}

/// A validation or decoding diagnostic produced while evaluating a schema against a file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SchemaDiagnostic {
    /// Hierarchical tree path of the chunk or mini-chunk where the issue occurred.
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
    /// Module ID of the parent `MiniChunks` container that matched this mini-chunk, if matched.
    pub field_owner_module_id: Option<SchemaId>,
    /// Module ID owning [`Self::schema_type_ref`], if this mini-chunk maps to a named type.
    pub schema_module_id: Option<SchemaId>,
    /// Local type name inside [`Self::schema_module_id`] when targeting a named `ByteBlock`.
    pub schema_type_ref: Option<String>,
    /// Decoded value of this mini-chunk according to the schema, if matched.
    pub decoded: Option<DynamicValue>,
    /// Byte span annotations within `raw_bytes`.
    pub spans: Vec<ByteSpanAnnotation>,
    /// Any validation/decoding errors specific to this mini-chunk.
    pub errors: Vec<String>,
}

/// Parsed content of a [`DynamicChunk`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DynamicChunkContent {
    /// Container chunk (`0x8000_0000` flag bit set) holding child 32-bit chunks.
    Children(Vec<DynamicChunk>),
    /// Leaf data chunk interpreted as a sequence or map of 8-bit mini-chunks.
    MiniChunks {
        /// Raw payload bytes containing the concatenated mini-chunks.
        raw_bytes: Vec<u8>,
        /// Parsed 8-bit mini-chunks inside `raw_bytes`.
        mini_chunks: Vec<DynamicMiniChunk>,
    },
    /// Leaf data chunk interpreted as a raw byte block.
    Data {
        /// Raw payload bytes of the leaf chunk.
        raw_bytes: Vec<u8>,
        /// Decoded value when matched against a schema type.
        decoded: Option<DynamicValue>,
        /// Byte span annotations recorded while decoding `raw_bytes`.
        spans: Vec<ByteSpanAnnotation>,
    },
}

/// A dynamically parsed 32-bit chunk in a chunked binary file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DynamicChunk {
    /// Unique tree path for selection and `force_plain_data_paths` overrides (e.g. `"0"`, `"0/1"`).
    pub tree_path: String,
    /// 32-bit chunk ID.
    pub id: u32,
    /// `true` if the chunk header's container bit (`0x8000_0000`) was set.
    pub is_container: bool,
    /// Absolute file byte offset of the 8-byte chunk header.
    pub header_offset: usize,
    /// Absolute file byte offset of the chunk payload.
    pub payload_offset: usize,
    /// Size of the chunk payload in bytes.
    pub size: usize,
    /// Schema field name matched to this chunk, if any.
    pub schema_field_name: Option<String>,
    /// Human-readable schema type or container label matched to this chunk, if any.
    pub schema_type_name: Option<String>,
    /// Module ID of the container whose field matched this chunk, if matched.
    pub field_owner_module_id: Option<SchemaId>,
    /// Module ID owning [`Self::schema_type_ref`], if this chunk's payload maps to a named type.
    pub schema_module_id: Option<SchemaId>,
    /// Local type name inside [`Self::schema_module_id`] when targeting a named `TypeDef`.
    pub schema_type_ref: Option<String>,
    /// `true` if this leaf chunk's bytes can be cleanly segmented as 8-bit mini-chunks.
    pub looks_like_mini_chunks: bool,
    /// Parsed content of this chunk.
    pub content: DynamicChunkContent,
    /// Validation or decoding errors attached to this chunk.
    pub errors: Vec<String>,
}

/// Top-level result of loading a chunked binary file into dynamic values.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DynamicFile {
    /// Total file size in bytes.
    pub total_size: usize,
    /// Top-level 32-bit chunks in the file.
    pub chunks: Vec<DynamicChunk>,
    /// All schema validation and decoding diagnostics collected across the file.
    pub diagnostics: Vec<SchemaDiagnostic>,
}
