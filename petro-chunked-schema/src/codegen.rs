//! AST-based Rust code generator converting a [`ResolvedSchema`] into declarative
//! `petro-chunked-derive` (`FromByteBlock`, `MiniChunkStructure`, `ChunkStructure`,
//! `ChunkStreamItem`) definitions formatted via `prettyplease`.

use std::collections::BTreeSet;

use proc_macro2::{Span, TokenStream};
use quote::quote;

use crate::error::CodegenError;
use crate::resolve::{ResolvedModule, ResolvedSchema, SchemaId};
use crate::schema::{
    ByteBlockDef, Cardinality, ChunkContainerDef, ChunkEnumDef, ChunkFieldDef, ChunkFieldSource,
    ChunkPayloadSpec, ChunkStreamSeqDef, DataType, MiniChunkContainerDef, OrderMode, TypeDef,
};

/// Generates a formatted Rust source file implementing `resolved` using `petro-chunked-derive`.
///
/// Built with `quote` and formatted through `syn` + `prettyplease`. Any imported schemas are
/// emitted as `pub mod <module_name> { ... }` submodules alongside the root definitions.
pub fn generate_rust_code(resolved: &ResolvedSchema) -> Result<String, CodegenError> {
    let stream_types = resolved.referenced_stream_types();
    let root_schema = resolved.root_schema();
    let (root_mod_id, root_type_name, root_def) = resolved.root_type();

    let mut header_lines = vec![format!(
        " Auto-generated `petro-chunked-derive` definitions for `{}`.",
        root_schema.name
    )];
    if let Some(desc) = &root_schema.description {
        header_lines.push(String::new());
        for line in desc.lines() {
            header_lines.push(if line.is_empty() {
                String::new()
            } else {
                format!(" {line}")
            });
        }
    }
    let inner_docs: Vec<TokenStream> = header_lines
        .into_iter()
        .map(|text| quote!(#![doc = #text]))
        .collect();

    let prelude = emit_prelude();
    let root_module = resolved.module(SchemaId::ROOT);
    let root_items = emit_module_types(resolved, root_module, &stream_types, Some((root_mod_id, root_type_name, root_def)))?;

    let mut submodules = Vec::new();
    for module in resolved.modules().iter().skip(1) {
        let mod_name_str = module
            .module_name
            .as_deref()
            .unwrap_or("imported_schema");
        let mod_ident = make_ident(mod_name_str, "module name")?;
        let mod_docs = emit_inner_docs(&module.schema.description, &module.schema.name);
        let mod_prelude = emit_prelude();
        let mod_items = emit_module_types(
            resolved,
            module,
            &stream_types,
            (root_mod_id == module.id).then_some((root_mod_id, root_type_name, root_def)),
        )?;
        submodules.push(quote! {
            pub mod #mod_ident {
                #(#mod_docs)*
                #mod_prelude
                #(#mod_items)*
            }
        });
    }

    let tokens = quote! {
        #(#inner_docs)*
        #prelude
        #(#root_items)*
        #(#submodules)*
    };

    let file = syn::parse2::<syn::File>(tokens)
        .map_err(|e| CodegenError::SynParse(e.to_string()))?;
    Ok(prettyplease::unparse(&file))
}

fn emit_prelude() -> TokenStream {
    quote! {
        use std::path::PathBuf;
        use petro_chunked::byteblock::FromByteBlock;
        use petro_chunked::wire_types::{U8Bool, U32Bool, U32Usize, U32UsizeVec};
        use petro_chunked::{ChunkErrorV2, ChunkStreamItem, ChunkStructure, MiniChunkStructure, iter_chunks};
    }
}

fn emit_inner_docs(description: &Option<String>, schema_name: &str) -> Vec<TokenStream> {
    let mut lines = vec![format!(" Schema module `{schema_name}`.")];
    if let Some(desc) = description {
        lines.push(String::new());
        for line in desc.lines() {
            lines.push(if line.is_empty() {
                String::new()
            } else {
                format!(" {line}")
            });
        }
    }
    lines.into_iter().map(|s| quote!(#![doc = #s])).collect()
}

fn emit_outer_docs(description: &Option<String>) -> Vec<TokenStream> {
    let Some(desc) = description else {
        return Vec::new();
    };
    desc.lines()
        .map(|line| {
            let text = if line.is_empty() {
                String::new()
            } else {
                format!(" {line}")
            };
            quote!(#[doc = #text])
        })
        .collect()
}

fn emit_module_types(
    resolved: &ResolvedSchema,
    module: &ResolvedModule,
    stream_types: &BTreeSet<(SchemaId, String)>,
    root_here: Option<(SchemaId, &str, &TypeDef)>,
) -> Result<Vec<TokenStream>, CodegenError> {
    let mut items = Vec::new();
    for (type_name, typedef) in &module.schema.types {
        let is_stream_item = stream_types.contains(&(module.id, type_name.clone()));
        let item = match typedef {
            TypeDef::ByteBlock(bb) => emit_byte_block_struct(resolved, module.id, type_name, bb)?,
            TypeDef::MiniChunks(mc) => {
                emit_mini_chunk_struct(resolved, module.id, type_name, mc)?
            }
            TypeDef::Chunks(cc) => {
                emit_chunk_container_struct(resolved, module.id, type_name, cc, is_stream_item)?
            }
            TypeDef::ChunkStreamSeq(seq) => {
                emit_chunk_stream_seq_struct(resolved, module.id, type_name, seq)?
            }
            TypeDef::ChunkEnum(ce) => emit_chunk_enum(resolved, module.id, type_name, ce)?,
        };
        items.push(item);

        if let Some((root_mod, root_name, root_def)) = root_here
            && root_mod == module.id
            && root_name == type_name
        {
            items.push(emit_root_parse_impl(root_name, root_def)?);
        }
    }
    Ok(items)
}

fn emit_root_parse_impl(root_name: &str, root_def: &TypeDef) -> Result<TokenStream, CodegenError> {
    let root_ident = make_ident(root_name, "root type")?;
    match root_def {
        TypeDef::Chunks(_) => Ok(quote! {
            impl #root_ident {
                pub fn parse(bytes: &[u8]) -> Result<Self, ChunkErrorV2> {
                    iter_chunks(bytes).expect_struct()
                }
            }
        }),
        TypeDef::ChunkEnum(_) | TypeDef::ChunkStreamSeq(_) => Ok(quote! {
            impl #root_ident {
                pub fn parse(bytes: &[u8]) -> Result<Self, ChunkErrorV2> {
                    let mut chunks = iter_chunks(bytes);
                    let value = chunks.expect_item()?;
                    chunks.expect_empty()?;
                    Ok(value)
                }
            }
        }),
        TypeDef::ByteBlock(_) | TypeDef::MiniChunks(_) => Err(CodegenError::InvalidRootType {
            type_name: root_name.to_owned(),
        }),
    }
}

fn emit_byte_block_struct(
    resolved: &ResolvedSchema,
    module_id: SchemaId,
    name: &str,
    bb: &ByteBlockDef,
) -> Result<TokenStream, CodegenError> {
    let docs = emit_outer_docs(&bb.description);
    let ident = make_ident(name, "ByteBlock type")?;
    let container_attr = match bb.pad_after {
        Some(pad) if pad > 0 => {
            let pad_lit = syn::LitInt::new(&pad.to_string(), Span::call_site());
            quote!(#[byte_block(pad_after = #pad_lit)])
        }
        _ => quote!(),
    };

    let mut fields_tokens = Vec::with_capacity(bb.fields.len());
    for f in &bb.fields {
        let fdocs = emit_outer_docs(&f.description);
        let fident = make_ident(&f.name, &format!("field on ByteBlock `{name}`"))?;
        let fty = rust_type_for_data(resolved, module_id, &f.ty, &format!("field `{}`", f.name))?;

        let mut attrs = Vec::new();
        if let Some(wf) = f.wire_format.or_else(|| f.ty.default_wire_format()) {
            let wf_ident = make_ident(wf.rust_type_name(), "wire_format")?;
            attrs.push(quote!(wire_format = #wf_ident));
        }
        if let Some(pad) = f.pad_after
            && pad > 0
        {
            let pad_lit = syn::LitInt::new(&pad.to_string(), Span::call_site());
            attrs.push(quote!(pad_after = #pad_lit));
        }
        let field_attr = if attrs.is_empty() {
            quote!()
        } else {
            quote!(#[byte_block(#(#attrs),*)])
        };

        fields_tokens.push(quote! {
            #(#fdocs)*
            #field_attr
            pub #fident: #fty,
        });
    }

    Ok(quote! {
        #(#docs)*
        #[derive(Debug, Clone, PartialEq, FromByteBlock)]
        #container_attr
        pub struct #ident {
            #(#fields_tokens)*
        }
    })
}

fn emit_mini_chunk_struct(
    resolved: &ResolvedSchema,
    module_id: SchemaId,
    name: &str,
    mc: &MiniChunkContainerDef,
) -> Result<TokenStream, CodegenError> {
    let docs = emit_outer_docs(&mc.description);
    let ident = make_ident(name, "MiniChunks type")?;
    let mode_ident = match mc.mode {
        OrderMode::Map => quote!(map),
        OrderMode::Sequence => quote!(sequence),
    };
    let container_attr = if mc.ignore_unknown {
        quote!(#[mini_chunk(#mode_ident, ignore_unknown)])
    } else {
        quote!(#[mini_chunk(#mode_ident)])
    };

    let mut fields_tokens = Vec::with_capacity(mc.fields.len());
    for f in &mc.fields {
        let fdocs = emit_outer_docs(&f.description);
        let fident = make_ident(&f.name, &format!("field on MiniChunks `{name}`"))?;
        let id_lit: syn::LitInt =
            syn::parse_str(&format!("0x{:02x}", f.id)).expect("valid u8 hex literal");

        let mut attr_parts = vec![quote!(#id_lit)];
        append_cardinality_tokens(&mut attr_parts, &f.name, &f.cardinality)?;
        if let Some(wf) = f.wire_format.or_else(|| f.ty.default_wire_format()) {
            let wf_ident = make_ident(wf.rust_type_name(), "wire_format")?;
            attr_parts.push(quote!(wire_format = #wf_ident));
        }

        let inner_ty =
            rust_type_for_data(resolved, module_id, &f.ty, &format!("mini-chunk `{}`", f.name))?;
        let field_ty = wrap_type_with_cardinality(inner_ty, &f.cardinality);

        fields_tokens.push(quote! {
            #(#fdocs)*
            #[mini_chunk(#(#attr_parts),*)]
            pub #fident: #field_ty,
        });
    }

    Ok(quote! {
        #(#docs)*
        #[derive(Debug, Clone, PartialEq, MiniChunkStructure)]
        #container_attr
        pub struct #ident {
            #(#fields_tokens)*
        }
    })
}

fn emit_chunk_container_struct(
    resolved: &ResolvedSchema,
    module_id: SchemaId,
    name: &str,
    cc: &ChunkContainerDef,
    also_stream_item: bool,
) -> Result<TokenStream, CodegenError> {
    if also_stream_item && cc.mode == OrderMode::Map {
        return Err(CodegenError::MapUsedAsStreamItem {
            type_name: name.to_owned(),
        });
    }

    let docs = emit_outer_docs(&cc.description);
    let ident = make_ident(name, "Chunks type")?;
    let mode_ident = match cc.mode {
        OrderMode::Map => quote!(map),
        OrderMode::Sequence => quote!(sequence),
    };
    let container_attr = if cc.ignore_unknown {
        quote!(#[chunk(#mode_ident, ignore_unknown)])
    } else {
        quote!(#[chunk(#mode_ident)])
    };
    let derive_attr = if also_stream_item {
        quote!(#[derive(Debug, Clone, PartialEq, ChunkStructure, ChunkStreamItem)])
    } else {
        quote!(#[derive(Debug, Clone, PartialEq, ChunkStructure)])
    };

    let mut fields_tokens = Vec::with_capacity(cc.fields.len());
    for f in &cc.fields {
        fields_tokens.push(emit_chunk_field(resolved, module_id, name, f)?);
    }

    Ok(quote! {
        #(#docs)*
        #derive_attr
        #container_attr
        pub struct #ident {
            #(#fields_tokens)*
        }
    })
}

fn emit_chunk_stream_seq_struct(
    resolved: &ResolvedSchema,
    module_id: SchemaId,
    name: &str,
    seq: &ChunkStreamSeqDef,
) -> Result<TokenStream, CodegenError> {
    let docs = emit_outer_docs(&seq.description);
    let ident = make_ident(name, "ChunkStreamSeq type")?;
    let mut fields_tokens = Vec::with_capacity(seq.fields.len());
    for f in &seq.fields {
        fields_tokens.push(emit_chunk_field(resolved, module_id, name, f)?);
    }

    Ok(quote! {
        #(#docs)*
        #[derive(Debug, Clone, PartialEq, ChunkStreamItem)]
        pub struct #ident {
            #(#fields_tokens)*
        }
    })
}

fn emit_chunk_field(
    resolved: &ResolvedSchema,
    module_id: SchemaId,
    container_name: &str,
    f: &ChunkFieldDef,
) -> Result<TokenStream, CodegenError> {
    let fdocs = emit_outer_docs(&f.description);
    let fident = make_ident(&f.name, &format!("field on `{container_name}`"))?;
    let ctx_label = format!("field `{container_name}.{}`", f.name);

    let (mut attr_parts, inner_ty, default_wf) = match &f.source {
        ChunkFieldSource::Id { id, payload } => {
            let id_lit: syn::LitInt =
                syn::parse_str(&format!("0x{id:08x}")).expect("valid u32 hex literal");
            let mut parts = vec![quote!(#id_lit)];
            let (ty, dwf) = match payload {
                ChunkPayloadSpec::Data(dt) => (
                    rust_type_for_data(resolved, module_id, dt, &ctx_label)?,
                    dt.default_wire_format(),
                ),
                ChunkPayloadSpec::MiniChunks(type_ref) => {
                    parts.push(quote!(mini_chunks));
                    (
                        resolve_rust_type_path(resolved, module_id, type_ref, &ctx_label)?,
                        None,
                    )
                }
                ChunkPayloadSpec::Children(type_ref) => {
                    parts.push(quote!(children));
                    (
                        resolve_rust_type_path(resolved, module_id, type_ref, &ctx_label)?,
                        None,
                    )
                }
                ChunkPayloadSpec::Ignore => {
                    parts.push(quote!(ignore));
                    (quote!(()), None)
                }
            };
            (parts, ty, dwf)
        }
        ChunkFieldSource::Stream { item_type } => {
            let item_ty = resolve_rust_type_path(resolved, module_id, item_type, &ctx_label)?;
            (vec![quote!(stream)], item_ty, None)
        }
    };

    append_cardinality_tokens(&mut attr_parts, &f.name, &f.cardinality)?;
    if let Some(wf) = f.wire_format.or(default_wf) {
        let wf_ident = make_ident(wf.rust_type_name(), "wire_format")?;
        attr_parts.push(quote!(wire_format = #wf_ident));
    }

    let field_ty = wrap_type_with_cardinality(inner_ty, &f.cardinality);

    Ok(quote! {
        #(#fdocs)*
        #[chunk(#(#attr_parts),*)]
        pub #fident: #field_ty,
    })
}

fn emit_chunk_enum(
    resolved: &ResolvedSchema,
    module_id: SchemaId,
    name: &str,
    ce: &ChunkEnumDef,
) -> Result<TokenStream, CodegenError> {
    let docs = emit_outer_docs(&ce.description);
    let ident = make_ident(name, "ChunkEnum type")?;
    let mut variants_tokens = Vec::with_capacity(ce.variants.len());

    for v in &ce.variants {
        let vdocs = emit_outer_docs(&v.description);
        let vident = make_ident(&v.name, &format!("variant on `{name}`"))?;
        let ctx_label = format!("variant `{name}::{}`", v.name);

        let variant_tokens = match &v.source {
            ChunkFieldSource::Id { id, payload } => {
                let id_lit: syn::LitInt =
                    syn::parse_str(&format!("0x{id:08x}")).expect("valid u32 hex literal");
                let mut attr_parts = vec![quote!(#id_lit)];
                let (inner_ty, default_wf) = match payload {
                    ChunkPayloadSpec::Data(dt) => (
                        Some(rust_type_for_data(resolved, module_id, dt, &ctx_label)?),
                        dt.default_wire_format(),
                    ),
                    ChunkPayloadSpec::MiniChunks(type_ref) => {
                        attr_parts.push(quote!(mini_chunks));
                        (
                            Some(resolve_rust_type_path(
                                resolved, module_id, type_ref, &ctx_label,
                            )?),
                            None,
                        )
                    }
                    ChunkPayloadSpec::Children(type_ref) => {
                        attr_parts.push(quote!(children));
                        (
                            Some(resolve_rust_type_path(
                                resolved, module_id, type_ref, &ctx_label,
                            )?),
                            None,
                        )
                    }
                    ChunkPayloadSpec::Ignore => (None, None),
                };
                if let Some(wf) = v.wire_format.or(default_wf) {
                    let wf_ident = make_ident(wf.rust_type_name(), "wire_format")?;
                    attr_parts.push(quote!(wire_format = #wf_ident));
                }
                match inner_ty {
                    Some(ty) => quote! {
                        #(#vdocs)*
                        #[chunk(#(#attr_parts),*)]
                        #vident(#ty),
                    },
                    None => quote! {
                        #(#vdocs)*
                        #[chunk(#(#attr_parts),*)]
                        #vident,
                    },
                }
            }
            ChunkFieldSource::Stream { item_type } => {
                let mut attr_parts = vec![quote!(stream)];
                if let Some(wf) = v.wire_format {
                    let wf_ident = make_ident(wf.rust_type_name(), "wire_format")?;
                    attr_parts.push(quote!(wire_format = #wf_ident));
                }
                let item_ty = resolve_rust_type_path(resolved, module_id, item_type, &ctx_label)?;
                quote! {
                    #(#vdocs)*
                    #[chunk(#(#attr_parts),*)]
                    #vident(#item_ty),
                }
            }
        };
        variants_tokens.push(variant_tokens);
    }

    Ok(quote! {
        #(#docs)*
        #[derive(Debug, Clone, PartialEq, ChunkStreamItem)]
        pub enum #ident {
            #(#variants_tokens)*
        }
    })
}

fn append_cardinality_tokens(
    parts: &mut Vec<TokenStream>,
    field_name: &str,
    card: &Cardinality,
) -> Result<(), CodegenError> {
    match card {
        Cardinality::Required => {}
        Cardinality::Optional => parts.push(quote!(optional)),
        Cardinality::Repeated => parts.push(quote!(repeated)),
        Cardinality::Default => parts.push(quote!(default)),
        Cardinality::DefaultExpr(expr_str) => {
            let expr: syn::Expr =
                syn::parse_str(expr_str).map_err(|e| CodegenError::InvalidDefaultExpr {
                    field: field_name.to_owned(),
                    expr: expr_str.clone(),
                    message: e.to_string(),
                })?;
            parts.push(quote!(default = #expr));
        }
    }
    Ok(())
}

fn wrap_type_with_cardinality(inner_ty: TokenStream, card: &Cardinality) -> TokenStream {
    match card {
        Cardinality::Optional => quote!(Option<#inner_ty>),
        Cardinality::Repeated => quote!(Vec<#inner_ty>),
        Cardinality::Required | Cardinality::Default | Cardinality::DefaultExpr(_) => inner_ty,
    }
}

fn rust_type_for_data(
    resolved: &ResolvedSchema,
    module_id: SchemaId,
    dt: &DataType,
    context: &str,
) -> Result<TokenStream, CodegenError> {
    match dt {
        DataType::U8 => Ok(quote!(u8)),
        DataType::I8 => Ok(quote!(i8)),
        DataType::U16 => Ok(quote!(u16)),
        DataType::I16 => Ok(quote!(i16)),
        DataType::U32 => Ok(quote!(u32)),
        DataType::I32 => Ok(quote!(i32)),
        DataType::U64 => Ok(quote!(u64)),
        DataType::I64 => Ok(quote!(i64)),
        DataType::F32 => Ok(quote!(f32)),
        DataType::F64 => Ok(quote!(f64)),
        DataType::Bool => Ok(quote!(bool)),
        DataType::Usize => Ok(quote!(usize)),
        DataType::CString => Ok(quote!(String)),
        DataType::Path => Ok(quote!(PathBuf)),
        DataType::Vec2 => Ok(quote!([f32; 2])),
        DataType::Vec3 => Ok(quote!([f32; 3])),
        DataType::Vec4 => Ok(quote!([f32; 4])),
        DataType::Mat3x4 => Ok(quote!([[f32; 4]; 3])),
        DataType::Array { elem, len } => {
            let elem_ty = rust_type_for_data(resolved, module_id, elem, context)?;
            let len_lit = syn::LitInt::new(&len.to_string(), Span::call_site());
            Ok(quote!([#elem_ty; #len_lit]))
        }
        DataType::List(elem) => {
            let elem_ty = rust_type_for_data(resolved, module_id, elem, context)?;
            Ok(quote!(Vec<#elem_ty>))
        }
        DataType::Tuple(items) => {
            let mut elems = Vec::with_capacity(items.len());
            for item in items {
                elems.push(rust_type_for_data(resolved, module_id, item, context)?);
            }
            match elems.as_slice() {
                [] => Ok(quote!(())),
                [single] => Ok(quote!((#single,))),
                _ => Ok(quote!((#(#elems),*))),
            }
        }
        DataType::RawBytes => Ok(quote!(Vec<u8>)),
        DataType::Named(type_ref) => resolve_rust_type_path(resolved, module_id, type_ref, context),
    }
}

fn resolve_rust_type_path(
    resolved: &ResolvedSchema,
    from_mod: SchemaId,
    type_ref: &str,
    context: &str,
) -> Result<TokenStream, CodegenError> {
    let Some((target_mod, target_name, _)) = resolved.resolve_type(from_mod, type_ref) else {
        let mod_label = resolved
            .module(from_mod)
            .module_name
            .clone()
            .unwrap_or_else(|| "root".to_owned());
        return Err(CodegenError::UnresolvedType {
            module: mod_label,
            type_ref: type_ref.to_owned(),
        });
    };

    let target_ident = make_ident(target_name, context)?;
    if target_mod == from_mod {
        Ok(quote!(#target_ident))
    } else if from_mod == SchemaId::ROOT {
        let target_mod_name = resolved
            .module(target_mod)
            .module_name
            .as_deref()
            .unwrap_or("imported_schema");
        let target_mod_ident = make_ident(target_mod_name, "module name")?;
        Ok(quote!(#target_mod_ident::#target_ident))
    } else if target_mod == SchemaId::ROOT {
        Ok(quote!(super::#target_ident))
    } else {
        let target_mod_name = resolved
            .module(target_mod)
            .module_name
            .as_deref()
            .unwrap_or("imported_schema");
        let target_mod_ident = make_ident(target_mod_name, "module name")?;
        Ok(quote!(super::#target_mod_ident::#target_ident))
    }
}

fn make_ident(raw: &str, context: &str) -> Result<syn::Ident, CodegenError> {
    if let Ok(ident) = syn::parse_str::<syn::Ident>(raw) {
        return Ok(ident);
    }
    if let Ok(raw_ident) = syn::parse_str::<syn::Ident>(&format!("r#{raw}")) {
        return Ok(raw_ident);
    }
    Err(CodegenError::InvalidIdentifier {
        name: raw.to_owned(),
        context: context.to_owned(),
    })
}
