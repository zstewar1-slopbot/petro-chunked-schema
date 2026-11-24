//! Declarative YAML schema descriptors, multi-module import resolution, dynamic binary loading,
//! and optional `petro-chunked-derive` Rust code generation for Petroglyph's chunked binary file
//! formats.
//!
//! `petro-chunked-schema` sits alongside [`petro_chunked`]: while `petro_chunked` provides the
//! low-level chunk/mini-chunk iterators and compile-time derive macros, this crate describes those
//! binary layouts dynamically in YAML so tools, validators, and interactive editors can inspect,
//! validate, and reverse-engineer Petroglyph formats (`.alo`, `.ala`, `.ted`, `.bui`, etc.)
//! without recompiling.
//!
//! # Crate Features
//!
//! | Feature | Default | Description |
//! |---|---|---|
//! | `known-formats` | **Yes** | Embeds built-in YAML schemas (`known_formats`) for `.alo` models (`AloModel`), `.alo` particle systems (`AloParticle`), the combined `.alo` dispatcher (`Alo`), and `.ala` animations (`AlaAnimation`). |
//! | `codegen` | No | Enables `codegen::generate_rust_code` (via `quote`, `syn`, and `prettyplease`) to emit formatted Rust modules using `petro-chunked-derive`. |
//!
//! # Quick Start
//!
//! ## Parsing an inline YAML schema and inspecting bytes dynamically
//!
//! ```rust
//! use petro_chunked_schema::dynamic::{LoadOptions, load_dynamic_file, validate_bytes};
//! use petro_chunked_schema::{Schema, SchemaLoader};
//!
//! let yaml = r#"
//! name: SampleFormat
//! root: SampleFile
//! types:
//!   SampleFile:
//!     kind: chunks
//!     mode: sequence
//!     fields:
//!       - name: header
//!         source_kind: id
//!         id: 0x00000001
//!         payload_kind: data
//!         payload_type: Header
//!   Header:
//!     kind: byte_block
//!     fields:
//!       - name: version
//!         ty: u32
//! "#;
//!
//! let schema = Schema::from_yaml_str(yaml)?;
//! let resolved = SchemaLoader::resolve(schema, None)?;
//!
//! // Chunk 0x00000001 (4-byte data payload containing version = 7)
//! let mut bytes = Vec::new();
//! bytes.extend_from_slice(&0x0000_0001u32.to_le_bytes());
//! bytes.extend_from_slice(&4u32.to_le_bytes());
//! bytes.extend_from_slice(&7u32.to_le_bytes());
//!
//! let dyn_file = validate_bytes(&resolved, &bytes).expect("valid chunk stream");
//! assert_eq!(dyn_file.chunks[0].schema_field_name.as_deref(), Some("header"));
//!
//! let inspected = load_dynamic_file(&bytes, Some(&resolved), &LoadOptions::default())?;
//! assert!(inspected.diagnostics.is_empty());
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! ## Loading a built-in schema by URI (`known-formats` feature)
//!
//! [`SchemaRef`] accepts URI-like references (`builtin:<Name>` or `file:<Path>`, defaulting bare
//! strings to file paths):
//!
//! ```rust
//! # #[cfg(feature = "known-formats")]
//! # fn run() -> Result<(), Box<dyn std::error::Error>> {
//! use petro_chunked_schema::{SchemaLoader, SchemaRef};
//!
//! // Loads `Alo`, which imports both `builtin:AloModel` and `builtin:AloParticle`.
//! let alo_ref = SchemaRef::parse("builtin:Alo")?;
//! let resolved = SchemaLoader::load(&alo_ref)?;
//! assert_eq!(resolved.root_schema().name, "Alo");
//! assert_eq!(resolved.modules().len(), 3);
//! # Ok(())
//! # }
//! # #[cfg(feature = "known-formats")]
//! # run().unwrap();
//! ```
//!
//! ## Generating `petro-chunked-derive` Rust definitions (`codegen` feature)
//!
//! ```rust
//! # #[cfg(all(feature = "codegen", feature = "known-formats"))]
//! # fn run() -> Result<(), Box<dyn std::error::Error>> {
//! use petro_chunked_schema::{SchemaLoader, SchemaRef, generate_rust_code};
//!
//! let resolved = SchemaLoader::load(&SchemaRef::parse("builtin:AlaAnimation")?)?;
//! let rust_src = generate_rust_code(&resolved)?;
//! assert!(rust_src.contains("pub struct AlaAnimation"));
//! assert!(rust_src.contains("pub fn parse"));
//! # Ok(())
//! # }
//! # #[cfg(all(feature = "codegen", feature = "known-formats"))]
//! # run().unwrap();
//! ```
//!
//! # YAML Schema Format Guide
//!
//! Every schema YAML document has the same uniform top-level structure:
//!
//! - `name`: Identifier for the format (used by Rust codegen and UI labels).
//! - `description` *(optional)*: Human-readable summary emitted as doc comments in generated code.
//! - `file_extensions` *(optional)*: Lowercase extensions without leading dots (e.g. `[alo]`).
//! - `imports` *(optional)*: Map of `alias: <SchemaRef>` bringing external schemas into scope as
//!   `alias::TypeName` (for example `model: builtin:AloModel` or `common: ./common.yaml`).
//! - `root`: Name of the entry-point type in `types` (or `alias::TypeName`), which must be a
//!   `chunks`, `chunk_enum`, or `chunk_stream_seq` definition.
//! - `types`: Ordered map (`IndexMap<String, TypeDef>`) of named type definitions, conventionally
//!   ordered parents-before-children:
//!   - `kind: chunks` — 32-bit chunk container (`mode: map | sequence`, `ignore_unknown: bool`).
//!   - `kind: mini_chunks` — 8-bit mini-chunk container (`mode: map | sequence`, `ignore_unknown: bool`).
//!   - `kind: byte_block` — sequential binary struct with optional `pad_after`.
//!   - `kind: chunk_stream_seq` — sequential multi-chunk stream item pulling sibling chunks.
//!   - `kind: chunk_enum` — chunk-tagged enum dispatching on the next chunk ID in a stream.
//!
//! ### `SchemaRef` URI Schemes
//!
//! - `builtin:<Name>` resolves against `known_formats` (e.g. `builtin:Alo`, `builtin:AloModel`,
//!   `builtin:AloParticle`, `builtin:AlaAnimation`).
//! - `file:<Path>` forces local filesystem resolution even when a path starts with `builtin:` or
//!   `file:` (e.g. `file:builtin:foo`).
//! - Any string without a recognized scheme is parsed as [`SchemaRef::File`].
//!
//! ### `DataType` String Syntax
//!
//! Data fields and leaf payloads accept concise Rust-like type strings:
//! `u8`, `i8`, `u16`, `i16`, `u32`, `i32`, `u64`, `i64`, `f32`, `f64`, `bool`, `usize`, `c_string`,
//! `path`, `vec2`, `vec3`, `vec4`, `mat3x4`, `raw_bytes`, `[T; N]`, `Vec<T>`, `()`, `(T,)`,
//! `(T1, T2)`, and named `ByteBlock` references (`SkeletonHeader` or `model::VertexV1`).
//!
//! # Module Guide
//!
//! - [`schema`] — Core YAML AST types (`Schema`, `TypeDef`, `DataType`, `Cardinality`, `WireFormat`).
//! - [`source`] — [`SchemaRef`] URI parsing, display formatting, and relative path resolution.
//! - [`resolve`] — [`SchemaLoader`] import graph resolution, cycle detection, and [`ResolvedSchema`].
//! - [`dynamic`] — Two-stage raw parser, schema evaluator, [`dynamic::ByteSpanAnnotation`], and
//!   multi-format cursor inspector ([`dynamic::inspect_bytes_at_offset`]).
//! - [`error`] — Structured `thiserror` error types across loading, resolution, and codegen.
#![cfg_attr(
    feature = "known-formats",
    doc = " - [`known_formats`] *(feature `known-formats`)* — Embedded built-in YAML formats."
)]
#![cfg_attr(
    not(feature = "known-formats"),
    doc = " - `known_formats` *(feature `known-formats`)* — Embedded built-in YAML formats."
)]
#![cfg_attr(
    feature = "codegen",
    doc = " - [`codegen`] *(feature `codegen`)* — `quote` + `prettyplease` `petro-chunked-derive` emitter."
)]
#![cfg_attr(
    not(feature = "codegen"),
    doc = " - `codegen` *(feature `codegen`)* — `quote` + `prettyplease` `petro-chunked-derive` emitter."
)]

#![warn(missing_docs)]

#[cfg(feature = "codegen")]
pub mod codegen;
pub mod dynamic;
pub mod error;
#[cfg(feature = "known-formats")]
pub mod known_formats;
pub mod resolve;
pub mod schema;
pub mod source;

#[cfg(feature = "codegen")]
pub use codegen::generate_rust_code;
pub use resolve::{ResolvedModule, ResolvedSchema, SchemaId, SchemaLoader};
pub use schema::Schema;
pub use source::SchemaRef;
