//! Shared file-tree mutations. Hosts supply only filesystem primitives.
use crate::{
    FileEntry,
    vfs::{VfsEntryKind, workspace_path},
};
use std::{collections::BTreeMap, future::Future};

pub trait WorkspaceEntries {
    fn canonicalize(&self, path: &str) -> impl Future<Output = Result<String, String>>;
    fn list(&self, path: &str) -> impl Future<Output = Result<Vec<FileEntry>, String>>;
    fn read_bytes(&self, path: &str) -> impl Future<Output = Result<Vec<u8>, String>>;
    fn create(&self, path: &str, kind: VfsEntryKind) -> impl Future<Output = Result<(), String>>;
    fn write_bytes(&self, path: &str, bytes: &[u8]) -> impl Future<Output = Result<(), String>>;
    fn delete(&self, path: &str) -> impl Future<Output = Result<(), String>>;
}

pub fn entry_path(path: &str) -> Result<String, String> {
    let path = workspace_path(path).map_err(|e| e.to_string())?;
    if path.is_empty()
        || path
            .split('/')
            .any(|part| part.eq_ignore_ascii_case(".git") || part == ".spin")
    {
        return Err("Choose a project file or folder outside Git/runtime internals".into());
    }
    Ok(path)
}
async fn validate_entry(files: &impl WorkspaceEntries, path: &str) -> Result<(), String> {
    let normalized = entry_path(path)?;
    let canonical = files.canonicalize(&normalized).await?;
    entry_path(&canonical)?;
    if canonical != normalized {
        return Err(
            "This operation does not support symbolic links or paths through symbolic links".into(),
        );
    }
    Ok(())
}

pub fn contains_path(parent: &str, path: &str) -> bool {
    path == parent
        || path
            .strip_prefix(parent)
            .is_some_and(|rest| rest.starts_with('/'))
}
pub fn parent(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(parent, _)| parent)
}
pub fn moved_path(path: &str, from: &str, to: &str) -> Option<String> {
    contains_path(from, path).then(|| format!("{to}{}", &path[from.len()..]))
}

async fn snapshot(
    files: &impl WorkspaceEntries,
    path: &str,
    current: impl Fn() -> bool,
) -> Result<BTreeMap<String, Option<Vec<u8>>>, String> {
    let mut found = BTreeMap::new();
    let mut pending = vec![path.to_string()];
    let mut bytes = 0_usize;
    while let Some(path) = pending.pop() {
        if !current() {
            return Err("Project changed; file operation stopped".into());
        }
        entry_path(&path)?;
        validate_entry(files, &path).await?;
        if found.contains_key(&path) {
            return Err("Repeated entry while traversing folder".into());
        }
        let entry = files
            .list(parent(&path))
            .await?
            .into_iter()
            .find(|entry| entry.path == path)
            .ok_or_else(|| format!("Path not found: {path}"))?;
        if found.len() >= 10_000 {
            return Err("Move exceeds 10,000 entries; original remains unchanged".into());
        }
        if entry.is_dir {
            for child in files.list(&path).await? {
                if parent(&child.path) != path {
                    return Err("Invalid child path returned by filesystem".into());
                }
                pending.push(child.path);
            }
            found.insert(path, None);
        } else {
            let content = files.read_bytes(&path).await?;
            bytes = bytes.saturating_add(content.len());
            if bytes > 32 * 1024 * 1024 {
                return Err("Move exceeds 32 MiB; original remains unchanged".into());
            }
            found.insert(path, Some(content));
        }
    }
    Ok(found)
}

/// Creation policy is shared; adapters retain exclusive file creation.
pub async fn create_entry(
    files: &impl WorkspaceEntries,
    path: &str,
    kind: VfsEntryKind,
    current: impl Fn() -> bool,
) -> Result<(), String> {
    let path = entry_path(path)?;
    validate_entry(files, &path).await?;
    let mut directory = String::new();
    let parts = path.split('/').collect::<Vec<_>>();
    for (index, part) in parts.iter().enumerate() {
        let candidate = if directory.is_empty() {
            part.to_string()
        } else {
            format!("{directory}/{part}")
        };
        let entries = files.list(&directory).await?;
        let Some(entry) = entries.iter().find(|entry| entry.path == candidate) else {
            break;
        };
        if index == parts.len() - 1 {
            return Err(format!("Path already exists: {path}"));
        }
        if !entry.is_dir {
            return Err(format!("Not a folder: {candidate}"));
        }
        directory = candidate;
    }
    if !current() {
        return Err("Project changed; creation stopped".into());
    }
    files.create(&path, kind).await
}

