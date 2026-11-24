//! Rust code generator that converts a [`Schema`] into declarative `petro-chunked-derive`
//! (`FromByteBlock`, `MiniChunkStructure`, `ChunkStructure`, `ChunkStreamItem`) definitions.

use std::fmt::Write;

use crate::error::CodegenError;
use crate::resolve::ResolvedSchema;
use crate::schema::{
    ByteBlockDef, Cardinality, ChunkContainerDef, ChunkEnumDef, ChunkFieldDef, ChunkFieldSource,
    ChunkPayloadSpec, DataType, MiniChunkContainerDef, OrderMode, TypeDef,
};

/// Generates a complete Rust module implementing `resolved` with `petro-chunked-derive`.
pub fn generate_rust_code(resolved: &ResolvedSchema) -> Result<String, CodegenError> {
    let schema = resolved.root_schema();
    let mut out = String::new();
    let _ = writeln!(
        out,
        "//! Auto-generated `petro-chunked-derive` definitions for `{}`.",
        schema.name
    );
    if let Some(desc) = &schema.description {
        let _ = writeln!(out, "//!\n//! {desc}");
    }
    let _ = writeln!(
        out,
        "\nuse std::path::PathBuf;\nuse petro_chunked::byteblock::FromByteBlock;\nuse petro_chunked::wire_types::{{U8Bool, U32Bool, U32Usize, U32UsizeVec}};\nuse petro_chunked::{{ChunkErrorV2, ChunkStreamItem, ChunkStructure, MiniChunkStructure, iter_chunks}};\n"
    );

    // Emit all named types in root schema
    for (type_name, typedef) in &schema.types {
        match typedef {
            TypeDef::ByteBlock(bb) => emit_byte_block_struct(&mut out, type_name, bb),
            TypeDef::MiniChunks(mc) => emit_mini_chunk_struct(&mut out, type_name, mc),
            TypeDef::Chunks(cc) => emit_chunk_container_struct(&mut out, type_name, cc, false),
            TypeDef::ChunkStreamSeq(seq) => {
                let cc = ChunkContainerDef {
                    description: seq.description.clone(),
                    mode: OrderMode::Sequence,
                    ignore_unknown: false,
                    fields: seq.fields.clone(),
                };
                emit_chunk_container_struct(&mut out, type_name, &cc, true);
            }
            TypeDef::ChunkEnum(ce) => emit_chunk_enum(&mut out, type_name, ce),
        }
    }

    Ok(out)
}

fn emit_doc_comment(out: &mut String, indent: &str, desc: &Option<String>) {
    if let Some(desc) = desc {
        for line in desc.lines() {
            let _ = writeln!(out, "{indent}/// {line}");
        }
    }
}

fn emit_byte_block_struct(out: &mut String, name: &str, bb: &ByteBlockDef) {
    emit_doc_comment(out, "", &bb.description);
    let _ = writeln!(out, "#[derive(Debug, Clone, PartialEq, FromByteBlock)]");
    if let Some(pad) = bb.pad_after
        && pad > 0
    {
        let _ = writeln!(out, "#[byte_block(pad_after = {pad})]");
    }
    let _ = writeln!(out, "pub struct {name} {{");
    for f in &bb.fields {
        emit_doc_comment(out, "    ", &f.description);
        let mut attrs = Vec::new();
        let wf = f.wire_format.or_else(|| f.ty.default_wire_format());
        if let Some(wf) = wf {
            attrs.push(format!("wire_format = {}", wf.rust_type_name()));
        }
        if let Some(pad) = f.pad_after
            && pad > 0
        {
            attrs.push(format!("pad_after = {pad}"));
        }
        if !attrs.is_empty() {
            let _ = writeln!(out, "    #[byte_block({})]", attrs.join(", "));
        }
        let _ = writeln!(out, "    pub {}: {},", f.name, rust_type_for_data(&f.ty));
    }
    let _ = writeln!(out, "}}\n");
}

