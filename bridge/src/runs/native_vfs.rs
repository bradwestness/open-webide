use std::path::PathBuf;

use openwebide_core::vfs::{SearchOptions, VfsFuture, skip_dir};
use openwebide_core::{FileEntry, SearchHit, Vfs, VfsError};
use tokio::fs;
use tokio::io::AsyncReadExt;

use crate::paths::resolve_file_in_root;

#[derive(Clone, Debug)]
pub struct NativeFsVfs {
    pub root: PathBuf,
}

fn io_error(error: std::io::Error, path: &str) -> VfsError {
    match error.kind() {
        std::io::ErrorKind::NotFound => VfsError::NotFound(path.into()),
        std::io::ErrorKind::AlreadyExists => VfsError::AlreadyExists(path.into()),
        std::io::ErrorKind::PermissionDenied => VfsError::PermissionDenied(path.into()),
        _ => VfsError::Io(error.to_string()),
    }
}

impl NativeFsVfs {
    fn resolve(&self, rel: &str) -> Result<PathBuf, VfsError> {
        resolve_file_in_root(&self.root, rel).map_err(VfsError::PathEscape)
    }
    fn directory(&self, rel: &str) -> Result<PathBuf, VfsError> {
        if rel.is_empty() {
            Ok(self.root.clone())
        } else {
            self.resolve(rel)
        }
    }
    async fn parents(&self, path: &std::path::Path) -> Result<(), VfsError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .await
                .map_err(|e| io_error(e, &parent.to_string_lossy()))?;
        }
        Ok(())
    }
}

