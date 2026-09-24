//! WebSocket and HTTP server for the process execution bridge daemon.
//!
//! HTTP/1 parsing and connection lifecycle are handled by `hyper` (see `crate::http`); this
//! module owns request routing, the Host/Origin/CORS security baseline, the WebSocket upgrade,
//! and the session-multiplexed WebSocket protocol.

use std::convert::Infallible;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use futures::{SinkExt, StreamExt};
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::header::{CONNECTION, CONTENT_TYPE, HOST, ORIGIN, UPGRADE};
use hyper::service::service_fn;
use hyper::{HeaderMap, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use openwebide_core::{BridgeClientMessage, BridgeServerMessage};
use serde::Deserialize;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::time::Instant;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::derive_accept_key;
use tokio_tungstenite::tungstenite::protocol::{Role, WebSocketConfig};

use crate::headless::{execute_command_direct, spawn_headless};
use crate::http::{HttpError, Limits, builder, read_body, read_json, respond};
use crate::pty::spawn_pty;
use crate::session::SessionManager;

const MAX_WS_MESSAGE: usize = 16 * 1024 * 1024;
const WS_PING_INTERVAL: Duration = Duration::from_secs(30);
const WS_IDLE_TIMEOUT: Duration = Duration::from_secs(90);

/// Bridge server configuration.
#[derive(Clone, Debug)]
pub struct ServerConfig {
    pub workspace_root: PathBuf,
    pub allowed_origins: Vec<String>,
    pub allowed_hosts: Vec<String>,
    pub limits: Limits,
}

impl ServerConfig {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self {
            workspace_root,
            allowed_origins: default_origins(),
            allowed_hosts: default_hosts(),
            limits: Limits::default(),
        }
    }
}

pub fn default_origins() -> Vec<String> {
    vec![
        "http://localhost:3000".to_string(),
        "http://127.0.0.1:3000".to_string(),
        "http://localhost:8080".to_string(),
        "http://127.0.0.1:8080".to_string(),
    ]
}

pub fn default_hosts() -> Vec<String> {
    let mut hosts = vec![
        "localhost".to_string(),
        "127.0.0.1".to_string(),
        "[::1]".to_string(),
    ];
    if let Some(h) = get_system_hostname() {
        let local_h = format!("{h}.local");
        hosts.push(h);
        hosts.push(local_h);
    }
    hosts
}

pub fn get_system_hostname() -> Option<String> {
    #[cfg(unix)]
    {
        let mut buf = [0u8; 256];
        // SAFETY: libc::gethostname is standard POSIX and buf is a valid buffer
        let res = unsafe { libc::gethostname(buf.as_mut_ptr() as *mut libc::c_char, buf.len()) };
        if res == 0 {
            let nul_pos = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
            let s = std::str::from_utf8(&buf[..nul_pos]).ok()?;
            let trimmed = s.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_ascii_lowercase());
            }
        }
    }
    None
}

fn extract_host_hostname(host: &str) -> &str {
    let trimmed = host.trim();
    if trimmed.starts_with('[')
        && let Some(close) = trimmed.find(']')
    {
        return &trimmed[..=close];
    }
    if let Some((h, port)) = trimmed.rsplit_once(':')
        && port.chars().all(|c| c.is_ascii_digit())
    {
        return h;
    }
    trimmed
}

fn extract_origin_hostname(origin: &str) -> Option<&str> {
    let rest = if let Some(r) = origin.strip_prefix("http://") {
        r
    } else if let Some(r) = origin.strip_prefix("https://") {
        r
    } else {
        let idx = origin.find("://")?;
        &origin[idx + 3..]
    };
    let host_and_port = rest.split('/').next().unwrap_or(rest);
    Some(extract_host_hostname(host_and_port))
}

fn is_ip_literal(h: &str) -> bool {
    let unbracketed = if h.starts_with('[') && h.ends_with(']') {
        &h[1..h.len() - 1]
    } else {
        h
    };
    unbracketed.parse::<std::net::IpAddr>().is_ok()
}

fn is_host_allowed(host_raw: &str, cfg: &ServerConfig) -> bool {
    let host_norm = extract_host_hostname(host_raw).to_ascii_lowercase();
    if is_ip_literal(&host_norm) {
        return true;
    }
    for allowed in &cfg.allowed_hosts {
        let allowed_norm = extract_host_hostname(allowed).to_ascii_lowercase();
        if host_norm == allowed_norm {
            return true;
        }
    }
    false
}

