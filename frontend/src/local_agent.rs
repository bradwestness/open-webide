//! Browser-driven agent loop for local-mode workspaces.
//!
//! When working on a folder picked via the Chromium File System Access API,
//! the agent loop executes directly in the browser against [`BrowserFsaVfs`],
//! requests LLM tool calls from the backend via `/api/chat-tools`, and persists
//! messages and tool steps to the backend store for full session parity.

use leptos::prelude::{GetUntracked, WithValue};
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
    EditorContext, ModelInfo, ProviderKind, Role, RunEvent, ToolCall, TurnTelemetry,
    WebSearchResult,
};
use openwebide_llm::{LlmProvider, ProviderError, StreamChunk, ToolStreamChunk, completion_chunks};

use crate::backend::Api;
use crate::local_fs::{BrowserFsaVfs, ForceSend};

use crate::util::sleep_ms;

/// An [`LlmProvider`] adapter that delegates completions to the backend's `/api/chat-tools`.
pub struct BrowserLlmProvider {
    api: Api,
    kind: ProviderKind,
    bridge: Option<crate::bridge::BridgeConn>,
}

unsafe impl Send for BrowserLlmProvider {}
unsafe impl Sync for BrowserLlmProvider {}

impl BrowserLlmProvider {
    pub fn new(api: Api, kind: ProviderKind, bridge: Option<crate::bridge::BridgeConn>) -> Self {
        Self { api, kind, bridge }
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
        let api = self.api;
        let request = request.clone();
        ForceSend(async move {
            match api.with_value(Clone::clone).chat_tools(&request).await {
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
        let api = self.api;
        let request = request.clone();
        ForceSend(async move {
            api.with_value(Clone::clone)
                .chat_tools(&request)
                .await
                .map_err(ProviderError::Http)
        })
    }

    fn chat_tools_stream(
        &self,
        request: &ChatRequest,
    ) -> Pin<Box<dyn Stream<Item = Result<ToolStreamChunk, ProviderError>> + Send + 'static>> {
        if let Some(bridge) = &self.bridge
            && bridge.status().get_untracked()
                == (crate::bridge::BridgeStatus::Ready { runs: true })
        {
            let id = format!("completion-{}", js_sys::Math::random());
            let receiver = bridge.register_completion(id.clone());
            let guard = CompletionGuard {
                bridge: bridge.clone(),
                id: id.clone(),
            };
            let error = bridge
                .send(openwebide_core::BridgeClientMessage::CompletionStart {
                    id,
                    request: request.clone(),
                })
                .err();
            return Box::pin(BrowserCompletionStream {
                receiver,
                _guard: guard,
                ended: false,
                error,
            });
        }
        let api = self.api;
        let request = request.clone();
        Box::pin(
            futures::stream::once(ForceSend(async move {
                let chunks = match api.with_value(Clone::clone).chat_tools(&request).await {
                    Ok(c) => completion_chunks(c).into_iter().map(Ok).collect::<Vec<_>>(),
                    Err(e) => vec![Err(ProviderError::Http(e))],
                };
                futures::stream::iter(chunks)
            }))
            .flatten(),
        )
    }

    fn context_limit(
        &self,
        _model: Option<&str>,
    ) -> impl Future<Output = Result<Option<usize>, ProviderError>> + Send {
        ForceSend(async move { Ok(None) })
    }
}

struct CompletionGuard {
    bridge: crate::bridge::BridgeConn,
    id: String,
}

impl Drop for CompletionGuard {
    fn drop(&mut self) {
        let _ = self
            .bridge
            .send(openwebide_core::BridgeClientMessage::CompletionCancel {
                id: self.id.clone(),
            });
        self.bridge.unregister_completion(&self.id);
    }
}

struct BrowserCompletionStream {
    receiver: futures::channel::mpsc::UnboundedReceiver<openwebide_core::BridgeServerMessage>,
    _guard: CompletionGuard,
    ended: bool,
    error: Option<String>,
}

// The browser agent polls and drops this stream only on the WASM main thread.
unsafe impl Send for BrowserCompletionStream {}

impl Stream for BrowserCompletionStream {
    type Item = Result<ToolStreamChunk, ProviderError>;

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        use openwebide_core::BridgeServerMessage;
        use std::task::Poll;
        if self.ended {
            return Poll::Ready(None);
        }
        if let Some(error) = self.error.take() {
            self.ended = true;
            return Poll::Ready(Some(Err(ProviderError::Http(error))));
        }
        match Pin::new(&mut self.receiver).poll_next(cx) {
            Poll::Ready(Some(BridgeServerMessage::CompletionChunk { chunk, .. })) => {
                Poll::Ready(Some(Ok(chunk)))
            }
            Poll::Ready(Some(BridgeServerMessage::CompletionEnd { error, .. })) => {
                self.ended = true;
                Poll::Ready(error.map(|error| Err(ProviderError::Http(error))))
            }
            Poll::Ready(None) => {
                self.ended = true;
                Poll::Ready(Some(Err(ProviderError::Http("bridge disconnected".into()))))
            }
            Poll::Pending => Poll::Pending,
            Poll::Ready(Some(_)) => unreachable!("only completion frames reach the receiver"),
        }
    }
}

/// Browser web client delegating web search and documentation fetching to the backend API.
#[derive(Clone)]
pub struct BrowserWebClient {
    api: Api,
}

impl BrowserWebClient {
    pub fn new(api: Api) -> Self {
        Self { api }
    }
}

impl WebClient for BrowserWebClient {
    fn search(
        &self,
        query: &str,
        limit: usize,
    ) -> impl Future<Output = Result<Vec<WebSearchResult>, String>> + Send {
        let api = self.api;
        let query = query.to_string();
        ForceSend(async move { api.with_value(Clone::clone).web_search(&query, limit).await })
    }

