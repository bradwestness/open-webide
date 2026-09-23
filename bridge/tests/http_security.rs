//! Integration tests for bridge HTTP and WebSocket security baselines.

mod common;

use common::{HttpResponse, TestDir, http, post, start, ws_with_headers};

#[tokio::test]
async fn evil_origin_text_plain_exec_is_rejected() {
    let test_dir = TestDir::new();
    let port = start(test_dir.path.clone()).await;
    let marker = test_dir.path.join("marker_evil");
    let payload = serde_json::json!({
        "command": format!("touch {}", marker.display()),
    })
    .to_string();

    let resp = post(
        port,
        "/exec",
        &payload,
        &[
            ("Origin", "http://evil.example"),
            ("Content-Type", "text/plain"),
        ],
    )
    .await;

    assert_eq!(resp.status, 403);
    assert!(!marker.exists());
}

#[tokio::test]
async fn allowed_origin_requires_json() {
    let test_dir = TestDir::new();
    let port = start(test_dir.path.clone()).await;
    let marker = test_dir.path.join("marker_text_plain");
    let payload = serde_json::json!({
        "command": format!("touch {}", marker.display()),
    })
    .to_string();

    let resp = post(
        port,
        "/exec",
        &payload,
        &[
            ("Origin", "http://localhost:3000"),
            ("Content-Type", "text/plain"),
        ],
    )
    .await;

    assert_eq!(resp.status, 415);
    assert!(!marker.exists());
}

#[tokio::test]
async fn allowed_origin_json_echoes_origin() {
    let test_dir = TestDir::new();
    let port = start(test_dir.path.clone()).await;
    let marker = test_dir.path.join("marker_allowed_json");
    let payload = serde_json::json!({
        "command": format!("touch {}", marker.display()),
    })
    .to_string();

    let resp = post(
        port,
        "/exec",
        &payload,
        &[
            ("Origin", "http://localhost:3000"),
            ("Content-Type", "application/json"),
        ],
    )
    .await;

    assert_eq!(resp.status, 200);
    assert!(marker.exists());
    let acao = resp.header("Access-Control-Allow-Origin");
    assert_eq!(acao, Some("http://localhost:3000"));
    assert_ne!(acao, Some("*"));
    assert_eq!(resp.header("Vary"), Some("Origin"));
}

#[tokio::test]
async fn no_origin_is_allowed() {
    let test_dir = TestDir::new();
    let port = start(test_dir.path.clone()).await;
    let marker = test_dir.path.join("marker_no_origin");
    let payload = serde_json::json!({
        "command": format!("touch {}", marker.display()),
    })
    .to_string();

    let resp = post(port, "/exec", &payload, &[]).await;

    assert_eq!(resp.status, 200);
    assert!(marker.exists());
    assert!(resp.header("Access-Control-Allow-Origin").is_none());
}

#[tokio::test]
async fn foreign_host_rejected() {
    let test_dir = TestDir::new();
    let port = start(test_dir.path.clone()).await;
    let marker = test_dir.path.join("marker_foreign_host");
    let payload = serde_json::json!({
        "command": format!("touch {}", marker.display()),
    })
    .to_string();

    let resp = post(
        port,
        "/exec",
        &payload,
        &[
            ("Host", "evil.example:3001"),
            ("Content-Type", "application/json"),
        ],
    )
    .await;

    assert_eq!(resp.status, 403);
    assert!(!marker.exists());
}

#[tokio::test]
async fn ip_literal_host_allowed() {
    let test_dir = TestDir::new();
    let port = start(test_dir.path.clone()).await;
    let marker = test_dir.path.join("marker_ip_literal");
    let payload = serde_json::json!({
        "command": format!("touch {}", marker.display()),
    })
    .to_string();

    let resp = post(
        port,
        "/exec",
        &payload,
        &[
            ("Host", &format!("127.0.0.1:{port}")),
            ("Content-Type", "application/json"),
        ],
    )
    .await;

    assert_eq!(resp.status, 200);
    assert!(marker.exists());
}

