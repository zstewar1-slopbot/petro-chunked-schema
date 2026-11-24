//! GameData directory scanner, in-memory Petroglyph `.meg` archive reader, and batch schema
//! validator for `petro-chunked-schema-editor`.

use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use petro_chunked_schema::dynamic::{SchemaDiagnostic, validate_bytes};
use petro_chunked_schema::schema::Schema;
use petro_meg::reader::{MegReadOptions, ReadMegMeta};

use crate::state::FileSource;

/// A file discovered in a configured `GameData` directory (either loose on disk or inside a `.meg` archive).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredFile {
    pub display_name: String,
    pub extension: String,
    pub size: u64,
    pub source: FileSource,
}

/// Result of a single file failing schema validation during a batch validation run.
#[derive(Debug, Clone, PartialEq)]
pub struct FileValidationFailure {
    pub display_name: String,
    pub source: FileSource,
    pub diagnostics: Vec<SchemaDiagnostic>,
}

/// Summary report from running "Validate" over all files matching an extension in all configured
/// `GameData` directories.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BatchValidationReport {
    pub schema_name: String,
    pub extension: String,
    pub total_checked: usize,
    pub passed_count: usize,
    pub failed_count: usize,
    pub skipped_count: usize,
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
                .with_context(|| format!("extracting `{entry_name}` from {}", meg_path.display()))?;
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

    // 1. Check loose disk files
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

    // 2. Check entries inside .meg archives
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

/// Runs batch validation of `schema` across all files with `extension` in all configured
/// `GameData` directories (both loose files and inside `.meg` archives).
pub fn run_batch_validation(
    schema: &Schema,
    game_data_dirs: &[PathBuf],
    extension: &str,
) -> BatchValidationReport {
    let ext_clean = extension
        .trim()
        .trim_start_matches('.')
        .to_ascii_lowercase();
    let mut report = BatchValidationReport {
        schema_name: schema.name.clone(),
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
        // Petroglyph uses the `.alo` extension for both 3D models (`AloModel`) and particle
        // systems (`AloParticle`, whose root chunk is `0x0900`). Skip the other sub-format when
        // validating specifically `AloParticle` or `AloModel`.
        if ext_clean == "alo" {
            let is_particle_root = bytes.len() >= 8
                && u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) == 0x0900;
            if schema.name.eq_ignore_ascii_case("AloParticle") && !is_particle_root {
                report.skipped_count += 1;
                return;
            }
            if schema.name.eq_ignore_ascii_case("AloModel") && is_particle_root {
                report.skipped_count += 1;
                return;
            }
        }

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
        if let Ok(bytes) = std::fs::read(&path) {
            let disp = path.display().to_string();
            check_buffer(&bytes, disp, FileSource::Disk { path });
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
            }
        }
    }

    report
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

/// Helper to load a schema from disk.
pub fn load_schema_from_path(path: &Path) -> anyhow::Result<Schema> {
    let yaml = std::fs::read_to_string(path)
        .with_context(|| format!("reading schema file {}", path.display()))?;
    let schema = Schema::from_yaml_str(&yaml)
        .with_context(|| format!("parsing YAML schema {}", path.display()))?;
    Ok(schema)
}

/// Helper to save a schema to disk.
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
    use super::*;
    use petro_chunked_schema::dynamic::{LoadOptions, load_dynamic_file};
    use petro_chunked_schema::known_formats;

    #[test]
    fn test_in_memory_meg_archive_discovery_and_loading() {
        let eaw_data = PathBuf::from(
            "/var/home/slopbot/Projects/eaw/Star Wars Empire at War/GameData/Data",
        );
        if !eaw_data.exists() {
            // Skip when game assets aren't present on the test machine
            return;
        }
        let dirs = vec![eaw_data];

        // 1. Discover .alo files inside .meg archives
        let alo_files = discover_files(&dirs, Some("alo"), "", 10);
        assert!(!alo_files.is_empty(), "Expected to find .alo entries in Models.meg");
        let first_alo = &alo_files[0];
        assert!(matches!(first_alo.source, FileSource::MegEntry { .. }));

        // 2. Load the .alo entry directly into memory without extracting to disk
        let bytes = load_file_bytes(&first_alo.source).expect("load_file_bytes from .meg");
        assert_eq!(bytes.len() as u64, first_alo.size);

        // 3. Validate with known_formats::alo_model() or alo_particle()
        let is_particle = bytes.len() >= 8
            && u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) == 0x0900;
        let schema = if is_particle {
            known_formats::alo_particle()
        } else {
            known_formats::alo_model()
        };
        let df = load_dynamic_file(&bytes, Some(&schema), &LoadOptions::default()).unwrap();
        assert!(
            df.diagnostics.is_empty(),
            "Unexpected diagnostics on {}: {:?}",
            first_alo.display_name,
            df.diagnostics
        );
    }
}
