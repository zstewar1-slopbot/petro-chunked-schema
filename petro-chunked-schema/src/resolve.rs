//! Multi-module schema loading, import resolution, and type lookup.
//!
//! A [`Schema`] may import other schemas via `imports: { alias: <SchemaRef> }` and refer to their
//! definitions with `alias::TypeName`. [`SchemaLoader`] resolves that import graph (supporting both
//! `builtin:` and `file:` references, cycle detection, and diamond import deduplication) into a
//! [`ResolvedSchema`].

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Component, Path, PathBuf};

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::error::SchemaLoadError;
use crate::schema::{ChunkFieldSource, DataType, Schema, TypeDef};
use crate::source::SchemaRef;

/// Identifier of a loaded schema module inside a [`ResolvedSchema`].
///
/// [`SchemaId::ROOT`] (`SchemaId(0)`) is always the entry-point schema; imported schemas receive
/// increasing indices `1..`.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SchemaId(
    /// Zero-based index of the module inside [`ResolvedSchema::modules`].
    pub usize,
);

impl SchemaId {
    /// Module ID of the root (entry-point) schema in any [`ResolvedSchema`].
    pub const ROOT: Self = Self(0);
}

/// One resolved schema module inside a [`ResolvedSchema`].
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedModule {
    /// Unique identifier of this module inside the parent [`ResolvedSchema`].
    pub id: SchemaId,
    /// Source [`SchemaRef`] this module was loaded from, or `None` for an unsaved in-memory root.
    pub source: Option<SchemaRef>,
    /// Unique snake_case Rust module identifier assigned for imported modules (`None` for root).
    pub module_name: Option<String>,
    /// Parsed YAML schema AST for this module.
    pub schema: Schema,
    /// Resolved mapping from local import aliases in `schema.imports` to target [`SchemaId`]s.
    pub import_targets: IndexMap<String, SchemaId>,
}

/// Complete, cycle-free schema graph ready for dynamic evaluation, inspection, and Rust codegen.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedSchema {
    modules: Vec<ResolvedModule>,
    root_target: (SchemaId, String),
}

impl ResolvedSchema {
    /// Returns the entry-point [`Schema`] (`SchemaId::ROOT`).
    pub fn root_schema(&self) -> &Schema {
        &self.modules[SchemaId::ROOT.0].schema
    }

    /// Returns the [`ResolvedModule`] for `id`.
    ///
    /// # Panics
    /// Panics if `id` does not belong to this [`ResolvedSchema`].
    pub fn module(&self, id: SchemaId) -> &ResolvedModule {
        &self.modules[id.0]
    }

    /// Returns all resolved modules in load order, starting with [`SchemaId::ROOT`].
    pub fn modules(&self) -> &[ResolvedModule] {
        &self.modules
    }

    /// Returns `(module_id, local_name, &TypeDef)` for the resolved root type of this schema.
    pub fn root_type(&self) -> (SchemaId, &str, &TypeDef) {
        let (mod_id, ref name) = self.root_target;
        let def = self.modules[mod_id.0]
            .schema
            .types
            .get(name)
            .expect("resolved root target must exist");
        (mod_id, name.as_str(), def)
    }

    /// Resolves a local type name (`"Skeleton"`) or qualified import reference
    /// (`"model::Skeleton"`) starting from module `from`.
    pub fn resolve_type<'a>(
        &'a self,
        from: SchemaId,
        type_ref: &'a str,
    ) -> Option<(SchemaId, &'a str, &'a TypeDef)> {
        if let Some((alias, name)) = type_ref.split_once("::") {
            let target_id = *self.modules.get(from.0)?.import_targets.get(alias)?;
            let def = self.modules.get(target_id.0)?.schema.types.get(name)?;
            Some((target_id, name, def))
        } else {
            let def = self.modules.get(from.0)?.schema.types.get(type_ref)?;
            Some((from, type_ref, def))
        }
    }

    /// If `dt` is a named type (or `Vec<Named>`), resolves the referenced [`TypeDef`].
    pub fn resolve_data_type_target<'a>(
        &'a self,
        from: SchemaId,
        dt: &'a DataType,
    ) -> Option<(SchemaId, &'a str, &'a TypeDef)> {
        match dt {
            DataType::Named(name) => self.resolve_type(from, name),
            DataType::List(inner) => match &**inner {
                DataType::Named(name) => self.resolve_type(from, name),
                _ => None,
            },
            _ => None,
        }
    }

    /// Formats a type name owned by `owner` for UI and diagnostics (prefixing the module name when
    /// `owner != SchemaId::ROOT`).
    pub fn format_type_display(&self, owner: SchemaId, local_name: &str) -> String {
        if owner == SchemaId::ROOT {
            local_name.to_owned()
        } else if let Some(mod_name) = &self.modules[owner.0].module_name {
            format!("{mod_name}::{local_name}")
        } else {
            local_name.to_owned()
        }
    }

    /// Collects all `(SchemaId, type_name)` pairs referenced anywhere across the schema graph via
    /// `ChunkFieldSource::Stream`.
    pub fn referenced_stream_types(&self) -> BTreeSet<(SchemaId, String)> {
        let mut out = BTreeSet::new();
        for module in &self.modules {
            for typedef in module.schema.types.values() {
                match typedef {
                    TypeDef::Chunks(cc) => {
                        collect_stream_fields(self, module.id, &cc.fields, &mut out);
                    }
                    TypeDef::ChunkStreamSeq(seq) => {
                        collect_stream_fields(self, module.id, &seq.fields, &mut out);
                    }
                    TypeDef::ChunkEnum(ce) => {
                        for variant in &ce.variants {
                            if let ChunkFieldSource::Stream { item_type } = &variant.source
                                && let Some((target_id, target_name, _)) =
                                    self.resolve_type(module.id, item_type)
                            {
                                out.insert((target_id, target_name.to_owned()));
                            }
                        }
                    }
                    TypeDef::ByteBlock(_) | TypeDef::MiniChunks(_) => {}
                }
            }
        }
        out
    }
}