fn emit_mini_chunk_struct(out: &mut String, name: &str, mc: &MiniChunkContainerDef) {
    emit_doc_comment(out, "", &mc.description);
    let _ = writeln!(out, "#[derive(Debug, Clone, PartialEq, MiniChunkStructure)]");
    let mode_str = match mc.mode {
        OrderMode::Map => "map",
        OrderMode::Sequence => "sequence",
    };
    if mc.ignore_unknown {
        let _ = writeln!(out, "#[mini_chunk({mode_str}, ignore_unknown)]");
    } else {
        let _ = writeln!(out, "#[mini_chunk({mode_str})]");
    }
    let _ = writeln!(out, "pub struct {name} {{");
    for f in &mc.fields {
        emit_doc_comment(out, "    ", &f.description);
        let mut parts = vec![format!("0x{:02x}", f.id)];
        append_cardinality_attr(&mut parts, &f.cardinality);
        let wf = f.wire_format.or_else(|| f.ty.default_wire_format());
        if let Some(wf) = wf {
            parts.push(format!("wire_format = {}", wf.rust_type_name()));
        }
        let _ = writeln!(out, "    #[mini_chunk({})]", parts.join(", "));
        let inner_ty = rust_type_for_data(&f.ty);
        let field_ty = wrap_type_with_cardinality(&inner_ty, &f.cardinality);
        let _ = writeln!(out, "    pub {}: {field_ty},", f.name);
    }
    let _ = writeln!(out, "}}\n");
}

fn emit_chunk_container_struct(
    out: &mut String,
    name: &str,
    cc: &ChunkContainerDef,
    is_stream_item: bool,
) {
    emit_doc_comment(out, "", &cc.description);
    if is_stream_item {
        let _ = writeln!(out, "#[derive(Debug, Clone, PartialEq, ChunkStreamItem)]");
    } else {
        let _ = writeln!(out, "#[derive(Debug, Clone, PartialEq, ChunkStructure)]");
        let mode_str = match cc.mode {
            OrderMode::Map => "map",
            OrderMode::Sequence => "sequence",
        };
        if cc.ignore_unknown {
            let _ = writeln!(out, "#[chunk({mode_str}, ignore_unknown)]");
        } else {
            let _ = writeln!(out, "#[chunk({mode_str})]");
        }
    }
    let _ = writeln!(out, "pub struct {name} {{");
    for f in &cc.fields {
        emit_chunk_field(out, f);
    }
    let _ = writeln!(out, "}}\n");
}

fn emit_chunk_field(out: &mut String, f: &ChunkFieldDef) {
    emit_doc_comment(out, "    ", &f.description);
    let (mut parts, inner_ty, default_wf) = match &f.source {
        ChunkFieldSource::Id { id, payload } => {
            let mut p = vec![format!("0x{id:08x}")];
            let (ty, dwf) = match payload {
                ChunkPayloadSpec::Data(dt) => (rust_type_for_data(dt), dt.default_wire_format()),
                ChunkPayloadSpec::MiniChunks(name) => {
                    p.push("mini_chunks".to_owned());
                    (name.clone(), None)
                }
                ChunkPayloadSpec::Children(name) => {
                    p.push("children".to_owned());
                    (name.clone(), None)
                }
                ChunkPayloadSpec::Ignore => {
                    p.push("ignore".to_owned());
                    ("()".to_owned(), None)
                }
            };
            (p, ty, dwf)
        }
        ChunkFieldSource::Stream { item_type } => {
            (vec!["stream".to_owned()], item_type.clone(), None)
        }
    };
    append_cardinality_attr(&mut parts, &f.cardinality);
    if let Some(wf) = f.wire_format.or(default_wf) {
        parts.push(format!("wire_format = {}", wf.rust_type_name()));
    }
    let _ = writeln!(out, "    #[chunk({})]", parts.join(", "));
    let field_ty = wrap_type_with_cardinality(&inner_ty, &f.cardinality);
    let _ = writeln!(out, "    pub {}: {field_ty},", f.name);
}

