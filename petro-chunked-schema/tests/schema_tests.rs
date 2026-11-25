use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use petro_chunked_schema::dynamic::{
    DynamicChunkContent, DynamicValue, FindStringsOptions, LoadOptions, StringMatchKind,
    find_strings, inspect_bytes_at_offset, load_dynamic_file, validate_bytes,
};
use petro_chunked_schema::error::SchemaLoadError;
use petro_chunked_schema::resolve::SchemaLoader;
use petro_chunked_schema::schema::{DataType, Schema};
use petro_chunked_schema::source::SchemaRef;

#[cfg(feature = "known-formats")]
use petro_chunked_schema::known_formats;
#[cfg(feature = "known-formats")]
use petro_chunked_schema::schema::{ChunkFieldSource, ChunkPayloadSpec, TypeDef};

#[cfg(feature = "codegen")]
use petro_chunked_schema::codegen::generate_rust_code;

fn make_chunk(id: u32, is_container: bool, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + payload.len());
    out.extend_from_slice(&id.to_le_bytes());
    let mut size = payload.len() as u32;
    if is_container {
        size |= 0x8000_0000;
    }
    out.extend_from_slice(&size.to_le_bytes());
    out.extend_from_slice(payload);
    out
}

fn make_mini_chunk(id: u8, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(2 + payload.len());
    out.push(id);
    out.push(payload.len() as u8);
    out.extend_from_slice(payload);
    out
}

#[cfg(feature = "known-formats")]
fn make_synthetic_model_bytes() -> Vec<u8> {
    let mut skel_header = Vec::from(1u32.to_le_bytes());
    skel_header.extend_from_slice(&[0u8; 124]);

    let mut bone_data = Vec::new();
    bone_data.extend_from_slice(&0xffff_ffffu32.to_le_bytes()); // parent
    bone_data.extend_from_slice(&1u32.to_le_bytes()); // visible = true
    bone_data.extend_from_slice(&[0u8; 48]); // 3x4 matrix

    let mut bone_children = Vec::new();
    bone_children.extend(make_chunk(0x0203, false, b"Root\0"));
    bone_children.extend(make_chunk(0x0205, false, &bone_data));

    let mut skel_children = Vec::new();
    skel_children.extend(make_chunk(0x0201, false, &skel_header));
    skel_children.extend(make_chunk(0x0202, true, &bone_children));

    let mut mesh_info = Vec::new();
    mesh_info.extend_from_slice(&0u32.to_le_bytes()); // submesh_count
    mesh_info.extend_from_slice(&[0u8; 24]); // bounds_min + bounds_max
    mesh_info.extend_from_slice(&0u32.to_le_bytes()); // unknown
    mesh_info.extend_from_slice(&0u32.to_le_bytes()); // hidden
    mesh_info.extend_from_slice(&0u32.to_le_bytes()); // collision_enabled
    mesh_info.extend_from_slice(&[0u8; 88]); // pad_after

    let mut mesh_children = Vec::new();
    mesh_children.extend(make_chunk(0x0401, false, b"Hull\0"));
    mesh_children.extend(make_chunk(0x0402, false, &mesh_info));

    let mut conn_count = Vec::new();
    conn_count.extend(make_mini_chunk(1, &1u32.to_le_bytes()));
    conn_count.extend(make_mini_chunk(4, &0u32.to_le_bytes()));

    let mut obj_conn = Vec::new();
    obj_conn.extend(make_mini_chunk(2, &0u32.to_le_bytes()));
    obj_conn.extend(make_mini_chunk(3, &0u32.to_le_bytes()));

    let mut conn_children = Vec::new();
    conn_children.extend(make_chunk(0x0601, false, &conn_count));
    conn_children.extend(make_chunk(0x0602, false, &obj_conn));

    let mut out = Vec::new();
    out.extend(make_chunk(0x0200, true, &skel_children));
    out.extend(make_chunk(0x0400, true, &mesh_children));
    out.extend(make_chunk(0x0600, true, &conn_children));
    out
}

#[cfg(feature = "known-formats")]
fn make_synthetic_particle_bytes() -> Vec<u8> {
    let mut sys_children = Vec::new();
    sys_children.extend(make_chunk(0x0000, false, b"TestParticle\0"));
    sys_children.extend(make_chunk(0x0001, false, &42u32.to_le_bytes()));
    sys_children.extend(make_chunk(0x0800, true, &[])); // empty emitters list
    make_chunk(0x0900, true, &sys_children)
}