fn collect_stream_fields(
    resolved: &ResolvedSchema,
    from: SchemaId,
    fields: &[crate::schema::ChunkFieldDef],
    out: &mut BTreeSet<(SchemaId, String)>,
) {
    for field in fields {
        if let ChunkFieldSource::Stream { item_type } = &field.source
            && let Some((target_id, target_name, _)) = resolved.resolve_type(from, item_type)
        {
            out.insert((target_id, target_name.to_owned()));
        }
    }
}

/// Loader and import resolver for [`Schema`] definitions.
#[derive(Debug, Default, Clone, Copy)]
pub struct SchemaLoader;

impl SchemaLoader {
    /// Loads a [`Schema`] from `schema_ref` and recursively resolves all `imports` and `root`.
    pub fn load(schema_ref: &SchemaRef) -> Result<ResolvedSchema, SchemaLoadError> {
        let root_schema = Self::load_single(schema_ref)?;
        Self::resolve(root_schema, Some(schema_ref))
    }

    /// Loads a single [`Schema`] YAML document from `schema_ref` without resolving its `imports`.
    pub fn load_single(schema_ref: &SchemaRef) -> Result<Schema, SchemaLoadError> {
        match schema_ref {
            SchemaRef::Builtin(name) => load_builtin_by_name(name),
            SchemaRef::File(path) => {
                let yaml = std::fs::read_to_string(path).map_err(|source| SchemaLoadError::Io {
                    path: path.clone(),
                    source,
                })?;
                Schema::from_yaml_str(&yaml).map_err(|source| SchemaLoadError::Yaml {
                    source_label: path.display().to_string(),
                    source,
                })
            }
        }
    }

    /// Resolves `root_schema`'s `imports` (relative to `base_ref` if provided) and validates that
    /// `root_schema.root` resolves to a chunk root type (`Chunks`, `ChunkEnum`, or
    /// `ChunkStreamSeq`).
    pub fn resolve(
        root_schema: Schema,
        base_ref: Option<&SchemaRef>,
    ) -> Result<ResolvedSchema, SchemaLoadError> {
        let mut ctx = ResolveCtx::default();
        if let Some(base) = base_ref {
            let key = canonicalize_ref(base);
            ctx.active_stack.push((key.clone(), base.to_string()));
            ctx.loaded_by_key.insert(key, SchemaId::ROOT);
        }

        Self::resolve_module(&mut ctx, root_schema, base_ref.cloned(), None)?;

        if base_ref.is_some() {
            ctx.active_stack.pop();
        }

        let root_name = ctx.modules[SchemaId::ROOT.0].schema.root.clone();
        let root_schema_name = ctx.modules[SchemaId::ROOT.0].schema.name.clone();

        let (root_target_id, root_target_name, root_kind_ok) = {
            let probe = ResolvedSchema {
                modules: ctx.modules,
                root_target: (SchemaId::ROOT, String::new()),
            };
            let Some((tid, tname, tdef)) = probe.resolve_type(SchemaId::ROOT, &root_name) else {
                return Err(SchemaLoadError::MissingRootType {
                    schema_name: root_schema_name,
                    root: root_name,
                });
            };
            let kind_ok = matches!(
                tdef,
                TypeDef::Chunks(_) | TypeDef::ChunkEnum(_) | TypeDef::ChunkStreamSeq(_)
            );
            let tname = tname.to_owned();
            ctx.modules = probe.modules;
            (tid, tname, kind_ok)
        };

        if !root_kind_ok {
            return Err(SchemaLoadError::UnsupportedRootKind {
                schema_name: root_schema_name,
                root: root_name,
            });
        }

        Ok(ResolvedSchema {
            modules: ctx.modules,
            root_target: (root_target_id, root_target_name),
        })
    }