/// Pure validation of Host, Origin, and Content-Type baseline.
/// Returns Ok(Some(origin_to_echo)) or Ok(None) if allowed, or Err(status_code) if rejected.
pub fn check_request(
    host: Option<&str>,
    origin: Option<&str>,
    method: &str,
    content_type: Option<&str>,
    cfg: &ServerConfig,
) -> Result<Option<String>, u16> {
    // 1. Host hostname (port ignored) must pass the Host rule; else 403.
    let host_str = match host {
        Some(h) if !h.trim().is_empty() => h.trim(),
        _ => return Err(403),
    };
    if !is_host_allowed(host_str, cfg) {
        return Err(403);
    }

    // 2. Origin present: must pass Origin rule; else 403. Origin: null -> 403.
    let echo_origin = if let Some(orig_raw) = origin {
        let orig = orig_raw.trim();
        if orig.eq_ignore_ascii_case("null") || orig.is_empty() {
            return Err(403);
        }

        let is_listed = cfg.allowed_origins.iter().any(|allowed| {
            allowed
                .trim_end_matches('/')
                .eq_ignore_ascii_case(orig.trim_end_matches('/'))
        });

        let same_host = if let Some(orig_host) = extract_origin_hostname(orig) {
            let host_host = extract_host_hostname(host_str);
            orig_host.eq_ignore_ascii_case(host_host)
        } else {
            false
        };

        if !is_listed && !same_host {
            return Err(403);
        }

        Some(orig.to_string())
    } else {
        None
    };

    // 3. POST with Origin and media type != application/json -> 415.
    if method.eq_ignore_ascii_case("POST") && echo_origin.is_some() {
        let ct = content_type.unwrap_or("");
        let media_type = ct.split(';').next().unwrap_or("").trim();
        if !media_type.eq_ignore_ascii_case("application/json") {
            return Err(415);
        }
    }

    // 4. Origin absent -> allowed.
    Ok(echo_origin)
}

/// Run the bridge server on the specified TCP listener.
pub async fn run_server(listener: TcpListener, config: ServerConfig) {
    run_accept_loop(listener, config).await;
}

/// Anything that can accept incoming connections like a `TcpListener`. Exists so the accept
/// loop's error/backoff handling can be unit-tested against a fake that fails on demand.
trait Accept {
    fn accept(
        &self,
    ) -> impl std::future::Future<Output = std::io::Result<(TcpStream, std::net::SocketAddr)>> + Send;
}

impl Accept for TcpListener {
    async fn accept(&self) -> std::io::Result<(TcpStream, std::net::SocketAddr)> {
        TcpListener::accept(self).await
    }
}

async fn run_accept_loop<A: Accept>(acceptor: A, config: ServerConfig) {
    let session_manager = SessionManager::new();
    let semaphore = Arc::new(Semaphore::new(config.limits.max_connections));
    let mut backoff = Duration::from_millis(10);
    const MAX_BACKOFF: Duration = Duration::from_secs(1);

    loop {
        let permit = semaphore
            .clone()
            .acquire_owned()
            .await
            .expect("semaphore is never closed");

        match acceptor.accept().await {
            Ok((stream, _)) => {
                backoff = Duration::from_millis(10);
                let mgr = session_manager.clone();
                let cfg = config.clone();
                tokio::spawn(async move {
                    handle_connection(stream, mgr, cfg, permit).await;
                });
            }
            Err(err) => {
                eprintln!("bridge: accept error: {err}");
                drop(permit);
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(MAX_BACKOFF);
            }
        }
    }
}

/// Holds the accept-loop's connection permit until either this HTTP exchange finishes or, for
/// an upgraded connection, the WebSocket session it hands off to finishes. `take()` lets the
/// WebSocket branch claim the permit for its own (much longer) lifetime instead of releasing it
/// when the HTTP dispatch for the upgrade request completes.
type PermitCell = Arc<Mutex<Option<OwnedSemaphorePermit>>>;

async fn handle_connection(
    stream: TcpStream,
    sessions: SessionManager,
    config: ServerConfig,
    permit: OwnedSemaphorePermit,
) {
    let io = TokioIo::new(stream);
    let limits = config.limits;
    let permit_cell: PermitCell = Arc::new(Mutex::new(Some(permit)));
    let service = service_fn(move |req| {
        let sessions = sessions.clone();
        let config = config.clone();
        let permit_cell = permit_cell.clone();
        async move { route(req, sessions, config, permit_cell).await }
    });

    let _ = builder(&limits)
        .serve_connection(io, service)
        .with_upgrades()
        .await;
}

