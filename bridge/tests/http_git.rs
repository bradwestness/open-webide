//! Integration tests for bridge Git HTTP endpoints.

mod common;

use common::{TestDir, post, start};
use tokio::process::Command;

async fn init_git_repo(path: &std::path::Path) {
    let mut cmd = Command::new("git");
    cmd.args(["init", "-b", "main"]).current_dir(path);
    assert!(cmd.status().await.unwrap().success());

    let mut cmd = Command::new("git");
    cmd.args(["config", "user.name", "Tester"])
        .current_dir(path);
    assert!(cmd.status().await.unwrap().success());

    let mut cmd = Command::new("git");
    cmd.args(["config", "user.email", "test@example.com"])
        .current_dir(path);
    assert!(cmd.status().await.unwrap().success());
}

#[tokio::test]
async fn checkout_with_dot_returns_bad_request_400() {
    let test_dir = TestDir::new();
    init_git_repo(&test_dir.path).await;
    let port = start(test_dir.path.clone()).await;

    let resp = post(
        port,
        "/git/checkout",
        r#"{"branch": "."}"#,
        &[("Content-Type", "application/json")],
    )
    .await;

    assert_eq!(resp.status, 400);
    assert!(
        resp.body.contains("error"),
        "expected error response body, got: {}",
        resp.body
    );
}

#[cfg(unix)]
#[tokio::test]
async fn sync_authentication_failures_share_remote_and_browser_contracts() {
    use std::os::unix::fs::PermissionsExt;

    let test_dir = TestDir::new();
    init_git_repo(&test_dir.path).await;
    let mut cmd = Command::new("git");
    cmd.args(["commit", "--allow-empty", "-m", "initial"])
        .current_dir(&test_dir.path);
    assert!(cmd.status().await.unwrap().success());
    let mut cmd = Command::new("git");
    cmd.args(["remote", "add", "origin", "ext::./ssh-fixture.sh"])
        .current_dir(&test_dir.path);
    assert!(cmd.status().await.unwrap().success());
    let port = start(test_dir.path.clone()).await;
    let ssh = test_dir.path.join("ssh-fixture.sh");
    let mut cmd = Command::new("git");
    cmd.args(["config", "protocol.ext.allow", "always"])
        .current_dir(&test_dir.path);
    assert!(cmd.status().await.unwrap().success());

    for diagnostic in [
        "Permission denied (publickey)",
        "Host key verification failed",
    ] {
        // A repo-scoped transport fixture avoids changing process-wide SSH settings.
        std::fs::write(
            &ssh,
            format!("#!/bin/sh\necho '{diagnostic}' >&2\nexit 255\n"),
        )
        .unwrap();
        std::fs::set_permissions(&ssh, std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut bodies = Vec::new();
        for browser in [false, true] {
            let mut headers = vec![("Content-Type", "application/json")];
            if browser {
                headers.push(("Origin", "http://localhost:3000"));
            }
            for action in ["pull", "push", "sync"] {
                let response = post(
                    port,
                    "/git/sync",
                    &format!(r#"{{"cwd":".","action":"{action}","branch":"main"}}"#),
                    &headers,
                )
                .await;
                assert_eq!(response.status, 500);
                assert!(response.body.contains(diagnostic));
                assert!(response.body.contains("ssh-add"));
                assert!(response.body.contains("known_hosts"));
                bodies.push(response.body);
            }
        }
        assert_eq!(&bodies[..3], &bodies[3..]);
    }
}
