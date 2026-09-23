//! Unified Virtual File System (VFS) abstraction.
//!
//! Provides a single async interface for file operations across both
//! remote mode (WASI filesystem / Spin preopens) and local mode
//! (browser File System Access API), plus in-memory testing.

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, RwLock};

use serde::{Deserialize, Serialize};

use crate::{FileEntry, SearchHit, find_content_matches};

/// Errors encountered during Virtual File System operations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum VfsError {
    NotFound(String),
    PermissionDenied(String),
    AlreadyExists(String),
    PathEscape(String),
    Io(String),
}

impl std::fmt::Display for VfsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(p) => write!(f, "path not found: {p}"),
            Self::PermissionDenied(p) => write!(f, "permission denied: {p}"),
            Self::AlreadyExists(p) => write!(f, "already exists: {p}"),
            Self::PathEscape(p) => write!(f, "path escapes workspace root: {p}"),
            Self::Io(msg) => write!(f, "I/O error: {msg}"),
        }
    }
}

impl std::error::Error for VfsError {}

/// A pinned, heap-allocated, Send-safe future returning a Vfs result.
pub type VfsFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, VfsError>> + Send + 'a>>;

/// Normalize a raw user or model-supplied path into a canonical workspace-relative POSIX path.
///
/// Rules:
/// 1. Replaces Windows backslashes with `/`.
/// 2. Resolves current-dir `.` and parent-dir `..` segments.
/// 3. Strips leading slashes (e.g. `/src/main.rs` -> `src/main.rs`).
/// 4. Rejects any path where `..` escapes above the workspace root.
/// 5. Root is represented as empty string `""`.
pub fn normalize_vfs_path(raw: &str) -> Result<String, VfsError> {
    let clean = raw.replace('\\', "/");
    let mut parts: Vec<&str> = Vec::new();

    for seg in clean.split('/') {
        match seg {
            "" | "." => continue,
            ".." => {
                if parts.pop().is_none() {
                    return Err(VfsError::PathEscape(raw.to_string()));
                }
            }
            s => parts.push(s),
        }
    }

    Ok(parts.join("/"))
}

/// Format a Unix timestamp (seconds since epoch) into a human-readable UTC string.
///
/// Example: `Tuesday, September 22, 2026 12:42 UTC`
pub fn format_utc_timestamp(secs: i64) -> String {
    let days = secs.div_euclid(86400);
    let rem_secs = secs.rem_euclid(86400) as u32;
    let hour = rem_secs / 3600;
    let minute = (rem_secs % 3600) / 60;

    // Howard Hinnant's civil day calculation
    // Shift epoch from 1970-01-01 to 0000-03-01
    let z = days + 719468;
    let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };

    let weekday = match (days + 4).rem_euclid(7) {
        0 => "Sunday",
        1 => "Monday",
        2 => "Tuesday",
        3 => "Wednesday",
        4 => "Thursday",
        5 => "Friday",
        6 => "Saturday",
        _ => "Unknown",
    };

    let month_name = match m {
        1 => "January",
        2 => "February",
        3 => "March",
        4 => "April",
        5 => "May",
        6 => "June",
        7 => "July",
        8 => "August",
        9 => "September",
        10 => "October",
        11 => "November",
        12 => "December",
        _ => "Unknown",
    };

    format!("{weekday}, {month_name} {d}, {y} {hour:02}:{minute:02} UTC")
}

/// The core asynchronous Virtual File System trait.
pub trait Vfs: Send + Sync {
    /// Read the full UTF-8 contents of a workspace file.
    fn read<'a>(&'a self, path: &'a str) -> VfsFuture<'a, String>;

    /// Write or overwrite a file with the given full contents.
    fn write<'a>(&'a self, path: &'a str, content: &'a str) -> VfsFuture<'a, ()>;

    /// List entries in a directory relative to workspace root (`""` for root).
    fn list<'a>(&'a self, dir: &'a str) -> VfsFuture<'a, Vec<FileEntry>>;

    /// Create an empty file or directory.
    fn create<'a>(&'a self, path: &'a str, is_dir: bool) -> VfsFuture<'a, ()>;

    /// Delete a file or recursively delete a directory.
    fn delete<'a>(&'a self, path: &'a str) -> VfsFuture<'a, ()>;

    /// Full-text search across file contents under the given directory (`""` for root).
    fn search_content<'a>(&'a self, query: &'a str, dir: &'a str) -> VfsFuture<'a, Vec<SearchHit>>;
}

