//! IndexedDB helpers for persisting File System Access API directory handles
//! so local-mode projects survive a page reload.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use futures::FutureExt;
use futures::channel::oneshot;
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;
use web_sys::{
    FileSystemDirectoryHandle, FileSystemHandle, FileSystemHandlePermissionDescriptor,
    FileSystemPermissionMode, IdbDatabase, IdbObjectStore, IdbRequest, IdbTransactionMode,
};

use crate::local_fs::js_error;

const DB_NAME: &str = "openwebide";
const STORE: &str = "directories";

/// A future that resolves when an `IdbRequest` fires its success or error
/// event. It owns the event-handler closures (so they stay alive until the
/// request settles) and the request itself.
struct IdbRequestFuture {
    _request: Option<IdbRequest>,
    rx: Option<oneshot::Receiver<Result<JsValue, String>>>,
    _closures: Vec<Closure<dyn FnMut()>>,
}

impl Future for IdbRequestFuture {
    type Output = Result<JsValue, String>;
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.rx.as_mut() {
            Some(rx) => rx
                .poll_unpin(cx)
                .map(|r| r.map_err(|_| "IndexedDB request failed".to_string())?),
            None => Poll::Ready(Err("IndexedDB request already settled".to_string())),
        }
    }
}

/// Attach success/error handlers to `request`, resolving `tx` when it settles.
/// The closures are pushed onto `closures` so they outlive the request.
fn attach(
    request: &IdbRequest,
    closures: &mut Vec<Closure<dyn FnMut()>>,
    tx: oneshot::Sender<Result<JsValue, String>>,
    event: &str,
) {
    let event = event.to_string();
    // Only one of the success/error handlers fires, but both must be able to
    // send, so share the (non-cloneable) sender and let the first to fire take it.
    let tx = std::rc::Rc::new(std::cell::RefCell::new(Some(tx)));
    let success = Closure::<dyn FnMut()>::new({
        let req = request.clone();
        let tx = std::rc::Rc::clone(&tx);
        move || {
            let result = req.result().unwrap_or(JsValue::NULL);
            if let Some(tx) = tx.borrow_mut().take() {
                let _ = tx.send(Ok(result));
            }
        }
    });
    let error = Closure::<dyn FnMut()>::new({
        let tx = std::rc::Rc::clone(&tx);
        move || {
            if let Some(tx) = tx.borrow_mut().take() {
                let _ = tx.send(Err(format!("IndexedDB {event} failed")));
            }
        }
    });
    request.set_onsuccess(Some(success.as_ref().unchecked_ref()));
    request.set_onerror(Some(error.as_ref().unchecked_ref()));
    closures.push(success);
    closures.push(error);
}

/// Open the database, creating the `directories` store on first use.
pub async fn open_db() -> Result<IdbDatabase, String> {
    let idb = web_sys::window()
        .and_then(|w| w.indexed_db().ok().flatten())
        .ok_or_else(|| "IndexedDB is not available".to_string())?;
    let request = idb
        .open_with_u32(DB_NAME, 1)
        .map_err(|e| e.as_string().unwrap_or_else(|| "open failed".to_string()))?;
    let (tx, rx) = oneshot::channel();
    let mut closures = Vec::new();
    // The onupgradeneeded handler reads the freshly opened DB off the request,
    // which requires the base `IdbRequest` type.
    let upgrade_request: IdbRequest = request.clone().into();
    let onupgradeneeded = Closure::<dyn FnMut()>::new(move || {
        if let Ok(value) = upgrade_request.result()
            && let Some(db) = value.dyn_ref::<IdbDatabase>()
        {
            let _ = db.create_object_store(STORE);
        }
    });
    request.set_onupgradeneeded(Some(onupgradeneeded.as_ref().unchecked_ref()));
    closures.push(onupgradeneeded);
    // Convert to the base `IdbRequest` for the success/error handlers and storage.
    let request: IdbRequest = request.into();
    attach(&request, &mut closures, tx, "open");
    let result = IdbRequestFuture {
        _request: Some(request),
        rx: Some(rx),
        _closures: closures,
    }
    .await?;
    result
        .dyn_into::<IdbDatabase>()
        .map_err(|_| "IndexedDB open did not return a database".to_string())
}

