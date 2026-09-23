//! File System Access API helpers for local-mode workspaces. Paths are
//! project-relative (the picked directory is the project root, path "").

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use openwebide_core::{FileEntry, SearchHit, Vfs, VfsFuture, find_content_matches};
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    Blob, DirectoryPickerOptions, FileSystemDirectoryHandle, FileSystemFileHandle,
    FileSystemGetDirectoryOptions, FileSystemHandle, FileSystemHandleKind,
    FileSystemPermissionMode, FileSystemWritableFileStream,
};

/// Extract a human-readable message from a rejected JS value. File System
/// Access API and IndexedDB failures reject with a `DOMException` (an
/// object), whose `as_string()` is `None`; defaulting that to `""` would
/// surface an empty error. Fall back to the object's `name`/`message` fields.
pub fn js_error(e: &JsValue) -> String {
    if let Some(s) = e.as_string() {
        return s;
    }
    let get = |k: &str| {
        js_sys::Reflect::get(e, &JsValue::from_str(k))
            .ok()
            .and_then(|v| v.as_string())
            .filter(|s| !s.is_empty())
    };
    match (get("name"), get("message")) {
        (Some(n), Some(m)) if n != "Error" => format!("{n}: {m}"),
        (Some(n), None) => n,
        (None, Some(m)) => m,
        _ => "operation failed".to_string(),
    }
}

/// Call a `FileSystemHandle` permission method with an explicit readwrite
/// descriptor and await its promise. web-sys 0.3.x only exposes the no-arg
/// overloads (which default to "read"), but writes need readwrite, so the JS
/// methods are called directly.
async fn call_permission_method(
    handle: &FileSystemDirectoryHandle,
    method: &str,
) -> Result<JsValue, String> {
    let desc = js_sys::Object::new();
    let _ = js_sys::Reflect::set(
        &desc,
        &JsValue::from_str("mode"),
        &JsValue::from_str("readwrite"),
    );
    let this: JsValue = handle.clone().unchecked_into();
    let fn_value =
        js_sys::Reflect::get(&this, &JsValue::from_str(method)).map_err(|e| js_error(&e))?;
    let fn_value: js_sys::Function = fn_value
        .dyn_into()
        .map_err(|_| format!("{method} is not a function"))?;
    let promise_value = fn_value.call1(&this, &desc).map_err(|e| js_error(&e))?;
    let promise: js_sys::Promise = promise_value
        .dyn_into()
        .map_err(|_| "permission method did not return a promise".to_string())?;
    JsFuture::from(promise).await.map_err(|e| js_error(&e))
}

/// Ensure the handle has readwrite permission, prompting the user if needed.
/// A freshly picked handle is already granted; one restored from IndexedDB
/// after a reload reverts to "prompt" and must be re-requested before use.
async fn ensure_permission(handle: &FileSystemDirectoryHandle) -> Result<(), String> {
    let query = call_permission_method(handle, "queryPermission").await?;
    if query.as_string().as_deref() == Some("granted") {
        return Ok(());
    }
    let request = call_permission_method(handle, "requestPermission").await?;
    if request.as_string().as_deref() == Some("granted") {
        Ok(())
    } else {
        Err("permission to access the folder was not granted".to_string())
    }
}

/// Prompt the user to pick a directory to work on.
///
/// Returns `Ok(Some(handle))` on a pick, `Ok(None)` if the user dismissed the
/// picker, and `Err` on a real failure. The dismissal case matters: the
/// picker rejects with a `DOMException` whose `name` is `"AbortError"`, and a
/// rejected promise's reason is an *object* (so `JsValue::as_string` is
/// `None`) — treating it as a string would surface an empty error.
pub async fn pick_directory() -> Result<Option<FileSystemDirectoryHandle>, String> {
    let window = web_sys::window().ok_or_else(|| "no window".to_string())?;
    let options = DirectoryPickerOptions::new();
    options.set_mode(FileSystemPermissionMode::Readwrite);
    let promise = window
        .show_directory_picker_with_options(&options)
        .map_err(|e| {
            e.as_string()
                .unwrap_or_else(|| "directory picker failed".to_string())
        })?;
    match JsFuture::from(promise).await {
        Ok(handle) => Ok(Some(handle)),
        Err(e) => {
            let name = js_sys::Reflect::get(&e, &JsValue::from_str("name"))
                .ok()
                .and_then(|v| v.as_string());
            if name.as_deref() == Some("AbortError") {
                Ok(None)
            } else {
                Err(e
                    .as_string()
                    .unwrap_or_else(|| "directory picker failed".to_string()))
            }
        }
    }
}