#[derive(Deserialize)]
struct ExecPayload {
    command: String,
    #[serde(default = "default_timeout")]
    timeout_seconds: u64,
    #[serde(default)]
    cwd: Option<String>,
}

fn default_timeout() -> u64 {
    30
}

/// Route a single request. `check_request` runs first for every method and path, before any
/// side effect; only requests that pass it reach a handler.
async fn route(
    req: Request<Incoming>,
    sessions: SessionManager,
    config: ServerConfig,
    permit_cell: PermitCell,
) -> Result<Response<Full<Bytes>>, Infallible> {
    let host = req.headers().get(HOST).and_then(|v| v.to_str().ok());
    let origin = req.headers().get(ORIGIN).and_then(|v| v.to_str().ok());
    let content_type = req
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok());
    let method = req.method().as_str().to_string();

    let echo_origin = match check_request(host, origin, &method, content_type, &config) {
        Ok(o) => o,
        Err(code) => return Ok(rejection_response(code)),
    };
    let allowed_origin = echo_origin.as_deref();

    if is_websocket_upgrade(req.headers()) {
        return Ok(handle_ws_upgrade(
            req,
            allowed_origin,
            sessions,
            config,
            permit_cell,
        ));
    }

    let path = req.uri().path().to_string();
    match (method.as_str(), path.as_str()) {
        ("OPTIONS", _) => Ok(preflight_response(req.headers(), allowed_origin)),
        ("GET", "/health") => Ok(respond(
            StatusCode::OK,
            r#"{"status":"ok"}"#,
            allowed_origin,
        )),
        ("POST", "/exec") => Ok(handle_exec(req, allowed_origin, &config).await),
        (m, p) if (m == "GET" || m == "POST") && p.starts_with("/git/") => {
            Ok(handle_git(req, allowed_origin, &config).await)
        }
        _ => Ok(respond(
            StatusCode::NOT_FOUND,
            r#"{"error":"not found"}"#,
            allowed_origin,
        )),
    }
}

/// Map a `check_request` rejection to a response. These never carry CORS headers: the request
/// failed the Host/Origin baseline, so there is no origin to trust.
fn rejection_response(code: u16) -> Response<Full<Bytes>> {
    let (status, body) = match code {
        403 => (StatusCode::FORBIDDEN, r#"{"error":"forbidden"}"#),
        415 => (
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            r#"{"error":"unsupported media type: application/json required"}"#,
        ),
        _ => (StatusCode::BAD_REQUEST, r#"{"error":"bad request"}"#),
    };
    respond(status, body, None)
}

fn preflight_response(headers: &HeaderMap, allowed_origin: Option<&str>) -> Response<Full<Bytes>> {
    let private_network = headers
        .get("access-control-request-private-network")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.eq_ignore_ascii_case("true"));

    let mut builder = Response::builder()
        .status(StatusCode::NO_CONTENT)
        .header("connection", "close")
        .header("access-control-allow-methods", "GET, POST, OPTIONS")
        .header(
            "access-control-allow-headers",
            "Content-Type, Authorization",
        )
        .header("access-control-max-age", "600");
    if let Some(origin) = allowed_origin {
        builder = builder
            .header("access-control-allow-origin", origin)
            .header("vary", "Origin");
    }
    if private_network {
        builder = builder.header("access-control-allow-private-network", "true");
    }
    builder
        .body(Full::new(Bytes::new()))
        .expect("static headers always produce a valid response")
}

fn is_websocket_upgrade(headers: &HeaderMap) -> bool {
    let is_upgrade = headers
        .get(UPGRADE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.eq_ignore_ascii_case("websocket"));
    let has_connection_upgrade = headers
        .get(CONNECTION)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.to_ascii_lowercase().contains("upgrade"));
    is_upgrade && has_connection_upgrade
}

/// A `Sec-WebSocket-Key` is always the base64 encoding of a 16-byte nonce: 24 characters, the
/// last two of which are the `==` padding forced by 16 not being a multiple of 3.
fn is_valid_ws_key(key: &str) -> bool {
    let bytes = key.as_bytes();
    bytes.len() == 24
        && bytes.ends_with(b"==")
        && bytes[..22]
            .iter()
            .all(|&b| b.is_ascii_alphanumeric() || b == b'+' || b == b'/')
}

