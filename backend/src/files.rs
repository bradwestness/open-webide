//! Remote-mode filesystem access over the WASI `wasi:filesystem` imports.
//!
//! Spin mounts a host folder as a preopened directory (see the `files` entry
//! in spin.toml). Every path the API receives is resolved against that root;
//! any path that would escape it is rejected.

use std::path::Component;

use anyhow::{Context, Result};
use openwebide_core::{FileEntry, SearchHit, find_content_matches};

use crate::wasi::filesystem::preopens;
use crate::wasi::filesystem::types::{
    Descriptor, DescriptorFlags, DescriptorType, DirectoryEntry, ErrorCode, OpenFlags, PathFlags,
};

/// Upper bound on a single file read, so a huge file can't blow up memory.
const MAX_READ_BYTES: u64 = 10 * 1024 * 1024;

fn fs_error(code: ErrorCode) -> anyhow::Error {
    anyhow::anyhow!("filesystem error: {code:?}")
}

/// Like [`fs_error`] but names the path, so a missing entry reads as
/// "not found in workspace: <path>" instead of a bare WASI code.
fn fs_error_at(code: ErrorCode, path: &str) -> anyhow::Error {
    match code {
        ErrorCode::NoEntry => anyhow::anyhow!("not found in workspace: {path}"),
        other => anyhow::anyhow!("filesystem error: {other:?} (at {path})"),
    }
}

/// The preopened workspace root descriptor.
fn root() -> Result<Descriptor> {
    preopens::get_directories()
        .into_iter()
        .next()
        .map(|(d, _)| d)
        .ok_or_else(|| anyhow::anyhow!("no preopened directory; add a `files` mount to spin.toml"))
}

/// Resolve a workspace-relative path, rejecting anything that would escape
/// the root (`..`, absolute paths, etc.).
fn sanitize(rel: &str) -> Result<String> {
    let mut out = String::new();
    for comp in std::path::Path::new(rel).components() {
        match comp {
            Component::Normal(c) => {
                out.push_str(&c.to_string_lossy());
                out.push('/');
            }
            Component::CurDir => {}
            _ => return Err(anyhow::anyhow!("path {rel:?} escapes the workspace root")),
        }
    }
    out.pop(); // drop the trailing '/'
    Ok(out)
}

/// Open a directory at a workspace-relative path (empty = the root).
async fn dir_at(rel: &str) -> Result<Descriptor> {
    let rel = sanitize(rel)?;
    let root = root()?;
    if rel.is_empty() {
        return Ok(root);
    }
    root.open_at(
        PathFlags::SYMLINK_FOLLOW,
        rel.clone(),
        OpenFlags::DIRECTORY,
        DescriptorFlags::empty(),
    )
    .await
    .map_err(|code| fs_error_at(code, &rel))
}

/// Open an existing file for reading.
async fn file_at_read(rel: &str) -> Result<Descriptor> {
    let rel = sanitize(rel)?;
    let root = root()?;
    root.open_at(
        PathFlags::SYMLINK_FOLLOW,
        rel.clone(),
        OpenFlags::empty(),
        DescriptorFlags::READ,
    )
    .await
    .map_err(|code| fs_error_at(code, &rel))
}

/// Open a file for writing, creating it (and any missing parent directories)
/// and truncating it to zero length.
async fn file_at_write(rel: &str) -> Result<Descriptor> {
    let rel = sanitize(rel)?;
    ensure_parent_dirs(&rel).await?;
    let root = root()?;
    root.open_at(
        PathFlags::SYMLINK_FOLLOW,
        rel.clone(),
        OpenFlags::CREATE | OpenFlags::TRUNCATE,
        DescriptorFlags::WRITE,
    )
    .await
    .map_err(|code| fs_error_at(code, &rel))
}

/// Create every ancestor directory of `rel`, ignoring "already exists".
async fn ensure_parent_dirs(rel: &str) -> Result<()> {
    let parts: Vec<&str> = rel.split('/').filter(|p| !p.is_empty()).collect();
    if parts.len() <= 1 {
        return Ok(());
    }
    let root = root()?;
    let mut prefix = String::new();
    for part in &parts[..parts.len() - 1] {
        if !prefix.is_empty() {
            prefix.push('/');
        }
        prefix.push_str(part);
        match root.create_directory_at(prefix.clone()).await {
            Ok(()) => {}
            Err(ErrorCode::Exist) => {}
            Err(code) => return Err(fs_error_at(code, &prefix)),
        }
    }
    Ok(())
}

/// Read the entries of a directory.
async fn read_dir_entries(dir: &Descriptor) -> Result<Vec<DirectoryEntry>> {
    let (stream, fut) = dir.read_directory();
    let entries = stream.collect().await;
    match fut.await {
        Ok(()) => Ok(entries),
        Err(code) => Err(fs_error(code)),
    }
}

/// List the entries of the directory at `rel` (empty = workspace root).
pub async fn list(rel: &str) -> Result<Vec<FileEntry>> {
    let dir = dir_at(rel).await?;
    let base = sanitize(rel)?;
    let entries = read_dir_entries(&dir).await?;
    let mut out = Vec::with_capacity(entries.len());
    for e in entries {
        let is_dir = matches!(e.type_, DescriptorType::Directory);
        let size = if is_dir {
            0
        } else {
            match dir.stat_at(PathFlags::SYMLINK_FOLLOW, e.name.clone()).await {
                Ok(st) => st.size,
                Err(_) => 0,
            }
        };
        let path = if base.is_empty() {
            e.name.clone()
        } else {
            format!("{base}/{}", e.name)
        };
        out.push(FileEntry {
            name: e.name,
            path,
            is_dir,
            size,
        });
    }
    Ok(out)
}