#[cfg(feature = "known-formats")]
fn make_synthetic_ala_bytes() -> Vec<u8> {
    let mut header_mc = Vec::new();
    header_mc.extend(make_mini_chunk(1, &1u32.to_le_bytes())); // num_frames = 1
    header_mc.extend(make_mini_chunk(2, &30.0f32.to_le_bytes())); // fps = 30.0
    header_mc.extend(make_mini_chunk(3, &1u32.to_le_bytes())); // num_bones = 1

    let mut bone_hdr_mc = Vec::new();
    bone_hdr_mc.extend(make_mini_chunk(4, b"Root\0"));
    bone_hdr_mc.extend(make_mini_chunk(5, &0u32.to_le_bytes()));
    bone_hdr_mc.extend(make_mini_chunk(6, &[0u8; 12]));
    bone_hdr_mc.extend(make_mini_chunk(7, &[0u8; 12]));
    bone_hdr_mc.extend(make_mini_chunk(8, &[0u8; 12]));
    bone_hdr_mc.extend(make_mini_chunk(9, &[0u8; 12]));

    let mut bone_children = Vec::new();
    bone_children.extend(make_chunk(0x1003, false, &bone_hdr_mc));
    bone_children.extend(make_chunk(0x1006, false, &[0u8; 8])); // 1 quaternion [i16; 4]

    let mut root_children = Vec::new();
    root_children.extend(make_chunk(0x1001, false, &header_mc));
    root_children.extend(make_chunk(0x1002, true, &bone_children));

    make_chunk(0x1000, true, &root_children)
}

struct TempDirGuard(PathBuf);

impl TempDirGuard {
    fn new(prefix: &str) -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "petro_chunked_schema_test_{prefix}_{}_{id}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp test dir");
        Self(dir)
    }

    fn path(&self) -> &PathBuf {
        &self.0
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(feature = "known-formats")]
fn collect_local_child_refs(typedef: &TypeDef) -> Vec<String> {
    let mut out = Vec::new();
    let collect_source = |src: &ChunkFieldSource, out: &mut Vec<String>| match src {
        ChunkFieldSource::Id { payload, .. } => match payload {
            ChunkPayloadSpec::Children(t) | ChunkPayloadSpec::MiniChunks(t) => {
                if !t.contains("::") {
                    out.push(t.clone());
                }
            }
            ChunkPayloadSpec::Data(dt) => collect_dt_refs(dt, out),
            ChunkPayloadSpec::Ignore => {}
        },
        ChunkFieldSource::Stream { item_type } => {
            if !item_type.contains("::") {
                out.push(item_type.clone());
            }
        }
    };

    match typedef {
        TypeDef::ByteBlock(bb) => {
            for f in &bb.fields {
                collect_dt_refs(&f.ty, &mut out);
            }
        }
        TypeDef::MiniChunks(mc) => {
            for f in &mc.fields {
                collect_dt_refs(&f.ty, &mut out);
            }
        }
        TypeDef::Chunks(cc) => {
            for f in &cc.fields {
                collect_source(&f.source, &mut out);
            }
        }
        TypeDef::ChunkStreamSeq(seq) => {
            for f in &seq.fields {
                collect_source(&f.source, &mut out);
            }
        }
        TypeDef::ChunkEnum(ce) => {
            for v in &ce.variants {
                collect_source(&v.source, &mut out);
            }
        }
    }
    out
}

#[cfg(feature = "known-formats")]
fn collect_dt_refs(dt: &DataType, out: &mut Vec<String>) {
    match dt {
        DataType::Named(name) if !name.contains("::") => out.push(name.clone()),
        DataType::Array { elem, .. } | DataType::List(elem) => collect_dt_refs(elem, out),
        DataType::Tuple(items) => {
            for item in items {
                collect_dt_refs(item, out);
            }
        }
        _ => {}
    }
}

