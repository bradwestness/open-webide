//! WebSocket and HTTP server for the process execution bridge daemon.

use std::io::Cursor;
use std::path::PathBuf;
use std::pin::Pin;
use std::task::{Context, Poll};

use futures::{SinkExt, StreamExt};
use openwebide_core::{BridgeClientMessage, BridgeServerMessage};
use serde::Deserialize;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::tungstenite::Message;

use crate::headless::{execute_command_direct, spawn_headless};
use crate::pty::spawn_pty;
use crate::session::SessionManager;

const MAX_HEAD_SIZE: usize = 8192;

/// Bridge server configuration.
#[derive(Clone, Debug)]
pub struct ServerConfig {
    pub workspace_root: PathBuf,
    pub allowed_origins: Vec<String>,
    pub allowed_hosts: Vec<String>,
}

impl ServerConfig {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self {
            workspace_root,
            allowed_origins: default_origins(),
            allowed_hosts: default_hosts(),
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

/// Extract a header value from raw HTTP request head by name (case-insensitive).
pub fn header<'a>(head: &'a str, name: &str) -> Option<&'a str> {
    for line in head.lines() {
        if let Some((k, v)) = line.split_once(':')
            && k.trim().eq_ignore_ascii_case(name)
        {
            return Some(v.trim());
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
    let session_manager = SessionManager::new();

    while let Ok((stream, _)) = listener.accept().await {
        let mgr = session_manager.clone();
        let cfg = config.clone();

        tokio::spawn(async move {
            handle_connection(stream, mgr, cfg).await;
        });
    }
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

/// A stream wrapper that yields bytes from an in-memory prefix buffer before delegating to an underlying stream.
struct PrefixedStream<S> {
    prefix: Cursor<Vec<u8>>,
    inner: S,
}

impl<S> PrefixedStream<S> {
    fn new(prefix: Vec<u8>, inner: S) -> Self {
        Self {
            prefix: Cursor::new(prefix),
            inner,
        }
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for PrefixedStream<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let this = self.get_mut();
        let prefix_len = this.prefix.get_ref().len();
        let pos = this.prefix.position() as usize;
        if pos < prefix_len {
            let available = &this.prefix.get_ref()[pos..];
            let amt = available.len().min(buf.remaining());
            buf.put_slice(&available[..amt]);
            this.prefix.set_position((pos + amt) as u64);
            Poll::Ready(Ok(()))
        } else {
            Pin::new(&mut this.inner).poll_read(cx, buf)
        }
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for PrefixedStream<S> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.get_mut().inner).poll_write(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[std::io::IoSlice<'_>],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.get_mut().inner).poll_write_vectored(cx, bufs)
    }

    fn is_write_vectored(&self) -> bool {
        self.inner.is_write_vectored()
    }
}

async fn handle_connection(mut stream: TcpStream, sessions: SessionManager, config: ServerConfig) {
    // Read incoming bytes into a buffer until the full HTTP header is received (\r\n\r\n)
    let mut head_buf = Vec::new();
    let mut temp = [0u8; 1024];
    loop {
        match stream.read(&mut temp).await {
            Ok(0) => {
                if head_buf.is_empty() {
                    return;
                }
                break;
            }
            Ok(k) => {
                head_buf.extend_from_slice(&temp[..k]);
                if let Some(pos) = head_buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    if pos + 4 > MAX_HEAD_SIZE {
                        let resp = "HTTP/1.1 431 Request Header Fields Too Large\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                        let _ = stream.write_all(resp.as_bytes()).await;
                        return;
                    }
                    break;
                }
                if head_buf.len() >= MAX_HEAD_SIZE {
                    let resp = "HTTP/1.1 431 Request Header Fields Too Large\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                    let _ = stream.write_all(resp.as_bytes()).await;
                    return;
                }
            }
            Err(_) => return,
        }
    }

    let header_str = String::from_utf8_lossy(&head_buf).into_owned();
    let mut stream = PrefixedStream::new(head_buf, stream);

    if header_str.contains("Upgrade: websocket") || header_str.contains("upgrade: websocket") {
        let cfg = config.clone();
        #[allow(clippy::result_large_err)]
        let callback =
            move |req: &tokio_tungstenite::tungstenite::handshake::server::Request,
                  mut resp: tokio_tungstenite::tungstenite::handshake::server::Response|
                  -> Result<
                tokio_tungstenite::tungstenite::handshake::server::Response,
                tokio_tungstenite::tungstenite::handshake::server::ErrorResponse,
            > {
                let host = req.headers().get("host").and_then(|v| v.to_str().ok());
                let origin = req.headers().get("origin").and_then(|v| v.to_str().ok());
                match check_request(host, origin, "GET", None, &cfg) {
                    Ok(echo_origin) => {
                        if let Some(orig) = echo_origin
                            && let Ok(val) = orig.parse()
                        {
                            resp.headers_mut()
                                .insert("access-control-allow-origin", val);
                            if let Ok(vary) = "Origin".parse() {
                                resp.headers_mut().insert("vary", vary);
                            }
                        }
                        Ok(resp)
                    }
                    Err(code) => {
                        let status =
                            tokio_tungstenite::tungstenite::http::StatusCode::from_u16(code)
                                .unwrap_or(
                                    tokio_tungstenite::tungstenite::http::StatusCode::FORBIDDEN,
                                );
                        let err =
                            tokio_tungstenite::tungstenite::handshake::server::Response::builder()
                                .status(status)
                                .body(Some("Forbidden".to_string()))
                                .unwrap();
                        Err(err)
                    }
                }
            };

        if let Ok(ws_stream) = tokio_tungstenite::accept_hdr_async(stream, callback).await {
            handle_websocket(ws_stream, sessions, config.workspace_root).await;
        }
    } else if header_str.starts_with("OPTIONS ") {
        let head = match header_str.split_once("\r\n\r\n") {
            Some((h, _)) => h,
            None => &header_str,
        };
        let host = header(head, "Host");
        let origin = header(head, "Origin");
        let req_pna = header(head, "Access-Control-Request-Private-Network")
            .map(|v| v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);

        match check_request(host, origin, "OPTIONS", None, &config) {
            Ok(echo_origin) => {
                let mut resp = String::from("HTTP/1.1 204 No Content\r\n");
                if let Some(ref orig) = echo_origin {
                    resp.push_str(&format!(
                        "Access-Control-Allow-Origin: {orig}\r\nVary: Origin\r\n"
                    ));
                }
                resp.push_str("Access-Control-Allow-Methods: GET, POST, OPTIONS\r\n");
                resp.push_str("Access-Control-Allow-Headers: Content-Type, Authorization\r\n");
                resp.push_str("Access-Control-Max-Age: 600\r\n");
                if req_pna {
                    resp.push_str("Access-Control-Allow-Private-Network: true\r\n");
                }
                resp.push_str("Content-Length: 0\r\n\r\n");
                let _ = stream.write_all(resp.as_bytes()).await;
            }
            Err(status_code) => {
                let resp = format!("HTTP/1.1 {status_code} Forbidden\r\nContent-Length: 0\r\n\r\n");
                let _ = stream.write_all(resp.as_bytes()).await;
            }
        }
    } else if header_str.starts_with("POST /exec") {
        handle_http_exec(stream, &config).await;
    } else if header_str.starts_with("GET /git/") || header_str.starts_with("POST /git/") {
        let (method, path) = if let Some(rest) = header_str.strip_prefix("GET ") {
            let path = rest.split_whitespace().next().unwrap_or("/git/status");
            ("GET", path)
        } else {
            let rest = header_str.strip_prefix("POST ").unwrap_or(&header_str);
            let path = rest.split_whitespace().next().unwrap_or("/git/status");
            ("POST", path)
        };
        handle_http_git(stream, &config, method, path).await;
    } else if header_str.starts_with("GET /health") {
        let head = match header_str.split_once("\r\n\r\n") {
            Some((h, _)) => h,
            None => &header_str,
        };
        let host = header(head, "Host");
        let origin = header(head, "Origin");
        match check_request(host, origin, "GET", None, &config) {
            Ok(echo_origin) => {
                let mut resp =
                    String::from("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n");
                if let Some(ref orig) = echo_origin {
                    resp.push_str(&format!(
                        "Access-Control-Allow-Origin: {orig}\r\nVary: Origin\r\n"
                    ));
                }
                let body = "{\"status\":\"ok\"}";
                resp.push_str(&format!("Content-Length: {}\r\n\r\n{}", body.len(), body));
                let _ = stream.write_all(resp.as_bytes()).await;
            }
            Err(status_code) => {
                let resp = format!("HTTP/1.1 {status_code} Forbidden\r\nContent-Length: 0\r\n\r\n");
                let _ = stream.write_all(resp.as_bytes()).await;
            }
        }
    } else {
        let resp = "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n";
        let _ = stream.write_all(resp.as_bytes()).await;
    }
}

async fn handle_http_exec<S>(mut stream: S, config: &ServerConfig)
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut buf = Vec::with_capacity(4096);
    let mut temp = [0u8; 1024];

    // Read full HTTP request
    loop {
        match stream.read(&mut temp).await {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&temp[..n]);
                if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
                if buf.len() > MAX_HEAD_SIZE {
                    let resp = "HTTP/1.1 431 Request Header Fields Too Large\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                    let _ = stream.write_all(resp.as_bytes()).await;
                    return;
                }
            }
            Err(_) => return,
        }
    }

