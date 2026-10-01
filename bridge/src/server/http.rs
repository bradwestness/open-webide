//! HTTP head/body limits and response helpers for the bridge's hyper server.
//!
//! `run_server` drops a connection's service future when the peer disconnects mid-request
//! (hyper stops polling it); step 25's `/exec` drop guard relies on that to kill the child
//! process group when the client goes away.

use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Full, Limited};
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::{Response, StatusCode};
use hyper_util::rt::TokioTimer;
use serde::de::DeserializeOwned;

/// Bridge HTTP/WebSocket resource limits.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub max_head: usize,
    pub max_body: usize,
    pub head_timeout: Duration,
    pub max_connections: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_head: 16 * 1024,
            max_body: 1024 * 1024,
            head_timeout: Duration::from_secs(10),
            max_connections: 256,
        }
    }
}

/// Errors from reading and decoding a JSON request body.
pub enum HttpError {
    /// Body exceeded the configured limit.
    TooLarge,
    /// Malformed body (I/O error or invalid JSON).
    BadRequest(String),
}

/// Build an HTTP/1 connection builder enforcing the bridge's head-size and idle-connection limits.
///
/// Connection-level `keep_alive(false)` is deliberately not set here: hyper stamps
/// `Connection: close` onto every response written on such a connection, including a `101`
/// WebSocket upgrade, which corrupts the RFC 6455 handshake (`Connection: upgrade` is
/// required). Instead, `respond` closes non-upgrade connections by setting the header itself,
/// which hyper honors per-response without touching the upgrade path.
pub fn builder(limits: &Limits) -> http1::Builder {
    let mut builder = http1::Builder::new();
    builder
        .timer(TokioTimer::new())
        .header_read_timeout(limits.head_timeout)
        .max_buf_size(limits.max_head);
    builder
}

/// Read a request body up to `max` bytes.
pub async fn read_body(body: Incoming, max: usize) -> Result<Bytes, HttpError> {
    let collected = Limited::new(body, max).collect().await.map_err(|err| {
        if err
            .downcast_ref::<http_body_util::LengthLimitError>()
            .is_some()
        {
            HttpError::TooLarge
        } else {
            HttpError::BadRequest(err.to_string())
        }
    })?;
    Ok(collected.to_bytes())
}

/// Read and JSON-decode a request body, enforcing `max` bytes.
pub async fn read_json<T: DeserializeOwned>(body: Incoming, max: usize) -> Result<T, HttpError> {
    let bytes = read_body(body, max).await?;
    serde_json::from_slice(&bytes)
        .map_err(|err| HttpError::BadRequest(format!("invalid JSON: {err}")))
}

/// Build a JSON response, applying step 08's CORS rules (echoed origin + `Vary: Origin`) when
/// `allowed_origin` is set.
pub fn respond(
    status: StatusCode,
    json: &str,
    allowed_origin: Option<&str>,
) -> Response<Full<Bytes>> {
    let mut builder = Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .header("connection", "close");
    if let Some(origin) = allowed_origin {
        builder = builder
            .header("access-control-allow-origin", origin)
            .header("vary", "Origin");
    }
    builder
        .body(Full::new(Bytes::from(json.to_string())))
        .expect("static headers and JSON body always produce a valid response")
}
