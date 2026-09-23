//! Browser-driven agent loop for local-mode workspaces.
//!
//! When working on a folder picked via the Chromium File System Access API,
//! the agent loop executes directly in the browser against [`BrowserFsaVfs`],
//! requests LLM tool calls from the backend via `/api/chat-tools`, and persists
//! messages and tool steps to the backend store for full session parity.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use futures::{Stream, StreamExt};
use openwebide_agent::{
    AgentConfig, AgentEvent, BridgeClient, CancelCheck, PermissionGate, VfsToolExecutor, WebClient,
};
use openwebide_core::{
    ChatCompletion, ChatMessage, ChatRequest, ChatResponse, CommandOutcome, ConversationEntry,
    EditorContext, ModelInfo, ProviderKind, Role, ToolCall, TurnTelemetry, WebSearchResult,
};
use openwebide_llm::{LlmProvider, ProviderError, StreamChunk};

use crate::api::{BackendApi, SseEvent};
use crate::local_fs::{BrowserFsaVfs, ForceSend};

async fn sleep_ms(ms: i32) {
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        if let Some(window) = web_sys::window() {
            let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms);
        }
    });
    let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
}

/// An [`LlmProvider`] adapter that delegates completions to the backend's `/api/chat-tools`.
pub struct BrowserLlmProvider {
    api: BackendApi,
    kind: ProviderKind,
}

unsafe impl Send for BrowserLlmProvider {}
unsafe impl Sync for BrowserLlmProvider {}

impl BrowserLlmProvider {
    pub fn new(api: BackendApi, kind: ProviderKind) -> Self {
        Self { api, kind }
    }
}

impl LlmProvider for BrowserLlmProvider {
    fn kind(&self) -> ProviderKind {
        self.kind
    }

    fn list_models(&self) -> impl Future<Output = Result<Vec<ModelInfo>, ProviderError>> + Send {
        ForceSend(async move {
            Err(ProviderError::NotImplemented(
                "list_models not supported in browser provider".into(),
            ))
        })
    }

    fn chat(
        &self,
        request: &ChatRequest,
    ) -> impl Future<Output = Result<String, ProviderError>> + Send {
        let api = self.api.clone();
        let request = request.clone();
        ForceSend(async move {
            match api.chat_tools(&request).await {
                Ok(completion) => match completion.response {
                    ChatResponse::Text(s) => Ok(s),
                    ChatResponse::ToolCalls(_) => Err(ProviderError::Parse("expected text".into())),
                },
                Err(e) => Err(ProviderError::Http(e)),
            }
        })
    }

    fn chat_stream(
        &self,
        _request: &ChatRequest,
    ) -> Pin<Box<dyn Stream<Item = Result<StreamChunk, ProviderError>> + Send + 'static>> {
        Box::pin(futures::stream::empty())
    }

    fn chat_tools(
        &self,
        request: &ChatRequest,
    ) -> impl Future<Output = Result<ChatCompletion, ProviderError>> + Send {
        let api = self.api.clone();
        let request = request.clone();
        ForceSend(async move { api.chat_tools(&request).await.map_err(ProviderError::Http) })
    }

    fn context_limit(
        &self,
        _model: Option<&str>,
    ) -> impl Future<Output = Result<Option<usize>, ProviderError>> + Send {
        ForceSend(async move { Ok(None) })
    }
}

/// Browser web client delegating web search and documentation fetching to the backend API.
#[derive(Clone)]
pub struct BrowserWebClient {
    api: BackendApi,
}

impl BrowserWebClient {
    pub fn new(api: BackendApi) -> Self {
        Self { api }
    }
}

impl WebClient for BrowserWebClient {
    fn search(
        &self,
        query: &str,
        limit: usize,
    ) -> impl Future<Output = Result<Vec<WebSearchResult>, String>> + Send {
        let api = self.api.clone();
        let query = query.to_string();
        ForceSend(async move { api.web_search(&query, limit).await })
    }

    fn fetch_page(&self, url: &str) -> impl Future<Output = Result<String, String>> + Send {
        let api = self.api.clone();
        let url = url.to_string();
        ForceSend(async move { api.fetch_web_page(&url).await })
    }
}

/// Browser bridge client sending command execution requests to the local bridge daemon.
#[derive(Clone)]
pub struct BrowserBridgeClient {
    endpoint: String,
}

impl Default for BrowserBridgeClient {
    fn default() -> Self {
        Self {
            endpoint: "http://127.0.0.1:3001/exec".to_string(),
        }
    }
}