/// List a directory's entries, returning project-relative paths.
pub async fn list(root: &FileSystemDirectoryHandle, dir: &str) -> Result<Vec<FileEntry>, String> {
    ensure_permission(root).await?;
    let dir_handle = resolve_dir(root, dir).await?;
    dir_entries(&dir_handle, dir).await
}

/// Read a file's contents as text.
pub async fn read(root: &FileSystemDirectoryHandle, path: &str) -> Result<String, String> {
    ensure_permission(root).await?;
    let (parent, name) = split_path(path);
    let parent_dir = resolve_dir(root, &parent).await?;
    let file_handle = file_handle(&parent_dir, &name).await?;
    let file_value = JsFuture::from(file_handle.get_file())
        .await
        .map_err(|e| js_error(&e))?;
    let blob: Blob = file_value
        .dyn_into()
        .map_err(|_| format!("no such file: {path}"))?;
    let text = JsFuture::from(blob.text())
        .await
        .map_err(|e| js_error(&e))?;
    Ok(text.as_string().unwrap_or_default())
}

/// Create an object URL (blob:...) for a local file to display media assets.
pub async fn read_blob_url(root: &FileSystemDirectoryHandle, path: &str) -> Result<String, String> {
    ensure_permission(root).await?;
    let (parent, name) = split_path(path);
    let parent_dir = resolve_dir(root, &parent).await?;
    let file_handle = file_handle(&parent_dir, &name).await?;
    let file_value = JsFuture::from(file_handle.get_file())
        .await
        .map_err(|e| js_error(&e))?;
    let blob: Blob = file_value
        .dyn_into()
        .map_err(|_| format!("no such file: {path}"))?;
    web_sys::Url::create_object_url_with_blob(&blob).map_err(|e| js_error(&e))
}

/// Write text to a file, creating parent directories as needed.
pub async fn write(
    root: &FileSystemDirectoryHandle,
    path: &str,
    content: &str,
) -> Result<(), String> {
    ensure_permission(root).await?;
    let (parent, name) = split_path(path);
    let parent_dir = ensure_dir(root, &parent).await?;
    let fh = file_handle_create(&parent_dir, &name).await?;
    write_file_handle(&fh, content).await
}

/// Create an empty file or a directory, creating parent directories as needed.
///
/// For files: exclusive — fails with an "already exists" error if the file
/// already exists, leaving it intact. For directories: idempotent.
pub async fn create(
    root: &FileSystemDirectoryHandle,
    path: &str,
    is_dir: bool,
) -> Result<(), String> {
    ensure_permission(root).await?;
    if is_dir {
        ensure_dir(root, path).await?;
        return Ok(());
    }
    let (parent, name) = split_path(path);
    let parent_dir = ensure_dir(root, &parent).await?;
    match file_handle(&parent_dir, &name).await {
        Ok(_) => return Err(format!("already exists: {path}")),
        Err(e) => {
            let is_not_found = e.contains("NotFoundError") || e.contains("no such file");
            if !is_not_found {
                return Err(e);
            }
        }
    }
    let fh = file_handle_create(&parent_dir, &name).await?;
    write_file_handle(&fh, "").await
}

/// Delete the file at `path`.
pub async fn delete(root: &FileSystemDirectoryHandle, path: &str) -> Result<(), String> {
    ensure_permission(root).await?;
    let (parent, name) = split_path(path);
    let parent_dir = resolve_dir(root, &parent).await?;
    JsFuture::from(parent_dir.remove_entry(&name))
        .await
        .map_err(|e| js_error(&e))?;
    Ok(())
}

/// Full-text search: return the lines of every readable text file under `dir`
/// whose content contains `query` (case-insensitive).
pub async fn search_content(
    root: &FileSystemDirectoryHandle,
    query: &str,
    dir: &str,
) -> Result<Vec<SearchHit>, String> {
    ensure_permission(root).await?;
    let start = resolve_dir(root, dir).await?;
    let mut results = Vec::new();
    content_search_recursive(&start, dir, query, &mut results).await?;
    Ok(results)
}

// -- internals ---------------------------------------------------------------

