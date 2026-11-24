# `petro-chunked-schema`

Declarative YAML schema descriptors, multi-module import resolution, dynamic binary inspection, and optional `petro-chunked-derive` Rust code generation for Petroglyph's chunked binary file formats.

> **Usage & API Documentation:** Detailed format reference, crate features, YAML syntax, and runnable examples live in [`src/lib.rs`](./src/lib.rs) (viewable locally with `cargo doc -p petro-chunked-schema --all-features --open`). This README provides a top-down architectural overview of how the crate is structured.

## Feature Flags

| Feature | Default | Purpose |
|---|---|---|
| `known-formats` | **Yes** | Embeds built-in YAML schemas for Petroglyph formats (`Alo`, `AloModel`, `AloParticle`, `AlaAnimation`) under `known_formats`. |
| `codegen` | No | Enables AST-based Rust module emission (`codegen::generate_rust_code`) using `quote`, `syn`, and `prettyplease`. |

---

## Top-Down Architecture

### 1. The Big Picture

Petroglyph engine assets (such as `.alo` models/particles and `.ala` animations) store data in nested trees of length-prefixed binary chunks (`u32` ID + `u32` size + payload), sometimes containing compact nested mini-chunks (`u8` ID + `u8` size + payload) or packed structs. While `petro-chunked` handles compile-time Rust parsing, `petro-chunked-schema` interprets and documents those layouts **dynamically from YAML** so validators, exploration tools, and GUI editors can work against incomplete or evolving format knowledge without recompiling.

### 2. Processing Pipeline

```text
  SchemaRef ("builtin:Alo", "./custom.yaml", "file:builtin:foo")
      │
      ▼
  SchemaLoader::load / resolve   ──► follows `imports`, deduplicates diamonds,
      │                              rejects cycles, validates type targets
      ▼
  ResolvedSchema                 ──► immutable module graph indexed by SchemaId
      │
      ├──► dynamic::load_dynamic_file / validate_bytes
      │       ├── Stage 1 (parser): scans raw chunk/mini-chunk framing into a tree
      │       └── Stage 2 (eval):   matches schema types, decodes values,
      │                             collects byte-span annotations & diagnostics
      │
      └──► codegen::generate_rust_code  (optional `codegen` feature)
              └── emits formatted `petro-chunked-derive` Rust modules via quote + prettyplease
```

### 3. Core Architectural Concepts

Each layer introduces a small set of abstractions:

1. **Uniform Schema AST (`schema`)**
   Every binary construct in a schema lives as a named entry in an ordered `types` map (`IndexMap<String, TypeDef>`), conventionally ordered **parents before children**:
   - `chunks`: 32-bit chunk containers (`map` or `sequence` order).
   - `mini_chunks`: 8-bit mini-chunk containers (`map` or `sequence` order).
   - `byte_block`: Sequential field structs with optional trailing `pad_after` bytes.
   - `chunk_stream_seq`: Logical multi-chunk sequence item pulling consecutive sibling chunks from a parent stream without creating an enclosing parent chunk.
   - `chunk_enum`: Tag-dispatched union choosing a variant from the next chunk's ID or stream lookahead.

   The schema's entry point (`root`) is simply the name of one of those entries in `types` (either a local `TypeDef` or an imported `alias::TypeName`), rather than a special-purpose root definition.

2. **URI References & Multi-Module Resolution (`source`, `resolve`)**
   Format variants that share a file extension (such as `.alo` 3D models vs `.alo` particle systems) belong in separate schemas so their chunk IDs do not collide or silence mismatches. Schemas can declare `imports` keyed by a local module alias pointing to a `SchemaRef` (`builtin:<Name>` or `file:<Path>`, where unprefixed strings default to filesystem paths and `file:` escapes literal paths beginning with `builtin:`/`file:`).

   `SchemaLoader` walks the import graph recursively, detects import cycles, deduplicates shared (diamond) imports by canonical identity, and locks every module behind an opaque `SchemaId` inside a `ResolvedSchema`. All downstream evaluators and code generators look up imported types through `(SchemaId, &str)` pairs inside that resolved graph.

