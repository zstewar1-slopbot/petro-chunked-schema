//! Hierarchical filesystem and virtual `.meg` archive browser backend for
//! `petro-chunked-schema-editor`.
//!
//! Provides a unified [`BrowseLocation`] abstraction that lets modal dialogs browse local
//! directories on disk and step directly inside Petroglyph `.meg` archives as virtual directory
//! trees without extracting files to disk.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};

use petro_meg::reader::ReadMegMeta;

use crate::state::FileSource;

/// Current folder location displayed in a file, directory, or schema browser dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrowseLocation {
    /// A real directory on the local filesystem.
    DiskDir(PathBuf),
    /// A virtual folder inside a Petroglyph `.meg` archive.
    ///
    /// `inner_prefix` uses forward slashes with no leading or trailing slash (`""` for the
    /// archive root, `"DATA/ART/MODELS"` for a nested folder).
    MegDir {
        /// Filesystem path to the `.meg` archive file.
        meg_path: PathBuf,
        /// Normalized forward-slash folder prefix inside the archive.
        inner_prefix: String,
    },
}

/// Classification of one row inside a browser listing.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum BrowseEntryKind {
    /// Real subdirectory on disk.
    Directory,
    /// Petroglyph `.meg` archive file that can be entered as a virtual folder.
    MegArchive,
    /// Virtual subdirectory synthesized from path separators inside a `.meg` archive.
    MegVirtualDir,
    /// Regular selectable file on disk.
    DiskFile,
    /// Selectable file entry inside a `.meg` archive.
    MegFile,
}

impl BrowseEntryKind {
    /// Returns `true` if activating this entry navigates into a nested [`BrowseLocation`].
    pub fn is_navigable(self) -> bool {
        matches!(self, Self::Directory | Self::MegArchive | Self::MegVirtualDir)
    }

    /// Short uppercase type badge displayed in modal list rows.
    pub fn badge(self) -> &'static str {
        match self {
            Self::Directory => "DIR",
            Self::MegArchive => "MEG",
            Self::MegVirtualDir => "DIR",
            Self::DiskFile => "FILE",
            Self::MegFile => "FILE",
        }
    }
}

/// One row returned by [`list_location`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowseEntry {
    /// Display name (basename, or relative path inside a `.meg` during recursive search).
    pub name: String,
    /// Entry classification (`DIR`, `MEG`, `FILE`).
    pub kind: BrowseEntryKind,
    /// File size in bytes when applicable.
    pub size: Option<u64>,
    /// Lowercase file extension without a leading dot (empty for folders).
    pub extension: String,
    /// Destination location when navigating into a folder or `.meg` archive.
    pub target_location: Option<BrowseLocation>,
    /// Selectable [`FileSource`] when this entry is a game/binary file.
    pub file_source: Option<FileSource>,
    /// Filesystem path when this entry lives directly on disk (files and directories).
    pub disk_path: Option<PathBuf>,
}

/// Filtering and display options for [`list_location`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowseOptions {
    /// When `true`, only disk directories are returned (for directory pickers).
    pub dirs_only: bool,
    /// When `true`, `.meg` files are shown as enterable [`BrowseEntryKind::MegArchive`] entries.
    pub allow_meg_browse: bool,
    /// Allowed lowercase file extensions without leading dots (empty means allow all files).
    pub extensions: Vec<String>,
    /// Case-insensitive substring filter applied to entry names (and recursively inside `.meg`
    /// archives when [`Self::recursive_in_meg`] is enabled).
    pub name_filter: String,
    /// When `true` and [`Self::name_filter`] is non-empty inside a `.meg` archive, returns all
    /// matching files beneath `inner_prefix` regardless of nesting depth.
    pub recursive_in_meg: bool,
    /// When `true`, includes Unix dotfiles/dotfolders.
    pub show_hidden: bool,
    /// Maximum number of entries returned to keep modal rendering responsive.
    pub max_entries: usize,
}