fn handle_ws_upgrade(
    req: Request<Incoming>,
    allowed_origin: Option<&str>,
    sessions: SessionManager,
    config: ServerConfig,
    permit_cell: PermitCell,
) -> Response<Full<Bytes>> {
    let version_ok = req
        .headers()
        .get("sec-websocket-version")
        .and_then(|v| v.to_str().ok())
        == Some("13");
    let key = req
        .headers()
        .get("sec-websocket-key")
        .and_then(|v| v.to_str().ok())
        .filter(|k| version_ok && is_valid_ws_key(k))
        .map(str::to_string);

    let Some(key) = key else {
        return respond(
            StatusCode::BAD_REQUEST,
            r#"{"error":"invalid websocket upgrade"}"#,
            allowed_origin,
        );
    };
    let accept_key = derive_accept_key(key.as_bytes());

    let workspace_root = config.workspace_root.clone();
    tokio::spawn(async move {
        // Claim the connection's accept permit for the life of the WebSocket session, instead
        // of letting it release when the HTTP dispatch for this upgrade request completes: an
        // open WebSocket must keep counting against `max_connections`.
        let _permit = permit_cell
            .lock()
            .expect("permit cell mutex poisoned")
            .take();
        match hyper::upgrade::on(req).await {
            Ok(upgraded) => {
                let ws_config = WebSocketConfig::default()
                    .max_message_size(Some(MAX_WS_MESSAGE))
                    .max_frame_size(Some(MAX_WS_MESSAGE));
                let ws_stream = WebSocketStream::from_raw_socket(
                    TokioIo::new(upgraded),
                    Role::Server,
                    Some(ws_config),
                )
                .await;
                handle_websocket(ws_stream, sessions, workspace_root).await;
            }
            Err(err) => eprintln!("bridge: websocket upgrade failed: {err}"),
        }
    });

    let mut builder = Response::builder()
        .status(StatusCode::SWITCHING_PROTOCOLS)
        .header("upgrade", "websocket")
        .header("connection", "upgrade")
        .header("sec-websocket-accept", accept_key);
    if let Some(origin) = allowed_origin {
        builder = builder
            .header("access-control-allow-origin", origin)
            .header("vary", "Origin");
    }
    builder
        .body(Full::new(Bytes::new()))
        .expect("static headers always produce a valid response")
}

async fn handle_exec(
    req: Request<Incoming>,
    allowed_origin: Option<&str>,
    config: &ServerConfig,
) -> Response<Full<Bytes>> {
    let payload: ExecPayload = match read_json(req.into_body(), config.limits.max_body).await {
        Ok(p) => p,
        Err(HttpError::TooLarge) => {
            return respond(
                StatusCode::PAYLOAD_TOO_LARGE,
                r#"{"error":"payload too large"}"#,
                allowed_origin,
            );
        }
        Err(HttpError::BadRequest(msg)) => {
            return respond(
                StatusCode::BAD_REQUEST,
                &serde_json::json!({ "error": msg }).to_string(),
                allowed_origin,
            );
        }
    };

    let effective_cwd =
        match crate::paths::resolve_in_root(&config.workspace_root, payload.cwd.as_deref()) {
            Ok(p) => p,
            Err(err) => {
                return respond(
                    StatusCode::BAD_REQUEST,
                    &serde_json::json!({ "error": err }).to_string(),
                    allowed_origin,
                );
            }
        };

    let outcome =
        execute_command_direct(&payload.command, &effective_cwd, payload.timeout_seconds).await;
    match outcome {
        Ok(res) => respond(
            StatusCode::OK,
            &serde_json::to_string(&res).unwrap_or_default(),
            allowed_origin,
        ),
        Err(err) => respond(
            StatusCode::INTERNAL_SERVER_ERROR,
            &serde_json::json!({ "error": err }).to_string(),
            allowed_origin,
        ),
    }
}