impl<V: Vfs + ?Sized> Vfs for &V {
    fn read<'a>(&'a self, path: &'a str) -> VfsFuture<'a, String> {
        (**self).read(path)
    }

    fn write<'a>(&'a self, path: &'a str, content: &'a str) -> VfsFuture<'a, ()> {
        (**self).write(path, content)
    }

    fn list<'a>(&'a self, dir: &'a str) -> VfsFuture<'a, Vec<FileEntry>> {
        (**self).list(dir)
    }

    fn create<'a>(&'a self, path: &'a str, is_dir: bool) -> VfsFuture<'a, ()> {
        (**self).create(path, is_dir)
    }

    fn delete<'a>(&'a self, path: &'a str) -> VfsFuture<'a, ()> {
        (**self).delete(path)
    }

    fn search_content<'a>(&'a self, query: &'a str, dir: &'a str) -> VfsFuture<'a, Vec<SearchHit>> {
        (**self).search_content(query, dir)
    }
}

impl<V: Vfs + ?Sized> Vfs for Arc<V> {
    fn read<'a>(&'a self, path: &'a str) -> VfsFuture<'a, String> {
        (**self).read(path)
    }

    fn write<'a>(&'a self, path: &'a str, content: &'a str) -> VfsFuture<'a, ()> {
        (**self).write(path, content)
    }

    fn list<'a>(&'a self, dir: &'a str) -> VfsFuture<'a, Vec<FileEntry>> {
        (**self).list(dir)
    }

    fn create<'a>(&'a self, path: &'a str, is_dir: bool) -> VfsFuture<'a, ()> {
        (**self).create(path, is_dir)
    }

    fn delete<'a>(&'a self, path: &'a str) -> VfsFuture<'a, ()> {
        (**self).delete(path)
    }

    fn search_content<'a>(&'a self, query: &'a str, dir: &'a str) -> VfsFuture<'a, Vec<SearchHit>> {
        (**self).search_content(query, dir)
    }
}

/// An in-memory Vfs implementation for unit tests and local staging.
#[derive(Debug, Clone, Default)]
pub struct MemoryVfs {
    files: Arc<RwLock<BTreeMap<String, String>>>,
    dirs: Arc<RwLock<BTreeSet<String>>>,
}

impl MemoryVfs {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Vfs for MemoryVfs {
    fn read<'a>(&'a self, path: &'a str) -> VfsFuture<'a, String> {
        Box::pin(async move {
            let norm = normalize_vfs_path(path)?;
            let files = self.files.read().map_err(|e| VfsError::Io(e.to_string()))?;
            files.get(&norm).cloned().ok_or(VfsError::NotFound(norm))
        })
    }