impl Vfs for NativeFsVfs {
    fn canonicalize<'a>(&'a self, path: &'a str) -> VfsFuture<'a, String> {
        Box::pin(async move {
            let root = fs::canonicalize(&self.root)
                .await
                .map_err(|e| io_error(e, path))?;
            let mut full = self.resolve(path)?;
            let mut missing = Vec::new();
            let mut symlinks = 0;
            let mut resolved = loop {
                match fs::canonicalize(&full).await {
                    Ok(resolved) => break resolved,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        if fs::symlink_metadata(&full)
                            .await
                            .is_ok_and(|meta| meta.file_type().is_symlink())
                        {
                            symlinks += 1;
                            if symlinks > 32 {
                                return Err(VfsError::Io("too many symbolic links".into()));
                            }
                            let target =
                                fs::read_link(&full).await.map_err(|e| io_error(e, path))?;
                            full.pop();
                            full.push(target);
                            continue;
                        }
                        let name = full.file_name().ok_or_else(|| io_error(error, path))?;
                        missing.push(name.to_os_string());
                        full.pop();
                    }
                    Err(error) => return Err(io_error(error, path)),
                }
            };
            for name in missing.into_iter().rev() {
                resolved.push(name);
            }
            let rel = resolved
                .strip_prefix(root)
                .map_err(|_| VfsError::PathEscape(path.into()))?;
            if rel.components().any(|part| part.as_os_str() == ".spin") {
                return Err(VfsError::PathEscape(path.into()));
            }
            Ok(rel.to_string_lossy().replace('\\', "/"))
        })
    }
    fn read<'a>(&'a self, path: &'a str) -> VfsFuture<'a, String> {
        Box::pin(async move {
            const CAP: u64 = 10 * 1024 * 1024;
            let file = fs::File::open(self.resolve(path)?)
                .await
                .map_err(|e| io_error(e, path))?;
            let mut bytes = Vec::new();
            file.take(CAP + 1)
                .read_to_end(&mut bytes)
                .await
                .map_err(|e| io_error(e, path))?;
            if bytes.len() as u64 > CAP {
                return Err(VfsError::Io("file exceeds 10 MiB".into()));
            }
            String::from_utf8(bytes).map_err(|_| VfsError::Io("file is not valid UTF-8".into()))
        })
    }
    fn write<'a>(&'a self, path: &'a str, content: &'a str) -> VfsFuture<'a, ()> {
        Box::pin(async move {
            let full = self.resolve(path)?;
            self.parents(&full).await?;
            fs::write(full, content)
                .await
                .map_err(|e| io_error(e, path))
        })
    }
    fn list<'a>(&'a self, dir: &'a str) -> VfsFuture<'a, Vec<FileEntry>> {
        Box::pin(async move {
            let mut reader = fs::read_dir(self.directory(dir)?)
                .await
                .map_err(|e| io_error(e, dir))?;
            let mut entries = Vec::new();
            while let Some(entry) = reader.next_entry().await.map_err(|e| io_error(e, dir))? {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name == ".spin" {
                    continue;
                }
                let path = if dir.is_empty() {
                    name.clone()
                } else {
                    format!("{}/{name}", dir.trim_end_matches('/'))
                };
                let metadata = entry.metadata().await.map_err(|e| io_error(e, &path))?;
                entries.push(FileEntry {
                    name,
                    path,
                    is_dir: metadata.is_dir(),
                    size: if metadata.is_dir() { 0 } else { metadata.len() },
                });
            }
            Ok(entries)
        })
    }
    fn create<'a>(&'a self, path: &'a str, is_dir: bool) -> VfsFuture<'a, ()> {
        Box::pin(async move {
            let full = self.resolve(path)?;
            self.parents(&full).await?;
            if is_dir {
                fs::create_dir_all(full)
                    .await
                    .map_err(|e| io_error(e, path))
            } else {
                fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(full)
                    .await
                    .map(|_| ())
                    .map_err(|e| io_error(e, path))
            }
        })
    }
    fn delete<'a>(&'a self, path: &'a str) -> VfsFuture<'a, ()> {
        Box::pin(async move {
            let full = self.resolve(path)?;
            let metadata = fs::symlink_metadata(&full)
                .await
                .map_err(|e| io_error(e, path))?;
            if metadata.is_dir() {
                fs::remove_dir_all(full)
                    .await
                    .map_err(|e| io_error(e, path))
            } else {
                fs::remove_file(full).await.map_err(|e| io_error(e, path))
            }
        })
    }
    fn copy<'a>(&'a self, from: &'a str, to: &'a str) -> VfsFuture<'a, ()> {
        Box::pin(async move {
            let from = self.resolve(from)?;
            let to = self.resolve(to)?;
            self.parents(&to).await?;
            fs::copy(from, to)
                .await
                .map(|_| ())
                .map_err(|e| io_error(e, "copy"))
        })
    }
    fn search_content<'a>(
        &'a self,
        query: &'a str,
        dir: &'a str,
        opts: SearchOptions,
    ) -> VfsFuture<'a, Vec<SearchHit>> {
        Box::pin(async move {
            self.directory(dir)?;
            let mut pending = vec![dir.to_string()];
            let mut out = Vec::new();
            let mut entries_left = 20_000;
            let mut bytes_left = 64 * 1024 * 1024;
            let query = query.to_lowercase();
            'walk: while let Some(dir) = pending.pop() {
                let Ok(mut reader) = fs::read_dir(self.directory(&dir)?).await else {
                    continue;
                };
                while let Ok(Some(entry)) = reader.next_entry().await {
                    if entries_left == 0 || bytes_left == 0 || out.len() == 500 {
                        break 'walk;
                    }
                    entries_left -= 1;
                    let name = entry.file_name().to_string_lossy().into_owned();
                    if name == ".spin" {
                        continue;
                    }
                    let path = if dir.is_empty() {
                        name.clone()
                    } else {
                        format!("{}/{name}", dir.trim_end_matches('/'))
                    };
                    let Ok(metadata) = entry.metadata().await else {
                        continue;
                    };
                    if metadata.is_dir() {
                        if !skip_dir(&name, opts) {
                            pending.push(path);
                        }
                        continue;
                    }
                    if metadata.len() > 10 * 1024 * 1024 {
                        continue;
                    }
                    if metadata.len() > bytes_left {
                        continue;
                    }
                    let Ok(file) = fs::File::open(self.resolve(&path)?).await else {
                        continue;
                    };
                    let mut bytes = Vec::new();
                    let read = file
                        .take(bytes_left.min(10 * 1024 * 1024 + 1))
                        .read_to_end(&mut bytes)
                        .await;
                    bytes_left -= bytes.len() as u64;
                    if read.is_err() || bytes.len() > 10 * 1024 * 1024 {
                        continue;
                    }
                    let Ok(text) = String::from_utf8(bytes) else {
                        continue;
                    };
                    for (line, text) in text.lines().enumerate() {
                        if text.to_lowercase().contains(&query) {
                            out.push(SearchHit {
                                path: path.clone(),
                                line: line + 1,
                                text: text.chars().take(400).collect(),
                            });
                            if out.len() == 500 {
                                break 'walk;
                            }
                        }
                    }
                }
            }
            Ok(out)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[tokio::test]
    async fn git_alias_writes_are_rejected_for_existing_and_new_files() {
        use openwebide_agent::{ToolExecutor, VfsToolExecutor};
        use openwebide_core::ToolCall;

        let dir = tempfile::tempdir().unwrap();
        let vfs = NativeFsVfs {
            root: dir.path().canonicalize().unwrap(),
        };
        vfs.write(".git/hooks/pre-commit", "original")
            .await
            .unwrap();
        std::os::unix::fs::symlink(".git/hooks", dir.path().join("hooks")).unwrap();
        let executor = VfsToolExecutor::new(vfs.clone());
        for path in ["hooks/pre-commit", "hooks/new/sub/hook"] {
            assert_eq!(
                vfs.canonicalize(path).await.unwrap(),
                format!(".git/{path}")
            );
            let outcome = executor
                .execute(&ToolCall {
                    id: "edit".into(),
                    name: "write_file".into(),
                    arguments: serde_json::json!({"path": path, "content": "unwanted"}).to_string(),
                })
                .await;
            assert!(!outcome.ok);
            assert!(outcome.content.contains("refusing to write inside .git"));
        }
        assert_eq!(vfs.read(".git/hooks/pre-commit").await.unwrap(), "original");
        assert!(!dir.path().join(".git/hooks/new").exists());
        assert_eq!(
            vfs.canonicalize("ordinary/new/file").await.unwrap(),
            "ordinary/new/file"
        );
        std::os::unix::fs::symlink(".git/new", dir.path().join("new-hooks")).unwrap();
        assert_eq!(
            vfs.canonicalize("new-hooks/sub/hook").await.unwrap(),
            ".git/new/sub/hook"
        );
        std::os::unix::fs::symlink("ordinary/new", dir.path().join("new-files")).unwrap();
        assert_eq!(
            vfs.canonicalize("new-files/file").await.unwrap(),
            "ordinary/new/file"
        );
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), dir.path().join("outside")).unwrap();
        assert!(matches!(
            vfs.canonicalize("outside/new").await,
            Err(VfsError::PathEscape(_))
        ));
    }

    #[tokio::test]
    async fn search_caps_hits_and_retained_line_characters() {
        let dir = tempfile::tempdir().unwrap();
        let vfs = NativeFsVfs {
            root: dir.path().to_path_buf(),
        };
        let line = format!("needle{}\n", "é".repeat(500));
        vfs.write("matches", &line.repeat(501)).await.unwrap();
        let hits = vfs
            .search_content("NEEDLE", "", SearchOptions::default())
            .await
            .unwrap();
        assert_eq!(hits.len(), 500);
        assert_eq!(hits.last().unwrap().line, 500);
        assert!(hits.iter().all(|hit| hit.text.chars().count() == 400));
    }

    #[tokio::test]
    async fn search_stops_at_aggregate_read_budget() {
        let dir = tempfile::tempdir().unwrap();
        let vfs = NativeFsVfs {
            root: dir.path().to_path_buf(),
        };
        let mut content = "x\n".repeat(5 * 1024 * 1024 - 4);
        content.push_str("needle!\n");
        assert_eq!(content.len(), 10 * 1024 * 1024);
        for index in 0..7 {
            vfs.write(&format!("file{index}"), &content).await.unwrap();
        }
        let hits = vfs
            .search_content("needle", "", SearchOptions::default())
            .await
            .unwrap();
        assert_eq!(hits.len(), 6);
    }

    #[tokio::test]
    async fn search_skips_files_exceeding_remaining_budget_and_visits_subdirectories() {
        let dir = tempfile::tempdir().unwrap();
        let vfs = NativeFsVfs {
            root: dir.path().to_path_buf(),
        };
        let content = "x\n".repeat(5 * 1024 * 1024);
        for index in 0..7 {
            vfs.write(&format!("file{index}"), &content).await.unwrap();
        }
        vfs.write("src/small.txt", "needle").await.unwrap();
        let hits = vfs
            .search_content("needle", "", SearchOptions::default())
            .await
            .unwrap();
        assert_eq!(
            hits,
            vec![SearchHit {
                path: "src/small.txt".into(),
                line: 1,
                text: "needle".into(),
            }]
        );
    }

    #[tokio::test]
    async fn crud_copy_search_and_confinement() {
        let dir = tempfile::tempdir().unwrap();
        let vfs = NativeFsVfs {
            root: dir.path().canonicalize().unwrap(),
        };
        vfs.create("folder", true).await.unwrap();
        vfs.create("folder/a", false).await.unwrap();
        vfs.write("folder/a", "Needle\nsecond needle")
            .await
            .unwrap();
        assert!(matches!(
            vfs.create("folder/a", false).await,
            Err(VfsError::AlreadyExists(_))
        ));
        assert_eq!(vfs.read("folder/a").await.unwrap(), "Needle\nsecond needle");
        assert_eq!(vfs.list("").await.unwrap()[0].name, "folder");
        vfs.copy("folder/a", "backup/new/a").await.unwrap();
        assert_eq!(
            vfs.read("backup/new/a").await.unwrap(),
            vfs.read("folder/a").await.unwrap()
        );
        vfs.write("node_modules/a", "needle").await.unwrap();
        let hits = vfs
            .search_content("needle", "", SearchOptions::default())
            .await
            .unwrap();
        assert_eq!(hits.len(), 4);
        assert_eq!(
            vfs.search_content(
                "needle",
                "",
                SearchOptions {
                    include_ignored: true
                }
            )
            .await
            .unwrap()
            .len(),
            5
        );
        for path in ["../outside", ".spin/db", "a/.spin/db", "/absolute"] {
            assert!(vfs.read(path).await.is_err());
            assert!(vfs.write(path, "x").await.is_err());
            assert!(vfs.create(path, false).await.is_err());
            assert!(vfs.delete(path).await.is_err());
            assert!(vfs.copy("folder/a", path).await.is_err());
        }
        assert!(matches!(
            vfs.read("missing").await,
            Err(VfsError::NotFound(_))
        ));
        assert!(matches!(
            vfs.delete("missing").await,
            Err(VfsError::NotFound(_))
        ));
        fs::write(dir.path().join("binary"), [0xff]).await.unwrap();
        assert!(vfs.read("binary").await.is_err());
        vfs.copy("binary", "binary-copy").await.unwrap();
        assert_eq!(
            fs::read(dir.path().join("binary-copy")).await.unwrap(),
            [0xff]
        );
        fs::write(dir.path().join("large"), vec![b'x'; 10 * 1024 * 1024 + 1])
            .await
            .unwrap();
        assert!(vfs.read("large").await.is_err());
        vfs.delete("folder").await.unwrap();
        assert!(matches!(
            vfs.read("folder/a").await,
            Err(VfsError::NotFound(_))
        ));
    }
}
