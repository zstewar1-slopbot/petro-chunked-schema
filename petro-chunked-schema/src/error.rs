//! Structured error types for schema parsing, URI resolution, dynamic loading, and Rust codegen.

use std::path::PathBuf;

use thiserror::Error;

/// Error returned when parsing a [`crate::source::SchemaRef`] from a string.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum SchemaRefError {
    /// Input string was empty.
    #[error("schema reference cannot be empty")]
    Empty,
    /// `builtin:` prefix was provided without a format name.
    #[error("builtin schema name after `builtin:` cannot be empty")]
    EmptyBuiltinName,
    /// `file:` prefix was provided without a file path.
    #[error("file path after `file:` cannot be empty")]
    EmptyFilePath,
}

/// Error returned when parsing a [`crate::schema::DataType`] from its concise string syntax.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum DataTypeParseError {
    /// Empty data type string.
    #[error("empty data type string")]
    Empty,
    /// Fixed-length array length could not be parsed as `usize`.
    #[error("invalid array length in `{input}`")]
    InvalidArrayLength {
        /// Original type string that failed to parse.
        input: String,
    },
    /// Malformed compound syntax or invalid type identifier.
    #[error("invalid data type `{input}`: {reason}")]
    InvalidSyntax {
        /// Original type string that failed to parse.
        input: String,
        /// Human-readable syntax explanation.
        reason: String,
    },
}

/// Error returned when loading or resolving a schema and its imports.
#[derive(Debug, Error)]
pub enum SchemaLoadError {
    /// Invalid schema reference string.
    #[error(transparent)]
    InvalidRef(#[from] SchemaRefError),
    /// Failed to read a schema YAML file from disk.
    #[error("failed to read schema file `{}`: {source}", path.display())]
    Io {
        /// File path that could not be read.
        path: PathBuf,
        /// Underlying filesystem error.
        #[source]
        source: std::io::Error,
    },
    /// Failed to deserialize YAML into a [`crate::schema::Schema`].
    #[error("failed to parse YAML schema `{source_label}`: {source}")]
    Yaml {
        /// Label identifying the schema source (file path or `builtin:<name>`).
        source_label: String,
        /// Underlying YAML deserialization error.
        #[source]
        source: serde_yaml::Error,
    },
    /// Requested `builtin:<name>` does not match any built-in schema.
    #[error("unknown built-in schema `{0}`")]
    UnknownBuiltin(String),
    /// A `builtin:<name>` reference was used while the `known-formats` feature is disabled.
    #[error("built-in schema `{0}` requested, but the `known-formats` feature is disabled")]
    BuiltinFormatsDisabled(String),
    /// An import graph cycle was encountered while resolving imports.
    #[error("cyclic schema import detected: {}", cycle.join(" -> "))]
    CyclicImport {
        /// Ordered sequence of schema references forming the cycle.
        cycle: Vec<String>,
    },
    /// `schema.root` did not resolve to any type definition in the schema or its imports.
    #[error("schema `{schema_name}` specifies root type `{root}`, which does not exist")]
    MissingRootType {
        /// Schema name whose `root` failed to resolve.
        schema_name: String,
        /// Unresolved `root` type reference.
        root: String,
    },
    /// `schema.root` resolved to a `ByteBlock` or `MiniChunks` rather than a top-level chunk type.
    #[error(
        "schema `{schema_name}` root type `{root}` must be `chunks`, `chunk_enum`, or `chunk_stream_seq`"
    )]
    UnsupportedRootKind {
        /// Schema name whose `root` has an unsupported kind.
        schema_name: String,
        /// Resolved root type name.
        root: String,
    },
}

/// Fatal error returned when dynamically parsing a raw chunked binary buffer.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum DynamicLoadError {
    /// Encountered a malformed chunk header or truncated chunk payload while scanning raw bytes.
    #[error("failed to read chunk at byte offset 0x{offset:x}: {message}")]
    ChunkRead {
        /// Byte offset where chunk parsing failed.
        offset: usize,
        /// Underlying `petro-chunked` error description.
        message: String,
    },
}

/// Error returned when generating `petro-chunked-derive` Rust code from a resolved schema.
#[cfg(feature = "codegen")]
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum CodegenError {
    /// A schema, type, field, variant, or module name is not a valid Rust identifier.
    #[error("invalid Rust identifier `{name}` ({context})")]
    InvalidIdentifier {
        /// Offending identifier text.
        name: String,
        /// Where in the schema the identifier appeared.
        context: String,
    },
    /// A `DefaultExpr` string could not be parsed as a valid Rust expression.
    #[error("invalid default expression `{expr}` on field `{field}`: {message}")]
    InvalidDefaultExpr {
        /// Field name carrying the invalid expression.
        field: String,
        /// Raw expression string.
        expr: String,
        /// `syn` parser message.
        message: String,
    },
    /// A cross-module or local type reference could not be resolved.
    #[error("unresolved type `{type_ref}` referenced from module `{module}`")]
    UnresolvedType {
        /// Module name or `"root"` where the reference originated.
        module: String,
        /// Referenced type name or `alias::Type`.
        type_ref: String,
    },
    /// A `chunks` type with `mode: map` was referenced via `source_kind: stream`.
    #[error(
        "type `{type_name}` is used as a `stream` item, which requires `mode: sequence` (cannot derive `ChunkStreamItem` on `#[chunk(map)]`)"
    )]
    MapUsedAsStreamItem {
        /// Name of the `map` container type referenced in a stream.
        type_name: String,
    },
    /// Root type is not a chunk-level root (`chunks`, `chunk_enum`, or `chunk_stream_seq`).
    #[error("root type `{type_name}` cannot be parsed as top-level chunk stream")]
    InvalidRootType {
        /// Name of the invalid root type.
        type_name: String,
    },
    /// Assembled `quote!` token stream could not be parsed by `syn` before formatting with `prettyplease`.
    #[error("generated Rust tokens failed to parse as a Rust file: {0}")]
    SynParse(String),
}
