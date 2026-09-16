//! Thin client for the backend REST API.

use gloo_net::http::Request;
use openwebide_core::{Connection, Health};
use serde::de::DeserializeOwned;

#[derive(Clone)]
pub struct BackendApi {
    base: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum HealthState {
    Online { version: String },
    Offline,
}

impl BackendApi {
    /// Same-origin by default; `?api=http://host:port/api` overrides the
    /// base for development against a separately served backend.
    pub fn from_location() -> Self {
        let location = web_sys::window().map(|w| w.location());
        let origin = location
            .as_ref()
            .and_then(|l| l.origin().ok())
            .unwrap_or_else(|| "http://localhost:3000".to_string());
        let base = location
            .and_then(|l| l.search().ok())
            .and_then(|search| query_param(&search, "api"))
            .map(|v| v.trim_end_matches('/').to_string())
            .unwrap_or_else(|| format!("{origin}/api"));
        Self { base }
    }

    pub async fn health(&self) -> Result<Health, String> {
        self.get("/health").await
    }

    pub async fn list_connections(&self) -> Result<Vec<Connection>, String> {
        self.get("/connections").await
    }

    async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, String> {
        let resp = Request::get(&format!("{}{}", self.base, path))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        resp.json().await.map_err(|e| e.to_string())
    }
}

fn query_param(query: &str, key: &str) -> Option<String> {
    query.trim_start_matches('?').split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=')?;
        (k == key).then(|| v.to_string())
    })
}
