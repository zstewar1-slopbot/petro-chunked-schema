//! Schema types for describing Petroglyph's chunked binary file formats in YAML.
//!
//! A [`Schema`] defines:
//! - Metadata about the file format (`name`, `description`, `file_extensions`).
//! - The `root` chunk container structure (`Map` or `Sequence` of top-level chunks).
//! - Named reusable type definitions (`types`), which may be:
//!   - [`TypeDef::ByteBlock`]: A sequential struct decoded from a chunk or mini-chunk's byte block.
//!   - [`TypeDef::MiniChunks`]: A map or sequence of 8-bit mini-chunks inside a leaf data chunk.
//!   - [`TypeDef::Chunks`]: A map or sequence of 32-bit child chunks inside a container chunk.
//!   - [`TypeDef::ChunkEnum`]: A chunk-tagged enum (implementing `ChunkStreamItem`) that dispatches
//!     on the next chunk ID in a stream.
//!   - [`TypeDef::ChunkStreamSeq`]: A multi-chunk sequence struct (implementing `ChunkStreamItem`)
//!     that pulls multiple sibling chunks from a parent stream (e.g. a submesh `(0x10100, 0x10000)`
//!     pair or a particle track `(0x0, 0x1)` pair).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Top-level schema descriptor for a Petroglyph chunked binary file format.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Schema {
    /// Human- and codegen-friendly identifier for this format (e.g. `"AloModel"`).
    pub name: String,
    /// Optional human-readable description of the file format.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Typical file extensions for this format without leading dots (e.g. `["alo"]`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub file_extensions: Vec<String>,
    /// Root chunk container specification at the top level of the file.
    pub root: ChunkContainerDef,
    /// Named type definitions referenced by `root` and by other types.
    #[serde(default)]
    pub types: BTreeMap<String, TypeDef>,
}

impl Schema {
    /// Parses a [`Schema`] from a YAML string.
    pub fn from_yaml_str(yaml: &str) -> Result<Self, serde_yaml::Error> {
        serde_yaml::from_str(yaml)
    }

    /// Serializes this [`Schema`] to a formatted YAML string.
    pub fn to_yaml_string(&self) -> Result<String, serde_yaml::Error> {
        serde_yaml::to_string(self)
    }

    /// Creates an empty starter [`Schema`] with the given name.
    pub fn new_empty(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: None,
            file_extensions: Vec::new(),
            root: ChunkContainerDef {
                description: None,
                mode: OrderMode::Map,
                ignore_unknown: true,
                fields: Vec::new(),
            },
            types: BTreeMap::new(),
        }
    }
}

/// Ordering rule for a chunk or mini-chunk container.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum OrderMode {
    /// Unordered map keyed by chunk/mini-chunk ID (`#[chunk(map)]` / `#[mini_chunk(map)]`).
    #[default]
    Map,
    /// Strict declaration order (`#[chunk(sequence)]` / `#[mini_chunk(sequence)]`).
    Sequence,
}

/// How many times a chunk or mini-chunk field may appear in its container.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Cardinality {
    /// Exactly 1 occurrence is required.
    #[default]
    Required,
    /// 0 or 1 occurrence (`Option<T>`).
    Optional,
    /// 0 or more occurrences (`Vec<T>`).
    Repeated,
    /// 0 or 1 occurrence, falling back to `Default::default()` when absent.
    Default,
    /// 0 or 1 occurrence, falling back to a Rust expression string when absent.
    DefaultExpr(String),
}

/// Wire format adapter type for `wire_format = <Type>` in `petro-chunked-derive`.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum WireFormat {
    /// 1-byte boolean (`0` or `1`) -> `bool`.
    U8Bool,
    /// 4-byte little-endian boolean (`0` or `1`) -> `bool`.
    U32Bool,
    /// 4-byte little-endian unsigned integer -> `usize`.
    U32Usize,
    /// Packed array of 4-byte little-endian unsigned integers -> `Vec<usize>`.
    U32UsizeVec,
}

impl WireFormat {
    /// Returns the Rust identifier in `petro_chunked::wire_types` for this wire format.
    pub fn rust_type_name(self) -> &'static str {
        match self {
            Self::U8Bool => "U8Bool",
            Self::U32Bool => "U32Bool",
            Self::U32Usize => "U32Usize",
            Self::U32UsizeVec => "U32UsizeVec",
        }
    }
}