#[cfg(feature = "known-formats")]
#[test]
fn test_known_formats_roundtrip_and_parents_before_children_order() {
    for schema in known_formats::all() {
        let yaml = schema.to_yaml_string().unwrap();
        let reparsed = Schema::from_yaml_str(&yaml).unwrap();
        assert_eq!(schema, reparsed, "YAML round-trip mismatch on {}", schema.name);

        let keys: Vec<&String> = schema.types.keys().collect();
        assert_eq!(
            keys.first().map(|s| s.as_str()),
            Some(schema.root.as_str()),
            "root type `{}` should appear first in `{}`",
            schema.root,
            schema.name
        );

        let mut seen = BTreeSet::new();
        for (parent_name, typedef) in &schema.types {
            seen.insert(parent_name.clone());
            let parent_idx = schema.types.get_index_of(parent_name).unwrap();
            for child_name in collect_local_child_refs(typedef) {
                let child_idx = schema
                    .types
                    .get_index_of(&child_name)
                    .unwrap_or_else(|| panic!("missing child `{child_name}` in `{}`", schema.name));
                assert!(
                    parent_idx < child_idx,
                    "In `{}`: parent `{parent_name}` (#{parent_idx}) must precede child `{child_name}` (#{child_idx})",
                    schema.name
                );
            }
        }

        let resolved = SchemaLoader::resolve(schema.clone(), None).unwrap();
        assert_eq!(resolved.root_schema(), &schema);
    }
}

#[test]
fn test_schema_ref_parsing_and_display_roundtrip() {
    let r_builtin = SchemaRef::parse("builtin:AloParticle").unwrap();
    assert_eq!(r_builtin, SchemaRef::Builtin("AloParticle".to_owned()));
    assert_eq!(r_builtin.to_string(), "builtin:AloParticle");

    let r_escaped_builtin = SchemaRef::parse("file:builtin:foo").unwrap();
    assert_eq!(r_escaped_builtin, SchemaRef::File(PathBuf::from("builtin:foo")));
    assert_eq!(r_escaped_builtin.to_string(), "file:builtin:foo");
    assert_eq!(
        SchemaRef::parse(&r_escaped_builtin.to_string()).unwrap(),
        r_escaped_builtin
    );

    let r_escaped_file = SchemaRef::parse("file:file:bar").unwrap();
    assert_eq!(r_escaped_file, SchemaRef::File(PathBuf::from("file:bar")));
    assert_eq!(r_escaped_file.to_string(), "file:file:bar");

    let r_plain = SchemaRef::parse("schemas/custom.yaml").unwrap();
    assert_eq!(r_plain, SchemaRef::File(PathBuf::from("schemas/custom.yaml")));
    assert_eq!(r_plain.to_string(), "schemas/custom.yaml");

    assert!(SchemaRef::parse("").is_err());
    assert!(SchemaRef::parse("builtin:").is_err());
    assert!(SchemaRef::parse("file:").is_err());
}

#[test]
fn test_data_type_parse_and_label_roundtrip() {
    let cases = [
        "u8",
        "i16",
        "f32",
        "bool",
        "usize",
        "c_string",
        "path",
        "vec2",
        "vec3",
        "vec4",
        "mat3x4",
        "raw_bytes",
        "Vec<u8>",
        "[[f32; 4]; 3]",
        "[vec2; 4]",
        "()",
        "(u32,)",
        "(u32, f32)",
        "Vec<model::VertexV1>",
    ];
    for input in cases {
        let dt = DataType::parse_str(input).unwrap_or_else(|e| panic!("failed `{input}`: {e}"));
        assert_eq!(dt.label(), input);
    }
    assert_eq!(DataType::parse_str("Vec<u8>").unwrap(), DataType::List(Box::new(DataType::U8)));
    assert_eq!(DataType::parse_str("raw_bytes").unwrap(), DataType::RawBytes);
    assert_ne!(DataType::parse_str("[[f32; 4]; 3]").unwrap(), DataType::Mat3x4);

    assert!(DataType::parse_str("").is_err());
    assert!(DataType::parse_str("Vec<>").is_err());
    assert!(DataType::parse_str("(u32)").is_err());
    assert!(DataType::parse_str("Vec<u32").is_err());
}

