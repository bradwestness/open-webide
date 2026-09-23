//! WebSocket and HTTP server for the process execution bridge daemon.

use std::path::{Path, PathBuf};

use futures::{SinkExt, StreamExt};
use openwebide_core::{BridgeClientMessage, BridgeServerMessage};
use serde::Deserialize;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::tungstenite::Message;

use crate::headless::{execute_command_direct, spawn_headless};
use crate::pty::spawn_pty;
use crate::session::SessionManager;

/// Bridge server configuration.
#[derive(Clone)]
pub struct ServerConfig {
    pub workspace_root: PathBuf,
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
}

fn default_timeout() -> u64 {
    30
}

async fn handle_connection(mut stream: TcpStream, sessions: SessionManager, config: ServerConfig) {
    // Peek incoming bytes to distinguish WebSocket upgrade from plain HTTP POST /exec
    let mut peek_buf = [0u8; 1024];
    let n = match stream.peek(&mut peek_buf).await {
        Ok(n) if n > 0 => n,
        _ => return,
    };

    let header_str = String::from_utf8_lossy(&peek_buf[..n]);

    if header_str.contains("Upgrade: websocket") || header_str.contains("upgrade: websocket") {
        if let Ok(ws_stream) = tokio_tungstenite::accept_async(stream).await {
            handle_websocket(ws_stream, sessions, config.workspace_root).await;
        }
    } else if header_str.starts_with("OPTIONS ") {
        let resp = "HTTP/1.1 204 No Content\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: POST, GET, OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type\r\nAccess-Control-Max-Age: 86400\r\nContent-Length: 0\r\n\r\n";
        let _ = stream.write_all(resp.as_bytes()).await;
    } else if header_str.starts_with("POST /exec") {
        handle_http_exec(stream, &config.workspace_root).await;
    } else if header_str.starts_with("GET /git/") || header_str.starts_with("POST /git/") {
        let (method, path) = if let Some(rest) = header_str.strip_prefix("GET ") {
            let path = rest.split_whitespace().next().unwrap_or("/git/status");
            ("GET", path)
        } else {
            let rest = header_str.strip_prefix("POST ").unwrap_or(&header_str);
            let path = rest.split_whitespace().next().unwrap_or("/git/status");
            ("POST", path)
        };
        handle_http_git(stream, &config.workspace_root, method, path).await;
    } else if header_str.starts_with("GET /health") {
        let resp = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: 15\r\n\r\n{\"status\":\"ok\"}";
        let _ = stream.write_all(resp.as_bytes()).await;
    } else {
        let resp = "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n";
        let _ = stream.write_all(resp.as_bytes()).await;
    }
}

async fn handle_http_exec(mut stream: TcpStream, workspace_root: &Path) {
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
            }
            Err(_) => return,
        }
    }

    let req_str = String::from_utf8_lossy(&buf);
    let body = match req_str.split_once("\r\n\r\n") {
        Some((_, b)) => b,
        None => "",
    };

    let payload: ExecPayload = match serde_json::from_str(body) {
        Ok(p) => p,
        Err(e) => {
            let err_json = serde_json::json!({ "error": format!("invalid JSON: {e}") }).to_string();
            let resp = format!(
                "HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: {}\r\n\r\n{}",
                err_json.len(),
                err_json
            );
            let _ = stream.write_all(resp.as_bytes()).await;
            return;
        }
    };

    let outcome =
        execute_command_direct(&payload.command, workspace_root, payload.timeout_seconds).await;
    let (status, resp_body) = match outcome {
        Ok(res) => ("200 OK", serde_json::to_string(&res).unwrap_or_default()),
        Err(err) => (
            "500 Internal Server Error",
            serde_json::json!({ "error": err }).to_string(),
        ),
    };

    let resp = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: {}\r\n\r\n{}",
        resp_body.len(),
        resp_body
    );
    let _ = stream.write_all(resp.as_bytes()).await;
}

async fn handle_http_git(mut stream: TcpStream, workspace_root: &Path, method: &str, path: &str) {
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
            }
            Err(_) => return,
        }
    }

    let req_str = String::from_utf8_lossy(&buf);
    let body = match req_str.split_once("\r\n\r\n") {
        Some((_, b)) => b,
        None => "",
    };

    let repo_dir = match serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| {
            v.get("cwd")
                .and_then(|c| c.as_str().map(std::path::PathBuf::from))
        }) {
        Some(d) => {
            if d.is_absolute() && d.exists() {
                d
            } else if workspace_root.join(&d).exists() {
                workspace_root.join(&d)
            } else {
                workspace_root.to_path_buf()
            }
        }
        None => workspace_root.to_path_buf(),
    };

    let result: Result<String, String> = match (method, path) {
        ("GET", "/git/status") | ("POST", "/git/status") => crate::git::get_repo_status(&repo_dir)
            .await
            .map(|s| serde_json::to_string(&s).unwrap_or_default()),
        ("GET", "/git/diff") | ("POST", "/git/diff") => {
            #[derive(Deserialize, Default)]
            struct DiffReq {
                path: Option<String>,
            }
            let req: DiffReq = serde_json::from_str(body).unwrap_or_default();
            crate::git::get_repo_diff(&repo_dir, req.path.as_deref())
                .await
                .map(|d| serde_json::json!({ "diff": d }).to_string())
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
                    .map(|c| serde_json::json!({ "content": c }).to_string()),
                None => Err("missing path parameter".into()),
            }
        }
        ("GET", "/git/branches") | ("POST", "/git/branches") => {
            crate::git::get_repo_branches(&repo_dir)
                .await
                .map(|b| serde_json::to_string(&b).unwrap_or_default())
        }
        ("POST", "/git/commit") => {
            match serde_json::from_str::<openwebide_core::GitCommitRequest>(body) {
                Ok(req) => crate::git::commit_changes(&repo_dir, &req)
                    .await
                    .map(|r| serde_json::to_string(&r).unwrap_or_default()),
                Err(e) => Err(format!("invalid commit payload: {e}")),
            }
        }
        ("POST", "/git/checkout") => {
            match serde_json::from_str::<openwebide_core::GitCheckoutRequest>(body) {
                Ok(req) => crate::git::checkout_branch(&repo_dir, &req)
                    .await
                    .map(|r| serde_json::to_string(&r).unwrap_or_default()),
                Err(e) => Err(format!("invalid checkout payload: {e}")),
            }
        }
        ("POST", "/git/sync") => {
            let req: openwebide_core::GitSyncRequest =
                serde_json::from_str(body).unwrap_or(openwebide_core::GitSyncRequest {
                    action: "sync".into(),
                    remote: None,
                    branch: None,
                });
            crate::git::sync_repo(&repo_dir, &req)
                .await
                .map(|r| serde_json::to_string(&r).unwrap_or_default())
        }
        _ => Err(format!("unrecognized git route: {method} {path}")),
    };

    let (status, resp_body) = match result {
        Ok(json) => ("200 OK", json),
        Err(err) => (
            "500 Internal Server Error",
            serde_json::json!({ "error": err }).to_string(),
        ),
    };

    let resp = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type\r\nContent-Length: {}\r\n\r\n{}",
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
