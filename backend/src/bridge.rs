use crate::git::BridgeError;
use crate::state::AppDb;
use http_body_util::BodyExt;
use openwebide_storage::Store;

use spin_sdk::http;

pub const BRIDGE_TOKEN_TTL_SECS: i64 = 120;

pub fn resolve_url(var: Option<String>) -> String {
    let url = var.unwrap_or_else(|| "http://127.0.0.1:3001".to_string());
    let trimmed = url.trim();
    if trimmed.is_empty() {
        "http://127.0.0.1:3001".to_string()
    } else {
        trimmed.trim_end_matches('/').to_string()
    }
}

pub fn is_loopback_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    lower.starts_with("http://127.0.0.1:")
        || lower.starts_with("https://127.0.0.1:")
        || lower.starts_with("http://[::1]:")
        || lower.starts_with("https://[::1]:")
        || lower.starts_with("http://localhost:")
        || lower.starts_with("https://localhost:")
        || lower == "http://127.0.0.1"
        || lower == "https://127.0.0.1"
        || lower == "http://[::1]"
        || lower == "https://[::1]"
        || lower == "http://localhost"
        || lower == "https://localhost"
}

pub async fn bridge_url() -> String {
    #[cfg(not(test))]
    let var = spin_sdk::variables::get("bridge_url").await.ok();
    #[cfg(test)]
    let var = None;
    resolve_url(var)
}

pub async fn bridge_secret(store: &Store<AppDb>) -> Result<Option<String>, BridgeError> {
    #[cfg(not(test))]
    if let Ok(var) = spin_sdk::variables::get("bridge_secret").await
        && !var.trim().is_empty()
    {
        return Ok(Some(var.trim().to_string()));
    }

    if let Ok(Some(cached)) = store.get_setting("bridge_secret_cache").await
        && !cached.trim().is_empty()
    {
        return Ok(Some(cached));
    }

    #[cfg(not(test))]
    {
        let url = bridge_url().await;
        if is_loopback_url(&url) {
            let endpoint = format!("{url}/secret");
            match http::post(&endpoint, "").await {
                Ok(resp) if resp.status().is_success() => {
                    if let Ok(collected) = resp.into_body().collect().await {
                        #[derive(serde::Deserialize)]
                        struct SecretResp {
                            secret: String,
                        }
                        if let Ok(val) = serde_json::from_slice::<SecretResp>(&collected.to_bytes())
                        {
                            let _ = store.set_setting("bridge_secret_cache", &val.secret).await;
                            return Ok(Some(val.secret));
                        }
                    }
                }
                Err(e) => {
                    return Err(BridgeError::Unreachable(e.to_string()));
                }
                _ => {}
            }
        }
    }

    Ok(None)
}

pub async fn send(
    store: &Store<AppDb>,
    path: &str,
    json: String,
) -> Result<(u16, Vec<u8>), BridgeError> {
    let mut secret_opt = bridge_secret(store).await?;

    let url = bridge_url().await;
    let endpoint = format!("{url}{path}");

    for attempt in 0..2 {
        let secret = secret_opt.clone().ok_or(BridgeError::NoSecret)?;
        let req = http::Request::builder()
            .method("POST")
            .uri(&endpoint)
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {secret}"))
            .body(json.clone())
            .map_err(|e| BridgeError::Unreachable(e.to_string()))?;

        let resp = spin_sdk::http::send(req)
            .await
            .map_err(|e| BridgeError::Unreachable(e.to_string()))?;
        let status = resp.status().as_u16();
        let bytes = resp
            .into_body()
            .collect()
            .await
            .map_err(|e| BridgeError::Unreachable(e.to_string()))?
            .to_bytes()
            .to_vec();

        if status == 401 && attempt == 0 {
            // Secret might have been invalidated, clear cache and retry
            let _ = store.delete_setting("bridge_secret_cache").await;
            secret_opt = bridge_secret(store).await?;
            continue;
        }

        if status == 401 {
            return Err(BridgeError::Unauthorized);
        }

        return Ok((status, bytes));
    }

    Err(BridgeError::Unauthorized)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_url() {
        assert_eq!(resolve_url(None), "http://127.0.0.1:3001");
        assert_eq!(resolve_url(Some("".to_string())), "http://127.0.0.1:3001");
        assert_eq!(
            resolve_url(Some("  \n ".to_string())),
            "http://127.0.0.1:3001"
        );
        assert_eq!(
            resolve_url(Some("http://foo:1234/".to_string())),
            "http://foo:1234"
        );
        assert_eq!(
            resolve_url(Some("http://foo:1234".to_string())),
            "http://foo:1234"
        );
    }

    #[test]
    fn test_is_loopback_url() {
        assert!(is_loopback_url("http://127.0.0.1:3001"));
        assert!(is_loopback_url("http://[::1]:3001"));
        assert!(is_loopback_url("http://localhost:3001"));

        assert!(!is_loopback_url("http://192.168.1.5:3001"));
        assert!(!is_loopback_url("http://host.docker.internal:3001"));
    }
}
