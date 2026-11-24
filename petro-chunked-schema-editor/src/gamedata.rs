//! GameData directory scanner, in-memory Petroglyph `.meg` archive reader, schema I/O, and
//! batch schema validator for `petro-chunked-schema-editor`.

use std::collections::BTreeSet;
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use petro_chunked_schema::dynamic::{SchemaDiagnostic, validate_bytes};
use petro_chunked_schema::resolve::{ResolvedSchema, SchemaLoader};
use petro_chunked_schema::schema::Schema;
use petro_chunked_schema::source::SchemaRef;
use petro_meg::reader::{MegReadOptions, ReadMegMeta};

use crate::state::FileSource;

/// A file discovered in a configured `GameData` directory (either loose on disk or inside a `.meg`
/// archive).
#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredFile {
    /// Human-readable display string (`path` or `archive.meg :: entry`).
    pub display_name: String,
    /// Lowercase file extension without a leading dot.
    pub extension: String,
    /// Uncompressed size in bytes.
    pub size: u64,
    /// Origin handle used to load bytes into the editor.
    pub source: FileSource,
}

/// Result of a single file failing schema validation during a batch validation run.
#[derive(Debug, Clone, PartialEq)]
pub struct FileValidationFailure {
    /// Human-readable display label for the failing file.
    pub display_name: String,
    /// Source handle allowing one-click opening in the editor.
    pub source: FileSource,
    /// Validation and decoding diagnostics collected for this file.
    pub diagnostics: Vec<SchemaDiagnostic>,
}

/// Summary report from running batch validation over all files matching an extension across all
/// configured `GameData` directories.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BatchValidationReport {
    /// Name of the evaluated schema.
    pub schema_name: String,
    /// Lowercase extension filter that was checked.
    pub extension: String,
    /// Total number of matching files tested.
    pub total_checked: usize,
    /// Number of files with zero diagnostics.
    pub passed_count: usize,
    /// Number of files with one or more diagnostics.
    pub failed_count: usize,
    /// Number of files skipped due to I/O errors.
    pub skipped_count: usize,
    /// Detailed failure entries (capped at 200 to keep UI responsive).
    pub failures: Vec<FileValidationFailure>,
}

/// Reads the raw bytes of a [`FileSource`], extracting directly into memory if it resides inside a
/// Petroglyph `.meg` archive.
pub fn load_file_bytes(source: &FileSource) -> anyhow::Result<Vec<u8>> {
    match source {
        FileSource::Disk { path } => {
            std::fs::read(path).with_context(|| format!("reading file {}", path.display()))
        }
        FileSource::MegEntry {
            meg_path,
            entry_name,
        } => {
            let mut file = File::open(meg_path)
                .with_context(|| format!("opening MEGA archive {}", meg_path.display()))?;
            let entries = None
                .read_meg_meta(BufReader::new(&mut file))
                .with_context(|| format!("reading MEGA header from {}", meg_path.display()))?;
            let target = entries
                .iter()
                .find(|e| e.name().as_str().eq_ignore_ascii_case(entry_name))
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "entry `{entry_name}` not found in MEGA archive {}",
                        meg_path.display()
                    )
                })?;
            let mut reader = target
                .extract_from(&mut file, &MegReadOptions::default())
                .with_context(|| {
                    format!("extracting `{entry_name}` from {}", meg_path.display())
                })?;
            let mut buf = Vec::with_capacity(target.size() as usize);
            reader
                .read_to_end(&mut buf)
                .with_context(|| format!("reading `{entry_name}` from {}", meg_path.display()))?;
            Ok(buf)
        }
    }
}

/// Scans all configured `GameData` directories for loose files and `.meg` archive entries matching
/// `ext_filter` (if provided) and `search_query` (case-insensitive substring), returning at most
/// `max_results` items.
#[cfg(test)]
pub fn discover_files(
    game_data_dirs: &[PathBuf],
    ext_filter: Option<&str>,
    search_query: &str,
    max_results: usize,
) -> Vec<DiscoveredFile> {
    let mut out = Vec::new();
    let query_lower = search_query.trim().to_ascii_lowercase();
    let ext_lower = ext_filter
        .map(|s| s.trim().trim_start_matches('.').to_ascii_lowercase())
        .filter(|s| !s.is_empty());

    let mut disk_files = Vec::new();
    let mut meg_files = Vec::new();
    for dir in game_data_dirs {
        collect_dir_files(dir, 0, 4, &mut disk_files, &mut meg_files);
    }

    for path in disk_files {
        if out.len() >= max_results {
            return out;
        }
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if let Some(req_ext) = &ext_lower {
            if &ext != req_ext {
                continue;
            }
        } else if !matches!(ext.as_str(), "alo" | "ala" | "ted" | "bui") {
            continue;
        }
        let disp = path.display().to_string();
        if !query_lower.is_empty() && !disp.to_ascii_lowercase().contains(&query_lower) {
            continue;
        }
        let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        out.push(DiscoveredFile {
            display_name: disp,
            extension: ext,
            size,
            source: FileSource::Disk { path },
        });
    }

    for meg_path in meg_files {
        if out.len() >= max_results {
            return out;
        }
        let Ok(file) = File::open(&meg_path) else {
            continue;
        };
        let Ok(entries) = None.read_meg_meta(BufReader::new(file)) else {
            continue;
        };
        let meg_name = meg_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("archive.meg");

        for entry in entries {
            if out.len() >= max_results {
                return out;
            }
            let entry_str = entry.name().as_str();
            let ext = entry_str
                .rsplit_once('.')
                .map(|(_, e)| e.to_ascii_lowercase())
                .unwrap_or_default();
            if let Some(req_ext) = &ext_lower {
                if &ext != req_ext {
                    continue;
                }
            } else if !matches!(ext.as_str(), "alo" | "ala" | "ted" | "bui") {
                continue;
            }
            let disp = format!("{meg_name} :: {entry_str}");
            if !query_lower.is_empty() && !disp.to_ascii_lowercase().contains(&query_lower) {
                continue;
            }
            out.push(DiscoveredFile {
                display_name: disp,
                extension: ext,
                size: entry.size() as u64,
                source: FileSource::MegEntry {
                    meg_path: meg_path.clone(),
                    entry_name: entry_str.to_owned(),
                },
            });
        }
    }

    out
}