/// Data type for a byte-block field, a mini-chunk payload, or a leaf data chunk payload.
///
/// Serializes to a concise type string (e.g. `"u32"`, `"vec3"`, `"[u16; 3]"`, `"Vec<VertexV1>"`,
/// `"(u32, f32)"`, `"SkeletonHeader"`) and deserializes from either that concise string or a
/// structured YAML map.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DataType {
    U8,
    I8,
    U16,
    I16,
    U32,
    I32,
    U64,
    I64,
    F32,
    F64,
    /// Host `bool` (must be paired with `WireFormat::U8Bool` or `WireFormat::U32Bool`, defaults to `U32Bool` if unspecified).
    Bool,
    /// Host `usize` (paired with `WireFormat::U32Usize`).
    Usize,
    /// Null-terminated ASCII/UTF-8 string (`&'b str` / `String`).
    CString,
    /// Null-terminated path string (`&'b Path` / `PathBuf`).
    Path,
    /// `[f32; 2]` / `glam::Vec2`.
    Vec2,
    /// `[f32; 3]` / `glam::Vec3`.
    Vec3,
    /// `[f32; 4]` / `glam::Vec4`.
    Vec4,
    /// 3x4 row-major transform matrix (`[[f32; 4]; 3]`).
    Mat3x4,
    /// Fixed-length array `[T; len]`.
    Array {
        elem: Box<DataType>,
        len: usize,
    },
    /// Variable-length packed sequence `Vec<T>` consuming until the end of the byte block.
    List(Box<DataType>),
    /// Heterogeneous tuple `(T1, T2, ...)`.
    Tuple(Vec<DataType>),
    /// Raw uninterpreted remaining bytes (`Vec<u8>`).
    RawBytes,
    /// Reference to a named [`TypeDef::ByteBlock`] in `Schema::types`.
    Named(String),
}

impl DataType {
    /// Short human-readable label for UI display and YAML serialization.
    pub fn label(&self) -> String {
        match self {
            Self::U8 => "u8".to_owned(),
            Self::I8 => "i8".to_owned(),
            Self::U16 => "u16".to_owned(),
            Self::I16 => "i16".to_owned(),
            Self::U32 => "u32".to_owned(),
            Self::I32 => "i32".to_owned(),
            Self::U64 => "u64".to_owned(),
            Self::I64 => "i64".to_owned(),
            Self::F32 => "f32".to_owned(),
            Self::F64 => "f64".to_owned(),
            Self::Bool => "bool".to_owned(),
            Self::Usize => "usize".to_owned(),
            Self::CString => "c_string".to_owned(),
            Self::Path => "path".to_owned(),
            Self::Vec2 => "vec2".to_owned(),
            Self::Vec3 => "vec3".to_owned(),
            Self::Vec4 => "vec4".to_owned(),
            Self::Mat3x4 => "mat3x4".to_owned(),
            Self::Array { elem, len } => format!("[{}; {len}]", elem.label()),
            Self::List(elem) => format!("Vec<{}>", elem.label()),
            Self::Tuple(items) => {
                let inner: Vec<String> = items.iter().map(|i| i.label()).collect();
                format!("({})", inner.join(", "))
            }
            Self::RawBytes => "raw_bytes".to_owned(),
            Self::Named(name) => name.clone(),
        }
    }

