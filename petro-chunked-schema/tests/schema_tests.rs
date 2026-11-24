use petro_chunked_schema::codegen::generate_rust_code;
use petro_chunked_schema::dynamic::{
    DynamicChunkContent, LoadOptions, inspect_bytes_at_offset, load_dynamic_file, validate_bytes,
};
use petro_chunked_schema::known_formats;
use petro_chunked_schema::schema::Schema;

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

#[test]
fn test_known_formats_parse_and_roundtrip_yaml() {
    for schema in [
        known_formats::alo_model(),
        known_formats::alo_particle(),
        known_formats::ala_animation(),
    ] {
        let yaml = schema.to_yaml_string().unwrap();
        let reparsed = Schema::from_yaml_str(&yaml).unwrap();
        assert_eq!(schema, reparsed);

        let rust_code = generate_rust_code(&schema);
        assert!(rust_code.contains(&format!("pub struct {}", schema.name)));
        assert!(rust_code.contains("ChunkStructure"));
    }
}

#[test]
fn test_dynamic_loader_and_byte_spans_on_synthetic_alo_and_ala() {
    let ala_schema = known_formats::ala_animation();

    // Build a minimal Format #1 .ala file (0x1000 -> 0x1001 header + 1 0x1002 bone)
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

    let file_bytes = make_chunk(0x1000, true, &root_children);

    let dyn_file = validate_bytes(&ala_schema, &file_bytes).unwrap();
    assert_eq!(dyn_file.chunks.len(), 1);
    assert_eq!(
        dyn_file.chunks[0].schema_field_name.as_deref(),
        Some("animation")
    );

    // Also test forcing plain data on chunk "0/0" (0x1001 header)
    let mut opts = LoadOptions::default();
    opts.force_plain_data_paths.insert("0/0".to_owned());
    let forced = load_dynamic_file(&file_bytes, Some(&ala_schema), &opts).unwrap();
    let DynamicChunkContent::Children(children) = &forced.chunks[0].content else {
        panic!("expected children");
    };
    assert!(matches!(
        children[0].content,
        DynamicChunkContent::Data { .. }
    ));

    // Test multi-format byte inspector
    let previews = inspect_bytes_at_offset(b"Root\0\x01\x00\x00\x00", 0);
    assert!(previews.iter().any(|p| p.label == "CString (ASCIIZ)"));
}