    let req_str = String::from_utf8_lossy(&buf);
    let (head, body) = match req_str.split_once("\r\n\r\n") {
        Some((h, b)) => (h, b),
        None => ("", ""),
    };

    let host = header(head, "Host");
    let origin = header(head, "Origin");
    let content_type = header(head, "Content-Type");

    let echo_origin = match check_request(host, origin, "POST", content_type, config) {
        Ok(o) => o,
        Err(status_code) => {
            let (status_text, err_body) = match status_code {
                403 => ("403 Forbidden", "{\"error\":\"forbidden\"}"),
                415 => (
                    "415 Unsupported Media Type",
                    "{\"error\":\"unsupported media type: application/json required\"}",
                ),
                _ => ("400 Bad Request", "{\"error\":\"bad request\"}"),
            };
            let resp = format!(
                "HTTP/1.1 {status_text}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                err_body.len(),
                err_body
            );
            let _ = stream.write_all(resp.as_bytes()).await;
            return;
        }
    };

    let payload: ExecPayload = match serde_json::from_str(body) {
        Ok(p) => p,
        Err(e) => {
            let mut cors_headers = String::new();
            if let Some(ref orig) = echo_origin {
                cors_headers = format!("Access-Control-Allow-Origin: {orig}\r\nVary: Origin\r\n");
            }
            let err_json = serde_json::json!({ "error": format!("invalid JSON: {e}") }).to_string();
            let resp = format!(
                "HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\n{cors_headers}Content-Length: {}\r\n\r\n{}",
                err_json.len(),
                err_json
            );
            let _ = stream.write_all(resp.as_bytes()).await;
            return;
        }
    };

