//! Thin client for the backend REST API, including SSE streaming.

use gloo_net::http::{Method, Request, RequestBuilder};
use leptos::prelude::*;
pub use openwebide_frontend::sse::SseEvent;
use openwebide_core::{
    ChatCompletion, ChatMessage, ChatRequest, ChatSession, Connection, ConversationEntry,
    EditorContext, FileDiff, FileEntry, GitBranchInfo, GitCheckoutRequest, GitCheckoutResult,
    GitCommitRequest, GitCommitResult, GitRepoStatus, GitSyncRequest, GitSyncResult, Health,
    ModelInfo, NewConnection, NewProject, NewSession, Project, ProviderKind, Role, SearchHit,
    SystemPrompt, TurnTelemetry, User, WebSearchResult, WorkspaceMode,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::json;
use wasm_bindgen_futures::JsFuture;
use web_sys::wasm_bindgen::JsCast;
use web_sys::{AbortSignal, ReadableStreamDefaultReader, ReadableStreamReadResult};

#[derive(Clone)]
pub struct BackendApi {
    base: String,
    token: RwSignal<Option<String>>,
}

/// The `{user, token}` payload returned by register and login.
#[derive(Deserialize)]
struct AuthResponse {
    user: User,
    token: String,
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
        Self {
            base,
            token: RwSignal::new(read_token_from_storage()),
        }
    }

    // -- auth --------------------------------------------------------------

    /// The current bearer token, if any.
    pub fn token(&self) -> Option<String> {
        self.token.get()
    }

    /// Set (or clear) the bearer token, persisting it to localStorage.
    pub fn set_token(&self, token: Option<String>) {
        self.token.set(token.clone());
        match &token {
            Some(t) => write_token_to_storage(t),
            None => clear_token_from_storage(),
        }
    }

    /// Register a new local account (only the first account may register).
    /// On success the returned token is stored for subsequent requests.
    pub async fn register(&self, username: &str, password: &str) -> Result<User, String> {
        let resp: AuthResponse = self
            .post(
                "/auth/register",
                &json!({ "username": username, "password": password }),
            )
            .await?;
        self.set_token(Some(resp.token));
        Ok(resp.user)
    }

    /// Log in with an existing account. On success the token is stored.
    pub async fn login(&self, username: &str, password: &str) -> Result<User, String> {
        let resp: AuthResponse = self
            .post(
                "/auth/login",
                &json!({ "username": username, "password": password }),
            )
            .await?;
        self.set_token(Some(resp.token));
        Ok(resp.user)
    }

    /// The account for the current token.
    pub async fn me(&self) -> Result<User, String> {
        let resp: serde_json::Value = self.get("/auth/me").await?;
        serde_json::from_value(resp["user"].clone()).map_err(|e| e.to_string())
    }

    pub async fn health(&self) -> Result<Health, String> {
        self.get("/health").await
    }

    pub async fn list_connections(&self) -> Result<Vec<Connection>, String> {
        self.get("/connections").await
    }

    pub async fn create_connection(
        &self,
        name: &str,
        kind: ProviderKind,
        base_url: &str,
        model: Option<&str>,
        context_limit: Option<usize>,
    ) -> Result<Connection, String> {
        let body = NewConnection {
            name: name.to_string(),
            kind,
            base_url: base_url.to_string(),
            model: model.map(str::to_string),
            context_limit,
        };
        self.post("/connections", &body).await
    }

    pub async fn update_connection(&self, connection: &Connection) -> Result<Connection, String> {
        self.put(&format!("/connections/{}", connection.id), connection)
            .await
    }

    pub async fn delete_connection(&self, id: i64) -> Result<(), String> {
        self.request::<(), _>(Method::DELETE, &format!("/connections/{id}"), None)
            .await
    }

    pub async fn list_sessions(&self) -> Result<Vec<ChatSession>, String> {
        self.get("/sessions").await
    }

    /// List the models a connection's provider reports.
    pub async fn list_models(&self, connection_id: i64) -> Result<Vec<ModelInfo>, String> {
        self.get(&format!("/models?connection_id={connection_id}"))
            .await
    }

    // -- system prompts ----------------------------------------------------

    pub async fn list_system_prompts(&self) -> Result<Vec<SystemPrompt>, String> {
        self.get("/system-prompts").await
    }

    pub async fn create_system_prompt(
        &self,
        name: &str,
        content: &str,
    ) -> Result<SystemPrompt, String> {
        self.post(
            "/system-prompts",
            &json!({ "name": name, "content": content }),
        )
        .await
    }

    pub async fn update_system_prompt(
        &self,
        id: i64,
        name: &str,
        content: &str,
    ) -> Result<SystemPrompt, String> {
        self.put(
            &format!("/system-prompts/{id}"),
            &json!({ "name": name, "content": content }),
        )
        .await
    }

    pub async fn delete_system_prompt(&self, id: i64) -> Result<(), String> {
        self.request::<(), _>(Method::DELETE, &format!("/system-prompts/{id}"), None)
            .await
    }

    // -- settings ----------------------------------------------------------

    pub async fn get_settings(&self) -> Result<std::collections::BTreeMap<String, String>, String> {
        self.get("/settings").await
    }

    pub async fn set_setting(&self, key: &str, value: &str) -> Result<(), String> {
        let _resp: serde_json::Value = self
            .put("/settings", &json!({ "key": key, "value": value }))
            .await?;
        Ok(())
    }

    pub async fn create_session(
        &self,
        name: &str,
        connection_id: Option<i64>,
        system_prompt_id: Option<i64>,
        project_id: Option<i64>,
    ) -> Result<ChatSession, String> {
        let body = NewSession {
            name: name.to_string(),
            connection_id,
            system_prompt_id,
            project_id,
        };
        self.post("/sessions", &body).await
    }

    // -- projects ----------------------------------------------------------

    pub async fn list_projects(&self) -> Result<Vec<Project>, String> {
        self.get("/projects").await
    }

    pub async fn create_project(
        &self,
        name: &str,
        mode: WorkspaceMode,
        path: Option<String>,
    ) -> Result<Project, String> {
        let body = NewProject {
            name: name.to_string(),
            mode,
            path,
        };
        self.post("/projects", &body).await
    }

    #[allow(dead_code)] // wired to the project UI in a later step
    pub async fn rename_project(&self, id: i64, name: &str) -> Result<Project, String> {
        self.put(&format!("/projects/{id}"), &json!({ "name": name }))
            .await
    }

    pub async fn delete_project(&self, id: i64) -> Result<(), String> {
        self.request::<(), _>(Method::DELETE, &format!("/projects/{id}"), None)
            .await
    }

    // -- project files (remote mode) ---------------------------------------

    pub async fn list_files(&self, project_id: i64, path: &str) -> Result<Vec<FileEntry>, String> {
        self.get(&format!(
            "/projects/{project_id}/files?path={}",
            urlenc(path)
        ))
        .await
    }

    pub async fn read_file_object_url(
        &self,
        project_id: i64,
        path: &str,
    ) -> Result<String, String> {
        let url = format!(
            "{}/projects/{project_id}/files/raw?path={}",
            self.base,
            urlenc(path)
        );
        let mut builder = RequestBuilder::new(&url).method(Method::GET);
        if let Some(token) = self.token.get() {
            builder = builder.header("authorization", &format!("Bearer {token}"));
        }
        let req = builder.build().map_err(|e| e.to_string())?;
        let resp = req.send().await.map_err(|e| e.to_string())?;
        if !resp.ok() {
            return Err(self.error_from(resp).await);
        }
        let web_resp = web_sys::Response::from(resp);
        let blob_promise = web_resp.blob().map_err(|e| format!("{e:?}"))?;
        let blob_js = JsFuture::from(blob_promise)
            .await
            .map_err(|e| format!("{e:?}"))?;
        let blob: web_sys::Blob = blob_js.unchecked_into();
        web_sys::Url::create_object_url_with_blob(&blob).map_err(|e| format!("{e:?}"))
    }

    /// Read a file's contents as text.
    pub async fn read_file(&self, project_id: i64, path: &str) -> Result<String, String> {
        let value: serde_json::Value = self
            .get(&format!(
                "/projects/{project_id}/files/read?path={}",
                urlenc(path)
            ))
            .await?;
        Ok(value["content"].as_str().unwrap_or_default().to_string())
    }

    /// Write text to a file (raw text body, not JSON).
    pub async fn write_file(
        &self,
        project_id: i64,
        path: &str,
        content: &str,
    ) -> Result<(), String> {
        let url = format!(
            "{}/projects/{project_id}/files/write?path={}",
            self.base,
            urlenc(path)
        );
        let mut builder = RequestBuilder::new(&url)
            .method(Method::PUT)
            .header("content-type", "text/plain");
        if let Some(token) = self.token.get() {
            builder = builder.header("authorization", &format!("Bearer {token}"));
        }
        let req = builder
            .body(content.to_string())
            .map_err(|e| e.to_string())?;
        let resp = req.send().await.map_err(|e| e.to_string())?;
        if !resp.ok() {
            return Err(self.error_from(resp).await);
        }
        Ok(())
    }

    pub async fn copy_file(&self, project_id: i64, from: &str, to: &str) -> Result<(), String> {
        let body = serde_json::json!({
            "from": from,
            "to": to,
        });
        self.request::<_, serde_json::Value>(
            Method::POST,
            &format!("/projects/{project_id}/files/copy"),
            Some(&body),
        )
        .await?;
        Ok(())
    }

    /// Create an empty file or a directory.
    pub async fn create_file(
        &self,
        project_id: i64,
        path: &str,
        is_dir: bool,
    ) -> Result<(), String> {
        let kind = if is_dir { "dir" } else { "file" };
        let _value: serde_json::Value = self
            .post(
                &format!(
                    "/projects/{project_id}/files/create?path={}&type={kind}",
                    urlenc(path)
                ),
                &json!({}),
            )
            .await?;
        Ok(())
    }

    /// Delete the file at `path`.
    pub async fn delete_file(&self, project_id: i64, path: &str) -> Result<(), String> {
        self.request::<(), _>(
            Method::DELETE,
            &format!("/projects/{project_id}/files/delete?path={}", urlenc(path)),
            None,
        )
        .await
    }

    /// List a directory of the host mount for the remote file browser.
    /// `path` is relative to the mount root (e.g. `~/source`); empty = the
    /// root itself.
    pub async fn browse(&self, path: &str) -> Result<Vec<FileEntry>, String> {
        self.get(&format!("/browse?path={}", urlenc(path))).await
    }

    /// Full-text search: return the lines of every file whose content contains
    /// `query` (case-insensitive).
    pub async fn search_content(
        &self,
        project_id: i64,
        query: &str,
        path: &str,
    ) -> Result<Vec<SearchHit>, String> {
        self.get(&format!(
            "/projects/{project_id}/files/content-search?q={}&path={}",
            urlenc(query),
            urlenc(path)
        ))
        .await
    }

    // -- git operations (Phase 13) -----------------------------------------

    fn git_endpoint(&self, project_id: Option<i64>, sub: &str) -> String {
        if let Some(id) = project_id {
            format!("/projects/{id}/git/{sub}")
        } else {
            format!("/git/{sub}")
        }
    }

    pub async fn git_status(&self, project_id: Option<i64>) -> Result<GitRepoStatus, String> {
        self.get(&self.git_endpoint(project_id, "status")).await
    }

    pub async fn git_diff(
        &self,
        project_id: Option<i64>,
        path: Option<&str>,
    ) -> Result<String, String> {
        let ep = self.git_endpoint(project_id, "diff");
        let query = match path {
            Some(p) => format!("{ep}?path={}", urlenc(p)),
            None => ep,
        };
        let res: serde_json::Value = self.get(&query).await?;
        Ok(res["diff"].as_str().unwrap_or_default().to_string())
    }

    pub async fn git_file_head(
        &self,
        project_id: Option<i64>,
        path: &str,
    ) -> Result<String, String> {
        let ep = self.git_endpoint(project_id, "show");
        let query = format!("{ep}?path={}", urlenc(path));
        let res: serde_json::Value = self.get(&query).await?;
        Ok(res["content"].as_str().unwrap_or_default().to_string())
    }

    pub async fn git_branches(
        &self,
        project_id: Option<i64>,
    ) -> Result<Vec<GitBranchInfo>, String> {
        self.get(&self.git_endpoint(project_id, "branches")).await
    }

    pub async fn git_commit(
        &self,
        project_id: Option<i64>,
        req: &GitCommitRequest,
    ) -> Result<GitCommitResult, String> {
        self.post(&self.git_endpoint(project_id, "commit"), req)
            .await
    }

    pub async fn git_checkout(
        &self,
        project_id: Option<i64>,
        req: &GitCheckoutRequest,
    ) -> Result<GitCheckoutResult, String> {
        self.post(&self.git_endpoint(project_id, "checkout"), req)
            .await
    }

    pub async fn git_sync(
        &self,
        project_id: Option<i64>,
        req: &GitSyncRequest,
    ) -> Result<GitSyncResult, String> {
        self.post(&self.git_endpoint(project_id, "sync"), req).await
    }

    pub async fn rename_session(&self, id: i64, name: &str) -> Result<ChatSession, String> {
        self.put(&format!("/sessions/{id}"), &json!({ "name": name }))
            .await
    }

    pub async fn delete_session(&self, id: i64) -> Result<(), String> {
        self.request::<(), _>(Method::DELETE, &format!("/sessions/{id}"), None)
            .await
    }

    /// Ask the backend to stop the session's in-flight run. The run ends at
    /// its next step boundary; the stream then emits `Cancelled`.
    pub async fn cancel_session(&self, session_id: i64) -> Result<(), String> {
        let _value: serde_json::Value = self
            .request::<(), _>(
                Method::POST,
                &format!("/sessions/{session_id}/cancel"),
                None,
            )
            .await?;
        Ok(())
    }

    /// Record the user's decision on a gated tool call. The in-flight run
    /// picks it up on its next poll (about half a second later).
    pub async fn set_permission(
        &self,
        session_id: i64,
        tool_call_id: &str,
        approved: bool,
    ) -> Result<(), String> {
        let enc_id = js_sys::encode_uri_component(tool_call_id).as_string().unwrap_or_else(|| tool_call_id.to_string());
        let _value: serde_json::Value = self
            .post(
                &format!("/sessions/{session_id}/permissions/{enc_id}"),
                &json!({ "approved": approved }),
            )
            .await?;
        Ok(())
    }

    /// The session's conversation: chat messages interleaved with the agent's
    /// tool steps, in order.
    pub async fn list_messages(&self, session_id: i64) -> Result<Vec<ConversationEntry>, String> {
        self.get(&format!("/sessions/{session_id}/messages")).await
    }

    /// Complete a tool-capable chat request via the backend provider.
    pub async fn chat_tools(&self, request: &ChatRequest) -> Result<ChatCompletion, String> {
        self.post("/chat-tools", request).await
    }

    /// Persist a user or assistant message to the session.
    pub async fn persist_message(
        &self,
        session_id: i64,
        role: Role,
        content: &str,
        usage: Option<&TurnTelemetry>,
    ) -> Result<ChatMessage, String> {
        self.post(
            &format!("/sessions/{session_id}/messages/persist"),
            &json!({ "role": role, "content": content, "usage": usage }),
        )
        .await
    }

    /// The model's context window, in tokens, as resolved by the backend
    /// (the connection's configured value, else provider discovery).
    /// `Ok(None)` when neither source reports one.
    pub async fn model_context(
        &self,
        connection_id: i64,
        model: Option<&str>,
    ) -> Result<Option<usize>, String> {
        let mut path = format!("/models/context?connection_id={connection_id}");
        if let Some(m) = model {
            path.push_str(&format!("&model={}", urlenc(m)));
        }
        let value: serde_json::Value = self.get(&path).await?;
        Ok(value
            .get("context_limit")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize))
    }

    /// Record (or refresh) an agent tool step.
    pub async fn upsert_tool_step(
        &self,
        session_id: i64,
        anchor_message_id: i64,
        tool_call_id: &str,
        name: &str,
        summary: &str,
    ) -> Result<(), String> {
        let _value: serde_json::Value = self
            .post(
                &format!("/sessions/{session_id}/tool-steps/upsert"),
                &json!({
                    "anchor_message_id": anchor_message_id,
                    "tool_call_id": tool_call_id,
                    "name": name,
                    "summary": summary,
                }),
            )
            .await?;
        Ok(())
    }

    /// Record the final outcome of an agent tool step.
    pub async fn complete_tool_step(
        &self,
        session_id: i64,
        tool_call_id: &str,
        ok: bool,
        result_summary: &str,
        diff: Option<&FileDiff>,
    ) -> Result<(), String> {
        let _value: serde_json::Value = self
            .post(
                &format!("/sessions/{session_id}/tool-steps/complete"),
                &json!({
                    "tool_call_id": tool_call_id,
                    "ok": ok,
                    "result_summary": result_summary,
                    "diff": diff,
                }),
            )
            .await?;
        Ok(())
    }

    /// Search the web for documentation, API references, or solutions.
    pub async fn web_search(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<WebSearchResult>, String> {
        self.get(&format!(
            "/web/search?query={}&limit={}",
            urlenc(query),
            limit
        ))
        .await
    }

    /// Fetch a web page and return sanitized Markdown.
    pub async fn fetch_web_page(&self, target_url: &str) -> Result<String, String> {
        let resp: serde_json::Value = self
            .get(&format!("/web/fetch?url={}", urlenc(target_url)))
            .await?;
        resp.get("content")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| "missing content in web fetch response".to_string())
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
        editor_context: Option<&EditorContext>,
        signal: Option<&AbortSignal>,
        mut on_event: impl FnMut(SseEvent),
    ) -> Result<(), String> {
        let mut builder = Request::post(&format!("{}/sessions/{session_id}/messages", self.base));
        if let Some(token) = self.token.get() {
            builder = builder.header("authorization", &format!("Bearer {token}"));
        }
        let req = builder
            .abort_signal(signal)
            .json(&json!({
                "content": content,
                "model": model,
                "editor_context": editor_context,
            }))
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
        let mut decoder = openwebide_core::utf8::Utf8Decoder::new();
        let mut frames = openwebide_frontend::sse::FrameBuffer::new();

        loop {
            let read = reader.read();
            // `reader.read()` resolves to a plain `{done, value}` object — a
            // dictionary type with no JS constructor — so `dyn_into` (an
            // `instanceof` check) would reject it. `unchecked_into` just
            // reinterprets the value, which is safe here because the shape is
            // guaranteed by the stream API.
            let chunk: ReadableStreamReadResult = JsFuture::from(read)
                .await
                .map_err(|e| format!("{e:?}"))?
                .unchecked_into();
            if chunk.get_done().unwrap_or(false) {
                break;
            }
            let value: js_sys::Uint8Array =
                chunk.get_value().dyn_into().map_err(|e| format!("{e:?}"))?;
            let bytes = value.to_vec();
            for f in frames.push(&decoder.push(&bytes)) {
                if let Some(event) = openwebide_frontend::sse::parse_frame(&f) {
                    on_event(event);
                }
            }
        }
        for f in frames.push(&decoder.finish()) {
            if let Some(event) = openwebide_frontend::sse::parse_frame(&f) {
                on_event(event);
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
        let mut builder = RequestBuilder::new(&url).method(method);
        if let Some(token) = self.token.get() {
            builder = builder.header("authorization", &format!("Bearer {token}"));
        }
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


fn query_param(query: &str, key: &str) -> Option<String> {
    query.trim_start_matches('?').split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=')?;
        (k == key).then(|| v.to_string())
    })
}

/// The bearer token cached in localStorage, if any.
fn read_token_from_storage() -> Option<String> {
    web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .and_then(|ls| ls.get_item("owide_token").ok().flatten())
}

fn write_token_to_storage(token: &str) {
    if let Some(ls) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) {
        let _ = ls.set_item("owide_token", token);
    }
}

fn clear_token_from_storage() {
    if let Some(ls) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) {
        let _ = ls.remove_item("owide_token");
    }
}

pub use openwebide_frontend::text::urlenc;