#[tokio::test]
async fn same_host_origin_allowed() {
    let test_dir = TestDir::new();
    let port = start(test_dir.path.clone()).await;
    let marker = test_dir.path.join("marker_same_host");
    let payload = serde_json::json!({
        "command": format!("touch {}", marker.display()),
    })
    .to_string();

    let resp = post(
        port,
        "/exec",
        &payload,
        &[
            ("Host", "192.168.1.10:3001"),
            ("Origin", "http://192.168.1.10:3000"),
            ("Content-Type", "application/json"),
        ],
    )
    .await;

    assert_eq!(resp.status, 200);
    assert!(marker.exists());
    assert_eq!(
        resp.header("Access-Control-Allow-Origin"),
        Some("http://192.168.1.10:3000")
    );
}

#[tokio::test]
async fn cross_ip_origin_rejected() {
    let test_dir = TestDir::new();
    let port = start(test_dir.path.clone()).await;
    let marker = test_dir.path.join("marker_cross_ip");
    let payload = serde_json::json!({
        "command": format!("touch {}", marker.display()),
    })
    .to_string();

    let resp = post(
        port,
        "/exec",
        &payload,
        &[
            ("Host", "192.168.1.10:3001"),
            ("Origin", "http://1.2.3.4"),
            ("Content-Type", "application/json"),
        ],
    )
    .await;

    assert_eq!(resp.status, 403);
    assert!(!marker.exists());
}

#[tokio::test]
async fn own_hostname_allowed() {
    let test_dir = TestDir::new();
    let port = start(test_dir.path.clone()).await;
    if let Some(h) = openwebide_bridge::server::get_system_hostname() {
        let marker = test_dir.path.join("marker_own_host");
        let payload = serde_json::json!({
            "command": format!("touch {}", marker.display()),
        })
        .to_string();

        let resp = post(
            port,
            "/exec",
            &payload,
            &[
                ("Host", &format!("{h}:{port}")),
                ("Content-Type", "application/json"),
            ],
        )
        .await;

        assert_eq!(resp.status, 200);
        assert!(marker.exists());
    }
}

#[tokio::test]
async fn preflight_disallowed_origin_403() {
    let test_dir = TestDir::new();
    let port = start(test_dir.path.clone()).await;

    let req = format!(
        "OPTIONS /exec HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nOrigin: http://evil.example\r\n\r\n"
    );
    let raw = http(port, &req).await;
    let resp = HttpResponse::parse(&raw);

    assert_eq!(resp.status, 403);
}

#[tokio::test]
async fn preflight_allowed_origin_204() {
    let test_dir = TestDir::new();
    let port = start(test_dir.path.clone()).await;

    let req = format!(
        "OPTIONS /exec HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nOrigin: http://localhost:3000\r\nAccess-Control-Request-Private-Network: true\r\n\r\n"
    );
    let raw = http(port, &req).await;
    let resp = HttpResponse::parse(&raw);

    assert_eq!(resp.status, 204);
    assert_eq!(
        resp.header("Access-Control-Allow-Origin"),
        Some("http://localhost:3000")
    );
    assert_eq!(resp.header("Vary"), Some("Origin"));
    assert_eq!(
        resp.header("Access-Control-Allow-Methods"),
        Some("GET, POST, OPTIONS")
    );
    assert_eq!(
        resp.header("Access-Control-Allow-Headers"),
        Some("Content-Type, Authorization")
    );
    assert_eq!(resp.header("Access-Control-Max-Age"), Some("600"));
    assert_eq!(
        resp.header("Access-Control-Allow-Private-Network"),
        Some("true")
    );
}

#[tokio::test]
async fn ws_foreign_origin_handshake_fails() {
    let test_dir = TestDir::new();
    let port = start(test_dir.path.clone()).await;

    let res = ws_with_headers(port, &[("Origin", "http://evil.example")]).await;
    assert!(
        res.is_err(),
        "foreign origin WS connection should fail handshake"
    );
}

