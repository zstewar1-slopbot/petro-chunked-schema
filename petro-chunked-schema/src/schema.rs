//! Schema AST types for describing Petroglyph's chunked binary file formats in YAML.
//!
//! Every [`Schema`] shares a uniform structure:
//! - Metadata (`name`, `description`, `file_extensions`).
//! - Optional named [`imports`](Schema::imports) mapping module aliases to [`SchemaRef`] locations
//!   (such as `builtin:AloModel` or `./common.yaml`).
//! - A `root` type name referencing an entry in [`types`](Schema::types) (or `alias::Type` from an
//!   import) rather than a bespoke inline root container.
//! - Ordered [`types`](Schema::types) definitions (`IndexMap<String, TypeDef>`) covering:
//!   - [`TypeDef::ByteBlock`]: sequential struct decoded from a chunk or mini-chunk's byte payload.
//!   - [`TypeDef::MiniChunks`]: map or sequence of 8-bit mini-chunks inside a leaf data chunk.
//!   - [`TypeDef::Chunks`]: map or sequence of 32-bit child chunks inside a container chunk.
//!   - [`TypeDef::ChunkStreamSeq`]: sequential multi-chunk stream item struct (`ChunkStreamItem`).
//!   - [`TypeDef::ChunkEnum`]: chunk-tagged enum (`ChunkStreamItem`) dispatching on the next chunk ID.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::error::DataTypeParseError;
use crate::source::SchemaRef;

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
    /// Named external schema imports (`alias -> SchemaRef`) referenced as `alias::TypeName`.
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub imports: IndexMap<String, SchemaRef>,
    /// Name of the root chunk type in [`Self::types`] (or `alias::TypeName` from [`Self::imports`]).
    pub root: String,
    /// Ordered type definitions referenced by [`Self::root`] and by other types.
    #[serde(default)]
    pub types: IndexMap<String, TypeDef>,
}

impl Schema {
    /// Parses a [`Schema`] from a YAML string.
    pub fn from_yaml_str(yaml: &str) -> Result<Self, serde_yaml::Error> {
        serde_yaml::from_str(yaml)
    }

    /// Serializes this [`Schema`] to a formatted YAML string, preserving definition order.
    pub fn to_yaml_string(&self) -> Result<String, serde_yaml::Error> {
        serde_yaml::to_string(self)
    }

    /// Creates an empty starter [`Schema`] whose [`Self::root`] points to an open `Chunks` map
    /// definition of the same name in [`Self::types`].
    pub fn new_empty(name: impl Into<String>) -> Self {
        let name = name.into();
        let mut types = IndexMap::new();
        types.insert(
            name.clone(),
            TypeDef::Chunks(ChunkContainerDef {
                description: None,
                mode: OrderMode::Map,
                ignore_unknown: true,
                fields: Vec::new(),
            }),
        );
        Self {
            name: name.clone(),
            description: None,
            file_extensions: Vec::new(),
            imports: IndexMap::new(),
            root: name,
            types,
        }
    }

    /// Looks up the local [`TypeDef`] named by [`Self::root`] in [`Self::types`].
    ///
    /// Returns `None` if [`Self::root`] points to an imported `alias::Type` or is missing.
    pub fn root_type(&self) -> Option<&TypeDef> {
        self.types.get(&self.root)
    }

    /// Mutably looks up the local [`TypeDef`] named by [`Self::root`] in [`Self::types`].
    pub fn root_type_mut(&mut self) -> Option<&mut TypeDef> {
        self.types.get_mut(&self.root)
    }