pub async fn delete_entry(
    files: &impl WorkspaceEntries,
    path: &str,
    current: impl Fn() -> bool,
) -> Result<(), String> {
    let path = entry_path(path)?;
    validate_entry(files, &path).await?;
    if !current() {
        return Err("Project changed; deletion stopped".into());
    }
    files.delete(&path).await
}

/// Copy completely, verify the source is unchanged, then remove it. A failed
/// copy never removes the original; cleanup never removes unrelated new data.
pub async fn move_entry(
    files: &impl WorkspaceEntries,
    from: &str,
    to: &str,
    current: impl Fn() -> bool,
) -> Result<(), String> {
    let (from, to) = (entry_path(from)?, entry_path(to)?);
    if contains_path(&from, &to) || contains_path(&to, &from) {
        return Err("Choose a different destination outside the source folder".into());
    }
    if files
        .list(parent(&to))
        .await?
        .iter()
        .any(|entry| entry.path == to)
    {
        return Err(format!("Destination already exists: {to}"));
    }
    validate_entry(files, &to).await?;
    let original = snapshot(files, &from, &current).await?;
    let mut created = Vec::new();
    let copying = async {
        for (path, contents) in &original {
            if !current() {
                return Err("Project changed; original preserved".into());
            }
            let destination = moved_path(path, &from, &to).unwrap();
            files
                .create(
                    &destination,
                    if contents.is_some() {
                        VfsEntryKind::File
                    } else {
                        VfsEntryKind::Directory
                    },
                )
                .await?;
            created.push((destination.clone(), None));
            if !current() {
                return Err("Project changed; original preserved".into());
            }
            if let Some(contents) = contents {
                files.write_bytes(&destination, contents).await?;
                created.last_mut().unwrap().1 = Some(contents);
            }
        }
        let expected_destination = original
            .iter()
            .map(|(path, contents)| (moved_path(path, &from, &to).unwrap(), contents.clone()))
            .collect::<BTreeMap<_, _>>();
        if snapshot(files, &to, &current).await? != expected_destination {
            return Err("Destination changed during the move; original preserved".into());
        }
        if snapshot(files, &from, &current).await? != original {
            return Err("Source changed during the move; original preserved".into());
        }
        if !current() {
            return Err("Project changed; original preserved".into());
        }
        Ok::<_, String>(())
    }
    .await;
    if let Err(error) = copying {
        if current() {
            for (path, contents) in created.iter().rev() {
                let safe = if let Some(expected) = contents {
                    files
                        .read_bytes(path)
                        .await
                        .is_ok_and(|actual| actual == **expected)
                } else {
                    files
                        .list(path)
                        .await
                        .is_ok_and(|entries| entries.is_empty())
                };
                if safe {
                    let _ = files.delete(path).await;
                }
            }
        }
        return Err(format!(
            "{error}. Original preserved; check {to} for any incomplete copy."
        ));
    }
    files.delete(&from).await.map_err(|error| {
        format!("Copied to {to}, but could not remove {from}: {error}. Both locations may remain.")
    })
}

/// A literal, project-root anchored gitignore rule; no shell or glob injection.
pub fn ignore_rule(path: &str, is_dir: bool) -> Result<String, String> {
    let path = entry_path(path)?;
    if path.contains(['\n', '\r']) {
        return Err("Git ignore rules cannot represent a filename containing a newline".into());
    }
    let mut pattern = String::from("/");
    for c in path.chars() {
        if matches!(c, '\\' | '*' | '?' | '[' | ']' | '!' | '#' | ' ') {
            pattern.push('\\');
        }
        pattern.push(c);
    }
    if is_dir {
        pattern.push('/');
    }
    Ok(pattern)
}