#[tokio::test]
async fn ws_allowed_origin_ok() {
    let test_dir = TestDir::new();
    let port = start(test_dir.path.clone()).await;

    let res = ws_with_headers(port, &[("Origin", "http://localhost:3000")]).await;
    assert!(res.is_ok(), "allowed origin WS connection should succeed");
}

#[tokio::test]
async fn head_exceeding_buffer_431() {
    let test_dir = TestDir::new();
    let port = start(test_dir.path.clone()).await;

    // Send a header exceeding MAX_HEAD_SIZE (8192 bytes)
    let huge_header = "X-Large: ".to_string() + &"a".repeat(9000);
    let req = format!("GET /health HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n{huge_header}\r\n\r\n");
    let raw = http(port, &req).await;
    let resp = HttpResponse::parse(&raw);

    assert_eq!(resp.status, 431);
}

#[tokio::test]
async fn fragmented_preflight_request_succeeds() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;

    let test_dir = TestDir::new();
    let port = start(test_dir.path.clone()).await;

    let mut stream = TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("failed to connect to bridge");

    // Send first packet without \r\n\r\n
    stream
        .write_all(b"OPTIONS /exec HTTP/1.1\r\n")
        .await
        .expect("failed to write first chunk");
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

    // Send remaining headers
    let rest = format!(
        "Host: 127.0.0.1:{port}\r\nOrigin: http://localhost:3000\r\nAccess-Control-Request-Private-Network: true\r\n\r\n"
    );
    stream
        .write_all(rest.as_bytes())
        .await
        .expect("failed to write second chunk");

    let mut resp = Vec::new();
    let _ = stream.read_to_end(&mut resp).await;
    let parsed = HttpResponse::parse(&String::from_utf8_lossy(&resp));

    assert_eq!(parsed.status, 204);
    assert_eq!(
        parsed.header("Access-Control-Allow-Origin"),
        Some("http://localhost:3000")
    );
}

#[tokio::test]
async fn fragmented_websocket_handshake_succeeds() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;

    let test_dir = TestDir::new();
    let port = start(test_dir.path.clone()).await;

    let mut stream = TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("failed to connect to bridge");

    // Send first chunk without Upgrade: websocket
    stream
        .write_all(b"GET / HTTP/1.1\r\n")
        .await
        .expect("failed to write first chunk");
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

    // Send remaining handshake headers
    let rest = format!(
        "Host: 127.0.0.1:{port}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\nOrigin: http://localhost:3000\r\n\r\n"
    );
    stream
        .write_all(rest.as_bytes())
        .await
        .expect("failed to write second chunk");

    let mut buf = [0u8; 1024];
    let n = stream
        .read(&mut buf)
        .await
        .expect("failed to read ws handshake response");
    let resp_str = String::from_utf8_lossy(&buf[..n]);
    assert!(
        resp_str.starts_with("HTTP/1.1 101 Switching Protocols"),
        "expected 101 Switching Protocols, got: {resp_str}"
    );
}

#[tokio::test]
async fn fragmented_health_request_succeeds() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;

    let test_dir = TestDir::new();
    let port = start(test_dir.path.clone()).await;

    let mut stream = TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("failed to connect to bridge");

    // Send first chunk without \r\n\r\n
    stream
        .write_all(b"GET /health HTTP/1.1\r\n")
        .await
        .expect("failed to write first chunk");
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

    // Send remaining headers
    let rest = format!("Host: 127.0.0.1:{port}\r\n\r\n");
    stream
        .write_all(rest.as_bytes())
        .await
        .expect("failed to write second chunk");

    let mut resp = Vec::new();
    let _ = stream.read_to_end(&mut resp).await;
    let parsed = HttpResponse::parse(&String::from_utf8_lossy(&resp));

    assert_eq!(parsed.status, 200);
    assert_eq!(parsed.body, "{\"status\":\"ok\"}");
}