impl Default for BrowseOptions {
    fn default() -> Self {
        Self {
            dirs_only: false,
            allow_meg_browse: true,
            extensions: Vec::new(),
            name_filter: String::new(),
            recursive_in_meg: true,
            show_hidden: false,
            max_entries: 500,
        }
    }
}

/// One clickable breadcrumb segment for the current [`BrowseLocation`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BreadcrumbSegment {
    /// Short label rendered on the breadcrumb button.
    pub label: String,
    /// Location navigated to when this segment is clicked.
    pub location: BrowseLocation,
}

/// Quick-access sidebar shortcut in file and directory browser modals.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuickPlace {
    /// Short human-readable label (e.g. `"GameData/Data"`, `"Home"`, `"Workspace"`).
    pub label: String,
    /// Optional secondary hint shown in muted text.
    pub subtitle: Option<String>,
    /// Destination directory or archive location.
    pub location: BrowseLocation,
}

/// Lists entries at `location` according to `options`, sorted with folders first, then `.meg`
/// archives, then regular files (case-insensitive alphabetical within each group).
pub fn list_location(
    location: &BrowseLocation,
    options: &BrowseOptions,
) -> Result<Vec<BrowseEntry>, String> {
    match location {
        BrowseLocation::DiskDir(dir) => list_disk_dir(dir, options),
        BrowseLocation::MegDir {
            meg_path,
            inner_prefix,
        } => list_meg_dir(meg_path, inner_prefix, options),
    }
}

fn list_disk_dir(dir: &Path, options: &BrowseOptions) -> Result<Vec<BrowseEntry>, String> {
    let read_dir = std::fs::read_dir(dir)
        .map_err(|e| format!("Cannot open directory {}: {e}", dir.display()))?;

    let filter_lower = options.name_filter.trim().to_ascii_lowercase();
    let allowed_exts: Vec<String> = options
        .extensions
        .iter()
        .map(|e| e.trim().trim_start_matches('.').to_ascii_lowercase())
        .filter(|e| !e.is_empty())
        .collect();

    let mut dirs = Vec::new();
    let mut megs = Vec::new();
    let mut files = Vec::new();

    for entry in read_dir.flatten() {
        let path = entry.path();
        let Some(name_os) = path.file_name() else {
            continue;
        };
        let name = name_os.to_string_lossy().to_string();
        if !options.show_hidden && name.starts_with('.') {
            continue;
        }
        if !filter_lower.is_empty() && !name.to_ascii_lowercase().contains(&filter_lower) {
            continue;
        }

        let meta = std::fs::metadata(&path).ok();
        let is_dir = meta.as_ref().is_some_and(|m| m.is_dir());

        if is_dir {
            dirs.push(BrowseEntry {
                name,
                kind: BrowseEntryKind::Directory,
                size: None,
                extension: String::new(),
                target_location: Some(BrowseLocation::DiskDir(path.clone())),
                file_source: None,
                disk_path: Some(path),
            });
            continue;
        }

        if options.dirs_only {
            continue;
        }

        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let size = meta.map(|m| m.len());

        if options.allow_meg_browse && ext == "meg" {
            megs.push(BrowseEntry {
                name,
                kind: BrowseEntryKind::MegArchive,
                size,
                extension: ext,
                target_location: Some(BrowseLocation::MegDir {
                    meg_path: path.clone(),
                    inner_prefix: String::new(),
                }),
                file_source: None,
                disk_path: Some(path),
            });
            continue;
        }

        if !allowed_exts.is_empty() && !allowed_exts.iter().any(|allowed| allowed == &ext) {
            continue;
        }

        files.push(BrowseEntry {
            name,
            kind: BrowseEntryKind::DiskFile,
            size,
            extension: ext,
            target_location: None,
            file_source: Some(FileSource::Disk { path: path.clone() }),
            disk_path: Some(path),
        });
    }

    sort_entries_by_name(&mut dirs);
    sort_entries_by_name(&mut megs);
    sort_entries_by_name(&mut files);

    let mut out = Vec::with_capacity((dirs.len() + megs.len() + files.len()).min(options.max_entries));
    for group in [dirs, megs, files] {
        for item in group {
            if out.len() >= options.max_entries {
                return Ok(out);
            }
            out.push(item);
        }
    }
    Ok(out)
}

