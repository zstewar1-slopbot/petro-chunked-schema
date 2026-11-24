//! YAML schema descriptor format, multi-module import resolver, dynamic chunked file loader,
//! and optional `petro-chunked-derive` Rust code generator for Petroglyph's chunked binary file
//! formats.

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
