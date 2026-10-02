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
use send_wrapper::SendWrapper;

use crate::backend::Api;
use crate::local_fs::BrowserFsaVfs;

use crate::util::sleep_ms;

/// An [`LlmProvider`] adapter that delegates completions to the backend's `/api/chat-tools`.
pub struct BrowserLlmProvider {
    // Local fields and futures stay on the browser spawn_local thread.
    // SendWrapper checks access/poll/drop without changing native provider contracts.
    api: SendWrapper<Api>,
    kind: ProviderKind,
    bridge: SendWrapper<Option<crate::bridge::BridgeConn>>,
}

impl BrowserLlmProvider {
    pub fn new(api: Api, kind: ProviderKind, bridge: Option<crate::bridge::BridgeConn>) -> Self {
        Self {
            api: SendWrapper::new(api),
            kind,
            bridge: SendWrapper::new(bridge),
        }
    }
}

impl LlmProvider for BrowserLlmProvider {
    fn kind(&self) -> ProviderKind {
        self.kind
    }

    fn list_models(&self) -> impl Future<Output = Result<Vec<ModelInfo>, ProviderError>> + Send {
        SendWrapper::new(async move {
            Err(ProviderError::NotImplemented(
                "list_models not supported in browser provider".into(),
            ))
        })
    }