fn list_meg_dir(
    meg_path: &Path,
    inner_prefix: &str,
    options: &BrowseOptions,
) -> Result<Vec<BrowseEntry>, String> {
    if options.dirs_only {
        return Ok(Vec::new());
    }
    let file = File::open(meg_path)
        .map_err(|e| format!("Cannot open MEGA archive {}: {e}", meg_path.display()))?;
    let meta_entries = None
        .read_meg_meta(BufReader::new(file))
        .map_err(|e| format!("Cannot read MEGA header {}: {e}", meg_path.display()))?;

    let raw_entries: Vec<(String, u64)> = meta_entries
        .into_iter()
        .map(|entry| (entry.name().as_str().to_owned(), entry.size() as u64))
        .collect();

    Ok(list_meg_entries_from_names(
        meg_path,
        &raw_entries,
        inner_prefix,
        options,
    ))
}

/// Pure helper that projects raw `.meg` archive `(entry_name, size)` pairs into virtual folders
/// and files beneath `inner_prefix`.
pub fn list_meg_entries_from_names(
    meg_path: &Path,
    entries: &[(String, u64)],
    inner_prefix: &str,
    options: &BrowseOptions,
) -> Vec<BrowseEntry> {
    if options.dirs_only {
        return Vec::new();
    }

    let norm_prefix = normalize_meg_prefix(inner_prefix);
    let prefix_with_slash = if norm_prefix.is_empty() {
        String::new()
    } else {
        format!("{norm_prefix}/")
    };
    let prefix_lower = prefix_with_slash.to_ascii_lowercase();

    let filter_lower = options.name_filter.trim().to_ascii_lowercase();
    let recursive = options.recursive_in_meg && !filter_lower.is_empty();

    let allowed_exts: Vec<String> = options
        .extensions
        .iter()
        .map(|e| e.trim().trim_start_matches('.').to_ascii_lowercase())
        .filter(|e| !e.is_empty())
        .collect();

    // Keyed by lowercase folder name so case variants inside archives merge cleanly.
    let mut virtual_dirs: BTreeMap<String, (String, String)> = BTreeMap::new();
    let mut files = Vec::new();

    for (raw_name, size) in entries {
        let normalized = normalize_meg_path(raw_name);
        let norm_lower = normalized.to_ascii_lowercase();
        let rel: &str = if prefix_lower.is_empty() {
            normalized.as_str()
        } else if norm_lower.starts_with(&prefix_lower) {
            &normalized[prefix_with_slash.len()..]
        } else {
            continue;
        };

        if rel.is_empty() {
            continue;
        }

        let ext = rel
            .rsplit_once('.')
            .map(|(_, e)| e.to_ascii_lowercase())
            .unwrap_or_default();

        if recursive {
            if !allowed_exts.is_empty() && !allowed_exts.iter().any(|allowed| allowed == &ext) {
                continue;
            }
            if !rel.to_ascii_lowercase().contains(&filter_lower) {
                continue;
            }
            files.push(BrowseEntry {
                name: rel.to_owned(),
                kind: BrowseEntryKind::MegFile,
                size: Some(*size),
                extension: ext,
                target_location: None,
                file_source: Some(FileSource::MegEntry {
                    meg_path: meg_path.to_path_buf(),
                    entry_name: raw_name.clone(),
                }),
                disk_path: None,
            });
            continue;
        }

        if let Some((dir_seg, _rest)) = rel.split_once('/') {
            if dir_seg.is_empty() {
                continue;
            }
            if !filter_lower.is_empty() && !dir_seg.to_ascii_lowercase().contains(&filter_lower) {
                continue;
            }
            let dir_key = dir_seg.to_ascii_lowercase();
            virtual_dirs.entry(dir_key).or_insert_with(|| {
                let next_prefix = if norm_prefix.is_empty() {
                    dir_seg.to_owned()
                } else {
                    format!("{norm_prefix}/{dir_seg}")
                };
                (dir_seg.to_owned(), next_prefix)
            });
        } else {
            if !allowed_exts.is_empty() && !allowed_exts.iter().any(|allowed| allowed == &ext) {
                continue;
            }
            if !filter_lower.is_empty() && !rel.to_ascii_lowercase().contains(&filter_lower) {
                continue;
            }
            files.push(BrowseEntry {
                name: rel.to_owned(),
                kind: BrowseEntryKind::MegFile,
                size: Some(*size),
                extension: ext,
                target_location: None,
                file_source: Some(FileSource::MegEntry {
                    meg_path: meg_path.to_path_buf(),
                    entry_name: raw_name.clone(),
                }),
                disk_path: None,
            });
        }
    }

    let mut dirs: Vec<BrowseEntry> = virtual_dirs
        .into_values()
        .map(|(display_name, next_prefix)| BrowseEntry {
            name: display_name,
            kind: BrowseEntryKind::MegVirtualDir,
            size: None,
            extension: String::new(),
            target_location: Some(BrowseLocation::MegDir {
                meg_path: meg_path.to_path_buf(),
                inner_prefix: next_prefix,
            }),
            file_source: None,
            disk_path: None,
        })
        .collect();

    sort_entries_by_name(&mut dirs);
    sort_entries_by_name(&mut files);

    let mut out = Vec::with_capacity((dirs.len() + files.len()).min(options.max_entries));
    for group in [dirs, files] {
        for item in group {
            if out.len() >= options.max_entries {
                return out;
            }
            out.push(item);
        }
    }
    out
}