    fn fetch_page(&self, url: &str) -> impl Future<Output = Result<String, String>> + Send {
        let api = self.api;
        let url = url.to_string();
        ForceSend(async move { api.with_value(Clone::clone).fetch_web_page(&url).await })
    }
}

/// Browser bridge client sending command execution requests to the local bridge daemon.
#[derive(Clone)]
pub struct BrowserBridgeClient {
    http_url: String,
    credentials: crate::bridge::BridgeCredentials,
}

impl BrowserBridgeClient {
    pub fn new(http_url: String, credentials: crate::bridge::BridgeCredentials) -> Self {
        Self {
            http_url,
            credentials,
        }
    }
}

impl BridgeClient for BrowserBridgeClient {
    fn execute_command(
        &self,
        command: &str,
        timeout_seconds: u64,
    ) -> impl Future<Output = Result<CommandOutcome, String>> + Send {
        let endpoint = format!("{}/exec", self.http_url);
        let credentials = self.credentials.clone();
        let payload = serde_json::json!({
            "command": command,
            "timeout_seconds": timeout_seconds,
        })
        .to_string();

        ForceSend(async move {
            let token = credentials
                .credential()
                .await
                .map_err(|e| format!("auth error: {e}"))?;
            let resp = gloo_net::http::Request::post(&endpoint)
                .header("Content-Type", "application/json")
                .header("Authorization", &format!("Bearer {token}"))
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
        let endpoint = format!("{}/git/status", self.http_url);
        let credentials = self.credentials.clone();
        ForceSend(async move {
            let token = credentials
                .credential()
                .await
                .map_err(|e| format!("auth error: {e}"))?;
            let resp = gloo_net::http::Request::post(&endpoint)
                .header("Content-Type", "application/json")
                .header("Authorization", &format!("Bearer {token}"))
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
        let endpoint = format!("{}/git/diff", self.http_url);
        let credentials = self.credentials.clone();
        ForceSend(async move {
            let token = credentials
                .credential()
                .await
                .map_err(|e| format!("auth error: {e}"))?;
            let payload = serde_json::json!({ "path": path }).to_string();
            let resp = gloo_net::http::Request::post(&endpoint)
                .header("Content-Type", "application/json")
                .header("Authorization", &format!("Bearer {token}"))
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
        let endpoint = format!("{}/git/commit", self.http_url);
        let credentials = self.credentials.clone();
        ForceSend(async move {
            let token = credentials
                .credential()
                .await
                .map_err(|e| format!("auth error: {e}"))?;
            let resp = gloo_net::http::Request::post(&endpoint)
                .header("Content-Type", "application/json")
                .header("Authorization", &format!("Bearer {token}"))
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
        let endpoint = format!("{}/git/checkout", self.http_url);
        let credentials = self.credentials.clone();
        ForceSend(async move {
            let token = credentials
                .credential()
                .await
                .map_err(|e| format!("auth error: {e}"))?;
            let resp = gloo_net::http::Request::post(&endpoint)
                .header("Content-Type", "application/json")
                .header("Authorization", &format!("Bearer {token}"))
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
    api: Api,
    session_id: i64,
    user_content: String,
    model: Option<String>,
    editor_context: Option<EditorContext>,
    connection_id: i64,
    system_prompt: Option<String>,
    vfs: BrowserFsaVfs,
    cancel_flag: Arc<AtomicBool>,
    local_decisions: Arc<Mutex<HashMap<String, bool>>>,
    mut on_event: impl FnMut(RunEvent),
    bridge_config: crate::bridge::BridgeConfig,
    bridge_credentials: crate::bridge::BridgeCredentials,
    bridge_connection: Option<crate::bridge::BridgeConn>,
) -> Result<(), String> {
    // 1. Fetch prior conversation history before persisting the new message
    let history_entries = api
        .with_value(Clone::clone)
        .list_messages(session_id)
        .await
        .unwrap_or_default();
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
        .with_value(Clone::clone)
        .persist_message(session_id, Role::User, &full_content, None)
        .await?;
    on_event(RunEvent::Message {
        message: user_message.clone(),
    });
    messages.push(user_message.clone());

    // 3. Assemble chat request with standard workspace tools
    let request = ChatRequest {
        connection_id,
        system_prompt,
        model,
        messages,
        tools: openwebide_agent::vfs_tools(),
    };

    let web = BrowserWebClient::new(api);
    let bridge = BrowserBridgeClient::new(bridge_config.http_url, bridge_credentials);
    let executor = VfsToolExecutor::with_web_and_bridge(vfs, web, bridge);
    let provider = BrowserLlmProvider::new(api, ProviderKind::Ollama, bridge_connection);
    let cancel = LocalCancelCheck {
        flag: cancel_flag.clone(),
    };
    let gate = LocalPermissionGate {
        decisions: local_decisions,
        cancel: cancel_flag,
    };

    // 4. Drive agent stream
    let anchor_id = user_message.id;
    let mut display_anchor = anchor_id;
    let mut stream = openwebide_agent::run(
        provider,
        executor,
        request,
        AgentConfig::default(),
        cancel,
        gate,
        anchor_id,
    );

    let mut last_usage: Option<TurnTelemetry> = None;
    while let Some(event) = stream.next().await {
        match event {
            AgentEvent::TextDelta(delta) => on_event(RunEvent::Delta { content: delta }),
            AgentEvent::TurnText(text) => {
                let usage = last_usage.take();
                let message = api
                    .with_value(Clone::clone)
                    .persist_message(session_id, Role::Assistant, &text, usage.as_ref())
                    .await;
                let message = match message {
                    Ok(message) => {
                        display_anchor = message.id;
                        message
                    }
                    Err(_) => ChatMessage {
                        id: 0,
                        session_id,
                        role: Role::Assistant,
                        content: text,
                        created_at: 0,
                        tool_calls: None,
                        tool_call_id: None,
                        usage,
                    },
                };
                on_event(RunEvent::Interim { message });
            }
            AgentEvent::Telemetry(usage) => {
                last_usage = Some(usage);
                on_event(RunEvent::Telemetry { usage });
            }
            AgentEvent::ToolCall { id, name, summary } => {
                last_usage = None;
                let _ = api
                    .with_value(Clone::clone)
                    .upsert_tool_step(session_id, display_anchor, &id, &name, &summary)
                    .await;
                on_event(RunEvent::ToolCall { id, name, summary });
            }
            AgentEvent::PermissionRequest { id, name, summary } => {
                last_usage = None;
                let _ = api
                    .with_value(Clone::clone)
                    .upsert_tool_step(session_id, display_anchor, &id, &name, &summary)
                    .await;
                on_event(RunEvent::PermissionRequest { id, name, summary });
            }
            AgentEvent::ToolResult {
                id,
                name,
                ok,
                summary,
                diff,
            } => {
                let _ = api
                    .with_value(Clone::clone)
                    .complete_tool_step(session_id, &id, ok, &summary, diff.as_ref())
                    .await;
                on_event(RunEvent::ToolResult {
                    id,
                    name,
                    ok,
                    summary,
                    diff,
                });
            }
            AgentEvent::FinalText(text) => {
                match api
                    .with_value(Clone::clone)
                    .persist_message(
                        session_id,
                        Role::Assistant,
                        &text,
                        last_usage.take().as_ref(),
                    )
                    .await
                {
                    Ok(msg) => on_event(RunEvent::Done { message: msg }),
                    Err(e) => on_event(RunEvent::Error {
                        message: format!("failed to save reply: {e}"),
                    }),
                }
            }
            AgentEvent::Cancelled => {
                on_event(RunEvent::Cancelled);
            }
            AgentEvent::Error(message) => {
                on_event(RunEvent::Error { message });
            }
        }
    }

    Ok(())
}