/// Runs batch validation of `schema` across every loose file and `.meg` archive entry matching
/// `extension` in `game_data_dirs`.
pub fn run_batch_validation(
    schema: &ResolvedSchema,
    game_data_dirs: &[PathBuf],
    extension: &str,
) -> BatchValidationReport {
    let ext_clean = extension
        .trim()
        .trim_start_matches('.')
        .to_ascii_lowercase();
    let mut report = BatchValidationReport {
        schema_name: schema.root_schema().name.clone(),
        extension: ext_clean.clone(),
        ..Default::default()
    };

    if ext_clean.is_empty() {
        return report;
    }

    let mut disk_files = Vec::new();
    let mut meg_files = Vec::new();
    for dir in game_data_dirs {
        collect_dir_files(dir, 0, 4, &mut disk_files, &mut meg_files);
    }

    let mut check_buffer = |bytes: &[u8], display_name: String, source: FileSource| {
        report.total_checked += 1;
        match validate_bytes(schema, bytes) {
            Ok(_) => {
                report.passed_count += 1;
            }
            Err(diags) => {
                report.failed_count += 1;
                if report.failures.len() < 200 {
                    report.failures.push(FileValidationFailure {
                        display_name,
                        source,
                        diagnostics: diags,
                    });
                }
            }
        }
    };

    for path in disk_files {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if ext != ext_clean {
            continue;
        }
        match std::fs::read(&path) {
            Ok(bytes) => {
                let disp = path.display().to_string();
                check_buffer(&bytes, disp, FileSource::Disk { path });
            }
            Err(_) => {
                report.skipped_count += 1;
            }
        }
    }

    for meg_path in meg_files {
        let Ok(mut file) = File::open(&meg_path) else {
            continue;
        };
        let Ok(entries) = None.read_meg_meta(BufReader::new(&mut file)) else {
            continue;
        };
        let meg_name = meg_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("archive.meg")
            .to_owned();

        for entry in entries {
            let entry_str = entry.name().as_str();
            let ext = entry_str
                .rsplit_once('.')
                .map(|(_, e)| e.to_ascii_lowercase())
                .unwrap_or_default();
            if ext != ext_clean {
                continue;
            }
            let Ok(mut reader) = entry.extract_from(&mut file, &MegReadOptions::default()) else {
                report.skipped_count += 1;
                continue;
            };
            let mut buf = Vec::with_capacity(entry.size() as usize);
            if reader.read_to_end(&mut buf).is_ok() {
                let disp = format!("{meg_name} :: {entry_str}");
                let src = FileSource::MegEntry {
                    meg_path: meg_path.clone(),
                    entry_name: entry_str.to_owned(),
                };
                check_buffer(&buf, disp, src);
            } else {
                report.skipped_count += 1;
            }
        }
    }

    report
}

/// Discovers candidate Petroglyph `GameData` directories from environment variables
/// (`EAW_GAME_DATA`, `EAW_DATA_DIR`), standard Steam library paths, and workspace-relative paths.
pub fn detect_default_gamedata_dirs() -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    for var in ["EAW_GAME_DATA", "EAW_DATA_DIR"] {
        if let Some(val) = std::env::var_os(var)
            && !val.is_empty()
        {
            candidates.push(PathBuf::from(val));
        }
    }

    if let Some(home) = dirs::home_dir() {
        for steam_root in [
            home.join(".local/share/Steam/steamapps/common/Star Wars Empire at War"),
            home.join(".steam/steam/steamapps/common/Star Wars Empire at War"),
        ] {
            candidates.push(steam_root.join("GameData/Data"));
            candidates.push(steam_root.join("corruption/Data"));
        }
    }

    for rel in [
        "Star Wars Empire at War/GameData/Data",
        "Star Wars Empire at War/corruption/Data",
        "../Star Wars Empire at War/GameData/Data",
        "../Star Wars Empire at War/corruption/Data",
        "GameData/Data",
    ] {
        candidates.push(PathBuf::from(rel));
    }

    let mut seen = BTreeSet::new();
    let mut found = Vec::new();
    for path in candidates {
        if path.is_dir() {
            let key = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
            if seen.insert(key) {
                found.push(path);
            }
        }
    }
    found
}