/// Read a file's contents as UTF-8 text.
pub async fn read(rel: &str) -> Result<String> {
    let rel = sanitize(rel)?;
    let root = root()?;
    let st = root
        .stat_at(PathFlags::SYMLINK_FOLLOW, rel.clone())
        .await
        .map_err(|code| fs_error_at(code, &rel))?;
    if st.size > MAX_READ_BYTES {
        return Err(anyhow::anyhow!(
            "file is too large to read ({size} bytes, max {MAX_READ_BYTES})",
            size = st.size
        ));
    }
    let file = file_at_read(&rel).await?;
    let (stream, fut) = file.read_via_stream(0);
    let bytes = stream.collect().await;
    match fut.await {
        Ok(()) => {}
        Err(code) => return Err(fs_error(code)),
    }
    String::from_utf8(bytes).context("file is not valid UTF-8")
}

/// Write `bytes` to `file` by streaming them through a component-model
/// stream that the host drains into the file.
///
/// `write_via_stream` starts the host's read of the stream (and its write to
/// the file) synchronously, so the host is already consuming from the stream
/// by the time we write. We then write all bytes with `write_all`; each
/// `await` yields to the runtime, which drives the host's read forward, and
/// the write completes once the host has the bytes. Dropping the writer
/// signals EOF, and awaiting the host's future finalizes the file (this is
/// where a file-level error such as `NotPermitted` surfaces).
///
/// This mirrors the working `read` path (`read_via_stream` + `collect`). A
/// tight manual-poll loop with a `noop_waker` does not work here: these
/// operations complete via a host callback, so a loop that never yields to
/// the runtime stalls, and dropping the still-pending write future cancels
/// it, silently losing the bytes (a 0-byte file).
async fn write_stream(file: &Descriptor, bytes: Vec<u8>) -> Result<()> {
    let total = bytes.len();
    let (mut writer, reader) = crate::wit_stream::new::<u8>();
    let drain = file.write_via_stream(reader, 0);

    let remaining = writer.write_all(bytes).await;
    if !remaining.is_empty() {
        return Err(anyhow::anyhow!(
            "stream closed before all {total} bytes were written ({} delivered)",
            total - remaining.len()
        ));
    }

    drop(writer);
    match drain.await {
        Ok(()) => Ok(()),
        Err(code) => Err(fs_error(code)),
    }
}

/// Write `content` to a file, creating it (and parent dirs) if needed.
pub async fn write(rel: &str, content: &str) -> Result<()> {
    let file = file_at_write(rel).await?;
    write_stream(&file, content.as_bytes().to_vec()).await
}

/// Create an empty file or a directory at `rel`.
pub async fn create(rel: &str, is_dir: bool) -> Result<()> {
    let rel = sanitize(rel)?;
    let root = root()?;
    if is_dir {
        ensure_parent_dirs(&rel).await?;
        match root.create_directory_at(rel.clone()).await {
            Ok(()) => Ok(()),
            Err(ErrorCode::Exist) => Ok(()),
            Err(code) => Err(fs_error_at(code, &rel)),
        }
    } else {
        let file = file_at_write(&rel).await?;
        write_stream(&file, Vec::new()).await
    }
}

/// Delete the file at `rel`.
pub async fn delete(rel: &str) -> Result<()> {
    let rel = sanitize(rel)?;
    let root = root()?;
    root.unlink_file_at(rel.clone())
        .await
        .map_err(|code| fs_error_at(code, &rel))
}

/// Recursively walk the directory at `rel` and return files whose path
/// contains `query` (case-insensitive).
pub async fn search(rel: &str, query: &str) -> Result<Vec<FileEntry>> {
    let query = query.to_lowercase();
    let mut out = Vec::new();
    walk(rel, &query, &mut out).await?;
    Ok(out)
}

async fn walk(dir_rel: &str, query: &str, out: &mut Vec<FileEntry>) -> Result<()> {
    let entries = match list(dir_rel).await {
        Ok(e) => e,
        Err(_) => return Ok(()), // not a directory or unreadable; skip
    };
    for e in entries {
        if e.is_dir {
            Box::pin(walk(&e.path, query, out)).await?;
        } else if e.path.to_lowercase().contains(query) {
            out.push(e);
        }
    }
    Ok(())
}

/// Recursively walk the directory at `rel` and return the lines of every
/// readable text file whose content contains `query` (case-insensitive).
///
/// Files that are too large or not valid UTF-8 are skipped (their `read`
/// errors are ignored), so a binary or huge file can't break the search.
pub async fn full_text_search(rel: &str, query: &str) -> Result<Vec<SearchHit>> {
    let mut out = Vec::new();
    walk_content(rel, query, &mut out).await?;
    Ok(out)
}

async fn walk_content(dir_rel: &str, query: &str, out: &mut Vec<SearchHit>) -> Result<()> {
    let entries = match list(dir_rel).await {
        Ok(e) => e,
        Err(_) => return Ok(()), // not a directory or unreadable; skip
    };
    for e in entries {
        if e.is_dir {
            Box::pin(walk_content(&e.path, query, out)).await?;
        } else if let Ok(content) = read(&e.path).await {
            for (line, text) in find_content_matches(&content, query) {
                out.push(SearchHit {
                    path: e.path.clone(),
                    line,
                    text,
                });
            }
        }
    }
    Ok(())
}