/// Get a file handle by name, or fail if it does not exist.
async fn file_handle(
    dir: &FileSystemDirectoryHandle,
    name: &str,
) -> Result<FileSystemFileHandle, String> {
    let value = JsFuture::from(dir.get_file_handle(name))
        .await
        .map_err(|e| js_error(&e))?;
    value
        .dyn_into::<FileSystemFileHandle>()
        .map_err(|_| format!("no such file: {name}"))
}

/// Get a file handle by name, creating it if it does not exist.
async fn file_handle_create(
    dir: &FileSystemDirectoryHandle,
    name: &str,
) -> Result<FileSystemFileHandle, String> {
    let options = web_sys::FileSystemGetFileOptions::new();
    options.set_create(true);
    let value = JsFuture::from(dir.get_file_handle_with_options(name, &options))
        .await
        .map_err(|e| js_error(&e))?;
    value
        .dyn_into::<FileSystemFileHandle>()
        .map_err(|_| format!("cannot create file: {name}"))
}

/// Get an existing directory handle by name, or fail if it does not exist.
async fn dir_handle(
    dir: &FileSystemDirectoryHandle,
    name: &str,
) -> Result<FileSystemDirectoryHandle, String> {
    let value = JsFuture::from(dir.get_directory_handle(name))
        .await
        .map_err(|e| js_error(&e))?;
    value
        .dyn_into::<FileSystemDirectoryHandle>()
        .map_err(|_| format!("no such directory: {name}"))
}

/// Get a directory handle by name, creating it if it does not exist.
async fn dir_handle_create(
    dir: &FileSystemDirectoryHandle,
    name: &str,
) -> Result<FileSystemDirectoryHandle, String> {
    let options = FileSystemGetDirectoryOptions::new();
    options.set_create(true);
    let value = JsFuture::from(dir.get_directory_handle_with_options(name, &options))
        .await
        .map_err(|e| js_error(&e))?;
    value
        .dyn_into::<FileSystemDirectoryHandle>()
        .map_err(|_| format!("cannot create directory: {name}"))
}

async fn write_file_handle(
    file_handle: &FileSystemFileHandle,
    content: &str,
) -> Result<(), String> {
    let writable_value = JsFuture::from(file_handle.create_writable())
        .await
        .map_err(|e| js_error(&e))?;
    let writable: FileSystemWritableFileStream = writable_value
        .dyn_into()
        .map_err(|_| "failed to open writable stream".to_string())?;
    let write_promise = writable.write_with_str(content).map_err(|e| js_error(&e))?;
    JsFuture::from(write_promise)
        .await
        .map_err(|e| js_error(&e))?;
    // web-sys does not expose `FileSystemWritableFileStream::close`, so call it
    // through Reflect.
    let writable_js: JsValue = writable.unchecked_into();
    let close_fn_value = js_sys::Reflect::get(&writable_js, &JsValue::from_str("close"))
        .map_err(|e| js_error(&e))?;
    let close_fn: js_sys::Function = close_fn_value
        .dyn_into()
        .map_err(|_| "close is not a function".to_string())?;
    let close_promise_value =
        js_sys::Reflect::apply(&close_fn, &writable_js, &js_sys::Array::new())
            .map_err(|e| js_error(&e))?;
    let close_promise: js_sys::Promise = close_promise_value.unchecked_into();
    JsFuture::from(close_promise)
        .await
        .map_err(|e| js_error(&e))?;
    Ok(())
}

/// Walk `dir` (project-relative) from `root`, returning the directory handle.
async fn resolve_dir(
    root: &FileSystemDirectoryHandle,
    dir: &str,
) -> Result<FileSystemDirectoryHandle, String> {
    let mut current = root.clone();
    for part in dir.split('/').filter(|p| !p.is_empty()) {
        current = dir_handle(&current, part).await?;
    }
    Ok(current)
}

/// Walk `path` from `root`, creating any missing directories.
async fn ensure_dir(
    root: &FileSystemDirectoryHandle,
    path: &str,
) -> Result<FileSystemDirectoryHandle, String> {
    let mut current = root.clone();
    for part in path.split('/').filter(|p| !p.is_empty()) {
        current = dir_handle_create(&current, part).await?;
    }
    Ok(current)
}