    /// Inserts `name -> def` immediately after `after` in [`Self::types`] (or at the end if
    /// `after` is `None` or absent), preserving parents-before-children order.
    ///
    /// Returns `false` without modifying `self.types` if `name` is already defined.
    pub fn insert_type_after(&mut self, after: Option<&str>, name: String, def: TypeDef) -> bool {
        if self.types.contains_key(&name) {
            return false;
        }
        let insert_idx = after
            .and_then(|key| self.types.get_index_of(key))
            .map_or(self.types.len(), |idx| idx + 1);
        self.types.shift_insert(insert_idx, name, def);
        true
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
    /// Returns the Rust type name in `petro_chunked::wire_types` for this wire format.
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
/// `"(u32, f32)"`, `"SkeletonHeader"`, `"model::Vertex"`) and deserializes from either that concise
/// string or a structured YAML map.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DataType {
    /// Unsigned 8-bit integer (`u8`).
    U8,
    /// Signed 8-bit integer (`i8`).
    I8,
    /// Little-endian unsigned 16-bit integer (`u16`).
    U16,
    /// Little-endian signed 16-bit integer (`i16`).
    I16,
    /// Little-endian unsigned 32-bit integer (`u32`).
    U32,
    /// Little-endian signed 32-bit integer (`i32`).
    I32,
    /// Little-endian unsigned 64-bit integer (`u64`).
    U64,
    /// Little-endian signed 64-bit integer (`i64`).
    I64,
    /// Little-endian 32-bit IEEE 754 float (`f32`).
    F32,
    /// Little-endian 64-bit IEEE 754 float (`f64`).
    F64,
    /// Host `bool` (paired with `WireFormat::U8Bool` or `WireFormat::U32Bool`; defaults to `U32Bool`).
    Bool,
    /// Host `usize` (paired with `WireFormat::U32Usize`).
    Usize,
    /// Null-terminated UTF-8/ASCII string (`&'b str` / `String`).
    CString,
    /// Null-terminated filesystem path string (`&'b Path` / `PathBuf`).
    Path,
    /// Two-component `f32` vector (`[f32; 2]`).
    Vec2,
    /// Three-component `f32` vector (`[f32; 3]`).
    Vec3,
    /// Four-component `f32` vector (`[f32; 4]`).
    Vec4,
    /// 3x4 row-major transform matrix (`[[f32; 4]; 3]`).
    Mat3x4,
    /// Fixed-length array `[T; len]`.
    Array {
        /// Element type repeated `len` times.
        elem: Box<DataType>,
        /// Fixed number of elements.
        len: usize,
    },
    /// Variable-length packed sequence `Vec<T>` consuming until the end of the byte block.
    List(Box<DataType>),
    /// Heterogeneous tuple `(T1, T2, ...)`.
    Tuple(Vec<DataType>),
    /// Raw uninterpreted remaining bytes (`Vec<u8>`).
    RawBytes,
    /// Reference to a named [`TypeDef::ByteBlock`] in `Schema::types` or an imported module.
    Named(String),
}

impl DataType {
    /// Short human-readable label used in UI display and YAML serialization.
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
            Self::Tuple(items) => match items.as_slice() {
                [] => "()".to_owned(),
                [single] => format!("({},)", single.label()),
                many => {
                    let inner: Vec<String> = many.iter().map(|i| i.label()).collect();
                    format!("({})", inner.join(", "))
                }
            },
            Self::RawBytes => "raw_bytes".to_owned(),
            Self::Named(name) => name.clone(),
        }
    }

    /// Parses a [`DataType`] from its concise string representation (e.g. `"u32"`, `"[u16; 3]"`,
    /// `"Vec<VertexV1>"`, `"(u32, f32)"`, `"SkeletonHeader"`, `"model::Vertex"`).
    pub fn parse_str(s: &str) -> Result<Self, DataTypeParseError> {
        let s = s.trim();
        if s.is_empty() {
            return Err(DataTypeParseError::Empty);
        }

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
            "mat3x4" | "Mat3x4" => return Ok(Self::Mat3x4),
            "raw_bytes" | "RawBytes" => return Ok(Self::RawBytes),
            _ => {}
        }

        if let Some(rest) = s.strip_prefix("Vec<") {
            let Some(inner) = rest.strip_suffix('>') else {
                return Err(DataTypeParseError::InvalidSyntax {
                    input: s.to_owned(),
                    reason: "unterminated `Vec<...>`".to_owned(),
                });
            };
            if inner.trim().is_empty() {
                return Err(DataTypeParseError::InvalidSyntax {
                    input: s.to_owned(),
                    reason: "empty element type in `Vec<...>`".to_owned(),
                });
            }
            return Ok(Self::List(Box::new(Self::parse_str(inner)?)));
        }

        if s.starts_with('[') {
            let Some(inner) = s.strip_prefix('[').and_then(|r| r.strip_suffix(']')) else {
                return Err(DataTypeParseError::InvalidSyntax {
                    input: s.to_owned(),
                    reason: "unterminated `[...; N]` array".to_owned(),
                });
            };
            let Some((elem_str, len_str)) = rsplit_top_level(inner, ';') else {
                return Err(DataTypeParseError::InvalidSyntax {
                    input: s.to_owned(),
                    reason: "expected `[elem; len]`".to_owned(),
                });
            };
            let len: usize =
                len_str
                    .trim()
                    .parse()
                    .map_err(|_| DataTypeParseError::InvalidArrayLength {
                        input: s.to_owned(),
                    })?;
            return Ok(Self::Array {
                elem: Box::new(Self::parse_str(elem_str)?),
                len,
            });
        }

        if s.starts_with('(') {
            let Some(inner) = s.strip_prefix('(').and_then(|r| r.strip_suffix(')')) else {
                return Err(DataTypeParseError::InvalidSyntax {
                    input: s.to_owned(),
                    reason: "unterminated `(...)` tuple".to_owned(),
                });
            };
            let (parts, saw_comma) = split_top_level_csv(inner, s)?;
            if parts.is_empty() && !saw_comma {
                return Ok(Self::Tuple(Vec::new()));
            }
            if !saw_comma {
                return Err(DataTypeParseError::InvalidSyntax {
                    input: s.to_owned(),
                    reason: "single-element tuples require a trailing comma, e.g. `(u32,)`"
                        .to_owned(),
                });
            }
            let mut items = Vec::with_capacity(parts.len());
            for part in parts {
                items.push(Self::parse_str(part)?);
            }
            return Ok(Self::Tuple(items));
        }

        if !is_valid_type_path(s) {
            return Err(DataTypeParseError::InvalidSyntax {
                input: s.to_owned(),
                reason: "expected a primitive, compound type, or valid type path".to_owned(),
            });
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

fn is_valid_type_path(s: &str) -> bool {
    let mut segments = s.split("::");
    let Some(first) = segments.next() else {
        return false;
    };
    if !is_valid_ident_segment(first) {
        return false;
    }
    segments.all(is_valid_ident_segment)
}

fn is_valid_ident_segment(seg: &str) -> bool {
    let mut chars = seg.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn rsplit_top_level(s: &str, sep: char) -> Option<(&str, &str)> {
    let mut depth = 0i32;
    for (idx, ch) in s.char_indices().rev() {
        match ch {
            ']' | ')' | '>' => depth += 1,
            '[' | '(' | '<' => {
                depth -= 1;
                if depth < 0 {
                    return None;
                }
            }
            c if c == sep && depth == 0 => {
                return Some((&s[..idx], &s[idx + c.len_utf8()..]));
            }
            _ => {}
        }
    }
    None
}

fn split_top_level_csv<'a>(
    s: &'a str,
    full_input: &str,
) -> Result<(Vec<&'a str>, bool), DataTypeParseError> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut start = 0usize;
    let mut saw_comma = false;
    for (idx, ch) in s.char_indices() {
        match ch {
            '[' | '(' | '<' => depth += 1,
            ']' | ')' | '>' => {
                depth -= 1;
                if depth < 0 {
                    return Err(DataTypeParseError::InvalidSyntax {
                        input: full_input.to_owned(),
                        reason: "unbalanced brackets in tuple".to_owned(),
                    });
                }
            }
            ',' if depth == 0 => {
                saw_comma = true;
                let item = s[start..idx].trim();
                if item.is_empty() {
                    return Err(DataTypeParseError::InvalidSyntax {
                        input: full_input.to_owned(),
                        reason: "empty tuple element".to_owned(),
                    });
                }
                out.push(item);
                start = idx + 1;
            }
            _ => {}
        }
    }
    if depth != 0 {
        return Err(DataTypeParseError::InvalidSyntax {
            input: full_input.to_owned(),
            reason: "unbalanced brackets in tuple".to_owned(),
        });
    }
    let tail = s[start..].trim();
    if !tail.is_empty() {
        out.push(tail);
    }
    Ok((out, saw_comma))
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
    ChunkStreamSeq(ChunkStreamSeqDef),
    /// A chunk-tagged enum (`#[derive(ChunkStreamItem)]`).
    ChunkEnum(ChunkEnumDef),
}

/// Definition of a sequential byte-block struct (`#[derive(FromByteBlock)]`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ByteBlockDef {
    /// Optional human-readable description of this struct.
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
    /// Rust-style snake_case field identifier.
    pub name: String,
    /// Optional human-readable description of this field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Binary field data type.
    pub ty: DataType,
    /// Optional wire-format adapter overriding [`DataType::default_wire_format`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wire_format: Option<WireFormat>,
    /// Trailing padding bytes skipped immediately after this field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pad_after: Option<usize>,
}

