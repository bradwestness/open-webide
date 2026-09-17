//! Remote-mode filesystem access over the WASI `wasi:filesystem` imports.
//!
//! Spin mounts a host folder as a preopened directory (see the `files` entry
//! in spin.toml). Every path the API receives is resolved against that root;
//! any path that would escape it is rejected.

use std::path::Component;

use anyhow::{Context, Result};
use openwebide_core::FileEntry;

use crate::wasi::filesystem::preopens;
use crate::wasi::filesystem::types::{
    Descriptor, DescriptorFlags, DescriptorType, DirectoryEntry, ErrorCode, OpenFlags, PathFlags,
};

/// Upper bound on a single file read, so a huge file can't blow up memory.
const MAX_READ_BYTES: u64 = 10 * 1024 * 1024;

fn fs_error(code: ErrorCode) -> anyhow::Error {
    anyhow::anyhow!("filesystem error: {code:?}")
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
        rel,
        OpenFlags::DIRECTORY,
        DescriptorFlags::empty(),
    )
    .await
    .map_err(fs_error)
}

/// Open an existing file for reading.
async fn file_at_read(rel: &str) -> Result<Descriptor> {
    let rel = sanitize(rel)?;
    let root = root()?;
    root.open_at(
        PathFlags::SYMLINK_FOLLOW,
        rel,
        OpenFlags::empty(),
        DescriptorFlags::READ,
    )
    .await
    .map_err(fs_error)
}

/// Open a file for writing, creating it (and any missing parent directories)
/// and truncating it to zero length.
async fn file_at_write(rel: &str) -> Result<Descriptor> {
    let rel = sanitize(rel)?;
    ensure_parent_dirs(&rel).await?;
    let root = root()?;
    root.open_at(
        PathFlags::SYMLINK_FOLLOW,
        rel,
        OpenFlags::CREATE | OpenFlags::TRUNCATE,
        DescriptorFlags::WRITE,
    )
    .await
    .map_err(fs_error)
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
            Err(code) => return Err(fs_error(code)),
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
        .map_err(fs_error)?;
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

/// Drive a file write: hand `bytes` to a fresh stream, let the host read them
/// via `write_via_stream`, then close the write end so the host sees EOF and
/// finalizes.
///
/// The host reads the stream until EOF before the `write_via_stream` future
/// resolves, so the writer must be dropped to signal EOF. But the write
/// future returned by `writer.write` holds a `&mut` borrow of the writer, so
/// we cannot drop the writer while the write future is alive. Dropping a
/// still-pending write future also cancels the in-flight write (losing the
/// bytes and trapping the worker), so the write future must be *completed*
/// before it is dropped.
///
/// The host's read is async and needs several polls before it reaches the
/// point where it consumes the queued bytes, so a single "poll the read once,
/// then poll the write once" is racy. Instead we drive both together: poll
/// the host's read and the write future in a loop until the write future is
/// ready (the host has the bytes). Only then is it safe to drop the write
/// future (a no-op now that it is done) and the writer (EOF), and await the
/// host to finalize.
async fn write_stream(file: &Descriptor, bytes: Vec<u8>) -> Result<()> {
    use std::future::{Future, IntoFuture};
    use std::task::Context;

    let (mut writer, reader) = crate::wit_stream::new::<u8>();
    let mut drain = Box::pin(file.write_via_stream(reader, 0).into_future());
    let mut write_fut = Box::pin(writer.write(bytes));

    let waker = futures::task::noop_waker();
    let mut cx = Context::from_waker(&waker);

    // Drive the host's read and the write together until the write completes
    // (the host has consumed the bytes). The host's read is async, so it may
    // take several polls to reach the point where it reads the queued bytes;
    // each drain poll advances the host, and the write future is started on
    // the first write poll and completes once the host has the bytes.
    for _ in 0..100 {
        let _ = drain.as_mut().poll(&mut cx);
        if write_fut.as_mut().poll(&mut cx).is_ready() {
            break;
        }
    }

    // The bytes are consumed. Drop the write future (a no-op now that it is
    // done) and the writer (EOF), then await the host to finalize.
    drop(write_fut);
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
        match root.create_directory_at(rel).await {
            Ok(()) => Ok(()),
            Err(ErrorCode::Exist) => Ok(()),
            Err(code) => Err(fs_error(code)),
        }
    } else {
        let file = file_at_write(&rel).await?;
        write_stream(&file, Vec::new()).await
    }
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