async fn handle_git(
    req: Request<Incoming>,
    allowed_origin: Option<&str>,
    config: &ServerConfig,
) -> Response<Full<Bytes>> {
    let method = req.method().as_str().to_string();
    let path = req.uri().path().to_string();
    let is_get = method == "GET";

    let body_bytes = match read_body(req.into_body(), config.limits.max_body).await {
        Ok(b) => b,
        Err(HttpError::TooLarge) => {
            return respond(
                StatusCode::PAYLOAD_TOO_LARGE,
                r#"{"error":"payload too large"}"#,
                allowed_origin,
            );
        }
        Err(HttpError::BadRequest(msg)) => {
            return respond(
                StatusCode::BAD_REQUEST,
                &serde_json::json!({ "error": msg }).to_string(),
                allowed_origin,
            );
        }
    };

    let repo_dir_opt = if is_get && body_bytes.is_empty() {
        None
    } else {
        match serde_json::from_slice::<serde_json::Value>(&body_bytes) {
            Ok(v) => v
                .get("cwd")
                .and_then(|c| c.as_str().filter(|s| !s.is_empty()).map(String::from)),
            Err(err) => {
                return respond(
                    StatusCode::BAD_REQUEST,
                    &serde_json::json!({ "error": format!("invalid JSON: {err}") }).to_string(),
                    allowed_origin,
                );
            }
        }
    };

    let repo_dir = match repo_dir_opt {
        Some(d) => match crate::paths::resolve_in_root(&config.workspace_root, Some(&d)) {
            Ok(p) => p,
            Err(err) => {
                return respond(
                    StatusCode::BAD_REQUEST,
                    &serde_json::json!({ "error": err }).to_string(),
                    allowed_origin,
                );
            }
        },
        None => {
            if is_get {
                config.workspace_root.clone()
            } else {
                return respond(
                    StatusCode::BAD_REQUEST,
                    r#"{"error":"cwd is missing or empty"}"#,
                    allowed_origin,
                );
            }
        }
    };

    #[derive(Debug)]
    enum HandlerError {
        BadRequest(String),
        Internal(String),
    }

    impl From<crate::git::GitError> for HandlerError {
        fn from(err: crate::git::GitError) -> Self {
            match err {
                crate::git::GitError::Validation(e) => HandlerError::BadRequest(e),
                crate::git::GitError::Execution(e) => HandlerError::Internal(e),
            }
        }
    }

    let result: Result<String, HandlerError> = match (method.as_str(), path.as_str()) {
        ("GET", "/git/status") | ("POST", "/git/status") => crate::git::get_repo_status(&repo_dir)
            .await
            .map(|s| serde_json::to_string(&s).unwrap_or_default())
            .map_err(HandlerError::from),
        ("GET", "/git/diff") | ("POST", "/git/diff") => {
            #[derive(Deserialize, Default)]
            struct DiffReq {
                path: Option<String>,
            }
            let req: DiffReq = serde_json::from_slice(&body_bytes).unwrap_or_default();
            crate::git::get_repo_diff(&repo_dir, req.path.as_deref())
                .await
                .map(|d| serde_json::json!({ "diff": d }).to_string())
                .map_err(HandlerError::from)
        }
        ("GET", "/git/show") | ("POST", "/git/show") => {
            #[derive(Deserialize, Default)]
            struct ShowReq {
                path: Option<String>,
            }
            let req: ShowReq = serde_json::from_slice(&body_bytes).unwrap_or_default();
            match req.path {
                Some(p) => crate::git::get_file_at_head(&repo_dir, &p)
                    .await
                    .map(|c| serde_json::json!({ "content": c }).to_string())
                    .map_err(HandlerError::from),
                None => Err(HandlerError::BadRequest("missing path parameter".into())),
            }
        }
        ("GET", "/git/branches") | ("POST", "/git/branches") => {
            crate::git::get_repo_branches(&repo_dir)
                .await
                .map(|b| serde_json::to_string(&b).unwrap_or_default())
                .map_err(HandlerError::from)
        }
        ("POST", "/git/commit") => {
            match serde_json::from_slice::<openwebide_core::GitCommitRequest>(&body_bytes) {
                Ok(req) => crate::git::commit_changes(&repo_dir, &req)
                    .await
                    .map(|r| serde_json::to_string(&r).unwrap_or_default())
                    .map_err(HandlerError::from),
                Err(e) => Err(HandlerError::BadRequest(format!(
                    "invalid commit payload: {e}"
                ))),
            }
        }
        ("POST", "/git/checkout") => {
            match serde_json::from_slice::<openwebide_core::GitCheckoutRequest>(&body_bytes) {
                Ok(req) => crate::git::checkout_branch(&repo_dir, &req)
                    .await
                    .map(|r| serde_json::to_string(&r).unwrap_or_default())
                    .map_err(HandlerError::from),
                Err(e) => Err(HandlerError::BadRequest(format!(
                    "invalid checkout payload: {e}"
                ))),
            }
        }
        ("POST", "/git/sync") => {
            match serde_json::from_slice::<openwebide_core::GitSyncRequest>(&body_bytes) {
                Ok(req) => crate::git::sync_repo(&repo_dir, &req)
                    .await
                    .map(|r| serde_json::to_string(&r).unwrap_or_default())
                    .map_err(HandlerError::from),
                Err(e) => Err(HandlerError::BadRequest(format!(
                    "invalid sync payload: {e}"
                ))),
            }
        }
        _ => Err(HandlerError::BadRequest(format!(
            "unrecognized git route: {method} {path}"
        ))),
    };

    match result {
        Ok(json) => respond(StatusCode::OK, &json, allowed_origin),
        Err(HandlerError::BadRequest(err)) => respond(
            StatusCode::BAD_REQUEST,
            &serde_json::json!({ "error": err }).to_string(),
            allowed_origin,
        ),
        Err(HandlerError::Internal(err)) => respond(
            StatusCode::INTERNAL_SERVER_ERROR,
            &serde_json::json!({ "error": err }).to_string(),
            allowed_origin,
        ),
    }
}