/// Navigates one level up from `location`, returning `None` only at the filesystem root.
///
/// Leaving the root of a `.meg` archive returns the disk directory containing that `.meg` file.
pub fn parent_location(location: &BrowseLocation) -> Option<BrowseLocation> {
    match location {
        BrowseLocation::DiskDir(dir) => dir
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(|p| BrowseLocation::DiskDir(p.to_path_buf())),
        BrowseLocation::MegDir {
            meg_path,
            inner_prefix,
        } => {
            let norm = normalize_meg_prefix(inner_prefix);
            if norm.is_empty() {
                let parent_dir = meg_path
                    .parent()
                    .filter(|p| !p.as_os_str().is_empty())
                    .map(Path::to_path_buf)
                    .unwrap_or_else(|| PathBuf::from("."));
                Some(BrowseLocation::DiskDir(parent_dir))
            } else if let Some((parent_prefix, _)) = norm.rsplit_once('/') {
                Some(BrowseLocation::MegDir {
                    meg_path: meg_path.clone(),
                    inner_prefix: parent_prefix.to_owned(),
                })
            } else {
                Some(BrowseLocation::MegDir {
                    meg_path: meg_path.clone(),
                    inner_prefix: String::new(),
                })
            }
        }
    }
}

/// Builds clickable breadcrumb segments representing `location`.
///
/// For `MegDir`, includes the parent disk path segments, the `.meg` archive root, and each
/// nested virtual folder segment inside the archive.
pub fn breadcrumbs(location: &BrowseLocation) -> Vec<BreadcrumbSegment> {
    match location {
        BrowseLocation::DiskDir(dir) => disk_breadcrumbs(dir),
        BrowseLocation::MegDir {
            meg_path,
            inner_prefix,
        } => {
            let mut segs = meg_path
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .map(disk_breadcrumbs)
                .unwrap_or_default();

            let meg_label = meg_path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("archive.meg")
                .to_owned();
            segs.push(BreadcrumbSegment {
                label: meg_label,
                location: BrowseLocation::MegDir {
                    meg_path: meg_path.clone(),
                    inner_prefix: String::new(),
                },
            });

            let norm = normalize_meg_prefix(inner_prefix);
            if !norm.is_empty() {
                let mut acc = String::new();
                for part in norm.split('/') {
                    if part.is_empty() {
                        continue;
                    }
                    if !acc.is_empty() {
                        acc.push('/');
                    }
                    acc.push_str(part);
                    segs.push(BreadcrumbSegment {
                        label: part.to_owned(),
                        location: BrowseLocation::MegDir {
                            meg_path: meg_path.clone(),
                            inner_prefix: acc.clone(),
                        },
                    });
                }
            }
            segs
        }
    }
}