/// Append an ignore rule without replacing existing rules or newline style.
pub async fn ignore_entry(
    files: &impl WorkspaceEntries,
    path: &str,
    is_dir: bool,
    current: impl Fn() -> bool,
) -> Result<(), String> {
    let rule = ignore_rule(path, is_dir)?;
    validate_entry(files, ".gitignore").await?;
    let original = if files
        .list("")
        .await?
        .iter()
        .any(|entry| entry.path == ".gitignore")
    {
        Some(files.read_bytes(".gitignore").await?)
    } else {
        None
    };
    let mut content = String::from_utf8(original.clone().unwrap_or_default())
        .map_err(|_| ".gitignore is not UTF-8; edit it manually".to_string())?;
    if content
        .lines()
        .any(|line| line.trim_end_matches('\r') == rule)
    {
        return Ok(());
    }
    let eol = if content.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    if !content.is_empty() && !content.ends_with('\n') {
        content.push_str(eol);
    }
    content.push_str(&rule);
    content.push_str(eol);
    if !current() {
        return Err("Project changed; ignore operation stopped".into());
    }
    if let Some(original) = original {
        if files.read_bytes(".gitignore").await? != original {
            return Err(".gitignore changed; retry the ignore operation".into());
        }
    } else {
        files.create(".gitignore", VfsEntryKind::File).await?;
    }
    if !current() {
        return Err("Project changed; ignore operation stopped".into());
    }
    files.write_bytes(".gitignore", content.as_bytes()).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};

    #[derive(Default)]
    struct Files {
        entries: RefCell<BTreeMap<String, Option<Vec<u8>>>>,
        fail_write: Cell<bool>,
        change_source: Cell<bool>,
        stale: Cell<bool>,
        alias: Cell<bool>,
    }
    impl Files {
        fn file(&self, path: &str, bytes: &[u8]) {
            self.entries
                .borrow_mut()
                .insert(path.into(), Some(bytes.into()));
        }
        fn dir(&self, path: &str) {
            self.entries.borrow_mut().insert(path.into(), None);
        }
        fn has(&self, path: &str) -> bool {
            self.entries.borrow().contains_key(path)
        }
    }
    impl WorkspaceEntries for Files {
        async fn canonicalize(&self, path: &str) -> Result<String, String> {
            if self.alias.get() && path == "source" {
                Ok("elsewhere".into())
            } else {
                entry_path(path)
            }
        }
        async fn list(&self, path: &str) -> Result<Vec<FileEntry>, String> {
            if !path.is_empty() && self.entries.borrow().get(path) != Some(&None) {
                return Err("Not a directory".into());
            }
            Ok(self
                .entries
                .borrow()
                .iter()
                .filter(|(entry, _)| parent(entry) == path)
                .map(|(path, bytes)| FileEntry {
                    path: path.clone(),
                    name: path.rsplit('/').next().unwrap().into(),
                    is_dir: bytes.is_none(),
                    size: bytes.as_ref().map_or(0, |bytes| bytes.len() as u64),
                })
                .collect())
        }
        async fn read_bytes(&self, path: &str) -> Result<Vec<u8>, String> {
            self.entries
                .borrow()
                .get(path)
                .cloned()
                .flatten()
                .ok_or("Missing file".into())
        }
        async fn create(&self, path: &str, kind: VfsEntryKind) -> Result<(), String> {
            if self.has(path) {
                return Err("Already exists".into());
            }
            self.entries
                .borrow_mut()
                .insert(path.into(), (!kind.is_dir()).then(Vec::new));
            Ok(())
        }
        async fn write_bytes(&self, path: &str, bytes: &[u8]) -> Result<(), String> {
            if self.fail_write.get() {
                return Err("Disk full".into());
            }
            self.file(path, bytes);
            if self.change_source.get() {
                self.file("source", b"External edit");
            }
            if path == "destination" {
                self.stale.set(true);
            }
            Ok(())
        }
        async fn delete(&self, path: &str) -> Result<(), String> {
            self.entries
                .borrow_mut()
                .retain(|entry, _| !contains_path(path, entry));
            Ok(())
        }
    }

    #[test]
    fn moves_nested_binary_files_and_empty_folders() {
        futures::executor::block_on(async {
            let files = Files::default();
            files.dir("source");
            files.dir("source/empty");
            files.file("source/data", &[0, 255, 128, 10]);
            move_entry(&files, "source", "renamed", || true)
                .await
                .unwrap();
            assert!(!files.has("source"));
            assert!(files.has("renamed/empty"));
            assert_eq!(
                files.read_bytes("renamed/data").await.unwrap(),
                [0, 255, 128, 10]
            );
        });
    }
    #[test]
    fn existing_destination_and_invalid_paths_leave_source_untouched() {
        futures::executor::block_on(async {
            let files = Files::default();
            files.file("source", b"Original");
            files.file("existing", b"Other");
            for destination in [
                "existing",
                "source/child",
                "../outside",
                ".git/config",
                ".spin/db",
            ] {
                assert!(
                    move_entry(&files, "source", destination, || true)
                        .await
                        .is_err()
                );
                assert_eq!(files.read_bytes("source").await.unwrap(), b"Original");
                assert_eq!(files.read_bytes("existing").await.unwrap(), b"Other");
            }
        });
    }
    #[test]
    fn symbolic_link_alias_is_rejected_before_copying() {
        futures::executor::block_on(async {
            let files = Files::default();
            files.file("source", b"Original");
            files.alias.set(true);
            assert!(
                move_entry(&files, "source", "target", || true)
                    .await
                    .unwrap_err()
                    .contains("symbolic links")
            );
            assert_eq!(files.read_bytes("source").await.unwrap(), b"Original");
            assert!(!files.has("target"));
        });
    }

    #[test]
    fn copy_failure_preserves_original() {
        futures::executor::block_on(async {
            let files = Files::default();
            files.file("source", b"Original");
            files.fail_write.set(true);
            assert!(
                move_entry(&files, "source", "target", || true)
                    .await
                    .unwrap_err()
                    .contains("Disk full")
            );
            assert_eq!(files.read_bytes("source").await.unwrap(), b"Original");
        });
    }
    #[test]
    fn concurrent_source_edit_is_preserved_and_copy_is_removed() {
        futures::executor::block_on(async {
            let files = Files::default();
            files.file("source", b"Original");
            files.change_source.set(true);
            assert!(
                move_entry(&files, "source", "target", || true)
                    .await
                    .unwrap_err()
                    .contains("Source changed")
            );
            assert_eq!(files.read_bytes("source").await.unwrap(), b"External edit");
            assert!(!files.has("target"));
        });
    }
    #[test]
    fn project_change_stops_before_source_deletion_or_cleanup() {
        futures::executor::block_on(async {
            let files = Files::default();
            files.file("source", b"Original");
            assert!(
                move_entry(&files, "source", "destination", || !files.stale.get())
                    .await
                    .is_err()
            );
            assert_eq!(files.read_bytes("source").await.unwrap(), b"Original");
            assert_eq!(files.read_bytes("destination").await.unwrap(), b"Original");
        });
    }
    #[test]
    fn ignore_preserves_existing_rules_and_crlf_and_is_idempotent() {
        futures::executor::block_on(async {
            let files = Files::default();
            files.file(".gitignore", b"# Existing\r\n/cache");
            ignore_entry(&files, "build", true, || true).await.unwrap();
            let expected = b"# Existing\r\n/cache\r\n/build/\r\n";
            assert_eq!(files.read_bytes(".gitignore").await.unwrap(), expected);
            ignore_entry(&files, "build", true, || true).await.unwrap();
            assert_eq!(files.read_bytes(".gitignore").await.unwrap(), expected);
            assert!(
                ignore_entry(&files, "other", false, || false)
                    .await
                    .is_err()
            );
            assert_eq!(files.read_bytes(".gitignore").await.unwrap(), expected);
            files.file(".gitignore", &[255]);
            assert!(ignore_entry(&files, "other", false, || true).await.is_err());
            assert_eq!(files.read_bytes(".gitignore").await.unwrap(), [255]);
        });
    }

    #[test]
    fn gitignore_patterns_are_anchored_and_literal() {
        assert_eq!(
            ignore_rule("a b/[x]*?.txt", false).unwrap(),
            "/a\\ b/\\[x\\]\\*\\?.txt"
        );
        assert_eq!(ignore_rule("build", true).unwrap(), "/build/");
        assert!(ignore_rule("bad\nname", false).is_err());
        assert!(ignore_rule(".git/config", false).is_err());
        assert!(ignore_rule("", true).is_err());
        assert_eq!(moved_path("src/file", "src", "lib").unwrap(), "lib/file");
        assert!(moved_path("src2/file", "src", "lib").is_none());
    }
}
