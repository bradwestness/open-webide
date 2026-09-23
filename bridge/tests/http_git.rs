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