3. **Two-Stage Dynamic Interpreter & Span Model (`dynamic`)**
   Interactive inspection requires viewing unknown or partially mapped files even when strict validation would abort:
   - **Stage 1 (`parser`)** frames raw bytes into `DynamicChunk` and `DynamicMiniChunk` nodes (recording byte offsets, container flags, and mini-chunk heuristics, while respecting per-path plain-data overrides).
   - **Stage 2 (`eval`)** walks the framed tree alongside the `ResolvedSchema` to assign field names, resolve imported `ChunkEnum` / `ChunkStreamSeq` items across sibling chunks, decode payloads (`DynamicValue`), build `ByteSpanAnnotation` ranges for hex highlighters, and emit non-fatal diagnostics.

   Callers tune trade-offs through `LoadOptions`: `LoadOptions::default()` decodes complete lists cleanly, `LoadOptions::for_validation()` disables span recording (`max_spans = 0`) and enables `fast_skip_large_lists` while checking exact byte lengths, and `LoadOptions::for_ui(...)` caps spans at 512, summarizes large fixed-size lists, and applies per-file `force_plain_data_paths` overrides. An adjacent `inspector` helper decodes arbitrary byte cursors into candidate primitive/vector previews.

4. **AST-Based Rust Codegen (`codegen`, feature `codegen`)**
   Instead of string concatenation, `generate_rust_code` walks each reachable `ResolvedModule`, generates `syn` / `quote` token streams annotated with `petro-chunked-derive` attributes (`ChunkStructure`, `MiniChunkStructure`, `FromByteBlock`, `ChunkStreamItem`), emits nested `pub mod <alias>` submodules for imports, parses the result with `syn::parse2`, and formats it cleanly via `prettyplease`.

### 4. Module Map

| Path | Responsibility |
|---|---|
| [`src/lib.rs`](./src/lib.rs) | Crate documentation, top-level re-exports, feature gating. |
| [`src/schema.rs`](./src/schema.rs) | Declarative YAML AST (`Schema`, `TypeDef`, `DataType`, `Cardinality`, `WireFormat`). |
| [`src/source.rs`](./src/source.rs) | `SchemaRef` URI parsing (`builtin:` / `file:`), lossless display, relative path resolution. |
| [`src/resolve.rs`](./src/resolve.rs) | `SchemaLoader`, cycle/diamond handling, `SchemaId`, `ResolvedModule`, `ResolvedSchema`. |
| [`src/error.rs`](./src/error.rs) | Structured `thiserror` errors for parsing, loading, evaluation, and codegen. |
| [`src/dynamic/mod.rs`](./src/dynamic/mod.rs) | Public `dynamic` entry points (`load_dynamic_file`, `validate_bytes`) and `LoadOptions`. |
| [`src/dynamic/model.rs`](./src/dynamic/model.rs) | Dynamic tree & annotation data types (`DynamicFile`, `DynamicChunk`, `DataValue`, `ByteSpanAnnotation`). |
| [`src/dynamic/parser.rs`](./src/dynamic/parser.rs) | Raw wire framing and mini-chunk detection heuristic (`looks_like_mini_chunks`). |
| [`src/dynamic/eval.rs`](./src/dynamic/eval.rs) | Schema-guided matching, payload decoding, stream lookahead, and diagnostic reporting. |
| [`src/dynamic/inspector.rs`](./src/dynamic/inspector.rs) | Offset cursor multi-format byte decoder (`inspect_bytes_at_offset`). |
| [`src/known_formats.rs`](./src/known_formats.rs) | Embedded built-in YAML formats (`Alo`, `AloModel`, `AloParticle`, `AlaAnimation`) gated on `known-formats`. |
| [`src/codegen.rs`](./src/codegen.rs) | `quote` + `prettyplease` Rust emitter gated on `codegen`. |