#[test]
fn test_multi_file_imports_diamond_dedup_and_cycle_detection() {
    let tmp = TempDirGuard::new("imports");
    let sub = tmp.path().join("sub");
    std::fs::create_dir_all(&sub).unwrap();

    // Shared header imported by both `a.yaml` and `b.yaml` (diamond import)
    std::fs::write(
        sub.join("common.yaml"),
        r#"
name: Common
root: SharedContainer
types:
  SharedContainer:
    kind: chunks
    mode: sequence
    fields:
      - name: tag
        source_kind: id
        id: 0x00000010
        payload_kind: data
        payload_type: Header
  Header:
    kind: byte_block
    fields:
      - name: version
        ty: u32
"#,
    )
    .unwrap();

    // `a.yaml` and `b.yaml` both define a type named `Header` to test namespace isolation
    std::fs::write(
        tmp.path().join("a.yaml"),
        r#"
name: SchemaA
imports:
  common: sub/common.yaml
root: RootA
types:
  RootA:
    kind: chunks
    mode: sequence
    fields:
      - name: shared
        source_kind: id
        id: 0x00000001
        payload_kind: data
        payload_type: common::Header
      - name: local
        source_kind: id
        id: 0x00000002
        payload_kind: data
        payload_type: Header
  Header:
    kind: byte_block
    fields:
      - name: a_val
        ty: u16
"#,
    )
    .unwrap();

    std::fs::write(
        tmp.path().join("b.yaml"),
        r#"
name: SchemaB
imports:
  common: sub/common.yaml
root: RootB
types:
  RootB:
    kind: chunks
    mode: sequence
    fields:
      - name: shared
        source_kind: id
        id: 0x00000003
        payload_kind: data
        payload_type: common::Header
      - name: local
        source_kind: id
        id: 0x00000004
        payload_kind: data
        payload_type: Header
  Header:
    kind: byte_block
    fields:
      - name: b_val
        ty: f32
"#,
    )
    .unwrap();

    let root_path = tmp.path().join("root.yaml");
    std::fs::write(
        &root_path,
        r#"
name: CombinedRoot
imports:
  mod_a: a.yaml
  mod_b: b.yaml
root: RootUnion
types:
  RootUnion:
    kind: chunks
    mode: map
    fields:
      - name: a
        source_kind: id
        id: 0x00000100
        payload_kind: children
        payload_type: mod_a::RootA
      - name: b
        source_kind: id
        id: 0x00000200
        payload_kind: children
        payload_type: mod_b::RootB
"#,
    )
    .unwrap();

    let resolved = SchemaLoader::load(&SchemaRef::File(root_path)).unwrap();
    // Root + mod_a + common + mod_b = 4 modules (common deduplicated despite diamond import)
    assert_eq!(resolved.modules().len(), 4);

    let mut payload = Vec::new();
    let mut a_children = Vec::new();
    a_children.extend(make_chunk(0x01, false, &9u32.to_le_bytes()));
    a_children.extend(make_chunk(0x02, false, &7u16.to_le_bytes()));
    let mut b_children = Vec::new();
    b_children.extend(make_chunk(0x03, false, &11u32.to_le_bytes()));
    b_children.extend(make_chunk(0x04, false, &2.5f32.to_le_bytes()));
    payload.extend(make_chunk(0x100, true, &a_children));
    payload.extend(make_chunk(0x200, true, &b_children));

    let dyn_file = validate_bytes(&resolved, &payload).unwrap();
    assert_eq!(dyn_file.chunks.len(), 2);

    // Cycle detection test
    let cyc_a = tmp.path().join("cyc_a.yaml");
    let cyc_b = tmp.path().join("cyc_b.yaml");
    std::fs::write(
        &cyc_a,
        "name: CycA\nimports:\n  b: cyc_b.yaml\nroot: A\ntypes:\n  A:\n    kind: chunks\n",
    )
    .unwrap();
    std::fs::write(
        &cyc_b,
        "name: CycB\nimports:\n  a: cyc_a.yaml\nroot: B\ntypes:\n  B:\n    kind: chunks\n",
    )
    .unwrap();
    let err = SchemaLoader::load(&SchemaRef::File(cyc_a)).unwrap_err();
    assert!(matches!(err, SchemaLoadError::CyclicImport { .. }));
}