/// List a directory's entries with their handles (for recursive search).
async fn dir_entries_with_handles(
    dir_handle: &FileSystemDirectoryHandle,
    prefix: &str,
) -> Result<Vec<(FileEntry, FileSystemHandle)>, String> {
    let iter = dir_handle.values();
    let mut entries = Vec::new();
    loop {
        let next_promise = iter.next().map_err(|e| js_error(&e))?;
        let result = JsFuture::from(next_promise)
            .await
            .map_err(|e| js_error(&e))?;
        let done =
            js_sys::Reflect::get(&result, &JsValue::from_str("done")).map_err(|e| js_error(&e))?;
        if done.as_bool().unwrap_or(true) {
            break;
        }
        let value =
            js_sys::Reflect::get(&result, &JsValue::from_str("value")).map_err(|e| js_error(&e))?;
        let handle: FileSystemHandle = value
            .dyn_into()
            .map_err(|_| "not a file system handle".to_string())?;
        let name = handle.name();
        let is_dir = handle.kind() == FileSystemHandleKind::Directory;
        let path = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}/{name}")
        };
        entries.push((
            FileEntry {
                name,
                path,
                is_dir,
                size: 0,
            },
            handle,
        ));
    }
    Ok(entries)
}

async fn dir_entries(
    dir_handle: &FileSystemDirectoryHandle,
    prefix: &str,
) -> Result<Vec<FileEntry>, String> {
    dir_entries_with_handles(dir_handle, prefix)
        .await
        .map(|pairs| pairs.into_iter().map(|(entry, _)| entry).collect())
}

async fn content_search_recursive(
    dir_handle: &FileSystemDirectoryHandle,
    prefix: &str,
    query: &str,
    results: &mut Vec<SearchHit>,
) -> Result<(), String> {
    let pairs = dir_entries_with_handles(dir_handle, prefix).await?;
    for (entry, handle) in pairs {
        if entry.is_dir {
            if let Some(sub) = handle.dyn_ref::<FileSystemDirectoryHandle>() {
                Box::pin(content_search_recursive(sub, &entry.path, query, results)).await?;
            }
        } else if let Some(file) = handle.dyn_ref::<FileSystemFileHandle>()
            && let Ok(content) = file_handle_text(file).await
        {
            for (line, text) in find_content_matches(&content, query) {
                results.push(SearchHit {
                    path: entry.path.clone(),
                    line,
                    text,
                });
            }
        }
    }
    Ok(())
}

/// Read a file handle's contents as text.
async fn file_handle_text(file: &FileSystemFileHandle) -> Result<String, String> {
    let file_value = JsFuture::from(file.get_file())
        .await
        .map_err(|e| js_error(&e))?;
    let blob: Blob = file_value
        .dyn_into()
        .map_err(|_| "not a file".to_string())?;
    let text = JsFuture::from(blob.text())
        .await
        .map_err(|e| js_error(&e))?;
    Ok(text.as_string().unwrap_or_default())
}

/// Split `path` into its parent directory and file name.
fn split_path(path: &str) -> (String, String) {
    match path.rfind('/') {
        Some(i) => (path[..i].to_string(), path[i + 1..].to_string()),
        None => (String::new(), path.to_string()),
    }
}

/// Wrap a future so it implements Send and Sync in single-threaded WASM environments.
pub struct ForceSend<F>(pub F);
unsafe impl<F> Send for ForceSend<F> {}
unsafe impl<F> Sync for ForceSend<F> {}

impl<F: Future> Future for ForceSend<F> {
    type Output = F::Output;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // SAFETY: Pinning projection is safe because inner future is pinned as long as ForceSend is pinned.
        unsafe {
            let inner = self.map_unchecked_mut(|s| &mut s.0);
            inner.poll(cx)
        }
    }
}

/// A Vfs implementation backed by the browser File System Access API.
#[derive(Clone)]
pub struct BrowserFsaVfs {
    root: FileSystemDirectoryHandle,
}

unsafe impl Send for BrowserFsaVfs {}
unsafe impl Sync for BrowserFsaVfs {}

impl BrowserFsaVfs {
    pub fn new(root: FileSystemDirectoryHandle) -> Self {
        Self { root }
    }
}


