//! Thin client for the backend REST API, including SSE streaming.

use gloo_net::http::{Method, Request, RequestBuilder};
use openwebide_core::{ChatMessage, ChatSession, Connection, Health, NewSession};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::json;
use wasm_bindgen_futures::JsFuture;
use web_sys::wasm_bindgen::JsCast;
use web_sys::{AbortSignal, ReadableStreamDefaultReader, ReadableStreamReadResult, TextDecoder};

#[derive(Clone)]
pub struct BackendApi {
    base: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum HealthState {
    Online { version: String },
    Offline,
}

/// A server-sent event from the message stream.
#[derive(Debug, Clone)]
pub enum SseEvent {
    /// The user message that was persisted before the stream started.
    Message(ChatMessage),
    /// A token delta appended to the in-progress assistant reply.
    Delta(String),
    /// The final, persisted assistant reply.
    Done(ChatMessage),
    /// A provider or persistence error; the stream ends after this.
    Error(String),
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

    pub async fn list_sessions(&self) -> Result<Vec<ChatSession>, String> {
        self.get("/sessions").await
    }

    pub async fn create_session(
        &self,
        name: &str,
        connection_id: Option<i64>,
        system_prompt_id: Option<i64>,
    ) -> Result<ChatSession, String> {
        let body = NewSession {
            name: name.to_string(),
            connection_id,
            system_prompt_id,
        };
        self.post("/sessions", &body).await
    }

    pub async fn rename_session(&self, id: i64, name: &str) -> Result<ChatSession, String> {
        self.put(&format!("/sessions/{id}"), &json!({ "name": name }))
            .await
    }

    pub async fn delete_session(&self, id: i64) -> Result<(), String> {
        self.request::<(), _>(Method::DELETE, &format!("/sessions/{id}"), None)
            .await
    }

    pub async fn list_messages(&self, session_id: i64) -> Result<Vec<ChatMessage>, String> {
        self.get(&format!("/sessions/{session_id}/messages")).await
    }

    /// Send a user message and stream the assistant reply back via SSE.
    ///
    /// `on_event` is invoked for every event as it arrives. Returns `Err` on
    /// transport failure (including abort) once the stream ends.
    pub async fn send_message(
        &self,
        session_id: i64,
        content: &str,
        model: Option<&str>,
        signal: Option<&AbortSignal>,
        mut on_event: impl FnMut(SseEvent),
    ) -> Result<(), String> {
        let req = Request::post(&format!("{}/sessions/{session_id}/messages", self.base))
            .abort_signal(signal)
            .json(&json!({ "content": content, "model": model }))
            .map_err(|e| e.to_string())?;
        let resp = req.send().await.map_err(|e| e.to_string())?;
        if !resp.ok() {
            return Err(self.error_from(resp).await);
        }
        let stream = resp
            .body()
            .ok_or_else(|| "response has no body".to_string())?;
        let reader: ReadableStreamDefaultReader = stream
            .get_reader()
            .dyn_into()
            .map_err(|e| format!("{e:?}"))?;
        let decoder = TextDecoder::new().map_err(|e| format!("{e:?}"))?;

        let mut pending = String::new();
        loop {
            let read = reader.read();
            let chunk: ReadableStreamReadResult = JsFuture::from(read)
                .await
                .map_err(|e| format!("{e:?}"))?
                .dyn_into()
                .map_err(|e| format!("{e:?}"))?;
            if chunk.get_done().unwrap_or(false) {
                break;
            }
            let value: js_sys::Uint8Array =
                chunk.get_value().dyn_into().map_err(|e| format!("{e:?}"))?;
            let text = decoder
                .decode_with_u8_array(&value.to_vec())
                .map_err(|e| format!("{e:?}"))?;
            pending.push_str(&text);
            while let Some(pos) = pending.find("\n\n") {
                let frame = pending[..pos].to_string();
                pending.drain(..pos + 2);
                if let Some(event) = parse_sse_frame(&frame) {
                    on_event(event);
                }
            }
        }
        Ok(())
    }

    async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, String> {
        self.request::<(), T>(Method::GET, path, None).await
    }

    async fn post<T: Serialize, R: DeserializeOwned>(
        &self,
        path: &str,
        body: &T,
    ) -> Result<R, String> {
        self.request(Method::POST, path, Some(body)).await
    }

    async fn put<T: Serialize, R: DeserializeOwned>(
        &self,
        path: &str,
        body: &T,
    ) -> Result<R, String> {
        self.request(Method::PUT, path, Some(body)).await
    }

    async fn request<T, R>(&self, method: Method, path: &str, body: Option<&T>) -> Result<R, String>
    where
        T: Serialize,
        R: DeserializeOwned,
    {
        let url = format!("{}{path}", self.base);
        let is_delete = method == Method::DELETE;
        let builder = RequestBuilder::new(&url).method(method);
        let req = match body {
            Some(body) => builder.json(body),
            None => builder.build(),
        }
        .map_err(|e| e.to_string())?;
        let resp = req.send().await.map_err(|e| e.to_string())?;
        if !resp.ok() {
            return Err(self.error_from(resp).await);
        }
        if is_delete {
            // The backend answers 204 No Content; nothing to decode.
            return serde_json::from_value(serde_json::Value::Null).map_err(|e| e.to_string());
        }
        resp.json().await.map_err(|e| e.to_string())
    }

    async fn error_from(&self, resp: gloo_net::http::Response) -> String {
        let text = resp.text().await.unwrap_or_default();
        serde_json::from_str::<serde_json::Value>(&text)
            .ok()
            .and_then(|v| v.get("error")?.as_str().map(String::from))
            .unwrap_or_else(|| {
                if text.is_empty() {
                    format!("HTTP {}", resp.status())
                } else {
                    text
                }
            })
    }
}

/// Parse one SSE frame (`event: <name>\ndata: <json>\n\n`) into an [`SseEvent`].
fn parse_sse_frame(frame: &str) -> Option<SseEvent> {
    let mut event = "message";
    let mut data = String::new();
    for line in frame.lines() {
        if let Some(v) = line.strip_prefix("event: ") {
            event = v.trim();
        } else if let Some(v) = line.strip_prefix("data: ") {
            data.push_str(v.trim());
        }
    }
    let value: serde_json::Value = serde_json::from_str(&data).ok()?;
    Some(match event {
        "message" => SseEvent::Message(serde_json::from_value(value).ok()?),
        "delta" => SseEvent::Delta(value.get("content")?.as_str()?.to_string()),
        "done" => SseEvent::Done(serde_json::from_value(value).ok()?),
        "error" => SseEvent::Error(value.get("error")?.as_str()?.to_string()),
        _ => return None,
    })
}

fn query_param(query: &str, key: &str) -> Option<String> {
    query.trim_start_matches('?').split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=')?;
        (k == key).then(|| v.to_string())
    })
}