fn emit_chunk_enum(out: &mut String, name: &str, ce: &ChunkEnumDef) {
    emit_doc_comment(out, "", &ce.description);
    let _ = writeln!(out, "#[derive(Debug, Clone, PartialEq, ChunkStreamItem)]");
    let _ = writeln!(out, "pub enum {name} {{");
    for v in &ce.variants {
        emit_doc_comment(out, "    ", &v.description);
        match &v.source {
            ChunkFieldSource::Id { id, payload } => {
                let mut parts = vec![format!("0x{id:08x}")];
                let (inner_ty, default_wf) = match payload {
                    ChunkPayloadSpec::Data(dt) => {
                        (Some(rust_type_for_data(dt)), dt.default_wire_format())
                    }
                    ChunkPayloadSpec::MiniChunks(t) => {
                        parts.push("mini_chunks".to_owned());
                        (Some(t.clone()), None)
                    }
                    ChunkPayloadSpec::Children(t) => {
                        parts.push("children".to_owned());
                        (Some(t.clone()), None)
                    }
                    ChunkPayloadSpec::Ignore => (None, None),
                };
                if let Some(wf) = v.wire_format.or(default_wf) {
                    parts.push(format!("wire_format = {}", wf.rust_type_name()));
                }
                let _ = writeln!(out, "    #[chunk({})]", parts.join(", "));
                match inner_ty {
                    Some(ty) => {
                        let _ = writeln!(out, "    {}({ty}),", v.name);
                    }
                    None => {
                        let _ = writeln!(out, "    {},", v.name);
                    }
                }
            }
            ChunkFieldSource::Stream { item_type } => {
                let mut parts = vec!["stream".to_owned()];
                if let Some(wf) = v.wire_format {
                    parts.push(format!("wire_format = {}", wf.rust_type_name()));
                }
                let _ = writeln!(out, "    #[chunk({})]", parts.join(", "));
                let _ = writeln!(out, "    {}({item_type}),", v.name);
            }
        }
    }
    let _ = writeln!(out, "}}\n");
}

fn append_cardinality_attr(parts: &mut Vec<String>, card: &Cardinality) {
    match card {
        Cardinality::Required => {}
        Cardinality::Optional => parts.push("optional".to_owned()),
        Cardinality::Repeated => parts.push("repeated".to_owned()),
        Cardinality::Default => parts.push("default".to_owned()),
        Cardinality::DefaultExpr(expr) => parts.push(format!("default = {expr}")),
    }
}

fn wrap_type_with_cardinality(inner_ty: &str, card: &Cardinality) -> String {
    match card {
        Cardinality::Optional => format!("Option<{inner_ty}>"),
        Cardinality::Repeated => format!("Vec<{inner_ty}>"),
        Cardinality::Required | Cardinality::Default | Cardinality::DefaultExpr(_) => {
            inner_ty.to_owned()
        }
    }
}

fn rust_type_for_data(dt: &DataType) -> String {
    match dt {
        DataType::U8 => "u8".to_owned(),
        DataType::I8 => "i8".to_owned(),
        DataType::U16 => "u16".to_owned(),
        DataType::I16 => "i16".to_owned(),
        DataType::U32 => "u32".to_owned(),
        DataType::I32 => "i32".to_owned(),
        DataType::U64 => "u64".to_owned(),
        DataType::I64 => "i64".to_owned(),
        DataType::F32 => "f32".to_owned(),
        DataType::F64 => "f64".to_owned(),
        DataType::Bool => "bool".to_owned(),
        DataType::Usize => "usize".to_owned(),
        DataType::CString => "String".to_owned(),
        DataType::Path => "PathBuf".to_owned(),
        DataType::Vec2 => "[f32; 2]".to_owned(),
        DataType::Vec3 => "[f32; 3]".to_owned(),
        DataType::Vec4 => "[f32; 4]".to_owned(),
        DataType::Mat3x4 => "[[f32; 4]; 3]".to_owned(),
        DataType::Array { elem, len } => format!("[{}; {len}]", rust_type_for_data(elem)),
        DataType::List(elem) => format!("Vec<{}>", rust_type_for_data(elem)),
        DataType::Tuple(items) => {
            let inner: Vec<String> = items.iter().map(rust_type_for_data).collect();
            format!("({})", inner.join(", "))
        }
        DataType::RawBytes => "Vec<u8>".to_owned(),
        DataType::Named(name) => name.clone(),
    }
}