fn disk_breadcrumbs(dir: &Path) -> Vec<BreadcrumbSegment> {
    let mut out = Vec::new();
    let mut acc = PathBuf::new();
    for comp in dir.components() {
        acc.push(comp.as_os_str());
        let label = match comp {
            std::path::Component::RootDir => "/".to_owned(),
            other => other.as_os_str().to_string_lossy().to_string(),
        };
        out.push(BreadcrumbSegment {
            label,
            location: BrowseLocation::DiskDir(acc.clone()),
        });
    }
    if out.is_empty() {
        out.push(BreadcrumbSegment {
            label: ".".to_owned(),
            location: BrowseLocation::DiskDir(PathBuf::from(".")),
        });
    }
    out
}

/// Formats `location` as a compact single-line display string for headers and status messages.
pub fn location_display(location: &BrowseLocation) -> String {
    match location {
        BrowseLocation::DiskDir(dir) => dir.display().to_string(),
        BrowseLocation::MegDir {
            meg_path,
            inner_prefix,
        } => {
            let norm = normalize_meg_prefix(inner_prefix);
            if norm.is_empty() {
                format!("{} :: /", meg_path.display())
            } else {
                format!("{} :: {norm}", meg_path.display())
            }
        }
    }
}

/// Builds deduplicated quick-navigation sidebar places from configured `GameData` directories,
/// the current working directory, `$HOME`, and the filesystem root.
pub fn default_quick_places(game_data_dirs: &[PathBuf]) -> Vec<QuickPlace> {
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();

    let mut push_dir = |label: String, subtitle: Option<String>, path: PathBuf| {
        if !path.is_dir() {
            return;
        }
        let key = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
        if seen.insert(key.clone()) {
            out.push(QuickPlace {
                label,
                subtitle,
                location: BrowseLocation::DiskDir(key),
            });
        }
    };

    for dir in game_data_dirs {
        let short = short_dir_label(dir);
        push_dir(short, Some("GameData".to_owned()), dir.clone());
    }

    if let Ok(cwd) = std::env::current_dir() {
        let sub = cwd
            .file_name()
            .and_then(|s| s.to_str())
            .map(str::to_owned);
        push_dir("Workspace".to_owned(), sub, cwd);
    }

    if let Some(home) = std::env::var_os("HOME") {
        let home_path = PathBuf::from(home);
        push_dir("Home".to_owned(), None, home_path);
    }

    push_dir("Filesystem /".to_owned(), None, PathBuf::from("/"));

    out
}