fn collect_dir_files(
    dir: &Path,
    depth: usize,
    max_depth: usize,
    disk_files: &mut Vec<PathBuf>,
    meg_files: &mut Vec<PathBuf>,
) {
    if depth > max_depth {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_dir_files(&path, depth + 1, max_depth, disk_files, meg_files);
        } else if path.is_file() {
            let is_meg = path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("meg"));
            if is_meg {
                meg_files.push(path);
            } else {
                disk_files.push(path);
            }
        }
    }
}

/// Loads and resolves a [`Schema`] and its imports from a [`SchemaRef`].
pub fn load_schema_from_ref(schema_ref: &SchemaRef) -> anyhow::Result<(Schema, ResolvedSchema)> {
    let resolved = SchemaLoader::load(schema_ref)
        .with_context(|| format!("loading schema reference `{schema_ref}`"))?;
    Ok((resolved.root_schema().clone(), resolved))
}

/// Serializes `schema` to YAML and writes it to `path`.
pub fn save_schema_to_path(schema: &Schema, path: &Path) -> anyhow::Result<()> {
    if path.as_os_str().is_empty() {
        bail!("Schema file path is empty");
    }
    let yaml = schema
        .to_yaml_string()
        .context("serializing schema to YAML")?;
    std::fs::write(path, yaml).with_context(|| format!("writing schema to {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    struct TempDirGuard(PathBuf);

    impl TempDirGuard {
        fn new(prefix: &str) -> Self {
            static COUNTER: AtomicUsize = AtomicUsize::new(0);
            let id = COUNTER.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!(
                "petro_editor_gamedata_{prefix}_{}_{id}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("create temp dir");
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

    fn make_particle_alo() -> Vec<u8> {
        let mut sys = Vec::new();
        sys.extend(make_chunk(0x0000, false, b"Fx\0"));
        sys.extend(make_chunk(0x0001, false, &1u32.to_le_bytes()));
        sys.extend(make_chunk(0x0800, true, &[]));
        make_chunk(0x0900, true, &sys)
    }

    fn make_model_alo() -> Vec<u8> {
        let mut skel_hdr = Vec::from(0u32.to_le_bytes());
        skel_hdr.extend_from_slice(&[0u8; 124]);
        let skel = make_chunk(0x0201, false, &skel_hdr);
        make_chunk(0x0200, true, &skel)
    }

    #[test]
    fn test_discover_and_batch_validate_loose_files() {
        let tmp = TempDirGuard::new("batch");
        std::fs::write(tmp.path().join("fx.alo"), make_particle_alo()).unwrap();
        std::fs::write(tmp.path().join("ship.alo"), make_model_alo()).unwrap();
        std::fs::write(tmp.path().join("broken.alo"), b"BAD!").unwrap();

        let dirs = vec![tmp.path().clone()];
        let found = discover_files(&dirs, Some("alo"), "", 50);
        assert_eq!(found.len(), 3);

        // Unified `Alo` passes both particle and model, rejects `broken.alo`, skips none.
        let (_, alo) = load_schema_from_ref(&SchemaRef::Builtin("Alo".to_owned())).unwrap();
        let report_alo = run_batch_validation(&alo, &dirs, "alo");
        assert_eq!(report_alo.total_checked, 3);
        assert_eq!(report_alo.passed_count, 2);
        assert_eq!(report_alo.failed_count, 1);
        assert_eq!(report_alo.skipped_count, 0);

        // `AloParticle` passes ONLY `fx.alo` and fails `ship.alo` + `broken.alo` (non-overlapping).
        let (_, particle) =
            load_schema_from_ref(&SchemaRef::Builtin("AloParticle".to_owned())).unwrap();
        let report_p = run_batch_validation(&particle, &dirs, "alo");
        assert_eq!(report_p.total_checked, 3);
        assert_eq!(report_p.passed_count, 1);
        assert_eq!(report_p.failed_count, 2);
        assert_eq!(report_p.skipped_count, 0);
    }

    #[test]
    fn test_env_gated_meg_archive_discovery_and_loading() {
        let Some(dir) = std::env::var_os("EAW_GAME_DATA_DIR") else {
            return;
        };
        let dirs = vec![PathBuf::from(dir)];
        let alo_files = discover_files(&dirs, Some("alo"), "", 5);
        assert!(!alo_files.is_empty());

        let bytes = load_file_bytes(&alo_files[0].source).expect("load_file_bytes");
        assert_eq!(bytes.len() as u64, alo_files[0].size);

        let (_, alo) = load_schema_from_ref(&SchemaRef::Builtin("Alo".to_owned())).unwrap();
        assert!(validate_bytes(&alo, &bytes).is_ok());
    }
}