impl BridgeClient for BrowserBridgeClient {
    fn execute_command(
        &self,
        command: &str,
        timeout_seconds: u64,
    ) -> impl Future<Output = Result<CommandOutcome, String>> + Send {
        let endpoint = self.endpoint.clone();
        let payload = serde_json::json!({
            "command": command,
            "timeout_seconds": timeout_seconds,
        })
        .to_string();

        ForceSend(async move {
            let resp = gloo_net::http::Request::post(&endpoint)
                .header("Content-Type", "application/json")
                .body(payload)
                .map_err(|e| format!("request error: {e}"))?
                .send()
                .await
                .map_err(|e| {
                    format!("Failed to connect to bridge daemon at {endpoint}: {e}. Ensure 'openwebide-bridge' is running.")
                })?;

            if !resp.ok() {
                let err_text = resp.text().await.unwrap_or_default();
                return Err(format!("Bridge error (HTTP {}): {err_text}", resp.status()));
            }

            resp.json::<CommandOutcome>()
                .await
                .map_err(|e| format!("Failed to parse bridge outcome JSON: {e}"))
        })
    }

    fn git_status(
        &self,
    ) -> impl Future<Output = Result<openwebide_core::GitRepoStatus, String>> + Send {
        ForceSend(async move {
            let resp = gloo_net::http::Request::post("http://127.0.0.1:3001/git/status")
                .header("Content-Type", "application/json")
                .body("{}")
                .map_err(|e| format!("request error: {e}"))?
                .send()
                .await
                .map_err(|e| format!("Bridge connection error: {e}"))?;
            if !resp.ok() {
                let err_text = resp.text().await.unwrap_or_default();
                return Err(format!("Bridge error (HTTP {}): {err_text}", resp.status()));
            }
            resp.json::<openwebide_core::GitRepoStatus>()
                .await
                .map_err(|e| format!("Parse error: {e}"))
        })
    }

    fn git_diff(&self, path: Option<&str>) -> impl Future<Output = Result<String, String>> + Send {
        let path = path.map(|s| s.to_string());
        ForceSend(async move {
            let payload = serde_json::json!({ "path": path }).to_string();
            let resp = gloo_net::http::Request::post("http://127.0.0.1:3001/git/diff")
                .header("Content-Type", "application/json")
                .body(payload)
                .map_err(|e| format!("request error: {e}"))?
                .send()
                .await
                .map_err(|e| format!("Bridge connection error: {e}"))?;
            if !resp.ok() {
                let err_text = resp.text().await.unwrap_or_default();
                return Err(format!("Bridge error (HTTP {}): {err_text}", resp.status()));
            }
            #[derive(serde::Deserialize)]
            struct DiffOut {
                diff: String,
            }
            resp.json::<DiffOut>()
                .await
                .map(|d| d.diff)
                .map_err(|e| format!("Parse error: {e}"))
        })
    }

    fn git_commit(
        &self,
        req: &openwebide_core::GitCommitRequest,
    ) -> impl Future<Output = Result<openwebide_core::GitCommitResult, String>> + Send {
        let payload = serde_json::to_string(req).unwrap_or_default();
        ForceSend(async move {
            let resp = gloo_net::http::Request::post("http://127.0.0.1:3001/git/commit")
                .header("Content-Type", "application/json")
                .body(payload)
                .map_err(|e| format!("request error: {e}"))?
                .send()
                .await
                .map_err(|e| format!("Bridge connection error: {e}"))?;
            if !resp.ok() {
                let err_text = resp.text().await.unwrap_or_default();
                return Err(format!("Bridge error (HTTP {}): {err_text}", resp.status()));
            }
            resp.json::<openwebide_core::GitCommitResult>()
                .await
                .map_err(|e| format!("Parse error: {e}"))
        })
    }

    fn git_checkout(
        &self,
        req: &openwebide_core::GitCheckoutRequest,
    ) -> impl Future<Output = Result<openwebide_core::GitCheckoutResult, String>> + Send {
        let payload = serde_json::to_string(req).unwrap_or_default();
        ForceSend(async move {
            let resp = gloo_net::http::Request::post("http://127.0.0.1:3001/git/checkout")
                .header("Content-Type", "application/json")
                .body(payload)
                .map_err(|e| format!("request error: {e}"))?
                .send()
                .await
                .map_err(|e| format!("Bridge connection error: {e}"))?;
            if !resp.ok() {
                let err_text = resp.text().await.unwrap_or_default();
                return Err(format!("Bridge error (HTTP {}): {err_text}", resp.status()));
            }
            resp.json::<openwebide_core::GitCheckoutResult>()
                .await
                .map_err(|e| format!("Parse error: {e}"))
        })
    }
}

/// Local-mode cancel checker observing a shared atomic flag.
#[derive(Clone)]
pub struct LocalCancelCheck {
    pub flag: Arc<AtomicBool>,
}

impl CancelCheck for LocalCancelCheck {
    fn check(&self) -> impl Future<Output = bool> + Send {
        let flag = self.flag.clone();
        ForceSend(async move { flag.load(Ordering::Relaxed) })
    }
}