/// Returns a sensible initial [`BrowseLocation`] given configured `GameData` directories and an
/// optional currently open [`FileSource`].
pub fn initial_browse_location(
    game_data_dirs: &[PathBuf],
    active_source: Option<&FileSource>,
) -> BrowseLocation {
    if let Some(source) = active_source {
        match source {
            FileSource::Disk { path } => {
                if let Some(parent) = path.parent()
                    && parent.is_dir()
                {
                    return BrowseLocation::DiskDir(parent.to_path_buf());
                }
            }
            FileSource::MegEntry {
                meg_path,
                entry_name,
            } => {
                if meg_path.is_file() {
                    let norm = normalize_meg_path(entry_name);
                    let inner_prefix = norm
                        .rsplit_once('/')
                        .map(|(dir, _)| dir.to_owned())
                        .unwrap_or_default();
                    return BrowseLocation::MegDir {
                        meg_path: meg_path.clone(),
                        inner_prefix,
                    };
                }
            }
        }
    }

    if let Some(first) = game_data_dirs.iter().find(|d| d.is_dir()) {
        return BrowseLocation::DiskDir(
            std::fs::canonicalize(first).unwrap_or_else(|_| first.clone()),
        );
    }

    if let Ok(cwd) = std::env::current_dir() {
        return BrowseLocation::DiskDir(cwd);
    }

    BrowseLocation::DiskDir(PathBuf::from("/"))
}