#[cfg(feature = "known-formats")]
#[test]
fn test_alo_non_overlap_and_combined_dispatch_and_ala() {
    let alo = SchemaLoader::load(&SchemaRef::Builtin("Alo".to_owned())).unwrap();
    let alo_model = SchemaLoader::load(&SchemaRef::Builtin("AloModel".to_owned())).unwrap();
    let alo_particle = SchemaLoader::load(&SchemaRef::Builtin("AloParticle".to_owned())).unwrap();
    let ala = SchemaLoader::load(&SchemaRef::Builtin("AlaAnimation".to_owned())).unwrap();

    let model_bytes = make_synthetic_model_bytes();
    let particle_bytes = make_synthetic_particle_bytes();
    let ala_bytes = make_synthetic_ala_bytes();

    // Model passes AloModel and combined Alo, fails AloParticle
    assert!(validate_bytes(&alo_model, &model_bytes).is_ok());
    assert!(validate_bytes(&alo, &model_bytes).is_ok());
    assert!(validate_bytes(&alo_particle, &model_bytes).is_err());

    // Particle passes AloParticle and combined Alo, fails AloModel
    assert!(validate_bytes(&alo_particle, &particle_bytes).is_ok());
    assert!(validate_bytes(&alo, &particle_bytes).is_ok());
    assert!(validate_bytes(&alo_model, &particle_bytes).is_err());

    // ALA animation validates and supports force_plain_data_paths
    let dyn_ala = validate_bytes(&ala, &ala_bytes).unwrap();
    assert_eq!(dyn_ala.chunks.len(), 1);
    assert_eq!(
        dyn_ala.chunks[0].schema_field_name.as_deref(),
        Some("animation")
    );

    let mut opts = LoadOptions::default();
    opts.force_plain_data_paths.insert("0/0".to_owned());
    let forced = load_dynamic_file(&ala_bytes, Some(&ala), &opts).unwrap();
    let DynamicChunkContent::Children(children) = &forced.chunks[0].content else {
        panic!("expected children");
    };
    assert!(matches!(
        children[0].content,
        DynamicChunkContent::Data { .. }
    ));

    // Multi-format byte inspector
    let previews = inspect_bytes_at_offset(b"Root\0\x01\x00\x00\x00", 0);
    assert!(previews.iter().any(|p| p.label == "CString (ASCIIZ)"));
}

#[test]
fn test_large_list_complete_decoding_vs_ui_summarization() {
    let yaml = r#"
name: ListTest
root: ListTest
types:
  ListTest:
    kind: chunks
    mode: sequence
    fields:
      - name: values
        source_kind: id
        id: 0x00000001
        payload_kind: data
        payload_type: Vec<u16>
"#;
    let schema = Schema::from_yaml_str(yaml).unwrap();
    let resolved = SchemaLoader::resolve(schema, None).unwrap();

    let mut payload = Vec::new();
    for i in 0u16..32 {
        payload.extend_from_slice(&i.to_le_bytes());
    }
    let file_bytes = make_chunk(0x00000001, false, &payload);

    // Default library LoadOptions decodes ALL 32 items completely
    let full = load_dynamic_file(&file_bytes, Some(&resolved), &LoadOptions::default()).unwrap();
    let DynamicChunkContent::Data { decoded: Some(DynamicValue::Array(items)), .. } = &full.chunks[0].content else {
        panic!("expected decoded array");
    };
    assert_eq!(items.len(), 32);

    // UI preset summarizes remaining fixed-size items into one trailing span after 16 items
    let ui = load_dynamic_file(
        &file_bytes,
        Some(&resolved),
        &LoadOptions::for_ui(BTreeSet::new()),
    )
    .unwrap();
    let DynamicChunkContent::Data { spans, .. } = &ui.chunks[0].content else {
        panic!("expected data chunk");
    };
    assert_eq!(spans.len(), 17);
    assert!(spans[16].field_path.contains("16..32"));
}

#[test]
fn test_byte_inspector_previews() {
    let previews = inspect_bytes_at_offset(b"Root\0\x01\x00\x00\x00", 0);
    assert!(previews.iter().any(|p| p.label == "CString (ASCIIZ)"));
    assert!(previews.iter().any(|p| p.label == "u32"));
}