impl Vfs for BrowserFsaVfs {
    fn copy<'a>(&'a self, from: &'a str, to: &'a str) -> VfsFuture<'a, ()> {
        let root = self.root.clone();
        let from = from.to_string();
        let to = to.to_string();
        Box::pin(ForceSend(async move {
            let (from_parent, from_name) = split_path(&from);
            let from_dir = resolve_dir(&root, &from_parent)
                .await
                .map_err(openwebide_frontend::vfs_err::map_vfs_err)?;
            let from_handle = file_handle(&from_dir, &from_name)
                .await
                .map_err(openwebide_frontend::vfs_err::map_vfs_err)?;
            
            let file_value = JsFuture::from(from_handle.get_file())
                .await
                .map_err(|e| openwebide_frontend::vfs_err::map_vfs_err(js_error(&e)))?;
            let file: web_sys::File = file_value
                .dyn_into()
                .map_err(|_| openwebide_frontend::vfs_err::map_vfs_err(format!("no such file: {from}")))?;
            
            let (to_parent, to_name) = split_path(&to);
            let to_dir = ensure_dir(&root, &to_parent)
                .await
                .map_err(openwebide_frontend::vfs_err::map_vfs_err)?;
            let to_handle = file_handle_create(&to_dir, &to_name)
                .await
                .map_err(openwebide_frontend::vfs_err::map_vfs_err)?;
            
            let writable_value = JsFuture::from(to_handle.create_writable())
                .await
                .map_err(|e| openwebide_frontend::vfs_err::map_vfs_err(js_error(&e)))?;
            let writable: FileSystemWritableFileStream = writable_value
                .dyn_into()
                .map_err(|_| openwebide_frontend::vfs_err::map_vfs_err("failed to open writable stream".to_string()))?;
                
            let write_promise = writable.write_with_blob(&file).map_err(|e| openwebide_frontend::vfs_err::map_vfs_err(js_error(&e)))?;
            JsFuture::from(write_promise)
                .await
                .map_err(|e| openwebide_frontend::vfs_err::map_vfs_err(js_error(&e)))?;
                
            let writable_js: JsValue = writable.unchecked_into();
            let close_fn_value = js_sys::Reflect::get(&writable_js, &JsValue::from_str("close")).map_err(|e| openwebide_frontend::vfs_err::map_vfs_err(js_error(&e)))?;
            let close_fn: js_sys::Function = close_fn_value.dyn_into().map_err(|_| openwebide_frontend::vfs_err::map_vfs_err("close is not a function".to_string()))?;
            let close_promise_value =
                js_sys::Reflect::apply(&close_fn, &writable_js, &js_sys::Array::new()).map_err(|e| openwebide_frontend::vfs_err::map_vfs_err(js_error(&e)))?;
            let close_promise: js_sys::Promise = close_promise_value.unchecked_into();
            JsFuture::from(close_promise)
                .await
                .map_err(|e| openwebide_frontend::vfs_err::map_vfs_err(js_error(&e)))?;
                
            Ok(())
        }))
    }

    fn read<'a>(&'a self, path: &'a str) -> VfsFuture<'a, String> {
        let root = self.root.clone();
        let path = path.to_string();
        Box::pin(ForceSend(async move {
            read(&root, &path).await.map_err(openwebide_frontend::vfs_err::map_vfs_err)
        }))
    }

    fn write<'a>(&'a self, path: &'a str, content: &'a str) -> VfsFuture<'a, ()> {
        let root = self.root.clone();
        let path = path.to_string();
        let content = content.to_string();
        Box::pin(ForceSend(async move {
            write(&root, &path, &content).await.map_err(openwebide_frontend::vfs_err::map_vfs_err)
        }))
    }

    fn list<'a>(&'a self, dir: &'a str) -> VfsFuture<'a, Vec<FileEntry>> {
        let root = self.root.clone();
        let dir = dir.to_string();
        Box::pin(ForceSend(async move {
            list(&root, &dir).await.map_err(openwebide_frontend::vfs_err::map_vfs_err)
        }))
    }

    fn create<'a>(&'a self, path: &'a str, is_dir: bool) -> VfsFuture<'a, ()> {
        let root = self.root.clone();
        let path = path.to_string();
        Box::pin(ForceSend(async move {
            create(&root, &path, is_dir).await.map_err(openwebide_frontend::vfs_err::map_vfs_err)
        }))
    }

    fn delete<'a>(&'a self, path: &'a str) -> VfsFuture<'a, ()> {
        let root = self.root.clone();
        let path = path.to_string();
        Box::pin(ForceSend(async move {
            delete(&root, &path).await.map_err(openwebide_frontend::vfs_err::map_vfs_err)
        }))
    }

    fn search_content<'a>(&'a self, query: &'a str, dir: &'a str) -> VfsFuture<'a, Vec<SearchHit>> {
        let root = self.root.clone();
        let query = query.to_string();
        let dir = dir.to_string();
        Box::pin(ForceSend(async move {
            search_content(&root, &query, &dir)
                .await
                .map_err(openwebide_frontend::vfs_err::map_vfs_err)
        }))
    }
}