async fn handle_websocket<S>(
    mut ws_stream: tokio_tungstenite::WebSocketStream<S>,
    sessions: SessionManager,
    workspace_root: PathBuf,
) where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    // If client attaches to a session, we subscribe to session broadcast
    let mut attached_rx: Option<tokio::sync::broadcast::Receiver<BridgeServerMessage>> = None;
    let mut attached_id: Option<String> = None;

    let mut ping_interval =
        tokio::time::interval_at(Instant::now() + WS_PING_INTERVAL, WS_PING_INTERVAL);
    ping_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut idle_deadline = Instant::now() + WS_IDLE_TIMEOUT;

    loop {
        tokio::select! {
            _ = ping_interval.tick() => {
                if ws_stream.send(Message::Ping(Bytes::new())).await.is_err() {
                    break;
                }
            }

            () = tokio::time::sleep_until(idle_deadline) => {
                break;
            }

            // Outbound message from attached session broadcast
            msg_opt = async {
                if let Some(rx) = attached_rx.as_mut() {
                    rx.recv().await.ok()
                } else {
                    std::future::pending().await
                }
            } => {
                if let Some(msg) = msg_opt
                    && let Ok(json_str) = serde_json::to_string(&msg)
                        && ws_stream.send(Message::Text(json_str.into())).await.is_err() {
                            break;
                        }
            }

            // Inbound WebSocket message from client
            inbound = ws_stream.next() => {
                let Some(msg_res) = inbound else { break };
                idle_deadline = Instant::now() + WS_IDLE_TIMEOUT;
                let msg = match msg_res {
                    Ok(m) => m,
                    Err(_) => break,
                };

                let text = match msg {
                    Message::Text(t) => t.to_string(),
                    Message::Binary(b) => String::from_utf8_lossy(&b).to_string(),
                    Message::Ping(data) => {
                        let _ = ws_stream.send(Message::Pong(data)).await;
                        continue;
                    }
                    Message::Close(_) => break,
                    _ => continue,
                };

                let client_msg: BridgeClientMessage = match serde_json::from_str(&text) {
                    Ok(m) => m,
                    Err(e) => {
                        let err = BridgeServerMessage::Error {
                            id: attached_id.clone().unwrap_or_default(),
                            message: format!("invalid message: {e}"),
                        };
                        let _ = send_server_msg(&mut ws_stream, &err).await;
                        continue;
                    }
                };

                match client_msg {
                    BridgeClientMessage::Spawn {
                        id,
                        command,
                        args,
                        cwd,
                        env,
                        pty,
                        cols,
                        rows,
                    } => {
                        let result = if pty {
                            spawn_pty(id.clone(), command.clone(), args, cwd, env, cols, rows, &workspace_root)
                        } else {
                            spawn_headless(id.clone(), command.clone(), args, cwd, env, &workspace_root)
                        };

                        match result {
                            Ok(session) => {
                                sessions.insert(session.clone());
                                let spawned = BridgeServerMessage::Spawned {
                                    id: id.clone(),
                                    pid: std::process::id(),
                                    pty,
                                };
                                let _ = send_server_msg(&mut ws_stream, &spawned).await;

                                // Automatically attach to live output stream
                                attached_id = Some(id.clone());
                                attached_rx = Some(session.broadcast_tx.subscribe());
                            }
                            Err(e) => {
                                let err = BridgeServerMessage::Error { id, message: e };
                                let _ = send_server_msg(&mut ws_stream, &err).await;
                            }
                        }
                    }

                    BridgeClientMessage::Input { id, data } => {
                        if let Some(sess) = sessions.get(&id) {
                            let _ = sess.stdin_tx.send(data).await;
                        }
                    }

                    BridgeClientMessage::Resize { id, cols, rows } => {
                        if let Some(sess) = sessions.get(&id) {
                            let _ = sess.resize_tx.send((cols, rows)).await;
                        }
                    }

                    BridgeClientMessage::Kill { id, signal } => {
                        if let Some(sess) = sessions.get(&id) {
                            let _ = sess.kill_tx.send(signal).await;
                        }
                    }

                    BridgeClientMessage::Attach { id, last_seq } => {
                        if let Some(sess) = sessions.get(&id) {
                            let (replays, rx) = sess.attach(last_seq);
                            attached_id = Some(id.clone());
                            attached_rx = Some(rx);

                            // Send buffered replay chunks first
                            for chunk in replays {
                                let _ = send_server_msg(&mut ws_stream, &chunk).await;
                            }
                        } else {
                            let err = BridgeServerMessage::Error {
                                id: id.clone(),
                                message: format!("session not found: {id}"),
                            };
                            let _ = send_server_msg(&mut ws_stream, &err).await;
                        }
                    }

                    BridgeClientMessage::List => {
                        let active = sessions.list();
                        let msg = BridgeServerMessage::Sessions { sessions: active };
                        let _ = send_server_msg(&mut ws_stream, &msg).await;
                    }
                }
            }
        }
    }
}