#[test]
fn test_find_strings_whole_blocks_minichunks_and_embedded() {
    let mut mc_payload = Vec::new();
    mc_payload.extend(make_mini_chunk(0x01, b"DiffuseMap\0"));
    mc_payload.extend(make_mini_chunk(0x02, &42u32.to_le_bytes()));

    let mut mixed_payload = Vec::new();
    mixed_payload.extend_from_slice(&1u32.to_le_bytes());
    mixed_payload.extend_from_slice(b"ShaderFX\0");
    mixed_payload.extend_from_slice(&[0xFF, 0xFE]);
    mixed_payload.extend_from_slice(b"ALBEDO");
    mixed_payload.push(0xFF);

    let mut child_chunks = Vec::new();
    child_chunks.extend(make_chunk(0x0101, false, b"Hull\0"));
    child_chunks.extend(make_chunk(0x0102, false, b"Bone01\0\0\0\0"));
    child_chunks.extend(make_chunk(0x0103, false, b"TAG4"));
    // 65u32 = [0x41, 0x00, 0x00, 0x00] must NOT match as WholeAsciizPadded even with min_whole_len = 1.
    child_chunks.extend(make_chunk(0x0104, false, &65u32.to_le_bytes()));
    child_chunks.extend(make_chunk(0x0105, false, &mc_payload));
    child_chunks.extend(make_chunk(0x0106, false, &mixed_payload));

    let file_bytes = make_chunk(0x0100, true, &child_chunks);
    let df = load_dynamic_file(&file_bytes, None, &LoadOptions::default()).unwrap();

    // Default scan: whole-block ASCII & ASCIIZ across leaf chunks and mini-chunks.
    let mut whole_opts = FindStringsOptions {
        min_whole_len: 1,
        ..FindStringsOptions::default()
    };
    let hits = find_strings(&df, &whole_opts);
    assert_eq!(hits.len(), 4);

    assert_eq!(hits[0].tree_path, "0/0");
    assert_eq!(hits[0].kind, StringMatchKind::WholeAsciiz);
    assert_eq!(hits[0].text, "Hull");
    assert_eq!(hits[0].byte_len, 5);
    assert!(!hits[0].is_mini_chunk);
    assert_eq!(hits[0].node_id, 0x0101);
    assert_eq!(hits[0].payload_offset, 0);

    assert_eq!(hits[1].tree_path, "0/1");
    assert_eq!(hits[1].kind, StringMatchKind::WholeAsciizPadded);
    assert_eq!(hits[1].text, "Bone01");
    assert_eq!(hits[1].byte_len, 10);

    assert_eq!(hits[2].tree_path, "0/2");
    assert_eq!(hits[2].kind, StringMatchKind::WholeAscii);
    assert_eq!(hits[2].text, "TAG4");
    assert_eq!(hits[2].byte_len, 4);

    assert_eq!(hits[3].tree_path, "0/4/mc0");
    assert!(hits[3].is_mini_chunk);
    assert_eq!(hits[3].node_id, 1);
    assert_eq!(hits[3].kind, StringMatchKind::WholeAsciiz);
    assert_eq!(hits[3].text, "DiffuseMap");

    // Kind filters: ASCIIZ only vs ASCII only.
    whole_opts.include_ascii = false;
    let asciiz_only = find_strings(&df, &whole_opts);
    assert_eq!(asciiz_only.len(), 3);
    assert!(asciiz_only.iter().all(|h| h.kind.is_nul_terminated()));

    whole_opts.include_ascii = true;
    whole_opts.include_asciiz = false;
    let ascii_only = find_strings(&df, &whole_opts);
    assert_eq!(ascii_only.len(), 1);
    assert_eq!(ascii_only[0].text, "TAG4");

    // Embedded scan discovers ASCIIZ and ASCII runs inside mixed_payload with exact file offsets.
    let DynamicChunkContent::Children(children) = &df.chunks[0].content else {
        panic!("expected root children");
    };
    let mixed_chunk_payload_offset = children[5].payload_offset;

    let embedded_opts = FindStringsOptions {
        include_embedded: true,
        min_embedded_len: 4,
        ..FindStringsOptions::default()
    };
    let emb_hits = find_strings(&df, &embedded_opts);
    assert_eq!(emb_hits.len(), 6);

    assert_eq!(emb_hits[4].tree_path, "0/5");
    assert_eq!(emb_hits[4].kind, StringMatchKind::EmbeddedAsciiz);
    assert_eq!(emb_hits[4].text, "ShaderFX");
    assert_eq!(emb_hits[4].payload_offset, 4);
    assert_eq!(emb_hits[4].byte_len, 9);
    assert_eq!(emb_hits[4].file_offset, mixed_chunk_payload_offset + 4);

    assert_eq!(emb_hits[5].tree_path, "0/5");
    assert_eq!(emb_hits[5].kind, StringMatchKind::EmbeddedAscii);
    assert_eq!(emb_hits[5].text, "ALBEDO");
    assert_eq!(emb_hits[5].payload_offset, 15);
    assert_eq!(emb_hits[5].byte_len, 6);
    assert_eq!(emb_hits[5].file_offset, mixed_chunk_payload_offset + 15);

    // max_results cap stops cleanly.
    let capped_opts = FindStringsOptions {
        max_results: 2,
        ..embedded_opts
    };
    assert_eq!(find_strings(&df, &capped_opts).len(), 2);
}

