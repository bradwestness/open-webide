//! Thin client for the backend REST API, including SSE streaming.

use gloo_net::http::{Method, RequestBuilder};

use leptos::prelude::{GetUntracked, Set, WithValue};
use openwebide_core::{
    ChatCompletion, ChatMessage, ChatRequest, ChatSession, Connection, ConversationEntry,
    EditorContext, FileDiff, FileEntry, GitBranchInfo, GitCheckoutRequest, GitCheckoutResult,
    GitCommitRequest, GitCommitResult, GitRepoStatus, GitSyncRequest, GitSyncResult, Health,
    ModelInfo, NewConnection, NewProject, NewSession, Project, ProviderKind, Role, RunEvent,
    SearchHit, SystemPrompt, TurnTelemetry, User, WebSearchResult, WorkspaceMode,
    vfs::SearchOptions,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::json;
use wasm_bindgen_futures::JsFuture;
use web_sys::wasm_bindgen::JsCast;
use web_sys::{AbortSignal, ReadableStreamDefaultReader, ReadableStreamReadResult};

pub(crate) struct CommandFetchGuard(pub(crate) web_sys::AbortController);

impl Drop for CommandFetchGuard {
    fn drop(&mut self) {
        // Dropping a fetch future alone leaves the browser's HTTP connection occupied.
        self.0.abort();
    }
}

#[derive(Clone, Copy)]
pub struct BackendApi {
    base: leptos::prelude::StoredValue<String>,
    pub signed_in: leptos::prelude::RwSignal<bool>,
    pub session_expired: leptos::prelude::RwSignal<bool>,
    cross_origin: bool,
}

/// The `{user, token}` payload returned by register and login.
#[derive(Deserialize)]
struct AuthResponse {
    user: User,
}

#[derive(Clone, Debug, PartialEq)]
pub enum HealthState {
    Online { version: String },
    Offline,
}

impl BackendApi {
    fn base(&self) -> String {
        self.base.with_value(String::clone)
    }

    pub fn session_expired(&self) -> leptos::prelude::ReadSignal<bool> {
        self.session_expired.read_only()
    }

    /// Same-origin by default; `?api=http://host:port/api` overrides the
    /// base for development against a separately served backend.
    pub fn from_location() -> Self {
        let location = web_sys::window().map(|w| w.location());
        let origin = location
            .as_ref()
            .and_then(|l| l.origin().ok())
            .unwrap_or_else(|| "http://localhost:3000".to_string());
        let base = location
            .clone()
            .and_then(|l| l.search().ok())
            .and_then(|search| query_param(&search, "api"))
            .map(|v| v.trim_end_matches('/').to_string())
            .unwrap_or_else(|| format!("{origin}/api"));

        let cross_origin = !base.starts_with(&origin);

        if let Some(window) = web_sys::window()
            && let Ok(Some(storage)) = window.local_storage()
        {
            let _ = storage.remove_item("owide_token");
        }

        Self {
            base: leptos::prelude::StoredValue::new(base),
            signed_in: leptos::prelude::RwSignal::new(false),
            session_expired: leptos::prelude::RwSignal::new(false),
            cross_origin,
        }
    }

    // -- auth --------------------------------------------------------------

    /// Register a new local account (only the first account may register).
    /// On success the returned token is stored for subsequent requests.
    pub async fn register(&self, username: &str, password: &str) -> Result<User, String> {
        let resp: AuthResponse = self
            .post(
                "/auth/register",
                &json!({ "username": username, "password": password }),
            )
            .await?;
        self.signed_in.set(true);
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
        self.signed_in.set(true);
        Ok(resp.user)
    }

    /// The account for the current token.
    pub async fn me(&self) -> Result<User, String> {
        let resp: serde_json::Value = self.get("/auth/me").await?;
        let user: User = serde_json::from_value(resp["user"].clone()).map_err(|e| e.to_string())?;
        self.signed_in.set(true);
        Ok(user)
    }

    pub async fn logout(&self) -> Result<(), String> {
        let _ = self
            .post::<_, serde_json::Value>("/auth/logout", &json!({}))
            .await;
        self.signed_in.set(false);
        Ok(())
    }

    pub async fn bridge_token(&self) -> Result<(String, i64), String> {
        #[derive(serde::Deserialize)]
        struct TokenResp {
            token: String,
            expires_at: i64,
        }
        let resp: TokenResp = self.post("/bridge/token", &json!({})).await?;
        Ok((resp.token, resp.expires_at))
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
        self.request::<(), _>(Method::DELETE, &format!("/connections/{id}"), None, false)
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
        self.request::<(), _>(
            Method::DELETE,
            &format!("/system-prompts/{id}"),
            None,
            false,
        )
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

    pub async fn rename_project(&self, id: i64, name: &str) -> Result<Project, String> {
        self.put(&format!("/projects/{id}"), &json!({ "name": name }))
            .await
    }

    pub async fn delete_project(&self, id: i64) -> Result<(), String> {
        self.request::<(), _>(Method::DELETE, &format!("/projects/{id}"), None, false)
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
            self.base(),
            urlenc(path)
        );
        let builder = self.builder(&url, Method::GET);
        let req = builder.build().map_err(|e| e.to_string())?;

        let is_signed_in = self.signed_in.get_untracked();
        let resp = req.send().await.map_err(|e| e.to_string())?;
        if !resp.ok() {
            if resp.status() == 401 && is_signed_in {
                self.signed_in.set(false);
                self.session_expired.set(true);
            }
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

    pub async fn read_file_lossy(&self, project_id: i64, path: &str) -> Result<String, String> {
        let url = format!(
            "{}/projects/{project_id}/files/raw?path={}",
            self.base(),
            urlenc(path)
        );
        let builder = self.builder(&url, Method::GET);
        let req = builder.build().map_err(|e| e.to_string())?;

        let is_signed_in = self.signed_in.get_untracked();
        let resp = req.send().await.map_err(|e| e.to_string())?;
        if !resp.ok() {
            if resp.status() == 401 && is_signed_in {
                self.signed_in.set(false);
                self.session_expired.set(true);
            }
            return Err(self.error_from(resp).await);
        }
        let bytes = resp.binary().await.map_err(|e| e.to_string())?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
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
            self.base(),
            urlenc(path)
        );
        let builder = self
            .builder(&url, Method::PUT)
            .header("content-type", "text/plain");
        let req = builder
            .body(content.to_string())
            .map_err(|e| e.to_string())?;

        let is_signed_in = self.signed_in.get_untracked();
        let resp = req.send().await.map_err(|e| e.to_string())?;
        if !resp.ok() {
            if resp.status() == 401 && is_signed_in {
                self.signed_in.set(false);
                self.session_expired.set(true);
            }
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
            false,
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
            false,
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
        opts: SearchOptions,
    ) -> Result<Vec<SearchHit>, String> {
        let mut url = format!(
            "/projects/{project_id}/files/content-search?q={}&path={}",
            urlenc(query),
            urlenc(path)
        );
        if opts.include_ignored {
            url.push_str("&include_ignored=1");
        }
        self.get(&url).await
    }

    // -- git operations (Phase 13) -----------------------------------------

    fn git_endpoint(project_id: Option<i64>, sub: &str) -> String {
        if let Some(id) = project_id {
            format!("/projects/{id}/git/{sub}")
        } else {
            format!("/git/{sub}")
        }
    }

    pub async fn git_status(&self, project_id: Option<i64>) -> Result<GitRepoStatus, String> {
        self.get(&Self::git_endpoint(project_id, "status")).await
    }

    pub async fn git_diff(
        &self,
        project_id: Option<i64>,
        path: Option<&str>,
    ) -> Result<String, String> {
        let ep = Self::git_endpoint(project_id, "diff");
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
        let ep = Self::git_endpoint(project_id, "show");
        let query = format!("{ep}?path={}", urlenc(path));
        let res: serde_json::Value = self.get(&query).await.map_err(|error| {
            if error == "binary file at HEAD" {
                "binary file".into()
            } else {
                error
            }
        })?;
        Ok(res["content"].as_str().unwrap_or_default().to_string())
    }

    pub async fn git_branches(
        &self,
        project_id: Option<i64>,
    ) -> Result<Vec<GitBranchInfo>, String> {
        self.get(&Self::git_endpoint(project_id, "branches")).await
    }

    pub async fn git_commit(
        &self,
        project_id: Option<i64>,
        req: &GitCommitRequest,
    ) -> Result<GitCommitResult, String> {
        self.post(&Self::git_endpoint(project_id, "commit"), req)
            .await
    }

    pub async fn git_checkout(
        &self,
        project_id: Option<i64>,
        req: &GitCheckoutRequest,
    ) -> Result<GitCheckoutResult, String> {
        self.post(&Self::git_endpoint(project_id, "checkout"), req)
            .await
    }

    pub async fn git_sync(
        &self,
        project_id: Option<i64>,
        req: &GitSyncRequest,
    ) -> Result<GitSyncResult, String> {
        self.post(&Self::git_endpoint(project_id, "sync"), req)
            .await
    }

    pub async fn rename_session(&self, id: i64, name: &str) -> Result<ChatSession, String> {
        self.put(&format!("/sessions/{id}"), &json!({ "name": name }))
            .await
    }

    pub async fn delete_session(&self, id: i64) -> Result<(), String> {
        self.request::<(), _>(Method::DELETE, &format!("/sessions/{id}"), None, false)
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
                false,
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
        let enc_id = js_sys::encode_uri_component(tool_call_id)
            .as_string()
            .unwrap_or_else(|| tool_call_id.to_string());
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
        tool_calls: Option<&[openwebide_core::ToolCall]>,
    ) -> Result<ChatMessage, String> {
        self.post(
            &format!("/sessions/{session_id}/messages/persist"),
            &json!({ "role": role, "content": content, "usage": usage, "tool_calls": tool_calls }),
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
            .and_then(serde_json::Value::as_u64)
            .map(|n| usize::try_from(n).unwrap_or(usize::MAX)))
    }

    /// Record (or refresh) an agent tool step.
    #[allow(clippy::too_many_arguments)]
    pub async fn upsert_tool_step(
        &self,
        session_id: i64,
        anchor_message_id: i64,
        tool_call_id: &str,
        name: &str,
        summary: &str,
        diff: Option<&FileDiff>,
    ) -> Result<(), String> {
        let _value: serde_json::Value = self
            .post(
                &format!("/sessions/{session_id}/tool-steps/upsert"),
                &json!({
                    "anchor_message_id": anchor_message_id,
                    "tool_call_id": tool_call_id,
                    "name": name,
                    "summary": summary,
                    "diff": diff,
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
        self.request::<(), _>(
            Method::GET,
            &format!("/web/search?query={}&limit={}", urlenc(query), limit),
            None,
            true,
        )
        .await
    }

    /// Fetch a web page and return sanitized Markdown.
    pub async fn fetch_web_page(&self, target_url: &str) -> Result<String, String> {
        let resp: serde_json::Value = self
            .request::<(), _>(
                Method::GET,
                &format!("/web/fetch?url={}", urlenc(target_url)),
                None,
                true,
            )
            .await?;
        resp.get("content")
            .and_then(|v| v.as_str())
            .map(ToString::to_string)
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
        mut on_event: impl FnMut(RunEvent),
    ) -> Result<(), String> {
        let url = format!("{}/sessions/{session_id}/messages", self.base());
        let builder = self.builder(&url, Method::POST);
        let req = builder
            .abort_signal(signal)
            .json(&json!({
                "content": content,
                "model": model,
                "editor_context": editor_context,
            }))
            .map_err(|e| e.to_string())?;

        let is_signed_in = self.signed_in.get_untracked();
        let resp = req.send().await.map_err(|e| e.to_string())?;
        if !resp.ok() {
            if resp.status() == 401 && is_signed_in {
                self.signed_in.set(false);
                self.session_expired.set(true);
            }
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
        let mut frames = crate::sse::FrameBuffer::new();

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
                if let Some(event) = crate::sse::parse_frame(&f) {
                    on_event(event);
                }
            }
        }
        for f in frames.push(&decoder.finish()) {
            if let Some(event) = crate::sse::parse_frame(&f) {
                on_event(event);
            }
        }
        Ok(())
    }

    async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, String> {
        self.request::<(), T>(Method::GET, path, None, false).await
    }

    async fn post<T: Serialize, R: DeserializeOwned>(
        &self,
        path: &str,
        body: &T,
    ) -> Result<R, String> {
        self.request(Method::POST, path, Some(body), false).await
    }

    async fn put<T: Serialize, R: DeserializeOwned>(
        &self,
        path: &str,
        body: &T,
    ) -> Result<R, String> {
        self.request(Method::PUT, path, Some(body), false).await
    }

    fn builder(&self, url: &str, method: Method) -> RequestBuilder {
        let mut builder = RequestBuilder::new(url)
            .method(method)
            .header("x-openwebide", "1");
        if self.cross_origin {
            builder = builder.credentials(web_sys::RequestCredentials::Include);
        }
        builder
    }

    async fn request<T, R>(
        &self,
        method: Method,
        path: &str,
        body: Option<&T>,
        abort_on_drop: bool,
    ) -> Result<R, String>
    where
        T: Serialize,
        R: DeserializeOwned,
    {
        let url = format!("{}{path}", self.base());
        let is_delete = method == Method::DELETE;
        let guard = if abort_on_drop {
            Some(CommandFetchGuard(
                web_sys::AbortController::new()
                    .map_err(|e| format!("request cancellation error: {e:?}"))?,
            ))
        } else {
            None
        };
        let signal = guard.as_ref().map(|guard| guard.0.signal());
        let builder = self.builder(&url, method).abort_signal(signal.as_ref());
        let req = match body {
            Some(body) => builder.json(body),
            None => builder.build(),
        }
        .map_err(|e| e.to_string())?;

        let is_signed_in = self.signed_in.get_untracked();
        let resp = req.send().await.map_err(|e| e.to_string())?;
        if !resp.ok() {
            if resp.status() == 401 && is_signed_in && !path.starts_with("/auth/") {
                self.signed_in.set(false);
                self.session_expired.set(true);
            }
            return Err(self.error_from(resp).await);
        }
        if is_delete {
            // Delete responses need no decoded body.
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

pub use crate::text::urlenc;

#[cfg(test)]
mod cancellation_tests {
    use super::*;
    use wasm_bindgen::prelude::*;
    use wasm_bindgen_test::*;

    wasm_bindgen_test_configure!(run_in_browser);

    #[wasm_bindgen(
        inline_js = "let original; let signal; export function captureFetch() { original = window.fetch; window.fetch = request => { signal = request.signal; return new Promise(() => {}); }; } export function capturedSignal() { return signal; } export function restoreFetch() { window.fetch = original; }"
    )]
    extern "C" {
        #[wasm_bindgen(js_name = captureFetch)]
        fn capture_fetch();
        #[wasm_bindgen(js_name = capturedSignal)]
        fn captured_signal() -> AbortSignal;
        #[wasm_bindgen(js_name = restoreFetch)]
        fn restore_fetch();
    }
    #[wasm_bindgen_test]
    async fn dropping_web_tool_requests_aborts_fetch() {
        let owner = leptos::prelude::Owner::new();
        let api = owner.with(BackendApi::from_location);
        capture_fetch();
        let mut fetch = Box::pin(api.fetch_web_page("https://example.com"));
        assert!(futures::poll!(&mut fetch).is_pending());
        let signal = captured_signal();
        assert!(!signal.aborted());
        drop(fetch);
        assert!(signal.aborted());
        let mut search = Box::pin(api.web_search("example", 1));
        assert!(futures::poll!(&mut search).is_pending());
        let signal = captured_signal();
        assert!(!signal.aborted());
        drop(search);
        assert!(signal.aborted());
        restore_fetch();
        owner.cleanup();
    }
}