async fn send_server_msg<S>(
    ws_stream: &mut tokio_tungstenite::WebSocketStream<S>,
    msg: &BridgeServerMessage,
) -> Result<(), ()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    if let Ok(json_str) = serde_json::to_string(msg) {
        ws_stream
            .send(Message::Text(json_str.into()))
            .await
            .map_err(|_| ())
    } else {
        Err(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> ServerConfig {
        ServerConfig::new(PathBuf::from("/tmp"))
    }

    #[test]
    fn test_missing_or_empty_host_rejected() {
        let cfg = test_config();
        assert_eq!(check_request(None, None, "GET", None, &cfg), Err(403));
        assert_eq!(check_request(Some(""), None, "GET", None, &cfg), Err(403));
        assert_eq!(
            check_request(Some("   "), None, "GET", None, &cfg),
            Err(403)
        );
    }

    #[test]
    fn test_foreign_host_rejected() {
        let cfg = test_config();
        assert_eq!(
            check_request(Some("evil.example:3001"), None, "GET", None, &cfg),
            Err(403)
        );
        assert_eq!(
            check_request(Some("attacker.com"), None, "GET", None, &cfg),
            Err(403)
        );
    }

    #[test]
    fn test_ip_literal_hosts_allowed() {
        let cfg = test_config();
        assert!(check_request(Some("127.0.0.1:3001"), None, "GET", None, &cfg).is_ok());
        assert!(check_request(Some("192.168.1.100:3001"), None, "GET", None, &cfg).is_ok());
        assert!(check_request(Some("10.0.0.1"), None, "GET", None, &cfg).is_ok());
        assert!(check_request(Some("[::1]:3001"), None, "GET", None, &cfg).is_ok());
        assert!(check_request(Some("[2001:db8::1]:8080"), None, "GET", None, &cfg).is_ok());
    }

    #[test]
    fn test_default_hosts_allowed() {
        let cfg = test_config();
        assert!(check_request(Some("localhost:3001"), None, "GET", None, &cfg).is_ok());
        assert!(check_request(Some("LOCALHOST"), None, "GET", None, &cfg).is_ok());
        if let Some(h) = get_system_hostname() {
            assert!(check_request(Some(&format!("{h}:3001")), None, "GET", None, &cfg).is_ok());
            assert!(
                check_request(Some(&format!("{h}.local:3001")), None, "GET", None, &cfg).is_ok()
            );
        }
    }

    #[test]
    fn test_custom_allowed_host() {
        let mut cfg = test_config();
        cfg.allowed_hosts.push("my-tunnel.ts.net".to_string());
        assert!(check_request(Some("my-tunnel.ts.net:3001"), None, "GET", None, &cfg).is_ok());
        assert_eq!(
            check_request(Some("other-tunnel.ts.net:3001"), None, "GET", None, &cfg),
            Err(403)
        );
    }

    #[test]
    fn test_origin_null_rejected() {
        let cfg = test_config();
        assert_eq!(
            check_request(Some("127.0.0.1:3001"), Some("null"), "GET", None, &cfg),
            Err(403)
        );
        assert_eq!(
            check_request(Some("127.0.0.1:3001"), Some("NULL"), "GET", None, &cfg),
            Err(403)
        );
        assert_eq!(
            check_request(Some("127.0.0.1:3001"), Some(""), "GET", None, &cfg),
            Err(403)
        );
    }

    #[test]
    fn test_default_origins_allowed() {
        let cfg = test_config();
        let res = check_request(
            Some("127.0.0.1:3001"),
            Some("http://localhost:3000"),
            "GET",
            None,
            &cfg,
        );
        assert_eq!(res, Ok(Some("http://localhost:3000".to_string())));

        let res8080 = check_request(
            Some("127.0.0.1:3001"),
            Some("http://127.0.0.1:8080"),
            "GET",
            None,
            &cfg,
        );
        assert_eq!(res8080, Ok(Some("http://127.0.0.1:8080".to_string())));
    }

    #[test]
    fn test_same_host_origin_allowed() {
        let cfg = test_config();
        let res = check_request(
            Some("192.168.1.10:3001"),
            Some("http://192.168.1.10:3000"),
            "GET",
            None,
            &cfg,
        );
        assert_eq!(res, Ok(Some("http://192.168.1.10:3000".to_string())));
    }

    #[test]
    fn test_cross_ip_origin_rejected() {
        let cfg = test_config();
        let res = check_request(
            Some("192.168.1.10:3001"),
            Some("http://1.2.3.4"),
            "GET",
            None,
            &cfg,
        );
        assert_eq!(res, Err(403));
    }

    #[test]
    fn test_custom_allowed_origin() {
        let mut cfg = test_config();
        cfg.allowed_origins
            .push("https://custom.app.example".to_string());
        let res = check_request(
            Some("127.0.0.1:3001"),
            Some("https://custom.app.example"),
            "GET",
            None,
            &cfg,
        );
        assert_eq!(res, Ok(Some("https://custom.app.example".to_string())));
    }

    #[test]
    fn test_post_content_type_rules() {
        let cfg = test_config();
        // With Origin: application/json is required for POST
        assert_eq!(
            check_request(
                Some("127.0.0.1:3001"),
                Some("http://localhost:3000"),
                "POST",
                Some("application/json"),
                &cfg,
            ),
            Ok(Some("http://localhost:3000".to_string()))
        );
        assert_eq!(
            check_request(
                Some("127.0.0.1:3001"),
                Some("http://localhost:3000"),
                "POST",
                Some("application/json; charset=utf-8"),
                &cfg,
            ),
            Ok(Some("http://localhost:3000".to_string()))
        );
        assert_eq!(
            check_request(
                Some("127.0.0.1:3001"),
                Some("http://localhost:3000"),
                "POST",
                Some("text/plain"),
                &cfg,
            ),
            Err(415)
        );
        assert_eq!(
            check_request(
                Some("127.0.0.1:3001"),
                Some("http://localhost:3000"),
                "POST",
                Some("application/x-www-form-urlencoded"),
                &cfg,
            ),
            Err(415)
        );
        assert_eq!(
            check_request(
                Some("127.0.0.1:3001"),
                Some("http://localhost:3000"),
                "POST",
                None,
                &cfg,
            ),
            Err(415)
        );

        // Without Origin: POST without Content-Type or with any Content-Type is allowed (backend / curl)
        assert_eq!(
            check_request(Some("127.0.0.1:3001"), None, "POST", None, &cfg),
            Ok(None)
        );
        assert_eq!(
            check_request(
                Some("127.0.0.1:3001"),
                None,
                "POST",
                Some("text/plain"),
                &cfg,
            ),
            Ok(None)
        );
    }

    /// A listener that fails its first `accept` and delegates every later call to a real one.
    struct FlakyListener {
        inner: TcpListener,
        failed_once: std::sync::atomic::AtomicBool,
    }

    impl Accept for FlakyListener {
        async fn accept(&self) -> std::io::Result<(TcpStream, std::net::SocketAddr)> {
            if !self
                .failed_once
                .swap(true, std::sync::atomic::Ordering::SeqCst)
            {
                return Err(std::io::Error::other("simulated accept failure"));
            }
            self.inner.accept().await
        }
    }

    #[tokio::test]
    async fn accept_loop_survives_errors() {
        let inner = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = inner.local_addr().unwrap().port();
        let flaky = FlakyListener {
            inner,
            failed_once: std::sync::atomic::AtomicBool::new(false),
        };
        let config = ServerConfig::new(std::env::temp_dir());
        tokio::spawn(run_accept_loop(flaky, config));

        // Give the loop time to hit its fake error and back off before retrying.
        tokio::time::sleep(Duration::from_millis(50)).await;

        let connected = tokio::time::timeout(
            Duration::from_secs(1),
            TcpStream::connect(("127.0.0.1", port)),
        )
        .await;
        assert!(
            matches!(connected, Ok(Ok(_))),
            "accept loop should still be serving after a transient accept error"
        );
    }
}