    fn chat(
        &self,
        request: &ChatRequest,
    ) -> impl Future<Output = Result<String, ProviderError>> + Send {
        let api = *self.api;
        let request = request.clone();
        SendWrapper::new(async move {
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
        let api = *self.api;
        let request = request.clone();
        SendWrapper::new(async move {
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
        if let Some(bridge) = &*self.bridge
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
            // Guard the whole stream so cancellation in CompletionGuard::drop
            // is also restricted to the thread that created the bridge receiver.
            return Box::pin(SendWrapper::new(BrowserCompletionStream {
                receiver,
                _guard: guard,
                ended: false,
                error,
            }));
        }
        let api = *self.api;
        let request = request.clone();
        Box::pin(
            futures::stream::once(SendWrapper::new(async move {
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
        SendWrapper::new(async move { Ok(None) })
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
        SendWrapper::new(
            async move { api.with_value(Clone::clone).web_search(&query, limit).await },
        )
    }

    fn fetch_page(&self, url: &str) -> impl Future<Output = Result<String, String>> + Send {
        let api = self.api;
        let url = url.to_string();
        SendWrapper::new(async move { api.with_value(Clone::clone).fetch_web_page(&url).await })
    }
}

/// Browser bridge client sending command execution requests to the local bridge daemon.
#[derive(Clone)]
pub struct BrowserBridgeClient {
    http_url: String,
    credentials: crate::bridge::BridgeCredentials,
    cwd: String,
    verified: Arc<AtomicBool>,
}

impl BrowserBridgeClient {
    pub fn for_project(
        http_url: String,
        cwd: String,
        credentials: crate::bridge::BridgeCredentials,
    ) -> Self {
        Self {
            http_url,
            credentials,
            cwd,
            verified: Arc::new(AtomicBool::new(true)),
        }
    }

    fn git_cwd(&self) -> &str {
        if self.cwd.is_empty() { "." } else { &self.cwd }
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
        let verified = self.verified.clone();
        let payload = serde_json::json!({
            "command": command,
            "cwd": self.cwd,
            "timeout_seconds": timeout_seconds,
        })
        .to_string();

        SendWrapper::new(async move {
            if !verified.load(Ordering::Relaxed) {
                return Err(
                    "Bridge cwd is no longer verified; send again to rediscover the folder.".into(),
                );
            }
            let token = credentials
                .credential()
                .await
                .map_err(|e| format!("auth error: {e}"))?;
            let guard = crate::api::CommandFetchGuard(
                web_sys::AbortController::new()
                    .map_err(|e| format!("request cancellation error: {e:?}"))?,
            );
            let resp = gloo_net::http::Request::post(&endpoint)
                .abort_signal(Some(&guard.0.signal()))
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
                if resp.status() == 400 && is_cwd_resolution_error(&err_text) {
                    verified.store(false, Ordering::Relaxed);
                }
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
        let verified = self.verified.clone();
        let payload = serde_json::json!({ "cwd": self.git_cwd() }).to_string();
        SendWrapper::new(async move {
            if !verified.load(Ordering::Relaxed) {
                return Err(
                    "Bridge cwd is no longer verified; send again to rediscover the folder.".into(),
                );
            }
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
                if resp.status() == 400 && is_cwd_resolution_error(&err_text) {
                    verified.store(false, Ordering::Relaxed);
                }
                return Err(format!("Bridge error (HTTP {}): {err_text}", resp.status()));
            }
            resp.json::<openwebide_core::GitRepoStatus>()
                .await
                .map_err(|e| format!("Parse error: {e}"))
        })
    }

    fn git_diff(&self, path: Option<&str>) -> impl Future<Output = Result<String, String>> + Send {
        let payload = serde_json::json!({ "path": path, "cwd": self.git_cwd() }).to_string();
        let endpoint = format!("{}/git/diff", self.http_url);
        let credentials = self.credentials.clone();
        let verified = self.verified.clone();
        SendWrapper::new(async move {
            if !verified.load(Ordering::Relaxed) {
                return Err(
                    "Bridge cwd is no longer verified; send again to rediscover the folder.".into(),
                );
            }
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
                if resp.status() == 400 && is_cwd_resolution_error(&err_text) {
                    verified.store(false, Ordering::Relaxed);
                }
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
        let mut payload = serde_json::to_value(req).unwrap_or_default();
        payload["cwd"] = serde_json::json!(self.git_cwd());
        let payload = payload.to_string();
        let endpoint = format!("{}/git/commit", self.http_url);
        let credentials = self.credentials.clone();
        let verified = self.verified.clone();
        SendWrapper::new(async move {
            if !verified.load(Ordering::Relaxed) {
                return Err(
                    "Bridge cwd is no longer verified; send again to rediscover the folder.".into(),
                );
            }
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
                if resp.status() == 400 && is_cwd_resolution_error(&err_text) {
                    verified.store(false, Ordering::Relaxed);
                }
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
        let mut payload = serde_json::to_value(req).unwrap_or_default();
        payload["cwd"] = serde_json::json!(self.git_cwd());
        let payload = payload.to_string();
        let endpoint = format!("{}/git/checkout", self.http_url);
        let credentials = self.credentials.clone();
        let verified = self.verified.clone();
        SendWrapper::new(async move {
            if !verified.load(Ordering::Relaxed) {
                return Err(
                    "Bridge cwd is no longer verified; send again to rediscover the folder.".into(),
                );
            }
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
                if resp.status() == 400 && is_cwd_resolution_error(&err_text) {
                    verified.store(false, Ordering::Relaxed);
                }
                return Err(format!("Bridge error (HTTP {}): {err_text}", resp.status()));
            }
            resp.json::<openwebide_core::GitCheckoutResult>()
                .await
                .map_err(|e| format!("Parse error: {e}"))
        })
    }
}

fn is_cwd_resolution_error(body: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|value| {
            value
                .get("error")
                .and_then(|error| error.as_str())
                .map(str::to_owned)
        })
        .is_some_and(|error| {
            error.starts_with("cwd does not exist:")
                || error.starts_with("cwd escapes workspace root:")
                || error == "cwd is missing or empty"
        })
}

async fn probe_command<B: BridgeClient>(
    bridge: B,
    command: &str,
    timeout_seconds: u64,
) -> Result<CommandOutcome, String> {
    let request = bridge.execute_command(command, timeout_seconds);
    let deadline = sleep_ms(
        i32::try_from(timeout_seconds.saturating_mul(1000).saturating_add(1000))
            .unwrap_or(i32::MAX),
    );
    futures::pin_mut!(request, deadline);
    match futures::future::select(request, deadline).await {
        futures::future::Either::Left((result, _)) => result,
        futures::future::Either::Right(_) => Err("Bridge probe timed out".into()),
    }
}

pub const BRIDGE_FOLDER_NOTICE: &str = "Command and git tools are off for this local project: the bridge can't see this folder. Start `openwebide-bridge --workspace <a folder containing it>` (or inside it) and send again.";

pub async fn resolve_bridge_cwd(
    api: Api,
    handle: web_sys::FileSystemDirectoryHandle,
    pid: i64,
    bridge_cfg: &crate::bridge::BridgeConfig,
    credentials: &crate::bridge::BridgeCredentials,
) -> Option<String> {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    // Math.random is in [0, 1), scaled values fit u32; fractional bits are discarded.
    let nonce = format!(
        "{:08x}{:08x}",
        (js_sys::Math::random() * 4294967296.0) as u32,
        (js_sys::Math::random() * 4294967296.0) as u32
    );
    resolve_bridge_cwd_with(api, &BrowserFsaVfs::new(handle), pid, &nonce, |cwd| {
        BrowserBridgeClient::for_project(bridge_cfg.http_url.clone(), cwd, credentials.clone())
    })
    .await
}

pub async fn resolve_bridge_cwd_with<V: openwebide_core::Vfs, B: BridgeClient>(
    api: Api,
    vfs: &V,
    pid: i64,
    nonce: &str,
    bridge: impl Fn(String) -> B,
) -> Option<String> {
    let probe = format!(".openwebide-probe-{nonce}");
    let written = vfs.write(&probe, "").await;
    let result = if written.is_ok() {
        let key = format!("local_bridge_cwd.{pid}");
        let candidate = api
            .with_value(Clone::clone)
            .get_settings()
            .await
            .ok()
            .and_then(|settings| settings.get(&key).cloned());
        let mut found = None;
        if let Some(candidate) = candidate
            && probe_command(bridge(candidate.clone()), &format!("test -f {probe}"), 5)
                .await
                .is_ok_and(|out| out.is_success())
        {
            found = Some(candidate);
        }
        if found.is_none() {
            let command = format!(
                r"find . -maxdepth 5 \( -name .git -o -name node_modules -o -name target -o -name .spin \) -prune -o -name {probe} -print -quit"
            );
            if let Ok(out) = probe_command(bridge(String::new()), &command, 10).await {
                found = crate::parse_probe_output(&out.stdout, nonce);
                if let Some(cwd) = &found {
                    let _ = api.with_value(Clone::clone).set_setting(&key, cwd).await;
                }
            }
        }
        found
    } else {
        None
    };
    // A failed write may still have created the file before its stream failed.
    if vfs.delete(&probe).await.is_err() {
        return None;
    }
    result
}

pub fn local_tools(cwd: Option<&str>) -> Vec<openwebide_core::ToolDefinition> {
    let mut tools = openwebide_agent::vfs_tools();
    if cwd.is_none() {
        tools.retain(|tool| !openwebide_agent::policy::BRIDGE_TOOLS.contains(&tool.name.as_str()));
    }
    tools
}

/// Local-mode cancel checker observing a shared atomic flag.
#[derive(Clone)]
pub struct LocalCancelCheck {
    pub flag: Arc<AtomicBool>,
}

impl CancelCheck for LocalCancelCheck {
    fn cancelled(&self) -> impl Future<Output = ()> + Send {
        let flag = self.flag.clone();
        SendWrapper::new(async move {
            while !flag.load(Ordering::Relaxed) {
                crate::util::sleep_ms(100).await;
            }
        })
    }

    fn check(&self) -> impl Future<Output = bool> + Send {
        let flag = self.flag.clone();
        SendWrapper::new(async move { flag.load(Ordering::Relaxed) })
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
        SendWrapper::new(async move {
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
    handle: web_sys::FileSystemDirectoryHandle,
    pid: i64,
    chat: crate::state::chat::ChatState,
    cancel_flag: Arc<AtomicBool>,
    local_decisions: Arc<Mutex<HashMap<String, bool>>>,
    mut on_event: impl FnMut(RunEvent),
    bridge_config: crate::bridge::BridgeConfig,
    bridge_credentials: crate::bridge::BridgeCredentials,
    bridge_connection: Option<crate::bridge::BridgeConn>,
    resume: Option<crate::state::chat::InterruptedRun>,
) -> Result<(), String> {
    let cwd = resolve_bridge_cwd(
        api,
        handle.clone(),
        pid,
        &bridge_config,
        &bridge_credentials,
    )
    .await;
    if cwd.is_none() {
        chat.notify_bridge_folder_once(session_id);
    }
    let vfs = BrowserFsaVfs::new(handle);
    // 1. Fetch prior conversation history before persisting the new message
    let history = api.with_value(Clone::clone).list_messages(session_id).await;
    let history_entries = if resume.is_some() {
        history?
    } else {
        history.unwrap_or_default()
    };
    let resume = if let Some(resume) = resume {
        let current = crate::state::chat::interrupted_run(
            &crate::state_actions::chat::history_items(history_entries.clone()),
        )
        .filter(|current| current.anchor_id == resume.anchor_id)
        .ok_or_else(|| "conversation changed; reload the session before resuming".to_string())?;
        Some(current)
    } else {
        None
    };
    let mut history_messages = Vec::new();
    let mut steps = Vec::new();
    for entry in history_entries {
        match entry {
            ConversationEntry::Message(message) => history_messages.push(message),
            ConversationEntry::ToolStep(step) => steps.push(step),
        }
    }
    let mut messages = openwebide_core::tool_history(history_messages, &steps);
    for message in &mut messages {
        if message.role == Role::Assistant {
            message.content = openwebide_core::strip_reasoning(&message.content).to_string();
        }
    }

    let (anchor_id, first_turn) = if let Some(resume) = resume {
        (resume.anchor_id, resume.first_turn)
    } else {
        let full_content = match &editor_context {
            Some(ctx) => format!("{}{}", ctx.format_prompt_injection(), user_content),
            None => user_content,
        };
        let user_message = api
            .with_value(Clone::clone)
            .persist_message(session_id, Role::User, &full_content, None, None)
            .await?;
        let anchor_id = user_message.id;
        on_event(RunEvent::Message {
            message: user_message.clone(),
        });
        messages.push(user_message);
        (anchor_id, 1)
    };

    // 3. Assemble chat request with standard workspace tools
    let request = ChatRequest {
        connection_id,
        system_prompt,
        model,
        messages,
        tools: local_tools(cwd.as_deref()),
    };

    let web = BrowserWebClient::new(api);
    let provider = BrowserLlmProvider::new(api, ProviderKind::Ollama, bridge_connection);
    let cancel = LocalCancelCheck {
        flag: cancel_flag.clone(),
    };
    let gate = LocalPermissionGate {
        decisions: local_decisions,
        cancel: cancel_flag,
    };

    // 4. Drive agent stream
    let mut display_anchor = anchor_id;
    let config = AgentConfig {
        first_turn,
        ..AgentConfig::default()
    };
    let mut stream = if let Some(cwd) = cwd {
        let bridge =
            BrowserBridgeClient::for_project(bridge_config.http_url, cwd, bridge_credentials);
        openwebide_agent::run(
            provider,
            VfsToolExecutor::with_web_and_bridge(vfs, web, bridge),
            request,
            config,
            cancel,
            gate,
            anchor_id,
        )
    } else {
        openwebide_agent::run(
            provider,
            VfsToolExecutor::with_web_and_bridge(vfs, web, openwebide_agent::NoopBridgeClient),
            request,
            config,
            cancel,
            gate,
            anchor_id,
        )
    };

    let mut reasoning = String::new();
    let mut last_usage: Option<TurnTelemetry> = None;
    while let Some(event) = stream.next().await {
        match event {
            AgentEvent::ReasoningDelta(content) => {
                reasoning.push_str(&content);
                on_event(RunEvent::ReasoningDelta { content });
            }
            AgentEvent::TextDelta(delta) => on_event(RunEvent::Delta { content: delta }),
            AgentEvent::TurnCalls { text, calls } => {
                let text = openwebide_core::with_reasoning(&std::mem::take(&mut reasoning), &text);
                let usage = last_usage.take();
                let message = api
                    .with_value(Clone::clone)
                    .persist_message(
                        session_id,
                        Role::Assistant,
                        &text,
                        usage.as_ref(),
                        Some(&calls),
                    )
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
                        tool_calls: Some(calls),
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
                    .upsert_tool_step(session_id, display_anchor, &id, &name, &summary, None)
                    .await;
                on_event(RunEvent::ToolCall { id, name, summary });
            }
            AgentEvent::PermissionRequest {
                id,
                name,
                summary,
                diff,
                note,
            } => {
                last_usage = None;
                let _ = api
                    .with_value(Clone::clone)
                    .upsert_tool_step(
                        session_id,
                        display_anchor,
                        &id,
                        &name,
                        &summary,
                        diff.as_ref(),
                    )
                    .await;
                on_event(RunEvent::PermissionRequest {
                    id,
                    name,
                    summary,
                    diff,
                    note,
                });
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
                let text = openwebide_core::with_reasoning(&reasoning, &text);
                match api
                    .with_value(Clone::clone)
                    .persist_message(
                        session_id,
                        Role::Assistant,
                        &text,
                        last_usage.take().as_ref(),
                        None,
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
