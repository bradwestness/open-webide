use crate::backend::Api;
use crate::idb;
use leptos::prelude::WithValue;
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BridgeConfig {
    pub ws_url: String,
    pub http_url: String,
}

impl BridgeConfig {
    pub fn new(url: &str) -> Self {
        let ws_url = url.trim().trim_end_matches('/').to_string();
        let http_url = if ws_url.starts_with("wss://") {
            ws_url.replacen("wss://", "https://", 1)
        } else if ws_url.starts_with("ws://") {
            ws_url.replacen("ws://", "http://", 1)
        } else {
            // Default to http if no scheme
            format!("http://{}", ws_url)
        };
        Self { ws_url, http_url }
    }
}

pub fn default_bridge_url() -> String {
    let hostname = web_sys::window()
        .and_then(|w| w.location().hostname().ok())
        .unwrap_or_else(|| "127.0.0.1".to_string());
    format!("ws://{}:3001", hostname)
}

#[derive(Clone)]
pub struct BridgeCredentials {
    api: Api,
    cached_token: Arc<Mutex<Option<(String, i64)>>>,
}

impl BridgeCredentials {
    pub fn new(api: Api) -> Self {
        Self {
            api,
            cached_token: Arc::new(Mutex::new(None)),
        }
    }

    pub async fn credential(&self) -> Result<String, String> {
        if let Ok(Some(pt)) = idb::get_bridge_pairing_token().await
            && !pt.trim().is_empty()
        {
            return Ok(pt);
        }

        let now = (js_sys::Date::now() / 1000.0) as i64;
        let needs_refresh = {
            let cache = self.cached_token.lock().unwrap();
            match &*cache {
                Some((_, expires_at)) => now >= *expires_at - 30,
                None => true,
            }
        };

        if needs_refresh {
            let (token, expires_at) = self.api.with_value(Clone::clone).bridge_token().await?;
            *self.cached_token.lock().unwrap() = Some((token.clone(), expires_at));
            Ok(token)
        } else {
            let cache = self.cached_token.lock().unwrap();
            Ok(cache.as_ref().unwrap().0.clone())
        }
    }

    pub fn clear_cache(&self) {
        *self.cached_token.lock().unwrap() = None;
    }
}