    let effective_cwd = match crate::paths::resolve_in_root(
        &config.workspace_root,
        payload.cwd.as_deref(),
    ) {
        Ok(p) => p,
        Err(err) => {
            let err_json = serde_json::json!({ "error": err }).to_string();
            let mut cors_headers = String::new();
            if let Some(ref orig) = echo_origin {
                cors_headers = format!("Access-Control-Allow-Origin: {orig}\r\nVary: Origin\r\n");
            }
            let resp = format!(
                "HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\n{cors_headers}Content-Length: {}\r\n\r\n{}",
                err_json.len(),
                err_json
            );
            let _ = stream.write_all(resp.as_bytes()).await;
            return;
        }
    };

    let outcome =
        execute_command_direct(&payload.command, &effective_cwd, payload.timeout_seconds).await;
    let (status, resp_body) = match outcome {
        Ok(res) => ("200 OK", serde_json::to_string(&res).unwrap_or_default()),
        Err(err) => (
            "500 Internal Server Error",
            serde_json::json!({ "error": err }).to_string(),
        ),
    };

    let mut cors_headers = String::new();
    if let Some(ref orig) = echo_origin {
        cors_headers = format!("Access-Control-Allow-Origin: {orig}\r\nVary: Origin\r\n");
    }

    let resp = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\n{cors_headers}Content-Length: {}\r\n\r\n{}",
        resp_body.len(),
        resp_body
    );
    let _ = stream.write_all(resp.as_bytes()).await;
}

