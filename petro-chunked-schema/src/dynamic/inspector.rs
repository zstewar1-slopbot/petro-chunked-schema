//! Multi-format cursor byte inspector for hex-editor preview panes.

use std::ffi::CStr;

use crate::schema::{DataType, WireFormat};

/// Preview entry produced by [`inspect_bytes_at_offset`] for a hex-editor data inspector.
#[derive(Debug, Clone, PartialEq)]
pub struct BytePreviewEntry {
    /// Human-readable interpretation label (e.g. `"f32"`, `"Vec3 ([f32; 3])"`, `"CString (ASCIIZ)"`).
    pub label: &'static str,
    /// Number of bytes consumed by this interpretation.
    pub byte_len: usize,
    /// Formatted value preview string.
    pub value: String,
    /// Suggested schema `(DataType, Option<WireFormat>)` pair for one-click field creation.
    pub suggested_type: Option<(DataType, Option<WireFormat>)>,
}

/// Decodes the bytes starting at `bytes[offset..]` into common primitive, vector, boolean, and
/// null-terminated string interpretations.
pub fn inspect_bytes_at_offset(bytes: &[u8], offset: usize) -> Vec<BytePreviewEntry> {
    let mut out = Vec::new();
    let Some(rem) = bytes.get(offset..) else {
        return out;
    };
    if rem.is_empty() {
        return out;
    }

    // 1-byte types
    let b0 = rem[0];
    out.push(BytePreviewEntry {
        label: "u8",
        byte_len: 1,
        value: format!("{b0} (0x{b0:02x})"),
        suggested_type: Some((DataType::U8, None)),
    });
    out.push(BytePreviewEntry {
        label: "i8",
        byte_len: 1,
        value: format!("{}", b0 as i8),
        suggested_type: Some((DataType::I8, None)),
    });
    if b0 <= 1 {
        out.push(BytePreviewEntry {
            label: "bool (U8Bool)",
            byte_len: 1,
            value: format!("{}", b0 == 1),
            suggested_type: Some((DataType::Bool, Some(WireFormat::U8Bool))),
        });
    }

    // 2-byte types
    if rem.len() >= 2 {
        let u = u16::from_le_bytes([rem[0], rem[1]]);
        let i = i16::from_le_bytes([rem[0], rem[1]]);
        out.push(BytePreviewEntry {
            label: "u16",
            byte_len: 2,
            value: format!("{u} (0x{u:04x})"),
            suggested_type: Some((DataType::U16, None)),
        });
        out.push(BytePreviewEntry {
            label: "i16",
            byte_len: 2,
            value: format!("{i}"),
            suggested_type: Some((DataType::I16, None)),
        });
    }

    // 4-byte types
    if rem.len() >= 4 {
        let arr = [rem[0], rem[1], rem[2], rem[3]];
        let u = u32::from_le_bytes(arr);
        let i = i32::from_le_bytes(arr);
        let f = f32::from_le_bytes(arr);
        out.push(BytePreviewEntry {
            label: "u32",
            byte_len: 4,
            value: format!("{u} (0x{u:08x})"),
            suggested_type: Some((DataType::U32, None)),
        });
        out.push(BytePreviewEntry {
            label: "i32",
            byte_len: 4,
            value: format!("{i}"),
            suggested_type: Some((DataType::I32, None)),
        });
        out.push(BytePreviewEntry {
            label: "f32",
            byte_len: 4,
            value: format!("{f:.6}"),
            suggested_type: Some((DataType::F32, None)),
        });
        if u <= 1 {
            out.push(BytePreviewEntry {
                label: "bool (U32Bool)",
                byte_len: 4,
                value: format!("{}", u == 1),
                suggested_type: Some((DataType::Bool, Some(WireFormat::U32Bool))),
            });
        }
        out.push(BytePreviewEntry {
            label: "usize (U32Usize)",
            byte_len: 4,
            value: format!("{u}"),
            suggested_type: Some((DataType::Usize, Some(WireFormat::U32Usize))),
        });
    }

    // 8-byte types
    if rem.len() >= 8 {
        let arr: [u8; 8] = rem[..8].try_into().unwrap();
        let u = u64::from_le_bytes(arr);
        let i = i64::from_le_bytes(arr);
        let f = f64::from_le_bytes(arr);
        out.push(BytePreviewEntry {
            label: "u64",
            byte_len: 8,
            value: format!("{u} (0x{u:016x})"),
            suggested_type: Some((DataType::U64, None)),
        });
        out.push(BytePreviewEntry {
            label: "i64",
            byte_len: 8,
            value: format!("{i}"),
            suggested_type: Some((DataType::I64, None)),
        });
        out.push(BytePreviewEntry {
            label: "f64",
            byte_len: 8,
            value: format!("{f:.6}"),
            suggested_type: Some((DataType::F64, None)),
        });
        let f0 = f32::from_le_bytes(rem[0..4].try_into().unwrap());
        let f1 = f32::from_le_bytes(rem[4..8].try_into().unwrap());
        out.push(BytePreviewEntry {
            label: "Vec2 ([f32; 2])",
            byte_len: 8,
            value: format!("[{f0:.4}, {f1:.4}]"),
            suggested_type: Some((DataType::Vec2, None)),
        });
    }

    // 12-byte Vec3
    if rem.len() >= 12 {
        let f0 = f32::from_le_bytes(rem[0..4].try_into().unwrap());
        let f1 = f32::from_le_bytes(rem[4..8].try_into().unwrap());
        let f2 = f32::from_le_bytes(rem[8..12].try_into().unwrap());
        out.push(BytePreviewEntry {
            label: "Vec3 ([f32; 3])",
            byte_len: 12,
            value: format!("[{f0:.4}, {f1:.4}, {f2:.4}]"),
            suggested_type: Some((DataType::Vec3, None)),
        });
    }

    // 16-byte Vec4
    if rem.len() >= 16 {
        let f0 = f32::from_le_bytes(rem[0..4].try_into().unwrap());
        let f1 = f32::from_le_bytes(rem[4..8].try_into().unwrap());
        let f2 = f32::from_le_bytes(rem[8..12].try_into().unwrap());
        let f3 = f32::from_le_bytes(rem[12..16].try_into().unwrap());
        out.push(BytePreviewEntry {
            label: "Vec4 ([f32; 4])",
            byte_len: 16,
            value: format!("[{f0:.4}, {f1:.4}, {f2:.4}, {f3:.4}]"),
            suggested_type: Some((DataType::Vec4, None)),
        });
    }

    // Null-terminated ASCII string preview
    if let Ok(cstr) = CStr::from_bytes_until_nul(rem)
        && let Ok(s) = cstr.to_str()
        && !s.is_empty()
        && s.chars().all(|c| !c.is_control() || c == '\t')
    {
        let len = cstr.to_bytes_with_nul().len();
        out.push(BytePreviewEntry {
            label: "CString (ASCIIZ)",
            byte_len: len,
            value: format!("{s:?}"),
            suggested_type: Some((DataType::CString, None)),
        });
    }

    out
}