    fn resolve_module(
        ctx: &mut ResolveCtx,
        schema: Schema,
        source: Option<SchemaRef>,
        module_name: Option<String>,
    ) -> Result<SchemaId, SchemaLoadError> {
        let id = SchemaId(ctx.modules.len());
        let imports = schema.imports.clone();
        ctx.modules.push(ResolvedModule {
            id,
            source: source.clone(),
            module_name,
            schema,
            import_targets: IndexMap::new(),
        });

        let mut import_targets = IndexMap::new();
        for (alias, raw_import_ref) in imports {
            let resolved_ref = raw_import_ref.resolve_relative(source.as_ref());
            let key = canonicalize_ref(&resolved_ref);

            if let Some(pos) = ctx.active_stack.iter().position(|(k, _)| k == &key) {
                let mut cycle: Vec<String> = ctx.active_stack[pos..]
                    .iter()
                    .map(|(_, label)| label.clone())
                    .collect();
                cycle.push(resolved_ref.to_string());
                return Err(SchemaLoadError::CyclicImport { cycle });
            }

            let target_id = if let Some(&existing) = ctx.loaded_by_key.get(&key) {
                existing
            } else {
                let child_schema = Self::load_single(&resolved_ref)?;
                let child_mod_name = ctx.allocate_module_name(&alias);
                ctx.active_stack
                    .push((key.clone(), resolved_ref.to_string()));
                let res = Self::resolve_module(
                    ctx,
                    child_schema,
                    Some(resolved_ref),
                    Some(child_mod_name),
                );
                ctx.active_stack.pop();
                let child_id = res?;
                ctx.loaded_by_key.insert(key, child_id);
                child_id
            };
            import_targets.insert(alias, target_id);
        }

        ctx.modules[id.0].import_targets = import_targets;
        Ok(id)
    }
}

#[derive(Default)]
struct ResolveCtx {
    modules: Vec<ResolvedModule>,
    loaded_by_key: HashMap<CanonicalKey, SchemaId>,
    active_stack: Vec<(CanonicalKey, String)>,
    used_mod_names: HashSet<String>,
}

impl ResolveCtx {
    fn allocate_module_name(&mut self, alias: &str) -> String {
        let base = sanitize_snake_ident(alias);
        if self.used_mod_names.insert(base.clone()) {
            return base;
        }
        let mut suffix = 2usize;
        loop {
            let candidate = format!("{base}_{suffix}");
            if self.used_mod_names.insert(candidate.clone()) {
                return candidate;
            }
            suffix += 1;
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum CanonicalKey {
    Builtin(String),
    File(PathBuf),
}

fn canonicalize_ref(r: &SchemaRef) -> CanonicalKey {
    match r {
        SchemaRef::Builtin(name) => {
            #[cfg(feature = "known-formats")]
            let canonical = crate::known_formats::canonical_name(name)
                .map(str::to_owned)
                .unwrap_or_else(|| name.to_ascii_lowercase());
            #[cfg(not(feature = "known-formats"))]
            let canonical = name.to_ascii_lowercase();
            CanonicalKey::Builtin(canonical)
        }
        SchemaRef::File(path) => {
            if let Ok(canon) = std::fs::canonicalize(path) {
                CanonicalKey::File(canon)
            } else {
                let abs = if path.is_absolute() {
                    path.clone()
                } else if let Ok(cwd) = std::env::current_dir() {
                    cwd.join(path)
                } else {
                    path.clone()
                };
                CanonicalKey::File(lexical_normalize(&abs))
            }
        }
    }
}

fn lexical_normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push(comp);
                }
            }
            other => out.push(other),
        }
    }
    out
}

fn sanitize_snake_ident(raw: &str) -> String {
    let mut out = String::new();
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.ends_with('_') {
            out.push('_');
        }
    }
    let trimmed = out.trim_matches('_').to_owned();
    if trimmed.is_empty() {
        "imported_schema".to_owned()
    } else if trimmed.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        format!("mod_{trimmed}")
    } else {
        trimmed
    }
}

#[cfg(feature = "known-formats")]
fn load_builtin_by_name(name: &str) -> Result<Schema, SchemaLoadError> {
    crate::known_formats::by_name(name).ok_or_else(|| SchemaLoadError::UnknownBuiltin(name.to_owned()))
}

#[cfg(not(feature = "known-formats"))]
fn load_builtin_by_name(name: &str) -> Result<Schema, SchemaLoadError> {
    Err(SchemaLoadError::BuiltinFormatsDisabled(name.to_owned()))
}