    fn write<'a>(&'a self, path: &'a str, content: &'a str) -> VfsFuture<'a, ()> {
        Box::pin(async move {
            let norm = normalize_vfs_path(path)?;
            if norm.is_empty() {
                return Err(VfsError::Io("cannot write to root".into()));
            }

            // Ensure parent directory components exist in dirs set
            if let Some(parent) = norm.rsplit_once('/').map(|(p, _)| p) {
                let mut dirs = self.dirs.write().map_err(|e| VfsError::Io(e.to_string()))?;
                let mut current = String::new();
                for part in parent.split('/') {
                    if !current.is_empty() {
                        current.push('/');
                    }
                    current.push_str(part);
                    dirs.insert(current.clone());
                }
            }

            let mut files = self
                .files
                .write()
                .map_err(|e| VfsError::Io(e.to_string()))?;
            files.insert(norm, content.to_string());
            Ok(())
        })
    }

    fn list<'a>(&'a self, dir: &'a str) -> VfsFuture<'a, Vec<FileEntry>> {
        Box::pin(async move {
            let norm = normalize_vfs_path(dir)?;
            let files = self.files.read().map_err(|e| VfsError::Io(e.to_string()))?;
            let dirs = self.dirs.read().map_err(|e| VfsError::Io(e.to_string()))?;

            let prefix = if norm.is_empty() {
                String::new()
            } else {
                format!("{norm}/")
            };

            let mut seen_children = BTreeSet::new();
            let mut entries = Vec::new();

            // Collect immediate subdirectories
            for d in dirs.iter() {
                if let Some(sub) = d.strip_prefix(&prefix)
                    && !sub.is_empty()
                {
                    let direct_child = sub.split('/').next().unwrap_or(sub);
                    if seen_children.insert((direct_child.to_string(), true)) {
                        let child_path = if norm.is_empty() {
                            direct_child.to_string()
                        } else {
                            format!("{norm}/{direct_child}")
                        };
                        entries.push(FileEntry {
                            name: direct_child.to_string(),
                            path: child_path,
                            is_dir: true,
                            size: 0,
                        });
                    }
                }
            }

            // Collect immediate files
            for (f, content) in files.iter() {
                if let Some(sub) = f.strip_prefix(&prefix)
                    && !sub.is_empty()
                {
                    if !sub.contains('/') {
                        if seen_children.insert((sub.to_string(), false)) {
                            entries.push(FileEntry {
                                name: sub.to_string(),
                                path: f.clone(),
                                is_dir: false,
                                size: content.len() as u64,
                            });
                        }
                    } else {
                        // Subdirectory implied by file path
                        let direct_child = sub.split('/').next().unwrap_or(sub);
                        if seen_children.insert((direct_child.to_string(), true)) {
                            let child_path = if norm.is_empty() {
                                direct_child.to_string()
                            } else {
                                format!("{norm}/{direct_child}")
                            };
                            entries.push(FileEntry {
                                name: direct_child.to_string(),
                                path: child_path,
                                is_dir: true,
                                size: 0,
                            });
                        }
                    }
                }
            }

            entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => a.name.cmp(&b.name),
            });

            Ok(entries)
        })
    }

    fn create<'a>(&'a self, path: &'a str, is_dir: bool) -> VfsFuture<'a, ()> {
        Box::pin(async move {
            let norm = normalize_vfs_path(path)?;
            if norm.is_empty() {
                return Ok(());
            }

            if is_dir {
                let mut dirs = self.dirs.write().map_err(|e| VfsError::Io(e.to_string()))?;
                dirs.insert(norm);
            } else {
                let mut files = self
                    .files
                    .write()
                    .map_err(|e| VfsError::Io(e.to_string()))?;
                files.entry(norm).or_default();
            }
            Ok(())
        })
    }

    fn delete<'a>(&'a self, path: &'a str) -> VfsFuture<'a, ()> {
        Box::pin(async move {
            let norm = normalize_vfs_path(path)?;
            if norm.is_empty() {
                return Err(VfsError::Io("cannot delete workspace root".into()));
            }

            let mut files = self
                .files
                .write()
                .map_err(|e| VfsError::Io(e.to_string()))?;
            let mut dirs = self.dirs.write().map_err(|e| VfsError::Io(e.to_string()))?;

            let prefix = format!("{norm}/");
            files.retain(|k, _| k != &norm && !k.starts_with(&prefix));
            dirs.retain(|k| k != &norm && !k.starts_with(&prefix));
            Ok(())
        })
    }

    fn search_content<'a>(&'a self, query: &'a str, dir: &'a str) -> VfsFuture<'a, Vec<SearchHit>> {
        Box::pin(async move {
            let norm = normalize_vfs_path(dir)?;
            let files = self.files.read().map_err(|e| VfsError::Io(e.to_string()))?;

            let prefix = if norm.is_empty() {
                String::new()
            } else {
                format!("{norm}/")
            };

            let mut hits = Vec::new();
            for (path, content) in files.iter() {
                if norm.is_empty() || path.starts_with(&prefix) {
                    for (line, text) in find_content_matches(content, query) {
                        hits.push(SearchHit {
                            path: path.clone(),
                            line,
                            text,
                        });
                    }
                }
            }

            Ok(hits)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_vfs_path_basic() {
        assert_eq!(normalize_vfs_path("src/main.rs").unwrap(), "src/main.rs");
        assert_eq!(normalize_vfs_path("./src/main.rs").unwrap(), "src/main.rs");
        assert_eq!(normalize_vfs_path("/src/main.rs").unwrap(), "src/main.rs");
        assert_eq!(normalize_vfs_path("src//main.rs").unwrap(), "src/main.rs");
        assert_eq!(normalize_vfs_path("src/./main.rs").unwrap(), "src/main.rs");
        assert_eq!(
            normalize_vfs_path("src/utils/../main.rs").unwrap(),
            "src/main.rs"
        );
        assert_eq!(
            normalize_vfs_path("crates\\core\\src\\lib.rs").unwrap(),
            "crates/core/src/lib.rs"
        );
    }

    #[test]
    fn test_normalize_vfs_path_root() {
        assert_eq!(normalize_vfs_path("").unwrap(), "");
        assert_eq!(normalize_vfs_path(".").unwrap(), "");
        assert_eq!(normalize_vfs_path("./").unwrap(), "");
        assert_eq!(normalize_vfs_path("/").unwrap(), "");
        assert_eq!(normalize_vfs_path("///").unwrap(), "");
    }

    #[test]
    fn test_normalize_vfs_path_escape_rejected() {
        assert!(matches!(
            normalize_vfs_path(".."),
            Err(VfsError::PathEscape(_))
        ));
        assert!(matches!(
            normalize_vfs_path("../foo"),
            Err(VfsError::PathEscape(_))
        ));
        assert!(matches!(
            normalize_vfs_path("foo/../../bar"),
            Err(VfsError::PathEscape(_))
        ));
        assert!(matches!(
            normalize_vfs_path("/.."),
            Err(VfsError::PathEscape(_))
        ));
    }

    #[test]
    fn test_memory_vfs_crud() {
        futures::executor::block_on(async {
            let vfs = MemoryVfs::new();

            // Write and read
            vfs.write("src/lib.rs", "pub fn hello() {}").await.unwrap();
            let content = vfs.read("src/lib.rs").await.unwrap();
            assert_eq!(content, "pub fn hello() {}");

            // Read non-existent
            assert!(matches!(
                vfs.read("src/missing.rs").await,
                Err(VfsError::NotFound(_))
            ));

            // Listing root
            vfs.write("README.md", "# Project").await.unwrap();
            let entries = vfs.list("").await.unwrap();
            assert_eq!(entries.len(), 2);
            assert_eq!(entries[0].name, "src");
            assert!(entries[0].is_dir);
            assert_eq!(entries[1].name, "README.md");
            assert!(!entries[1].is_dir);

            // Listing subdirectory
            let src_entries = vfs.list("src").await.unwrap();
            assert_eq!(src_entries.len(), 1);
            assert_eq!(src_entries[0].name, "lib.rs");
            assert!(!src_entries[0].is_dir);

            // Search
            let hits = vfs.search_content("hello", "").await.unwrap();
            assert_eq!(hits.len(), 1);
            assert_eq!(hits[0].path, "src/lib.rs");
            assert_eq!(hits[0].line, 1);

            // Delete
            vfs.delete("src").await.unwrap();
            assert!(matches!(
                vfs.read("src/lib.rs").await,
                Err(VfsError::NotFound(_))
            ));
            let entries_after = vfs.list("").await.unwrap();
            assert_eq!(entries_after.len(), 1);
            assert_eq!(entries_after[0].name, "README.md");
        });
    }

    #[test]
    fn test_format_utc_timestamp() {
        assert_eq!(
            format_utc_timestamp(0),
            "Thursday, January 1, 1970 00:00 UTC"
        );
        // 2026-09-22 18:14:05 UTC -> 1790100845
        assert_eq!(
            format_utc_timestamp(1790100845),
            "Tuesday, September 22, 2026 18:14 UTC"
        );
    }
}