async fn handle_http_git<S>(mut stream: S, config: &ServerConfig, method: &str, path: &str)
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut buf = Vec::with_capacity(4096);
    let mut temp = [0u8; 1024];

    loop {
        match stream.read(&mut temp).await {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&temp[..n]);
                if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
                if buf.len() > MAX_HEAD_SIZE {
                    let resp = "HTTP/1.1 431 Request Header Fields Too Large\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                    let _ = stream.write_all(resp.as_bytes()).await;
                    return;
                }
            }
            Err(_) => return,
        }
    }

    let req_str = String::from_utf8_lossy(&buf);
    let (head, body) = match req_str.split_once("\r\n\r\n") {
        Some((h, b)) => (h, b),
        None => ("", ""),
    };

    let host = header(head, "Host");
    let origin = header(head, "Origin");
    let content_type = header(head, "Content-Type");

    let echo_origin = match check_request(host, origin, method, content_type, config) {
        Ok(o) => o,
        Err(status_code) => {
            let (status_text, err_body) = match status_code {
                403 => ("403 Forbidden", "{\"error\":\"forbidden\"}"),
                415 => (
                    "415 Unsupported Media Type",
                    "{\"error\":\"unsupported media type: application/json required\"}",
                ),
                _ => ("400 Bad Request", "{\"error\":\"bad request\"}"),
            };
            let resp = format!(
                "HTTP/1.1 {status_text}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                err_body.len(),
                err_body
            );
            let _ = stream.write_all(resp.as_bytes()).await;
            return;
        }
    };

    let repo_dir_opt = if method == "GET" && body.trim().is_empty() {
        None
    } else {
        match serde_json::from_str::<serde_json::Value>(body) {
            Ok(v) => v
                .get("cwd")
                .and_then(|c| c.as_str().filter(|s| !s.is_empty()).map(String::from)),
            Err(e) => {
                let err_json =
                    serde_json::json!({ "error": format!("invalid JSON: {e}") }).to_string();
                let mut cors_headers = String::new();
                if let Some(ref orig) = echo_origin {
                    cors_headers =
                        format!("Access-Control-Allow-Origin: {orig}\r\nVary: Origin\r\n");
                }
                let resp = format!(
                    "HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\n{cors_headers}Content-Length: {}\r\n\r\n{}",
                    err_json.len(),
                    err_json
                );
                let _ = stream.write_all(resp.as_bytes()).await;
                return;
            }
        }
    };

    let repo_dir = match repo_dir_opt {
        Some(d) => match crate::paths::resolve_in_root(&config.workspace_root, Some(&d)) {
            Ok(p) => p,
            Err(e) => {
                let err_json = serde_json::json!({ "error": e }).to_string();
                let mut cors_headers = String::new();
                if let Some(ref orig) = echo_origin {
                    cors_headers =
                        format!("Access-Control-Allow-Origin: {orig}\r\nVary: Origin\r\n");
                }
                let resp = format!(
                    "HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\n{cors_headers}Content-Length: {}\r\n\r\n{}",
                    err_json.len(),
                    err_json
                );
                let _ = stream.write_all(resp.as_bytes()).await;
                return;
            }
        },
        None => {
            if method == "GET" {
                config.workspace_root.clone()
            } else {
                let err_json =
                    serde_json::json!({ "error": "cwd is missing or empty" }).to_string();
                let mut cors_headers = String::new();
                if let Some(ref orig) = echo_origin {
                    cors_headers =
                        format!("Access-Control-Allow-Origin: {orig}\r\nVary: Origin\r\n");
                }
                let resp = format!(
                    "HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\n{cors_headers}Content-Length: {}\r\n\r\n{}",
                    err_json.len(),
                    err_json
                );
                let _ = stream.write_all(resp.as_bytes()).await;
                return;
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

    let result: Result<String, HandlerError> = match (method, path) {
        ("GET", "/git/status") | ("POST", "/git/status") => crate::git::get_repo_status(&repo_dir)
            .await
            .map(|s| serde_json::to_string(&s).unwrap_or_default())
            .map_err(HandlerError::from),
        ("GET", "/git/diff") | ("POST", "/git/diff") => {
            #[derive(Deserialize, Default)]
            struct DiffReq {
                path: Option<String>,
            }
            let req: DiffReq = serde_json::from_str(body).unwrap_or_default();
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
            let req: ShowReq = serde_json::from_str(body).unwrap_or_default();
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
            match serde_json::from_str::<openwebide_core::GitCommitRequest>(body) {
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
            match serde_json::from_str::<openwebide_core::GitCheckoutRequest>(body) {
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
            match serde_json::from_str::<openwebide_core::GitSyncRequest>(body) {
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

    let (status, resp_body) = match result {
        Ok(json) => ("200 OK", json),
        Err(HandlerError::BadRequest(err)) => (
            "400 Bad Request",
            serde_json::json!({ "error": err }).to_string(),
        ),
        Err(HandlerError::Internal(err)) => (
            "500 Internal Server Error",
            serde_json::json!({ "error": err }).to_string(),
        ),
    };

    let mut cors_headers = String::new();
    if let Some(ref orig) = echo_origin {
        cors_headers = format!("Access-Control-Allow-Origin: {orig}\r\nVary: Origin\r\n");
    }

    let resp = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\n{cors_headers}Access-Control-Allow-Methods: GET, POST, OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type\r\nContent-Length: {}\r\n\r\n{}",
        resp_body.len(),
        resp_body
    );
    let _ = stream.write_all(resp.as_bytes()).await;
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

    loop {
        tokio::select! {
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
}
