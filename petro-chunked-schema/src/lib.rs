//! YAML schema descriptor format, dynamic chunked file loader, and `petro-chunked-derive` Rust
//! code generator for Petroglyph's chunked binary file formats.

pub mod codegen;
pub mod dynamic;
#[cfg(feature = "known-formats")]
pub mod known_formats;
pub mod schema;

pub use codegen::generate_rust_code;
pub use schema::Schema;