fn get_store(db: &IdbDatabase, mode: IdbTransactionMode) -> Result<IdbObjectStore, String> {
    let tx = db.transaction_with_str_and_mode(STORE, mode).map_err(|e| {
        e.as_string()
            .unwrap_or_else(|| "transaction failed".to_string())
    })?;
    tx.object_store(STORE).map_err(|e| {
        e.as_string()
            .unwrap_or_else(|| "object store failed".to_string())
    })
}

/// Build the IndexedDB key for a project. IndexedDB keys must be a number,
/// string, Date, or buffer — `JsValue::from(i64)` yields a BigInt, which
/// IndexedDB rejects with a DataError. Project ids are small, so an f64 number
/// is exact.
fn key(project_id: i64) -> JsValue {
    JsValue::from_f64(project_id as f64)
}

/// Persist a directory handle keyed by project id.
pub async fn save_handle(
    project_id: i64,
    handle: &FileSystemDirectoryHandle,
) -> Result<(), String> {
    let db = open_db().await?;
    let store = get_store(&db, IdbTransactionMode::Readwrite)?;
    let record = js_sys::Object::new();
    js_sys::Reflect::set(&record, &JsValue::from_str("projectId"), &key(project_id))
        .map_err(|e| js_error(&e))?;
    js_sys::Reflect::set(&record, &JsValue::from_str("handle"), handle)
        .map_err(|e| js_error(&e))?;
    let request = store
        .put_with_key(&record, &key(project_id))
        .map_err(|e| js_error(&e))?;
    let (tx, rx) = oneshot::channel();
    let mut closures = Vec::new();
    attach(&request, &mut closures, tx, "put");
    let _ = IdbRequestFuture {
        _request: Some(request),
        rx: Some(rx),
        _closures: closures,
    }
    .await?;
    Ok(())
}

/// Load a previously persisted directory handle for a project.
pub async fn load_handle(project_id: i64) -> Result<Option<FileSystemDirectoryHandle>, String> {
    let db = open_db().await?;
    let store = get_store(&db, IdbTransactionMode::Readonly)?;
    let request = store.get(&key(project_id)).map_err(|e| js_error(&e))?;
    let (tx, rx) = oneshot::channel();
    let mut closures = Vec::new();
    attach(&request, &mut closures, tx, "get");
    let result = IdbRequestFuture {
        _request: Some(request),
        rx: Some(rx),
        _closures: closures,
    }
    .await?;
    if result.is_null() || result.is_undefined() {
        return Ok(None);
    }
    let record = result.unchecked_into::<js_sys::Object>();
    let handle =
        js_sys::Reflect::get(&record, &JsValue::from_str("handle")).map_err(|e| js_error(&e))?;
    if handle.is_null() || handle.is_undefined() {
        return Ok(None);
    }
    handle
        .dyn_into::<FileSystemDirectoryHandle>()
        .map(Some)
        .map_err(|_| "stored handle is not a directory".to_string())
}

/// Re-request read/write permission for a handle (needed after a reload).
/// Returns true when permission is granted.
pub async fn request_permission(handle: &FileSystemDirectoryHandle) -> Result<bool, String> {
    let descriptor = FileSystemHandlePermissionDescriptor::new();
    descriptor.set_mode(FileSystemPermissionMode::Readwrite);
    let handle_ref: &FileSystemHandle = handle
        .dyn_ref::<FileSystemHandle>()
        .ok_or_else(|| "not a file system handle".to_string())?;
    let promise = handle_ref.request_permission_with_descriptor(&descriptor);
    let result = wasm_bindgen_futures::JsFuture::from(promise)
        .await
        .map_err(|e| js_error(&e))?;
    Ok(result.as_string().as_deref() == Some("granted"))
}
