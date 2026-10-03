//! Search policy shared by every filesystem adapter.
use std::collections::HashSet;

use crate::{
    FileEntry, SearchHit, Vfs, VfsError, find_content_matches, normalize_vfs_path,
    vfs::{MAX_READ_BYTES, SearchOptions, skip_dir},
};

pub const MAX_SEARCH_HITS: usize = 500;
pub const MAX_SEARCH_LINE_CHARS: usize = 400;

pub struct SearchBudget {
    pub hits: usize,
    pub bytes: usize,
    pub entries: usize,
}
impl SearchBudget {
    pub fn new() -> Self {
        Self {
            hits: MAX_SEARCH_HITS,
            bytes: 64 * 1024 * 1024,
            entries: 20_000,
        }
    }
    pub fn allow_entry(&mut self) -> bool {
        if self.entries == 0 {
            return false;
        }
        self.entries -= 1;
        true
    }
    pub fn allow_file(&mut self, size: u64) -> bool {
        let Ok(size) = usize::try_from(size) else {
            return false;
        };
        if size > self.bytes || size as u64 > MAX_READ_BYTES {
            return false;
        }
        self.bytes -= size;
        true
    }
    pub fn push_hit(&mut self) -> bool {
        if self.hits == 0 {
            return false;
        }
        self.hits -= 1;
        true
    }
    pub fn exhausted(&self) -> bool {
        self.hits == 0 || self.entries == 0
    }
}
impl Default for SearchBudget {
    fn default() -> Self {
        Self::new()
    }
}

/// One walker, deterministic ordering, cycle avoidance, and budgets for all hosts.
async fn walk<V: Vfs + ?Sized>(
    vfs: &V,
    dir: &str,
    query: &str,
    opts: SearchOptions,
    content: bool,
) -> Result<(Vec<FileEntry>, Vec<SearchHit>), VfsError> {
    let root = normalize_vfs_path(dir)?;
    let mut pending = vec![root.clone()];
    let mut visited = HashSet::new();
    let mut budget = SearchBudget::new();
    let mut files = Vec::new();
    let mut hits = Vec::new();
    let query = query.to_lowercase();
    while let Some(dir) = pending.pop() {
        if budget.exhausted() {
            break;
        }
        let canonical = match vfs.canonicalize(&dir).await {
            Ok(path) => path,
            Err(error) if dir == root => return Err(error),
            Err(_) => continue,
        };
        if !visited.insert(canonical) {
            continue;
        }
        let mut entries = match vfs.list(&dir).await {
            Ok(entries) => entries,
            Err(error) if dir == root => return Err(error),
            Err(_) => continue,
        };
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        let mut children = Vec::new();
        for entry in entries {
            if budget.exhausted() || !budget.allow_entry() {
                break;
            }
            if entry.name == ".spin" {
                continue;
            }
            if entry.is_dir {
                if !skip_dir(&entry.name, opts) {
                    children.push(entry.path);
                }
            } else if !content {
                if entry.path.to_lowercase().contains(&query) && budget.push_hit() {
                    files.push(entry);
                }
            } else if budget.allow_file(entry.size) {
                let Ok(text) = vfs.read(&entry.path).await else {
                    continue;
                };
                for (line, text) in find_content_matches(&text, &query) {
                    if !budget.push_hit() {
                        break;
                    }
                    hits.push(SearchHit {
                        path: entry.path.clone(),
                        line,
                        text: truncate_line(&text, MAX_SEARCH_LINE_CHARS),
                    });
                }
            }
        }
        pending.extend(children.into_iter().rev());
    }
    Ok((files, hits))
}

pub async fn content<V: Vfs + ?Sized>(
    vfs: &V,
    query: &str,
    dir: &str,
    opts: SearchOptions,
) -> Result<Vec<SearchHit>, VfsError> {
    walk(vfs, dir, query, opts, true)
        .await
        .map(|(_, hits)| hits)
}
pub async fn files<V: Vfs + ?Sized>(
    vfs: &V,
    query: &str,
    dir: &str,
    opts: SearchOptions,
) -> Result<Vec<FileEntry>, VfsError> {
    walk(vfs, dir, query, opts, false)
        .await
        .map(|(files, _)| files)
}

pub fn truncate_line(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MemoryVfs;

    #[test]
    fn walker_contract_filters_cycles_and_bounds_results() {
        futures::executor::block_on(async {
            let vfs = MemoryVfs::new();
            vfs.write("src/a.txt", "needle\nNEEDLE").await.unwrap();
            vfs.write("node_modules/hidden", "needle").await.unwrap();
            vfs.write(".spin/private", "needle").await.unwrap();
            vfs.add_symlink("src/loop", "src").unwrap();
            let hits = content(&vfs, "NEEDLE", "", SearchOptions::default())
                .await
                .unwrap();
            assert_eq!(hits.len(), 2);
            assert_eq!(hits[0].path, "src/a.txt");
            assert_eq!(hits[1].line, 2);
            vfs.write(
                "src/a.txt",
                &format!("needle{}\n", "é".repeat(500)).repeat(501),
            )
            .await
            .unwrap();
            let hits = content(&vfs, "needle", "src\\.", SearchOptions::default())
                .await
                .unwrap();
            assert_eq!(hits.len(), 500);
            assert!(hits.iter().all(|hit| hit.text.chars().count() == 400));
            assert!(matches!(
                content(&vfs, "needle", "../outside", SearchOptions::default()).await,
                Err(VfsError::PathEscape(_))
            ));
        });
    }
}
