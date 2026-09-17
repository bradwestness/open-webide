//! Spin's outbound HTTP, adapted to the provider-facing [`HttpClient`] trait.

use http_body_util::BodyExt;
use openwebide_llm::{HttpClient, ProviderError};
use spin_sdk::http::{self, Response};

/// Outbound HTTP client backed by Spin's WASI HTTP handler.
pub struct SpinHttpClient;

impl HttpClient for SpinHttpClient {
    async fn get_json(&self, url: &str) -> Result<serde_json::Value, ProviderError> {
        let response = http::get(url)
            .await
            .map_err(|e| ProviderError::Http(format!("cannot reach {url}: {e}")))?;
        json_from_response(response).await
    }

    async fn post_json(
        &self,
        url: &str,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value, ProviderError> {
        let payload = serde_json::to_string(body)
            .map_err(|e| ProviderError::Parse(format!("serialize request: {e}")))?;
        let response = http::post(url, payload)
            .await
            .map_err(|e| ProviderError::Http(format!("cannot reach {url}: {e}")))?;
        json_from_response(response).await
    }
}

async fn json_from_response(response: Response) -> Result<serde_json::Value, ProviderError> {
    let status = response.status();
    let collected = response
        .into_body()
        .collect()
        .await
        .map_err(|e| ProviderError::Http(format!("read response body: {e}")))?;
    let text = String::from_utf8(collected.to_bytes().to_vec())
        .map_err(|_| ProviderError::Parse("response body is not valid UTF-8".into()))?;

    if !status.is_success() {
        // Ollama and llama.cpp both report failures as {"error": "..."}.
        let detail = serde_json::from_str::<serde_json::Value>(&text)
            .ok()
            .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(str::to_string))
            .unwrap_or(text);
        return Err(ProviderError::Http(format!("{status}: {detail}")));
    }

    serde_json::from_str(&text).map_err(|e| ProviderError::Parse(format!("invalid JSON: {e}")))
}
