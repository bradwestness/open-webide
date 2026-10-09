//! Both plugin transports authenticate into isolated host caches.
mod common;
use common::{HttpResponse, http, post};
use openwebide_bridge::{ServerConfig, plugins::NativePluginInstaller, run_server};
use openwebide_core::plugins::{
    PreparedPlugin, content_digest,
    testing::{review_files, source},
};
use std::path::Path;

fn git(root: &Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().into()
}

#[tokio::test]
async fn local_pairing_and_remote_backend_install_the_same_package_without_workspace_writes() {
    let cache = tempfile::tempdir().unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let workspace = tempfile::tempdir().unwrap();
    for file in review_files() {
        let path = fixture.path().join("plugins/pr-review").join(file.path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, file.content).unwrap();
    }
    git(fixture.path(), &["init", "-q"]);
    git(fixture.path(), &["add", "."]);
    git(
        fixture.path(),
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.org",
            "commit",
            "-qm",
            "fixture",
        ],
    );
    let mut source = source();
    source.commit = git(fixture.path(), &["rev-parse", "HEAD"]);
    for owner in ["paired", "user:42"] {
        let repo = cache
            .path()
            .join(content_digest(owner.as_bytes()))
            .join("repositories")
            .join(content_digest(source.repository.as_bytes()));
        std::fs::create_dir_all(repo.parent().unwrap()).unwrap();
        git(
            cache.path(),
            &[
                "clone",
                "--bare",
                fixture.path().to_str().unwrap(),
                repo.to_str().unwrap(),
            ],
        );
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let mut config = ServerConfig::new(
        workspace.path().into(),
        "dummy_secret".into(),
        Some("paired-token".into()),
    );
    config.plugins = NativePluginInstaller::new(Some(cache.path().into()));
    let server = tokio::spawn(run_server(listener, config));
    let body = serde_json::json!({"source":source,"user":42}).to_string();
    let remote = post(
        port,
        "/plugins/prepare",
        &body,
        &[("Content-Type", "application/json")],
    )
    .await;
    assert_eq!(remote.status, 200, "{}", remote.body);
    let remote: PreparedPlugin = serde_json::from_str(&remote.body).unwrap();
    let signed_body = serde_json::json!({"source":source,"user":999}).to_string();
    let signed = post(
        port,
        "/plugins/prepare",
        &signed_body,
        &[
            ("Origin", "http://localhost:3000"),
            ("Content-Type", "application/json"),
        ],
    )
    .await;
    assert_eq!(signed.status, 200, "{}", signed.body);
    let local_body = serde_json::json!({"source":source,"user":999}).to_string();
    let raw = format!(
        "POST /plugins/prepare HTTP/1.1\r\nHost: localhost:{port}\r\nOrigin: http://localhost:3000\r\nAuthorization: Bearer paired-token\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{local_body}",
        local_body.len()
    );
    let local = HttpResponse::parse(&http(port, &raw).await);
    assert_eq!(local.status, 200, "{}", local.body);
    assert_eq!(
        local.header("Access-Control-Allow-Origin"),
        Some("http://localhost:3000")
    );
    let local: PreparedPlugin = serde_json::from_str(&local.body).unwrap();
    assert_eq!(local, remote);
    let remote_package = post(
        port,
        "/plugins/package",
        &serde_json::json!({"prepared":remote,"user":42}).to_string(),
        &[("Content-Type", "application/json")],
    )
    .await;
    assert_eq!(remote_package.status, 200, "{}", remote_package.body);
    let body = serde_json::json!({"prepared":local,"user":999}).to_string();
    let raw = format!(
        "POST /plugins/package HTTP/1.1\r\nHost: localhost:{port}\r\nOrigin: http://localhost:3000\r\nAuthorization: Bearer paired-token\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let local_package = HttpResponse::parse(&http(port, &raw).await);
    assert_eq!(local_package.status, 200, "{}", local_package.body);
    let remote_package: openwebide_core::plugins::PluginPackage =
        serde_json::from_str(&remote_package.body).unwrap();
    let local_package: openwebide_core::plugins::PluginPackage =
        serde_json::from_str(&local_package.body).unwrap();
    assert_eq!(local_package, remote_package);
    assert!(!local_package.skills[0].resources.is_empty());

    // Browser-supplied user IDs cannot select another cache namespace.
    assert!(!cache.path().join(content_digest(b"user:999")).exists());
    assert_eq!(std::fs::read_dir(workspace.path()).unwrap().count(), 0);
    let missing_user = post(
        port,
        "/plugins/prepare",
        &serde_json::json!({"source":source}).to_string(),
        &[("Content-Type", "application/json")],
    )
    .await;
    assert_eq!(missing_user.status, 400);
    let evil = post(
        port,
        "/plugins/prepare",
        &body,
        &[
            ("Origin", "http://evil.example"),
            ("Content-Type", "application/json"),
        ],
    )
    .await;
    assert_eq!(evil.status, 403);
    let unauth = format!(
        "POST /plugins/prepare HTTP/1.1\r\nHost: localhost:{port}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    );
    assert_eq!(HttpResponse::parse(&http(port, &unauth).await).status, 401);
    server.abort();
}