    /// Parses a [`DataType`] from its concise string representation (e.g. `"u32"`, `"[u16; 3]"`,
    /// `"Vec<VertexV1>"`, `"(u32, f32)"`, `"SkeletonHeader"`).
    pub fn parse_str(s: &str) -> Result<Self, String> {
        let s = s.trim();
        match s {
            "u8" => return Ok(Self::U8),
            "i8" => return Ok(Self::I8),
            "u16" => return Ok(Self::U16),
            "i16" => return Ok(Self::I16),
            "u32" => return Ok(Self::U32),
            "i32" => return Ok(Self::I32),
            "u64" => return Ok(Self::U64),
            "i64" => return Ok(Self::I64),
            "f32" => return Ok(Self::F32),
            "f64" => return Ok(Self::F64),
            "bool" => return Ok(Self::Bool),
            "usize" => return Ok(Self::Usize),
            "c_string" | "CString" | "string" | "str" => return Ok(Self::CString),
            "path" | "Path" => return Ok(Self::Path),
            "vec2" | "Vec2" => return Ok(Self::Vec2),
            "vec3" | "Vec3" => return Ok(Self::Vec3),
            "vec4" | "Vec4" => return Ok(Self::Vec4),
            "mat3x4" | "Mat3x4" | "[[f32; 4]; 3]" => return Ok(Self::Mat3x4),
            "raw_bytes" | "RawBytes" | "Vec<u8>" => return Ok(Self::RawBytes),
            _ => {}
        }

        if let Some(inner) = s.strip_prefix("Vec<").and_then(|r| r.strip_suffix('>')) {
            return Ok(Self::List(Box::new(Self::parse_str(inner)?)));
        }

        if let Some(inner) = s.strip_prefix('[').and_then(|r| r.strip_suffix(']'))
            && let Some((elem_str, len_str)) = rsplit_top_level(inner, ';')
        {
            let len: usize = len_str
                .trim()
                .parse()
                .map_err(|_| format!("Invalid array length in `{s}`"))?;
            return Ok(Self::Array {
                elem: Box::new(Self::parse_str(elem_str)?),
                len,
            });
        }

        if let Some(inner) = s.strip_prefix('(').and_then(|r| r.strip_suffix(')')) {
            let parts = split_top_level_csv(inner);
            let mut items = Vec::with_capacity(parts.len());
            for part in parts {
                items.push(Self::parse_str(part)?);
            }
            return Ok(Self::Tuple(items));
        }

        if s.is_empty() {
            return Err("Empty data type string".to_owned());
        }
        Ok(Self::Named(s.to_owned()))
    }

    /// Returns a sensible default [`WireFormat`] if this type requires one (e.g. `Bool` -> `U32Bool`,
    /// `Usize` -> `U32Usize`, `List(Usize)` -> `U32UsizeVec`).
    pub fn default_wire_format(&self) -> Option<WireFormat> {
        match self {
            Self::Bool => Some(WireFormat::U32Bool),
            Self::Usize => Some(WireFormat::U32Usize),
            Self::List(inner) if matches!(**inner, Self::Usize) => Some(WireFormat::U32UsizeVec),
            _ => None,
        }
    }
}

fn rsplit_top_level(s: &str, sep: char) -> Option<(&str, &str)> {
    let mut depth = 0i32;
    for (idx, ch) in s.char_indices().rev() {
        match ch {
            ']' | ')' | '>' => depth += 1,
            '[' | '(' | '<' => depth -= 1,
            c if c == sep && depth == 0 => {
                return Some((&s[..idx], &s[idx + c.len_utf8()..]));
            }
            _ => {}
        }
    }
    None
}

fn split_top_level_csv(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut start = 0usize;
    for (idx, ch) in s.char_indices() {
        match ch {
            '[' | '(' | '<' => depth += 1,
            ']' | ')' | '>' => depth -= 1,
            ',' if depth == 0 => {
                out.push(s[start..idx].trim());
                start = idx + 1;
            }
            _ => {}
        }
    }
    let tail = s[start..].trim();
    if !tail.is_empty() {
        out.push(tail);
    }
    out
}

impl Serialize for DataType {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.label())
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum RawDataType {
    Str(String),
    Named { named: String },
    Array { array: RawArrayDef },
    List { list: Box<DataType> },
    Tuple { tuple: Vec<DataType> },
}

#[derive(Deserialize)]
struct RawArrayDef {
    elem: Box<DataType>,
    len: usize,
}

impl<'de> Deserialize<'de> for DataType {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = RawDataType::deserialize(deserializer)?;
        match raw {
            RawDataType::Str(s) => Self::parse_str(&s).map_err(serde::de::Error::custom),
            RawDataType::Named { named } => Ok(Self::Named(named)),
            RawDataType::Array { array } => Ok(Self::Array {
                elem: array.elem,
                len: array.len,
            }),
            RawDataType::List { list } => Ok(Self::List(list)),
            RawDataType::Tuple { tuple } => Ok(Self::Tuple(tuple)),
        }
    }
}