/// Definition of a mini-chunk container struct (`#[derive(MiniChunkStructure)]`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MiniChunkContainerDef {
    /// Optional human-readable description of this mini-chunk container.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Whether mini-chunk fields appear as an unordered map or a strict declaration sequence.
    pub mode: OrderMode,
    /// When `true`, unrecognized mini-chunk IDs are skipped instead of producing errors.
    #[serde(default)]
    pub ignore_unknown: bool,
    /// Mini-chunk field declarations keyed by 8-bit mini-chunk ID.
    #[serde(default)]
    pub fields: Vec<MiniChunkFieldDef>,
}

/// A single mini-chunk field inside a [`MiniChunkContainerDef`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MiniChunkFieldDef {
    /// 8-bit mini-chunk ID.
    pub id: u8,
    /// Rust-style snake_case field identifier.
    pub name: String,
    /// Optional human-readable description of this field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Allowed occurrence count (`required`, `optional`, `repeated`, `default`, `default_expr`).
    #[serde(default)]
    pub cardinality: Cardinality,
    /// Payload data type decoded from this mini-chunk's body.
    pub ty: DataType,
    /// Optional wire-format adapter overriding [`DataType::default_wire_format`].
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

/// Where a field in a [`ChunkContainerDef`], [`ChunkStreamSeqDef`], or variant in a [`ChunkEnumDef`]
/// pulls its chunks from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "source_kind", rename_all = "snake_case")]
pub enum ChunkFieldSource {
    /// Matches a specific 32-bit chunk ID (`#[chunk(<id>, ...)]`).
    Id {
        /// 32-bit chunk ID.
        id: u32,
        /// How the matched chunk's body is interpreted.
        #[serde(flatten)]
        payload: ChunkPayloadSpec,
    },
    /// Delegates to a [`TypeDef::ChunkEnum`], [`TypeDef::ChunkStreamSeq`], or sequential
    /// [`TypeDef::Chunks`] (`#[chunk(stream)]`).
    Stream {
        /// Local or imported (`alias::Type`) name of the stream item type.
        item_type: String,
    },
}