/// Local-mode permission gate requiring approval for file modifications.
pub struct LocalPermissionGate {
    pub decisions: Arc<Mutex<HashMap<String, bool>>>,
    pub cancel: Arc<AtomicBool>,
}

impl PermissionGate for LocalPermissionGate {
    fn needs_approval(&self, call: &ToolCall) -> bool {
        call.name == "write_file"
            || call.name == "run_command"
            || call.name == "git_commit"
            || call.name == "git_branch"
    }

    fn approve(&self, call: &ToolCall) -> impl Future<Output = bool> + Send {
        let decisions = self.decisions.clone();
        let cancel = self.cancel.clone();
        let id = call.id.clone();
        ForceSend(async move {
            for _ in 0..3000 {
                if cancel.load(Ordering::Relaxed) {
                    return false;
                }
                if let Some(decision) = decisions.lock().unwrap().remove(&id) {
                    return decision;
                }
                sleep_ms(100).await;
            }
            false
        })
    }
}

/// Run the agent loop locally in the browser against a local folder handle.
#[allow(clippy::too_many_arguments)]
pub async fn run_local_agent(
    api: BackendApi,
    session_id: i64,
    user_content: String,
    model: Option<String>,
    editor_context: Option<EditorContext>,
    connection_id: i64,
    system_prompt: Option<String>,
    vfs: BrowserFsaVfs,
    cancel_flag: Arc<AtomicBool>,
    local_decisions: Arc<Mutex<HashMap<String, bool>>>,
    mut on_event: impl FnMut(SseEvent),
) -> Result<(), String> {
    // 1. Fetch prior conversation history before persisting the new message
    let history_entries = api.list_messages(session_id).await.unwrap_or_default();
    let mut messages: Vec<ChatMessage> = history_entries
        .into_iter()
        .filter_map(|item| match item {
            ConversationEntry::Message(m) => Some(m),
            ConversationEntry::ToolStep(_) => None,
        })
        .collect();

    // 2. Prepend editor context if present and persist user message to backend store
    let full_content = match &editor_context {
        Some(ctx) => format!("{}{}", ctx.format_prompt_injection(), user_content),
        None => user_content,
    };
    let user_message = api
        .persist_message(session_id, Role::User, &full_content, None)
        .await?;
    on_event(SseEvent::Message(user_message.clone()));
    messages.push(user_message.clone());

    // 3. Assemble chat request with standard workspace tools
    let request = ChatRequest {
        connection_id,
        system_prompt,
        model,
        messages,
        tools: openwebide_agent::vfs_tools(),
    };

    let web = BrowserWebClient::new(api.clone());
    let bridge = BrowserBridgeClient::default();
    let executor = VfsToolExecutor::with_web_and_bridge(vfs, web, bridge);
    let provider = BrowserLlmProvider::new(api.clone(), ProviderKind::Ollama);
    let cancel = LocalCancelCheck {
        flag: cancel_flag.clone(),
    };
    let gate = LocalPermissionGate {
        decisions: local_decisions,
        cancel: cancel_flag,
    };

    // 4. Drive agent stream
    let anchor = user_message.id;
    let mut stream = openwebide_agent::run(
        provider,
        executor,
        request,
        AgentConfig::default(),
        cancel,
        gate,
        anchor,
    );

    let mut last_usage: Option<TurnTelemetry> = None;
    while let Some(event) = stream.next().await {
        match event {
            AgentEvent::Telemetry(usage) => {
                last_usage = Some(usage);
                on_event(SseEvent::Telemetry(usage));
            }
            AgentEvent::ToolCall { id, name, summary } => {
                let _ = api
                    .upsert_tool_step(session_id, anchor, &id, &name, &summary)
                    .await;
                on_event(SseEvent::ToolCall { id, name, summary });
            }
            AgentEvent::PermissionRequest { id, name, summary } => {
                let _ = api
                    .upsert_tool_step(session_id, anchor, &id, &name, &summary)
                    .await;
                on_event(SseEvent::PermissionRequest { id, name, summary });
            }
            AgentEvent::ToolResult {
                id,
                name,
                ok,
                summary,
                diff,
            } => {
                let _ = api
                    .complete_tool_step(session_id, &id, ok, &summary, diff.as_ref())
                    .await;
                on_event(SseEvent::ToolResult {
                    id,
                    name,
                    ok,
                    summary,
                    diff,
                });
            }
            AgentEvent::FinalText(text) => {
                match api
                    .persist_message(session_id, Role::Assistant, &text, last_usage.as_ref())
                    .await
                {
                    Ok(msg) => on_event(SseEvent::Done(msg)),
                    Err(e) => on_event(SseEvent::Error(format!("failed to save reply: {e}"))),
                }
            }
            AgentEvent::Cancelled => {
                on_event(SseEvent::Cancelled);
            }
            AgentEvent::Error(message) => {
                on_event(SseEvent::Error(message));
            }
        }
    }

    Ok(())
}