/// Named type definition in a [`Schema`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TypeDef {
    /// A struct decoded sequentially from a raw byte block (`#[derive(FromByteBlock)]`).
    ByteBlock(ByteBlockDef),
    /// A struct decoded from an 8-bit mini-chunk stream (`#[derive(MiniChunkStructure)]`).
    MiniChunks(MiniChunkContainerDef),
    /// A struct decoded from a 32-bit child chunk stream (`#[derive(ChunkStructure)]`).
    Chunks(ChunkContainerDef),
    /// A sequential multi-chunk stream item struct (`#[derive(ChunkStreamItem)]`).
    ChunkStreamSeq(ChunkContainerDef),
    /// A chunk-tagged enum (`#[derive(ChunkStreamItem)]`).
    ChunkEnum(ChunkEnumDef),
}

/// Definition of a sequential byte-block struct (`#[derive(FromByteBlock)]`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ByteBlockDef {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Fields read sequentially from the byte block.
    #[serde(default)]
    pub fields: Vec<ByteFieldDef>,
    /// Trailing padding bytes skipped after all fields (`#[byte_block(pad_after = N)]`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pad_after: Option<usize>,
}

/// A single field inside a [`ByteBlockDef`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ByteFieldDef {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub ty: DataType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wire_format: Option<WireFormat>,
    /// Trailing padding bytes skipped immediately after this field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pad_after: Option<usize>,
}

/// Definition of a mini-chunk container struct (`#[derive(MiniChunkStructure)]`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MiniChunkContainerDef {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub mode: OrderMode,
    #[serde(default)]
    pub ignore_unknown: bool,
    #[serde(default)]
    pub fields: Vec<MiniChunkFieldDef>,
}

/// A single mini-chunk field inside a [`MiniChunkContainerDef`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MiniChunkFieldDef {
    /// 8-bit mini-chunk ID.
    pub id: u8,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub cardinality: Cardinality,
    pub ty: DataType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wire_format: Option<WireFormat>,
}

/// How the body of a 32-bit chunk is interpreted.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "payload_kind", content = "payload_type", rename_all = "snake_case")]
pub enum ChunkPayloadSpec {
    /// Leaf byte-block data (`#[chunk(id, data)]`).
    Data(DataType),
    /// Leaf mini-chunks decoded using the named [`TypeDef::MiniChunks`] type (`#[chunk(id, mini_chunks)]`).
    MiniChunks(String),
    /// Child chunks decoded using the named [`TypeDef::Chunks`] type (`#[chunk(id, children)]`).
    Children(String),
    /// Ignored payload (`#[chunk(id, ignore)]`).
    Ignore,
}

/// Where a field in a [`ChunkContainerDef`] pulls its chunks from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "source_kind", rename_all = "snake_case")]
pub enum ChunkFieldSource {
    /// Matches a specific 32-bit chunk ID (`#[chunk(<id>, ...)]`).
    Id {
        id: u32,
        #[serde(flatten)]
        payload: ChunkPayloadSpec,
    },
    /// Delegates to a [`TypeDef::ChunkEnum`] or [`TypeDef::ChunkStreamSeq`] (`#[chunk(stream)]`).
    Stream { item_type: String },
}

/// A single field in a [`ChunkContainerDef`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChunkFieldDef {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub cardinality: Cardinality,
    #[serde(flatten)]
    pub source: ChunkFieldSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wire_format: Option<WireFormat>,
}

/// Definition of a 32-bit chunk container struct (`#[derive(ChunkStructure)]` or `#[derive(ChunkStreamItem)]`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChunkContainerDef {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub mode: OrderMode,
    #[serde(default)]
    pub ignore_unknown: bool,
    #[serde(default)]
    pub fields: Vec<ChunkFieldDef>,
}

/// Definition of a chunk-tagged enum (`#[derive(ChunkStreamItem)]`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChunkEnumDef {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub variants: Vec<ChunkEnumVariantDef>,
}

/// A single variant of a [`ChunkEnumDef`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChunkEnumVariantDef {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(flatten)]
    pub source: ChunkFieldSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wire_format: Option<WireFormat>,
}