#[cfg(feature = "codegen")]
#[test]
fn test_codegen_handles_custom_constructs_and_error_cases() {
    let yaml = r#"
name: CustomFmt
root: CustomRoot
types:
  CustomRoot:
    kind: chunks
    mode: sequence
    ignore_unknown: true
    fields:
      - name: kind
        cardinality:
          default_expr: "42"
        source_kind: id
        id: 0x00000001
        payload_kind: data
        payload_type: u32
      - name: items
        cardinality: repeated
        source_kind: stream
        item_type: ItemEnum
  ItemEnum:
    kind: chunk_enum
    variants:
      - name: Skip
        source_kind: id
        id: 0x00000002
        payload_kind: ignore
      - name: Pair
        source_kind: stream
        item_type: PairSeq
  PairSeq:
    kind: chunk_stream_seq
    fields:
      - name: first
        source_kind: id
        id: 0x00000003
        payload_kind: data
        payload_type: HeaderBlock
      - name: second
        source_kind: id
        id: 0x00000004
        payload_kind: data
        payload_type: (u8, u16)
  HeaderBlock:
    kind: byte_block
    pad_after: 4
    fields:
      - name: flag
        ty: bool
        wire_format: U8Bool
        pad_after: 3
"#;
    let schema = Schema::from_yaml_str(yaml).unwrap();
    let resolved = SchemaLoader::resolve(schema, None).unwrap();
    let code = generate_rust_code(&resolved).unwrap();
    let parsed = syn::parse_file(&code).unwrap();
    assert!(!parsed.items.is_empty());
    assert!(code.contains("default = 42"));
    assert!(code.contains("pad_after = 4"));
    assert!(code.contains("pad_after = 3"));

    // Map container referenced as stream item must return CodegenError::MapUsedAsStreamItem
    let invalid_yaml = r#"
name: InvalidStream
root: Root
types:
  Root:
    kind: chunks
    mode: sequence
    fields:
      - name: bad
        source_kind: stream
        item_type: MapChild
  MapChild:
    kind: chunks
    mode: map
    fields: []
"#;
    let bad = SchemaLoader::resolve(Schema::from_yaml_str(invalid_yaml).unwrap(), None).unwrap();
    let err = generate_rust_code(&bad).unwrap_err();
    assert!(err.to_string().contains("MapChild"));
}

#[cfg(all(feature = "codegen", feature = "known-formats"))]
#[test]
fn test_codegen_produces_valid_rust_ast_for_all_known_formats() {
    for name in ["Alo", "AloModel", "AloParticle", "AlaAnimation"] {
        let resolved = SchemaLoader::load(&SchemaRef::Builtin(name.to_owned())).unwrap();
        let code = generate_rust_code(&resolved)
            .unwrap_or_else(|e| panic!("codegen failed for `{name}`: {e}"));
        let parsed = syn::parse_file(&code)
            .unwrap_or_else(|e| panic!("generated code for `{name}` failed to parse: {e}\n{code}"));
        assert!(!parsed.items.is_empty());

        if name == "Alo" {
            assert!(code.contains("pub enum AloFile"));
            assert!(code.contains("pub mod model"));
            assert!(code.contains("pub mod particle"));
            assert!(code.contains("ChunkStructure, ChunkStreamItem"));
        } else {
            assert!(!code.contains("pub mod "));
            assert!(code.contains(&format!("pub struct {name}")));
        }
    }
}
