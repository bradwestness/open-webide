//! Integration tests for bridge process lifecycle: killing the command's process group when the
//! `/exec` client disconnects mid-request.

mod common;

use std::time::Duration;

use common::{TestDir, start};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;

#[tokio::test]
async fn exec_client_disconnect_kills_group() {
    let test_dir = TestDir::new();
    let port = start(test_dir.path.clone()).await;
    // `direct` alone proves nothing about a process-group kill: `kill_on_drop` on the immediate
    // `sh` child would produce the same result. `grandchild` is only reachable by signalling the
    // whole group (what `GroupGuard` does), since a plain child-kill never touches it.
    let direct = test_dir.path.join("direct");
    let grandchild = test_dir.path.join("grandchild");
    let payload = serde_json::json!({
        "command": format!(
            "(sleep 3; touch {}) & sleep 3; touch {}",
            grandchild.display(),
            direct.display()
        ),
    })
    .to_string();

    let mut stream = TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("failed to connect to bridge");
    let req = format!(
        "POST /exec HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{payload}",
        payload.len()
    );
    stream
        .write_all(req.as_bytes())
        .await
        .expect("failed to write request");

    // Let the bridge accept the request and spawn the command, then disconnect mid-flight.
    tokio::time::sleep(Duration::from_millis(200)).await;
    drop(stream);

    tokio::time::sleep(Duration::from_secs(6)).await;
    assert!(
        !direct.exists() && !grandchild.exists(),
        "disconnecting the /exec client should kill the command's whole process group"
    );
}