/// A single field in a [`ChunkContainerDef`] or [`ChunkStreamSeqDef`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChunkFieldDef {
    /// Rust-style snake_case field identifier.
    pub name: String,
    /// Optional human-readable description of this field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Allowed occurrence count (`required`, `optional`, `repeated`, `default`, `default_expr`).
    #[serde(default)]
    pub cardinality: Cardinality,
    /// Chunk ID matcher or nested stream item delegation.
    #[serde(flatten)]
    pub source: ChunkFieldSource,
    /// Optional wire-format adapter overriding [`DataType::default_wire_format`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wire_format: Option<WireFormat>,
}

/// Definition of a 32-bit chunk container struct (`#[derive(ChunkStructure)]`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChunkContainerDef {
    /// Optional human-readable description of this chunk container.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Whether child chunks appear as an unordered map or a strict declaration sequence.
    #[serde(default)]
    pub mode: OrderMode,
    /// When `true`, unrecognized child chunk IDs are skipped instead of producing errors.
    #[serde(default)]
    pub ignore_unknown: bool,
    /// Child chunk or stream field declarations.
    #[serde(default)]
    pub fields: Vec<ChunkFieldDef>,
}

/// Definition of a sequential multi-chunk stream item struct (`#[derive(ChunkStreamItem)]`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChunkStreamSeqDef {
    /// Optional human-readable description of this stream sequence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Sequential fields pulled from the parent chunk stream when this item is matched.
    #[serde(default)]
    pub fields: Vec<ChunkFieldDef>,
}

/// Definition of a chunk-tagged enum (`#[derive(ChunkStreamItem)]`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChunkEnumDef {
    /// Optional human-readable description of this chunk-tagged enum.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Ordered variants checked against the next chunk ID in a stream.
    pub variants: Vec<ChunkEnumVariantDef>,
}

/// A single variant of a [`ChunkEnumDef`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChunkEnumVariantDef {
    /// UpperCamelCase enum variant identifier.
    pub name: String,
    /// Optional human-readable description of this variant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Chunk ID matcher or delegated stream item type for this variant.
    #[serde(flatten)]
    pub source: ChunkFieldSource,
    /// Optional wire-format adapter for this variant's decoded value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wire_format: Option<WireFormat>,
}