/// Formats a byte count compactly (`"512 B"`, `"14.2 KB"`, `"8.4 MB"`).
pub fn format_size_compact(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

fn short_dir_label(path: &Path) -> String {
    let mut parts: Vec<&str> = path
        .iter()
        .filter_map(|s| s.to_str())
        .filter(|s| *s != "/")
        .collect();
    if parts.len() >= 2 {
        let tail = parts.split_off(parts.len() - 2);
        format!("{}/{}", tail[0], tail[1])
    } else if let Some(last) = parts.last() {
        (*last).to_owned()
    } else {
        path.display().to_string()
    }
}

fn normalize_meg_path(raw: &str) -> String {
    raw.replace('\\', "/").trim_matches('/').to_owned()
}

fn normalize_meg_prefix(prefix: &str) -> String {
    normalize_meg_path(prefix)
}

fn sort_entries_by_name(entries: &mut [BrowseEntry]) {
    entries.sort_by(|a, b| {
        a.name
            .to_ascii_lowercase()
            .cmp(&b.name.to_ascii_lowercase())
            .then_with(|| a.name.cmp(&b.name))
    });
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
                "petro_editor_browser_{prefix}_{}_{id}",
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

    #[test]
    fn test_disk_listing_sorting_and_filtering() {
        let tmp = TempDirGuard::new("disk");
        std::fs::create_dir(tmp.path().join("SubFolder")).unwrap();
        std::fs::create_dir(tmp.path().join(".hidden_dir")).unwrap();
        std::fs::write(tmp.path().join("models.meg"), b"dummy").unwrap();
        std::fs::write(tmp.path().join("tie.alo"), b"1234").unwrap();
        std::fs::write(tmp.path().join("notes.txt"), b"hello").unwrap();

        let loc = BrowseLocation::DiskDir(tmp.path().clone());

        // Default options with `.alo` filter: SubFolder (DIR), models.meg (MEG), tie.alo (FILE).
        let entries = list_location(
            &loc,
            &BrowseOptions {
                extensions: vec!["alo".to_owned()],
                ..Default::default()
            },
        )
        .unwrap();
        let summary: Vec<(&str, BrowseEntryKind)> =
            entries.iter().map(|e| (e.name.as_str(), e.kind)).collect();
        assert_eq!(
            summary,
            vec![
                ("SubFolder", BrowseEntryKind::Directory),
                ("models.meg", BrowseEntryKind::MegArchive),
                ("tie.alo", BrowseEntryKind::DiskFile),
            ]
        );

        // `dirs_only` returns only visible directories.
        let dirs_only = list_location(
            &loc,
            &BrowseOptions {
                dirs_only: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(dirs_only.len(), 1);
        assert_eq!(dirs_only[0].name, "SubFolder");
    }

    #[test]
    fn test_meg_virtual_directory_navigation_and_recursive_search() {
        let meg_path = PathBuf::from("/games/Data/models.meg");
        let raw_entries = vec![
            (r"DATA\ART\MODELS\EV_TIEFIGHTER.ALO".to_owned(), 120_000),
            (r"DATA\ART\MODELS\EV_XWING.ALO".to_owned(), 98_000),
            (r"DATA\ART\MODELS\ANIM\IDLE.ALA".to_owned(), 4_500),
            (r"DATA\ART\TEXTURES\TIE.DDS".to_owned(), 65_000),
        ];

        // Archive root lists single virtual dir `DATA`.
        let root_items =
            list_meg_entries_from_names(&meg_path, &raw_entries, "", &BrowseOptions::default());
        assert_eq!(root_items.len(), 1);
        assert_eq!(root_items[0].name, "DATA");
        assert_eq!(root_items[0].kind, BrowseEntryKind::MegVirtualDir);
        assert_eq!(
            root_items[0].target_location,
            Some(BrowseLocation::MegDir {
                meg_path: meg_path.clone(),
                inner_prefix: "DATA".to_owned(),
            })
        );

        // Inside DATA/ART/MODELS with `.alo` filter: ANIM folder + two .ALO files.
        let model_opts = BrowseOptions {
            extensions: vec!["alo".to_owned()],
            ..Default::default()
        };
        let models_items = list_meg_entries_from_names(
            &meg_path,
            &raw_entries,
            "DATA/ART/MODELS",
            &model_opts,
        );
        let names: Vec<(&str, BrowseEntryKind)> = models_items
            .iter()
            .map(|e| (e.name.as_str(), e.kind))
            .collect();
        assert_eq!(
            names,
            vec![
                ("ANIM", BrowseEntryKind::MegVirtualDir),
                ("EV_TIEFIGHTER.ALO", BrowseEntryKind::MegFile),
                ("EV_XWING.ALO", BrowseEntryKind::MegFile),
            ]
        );
        // Preserves original backslash archive entry name for extraction.
        assert_eq!(
            models_items[1].file_source,
            Some(FileSource::MegEntry {
                meg_path: meg_path.clone(),
                entry_name: r"DATA\ART\MODELS\EV_TIEFIGHTER.ALO".to_owned(),
            })
        );

        // Recursive search from archive root finds `EV_TIEFIGHTER.ALO` directly.
        let search_opts = BrowseOptions {
            extensions: vec!["alo".to_owned()],
            name_filter: "tiefighter".to_owned(),
            recursive_in_meg: true,
            ..Default::default()
        };
        let search_hits = list_meg_entries_from_names(&meg_path, &raw_entries, "", &search_opts);
        assert_eq!(search_hits.len(), 1);
        assert_eq!(search_hits[0].name, "DATA/ART/MODELS/EV_TIEFIGHTER.ALO");
        assert_eq!(search_hits[0].kind, BrowseEntryKind::MegFile);
    }

    #[test]
    fn test_parent_location_and_breadcrumbs() {
        let meg = PathBuf::from("/games/Data/models.meg");
        let deep = BrowseLocation::MegDir {
            meg_path: meg.clone(),
            inner_prefix: "DATA/ART/MODELS".to_owned(),
        };

        let crumbs = breadcrumbs(&deep);
        let labels: Vec<&str> = crumbs.iter().map(|c| c.label.as_str()).collect();
        assert_eq!(
            labels,
            vec!["/", "games", "Data", "models.meg", "DATA", "ART", "MODELS"]
        );

        let up1 = parent_location(&deep).unwrap();
        assert_eq!(
            up1,
            BrowseLocation::MegDir {
                meg_path: meg.clone(),
                inner_prefix: "DATA/ART".to_owned(),
            }
        );

        let meg_root = BrowseLocation::MegDir {
            meg_path: meg,
            inner_prefix: String::new(),
        };
        assert_eq!(
            parent_location(&meg_root),
            Some(BrowseLocation::DiskDir(PathBuf::from("/games/Data")))
        );
    }
}
