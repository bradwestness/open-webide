use std::pin::Pin;

use futures::{Stream, StreamExt, stream};
use openwebide_core::{
    ChatCompletion, ChatRequest, ChatResponse, ModelInfo, ProviderKind, Role, ToolCall,
};
use serde_json::{Value, json};

use crate::{
    HttpClient, LineStream, LlmProvider, ProviderError, StreamChunk, StreamLine, UsageAcc,
    chat_messages, clock_now, round_ns_to_ms, stream_error, tools_wire, url_for,
};

/// Provider for [Ollama](https://ollama.com).
///
/// Talks to the Ollama HTTP API (`/api/tags`, `/api/chat`).
pub struct OllamaProvider<C: HttpClient> {
    base_url: String,
    model: Option<String>,
    http: C,
    num_ctx: Option<usize>,
}

impl<C: HttpClient> OllamaProvider<C> {
    pub fn new(base_url: impl Into<String>, model: Option<String>, http: C) -> Self {
        Self {
            base_url: base_url.into(),
            model,
            http,
            num_ctx: None,
        }
    }

    /// Set the context window sent as `options.num_ctx` on every request.
    /// `None` omits the `options` key entirely.
    pub fn with_num_ctx(mut self, n: Option<usize>) -> Self {
        self.num_ctx = n;
        self
    }
}

/// Read the usage fields Ollama reports and overwrite only the ones present.
fn usage_fields(value: &Value, acc: &mut UsageAcc) {
    if let Some(n) = value.get("prompt_eval_count").and_then(|v| v.as_u64()) {
        acc.prompt = Some(n as usize);
    }
    if let Some(n) = value.get("eval_count").and_then(|v| v.as_u64()) {
        acc.completion = Some(n as usize);
    }
    if let Some(ns) = value.get("eval_duration").and_then(|v| v.as_u64()) {
        acc.eval_ms = Some(round_ns_to_ms(ns));
    }
}

impl<C: HttpClient> LlmProvider for OllamaProvider<C> {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Ollama
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>, ProviderError> {
        let value = self
            .http
            .get_json(&url_for(&self.base_url, "/api/tags"))
            .await?;
        let models = value
            .get("models")
            .and_then(|m| m.as_array())
            .ok_or_else(|| {
                ProviderError::Parse("Ollama /api/tags: missing `models` array".into())
            })?;
        Ok(models
            .iter()
            .filter_map(|m| {
                m.get("name")
                    .and_then(|n| n.as_str())
                    .map(|name| ModelInfo {
                        name: name.to_string(),
                    })
            })
            .collect())
    }

    async fn chat(&self, request: &ChatRequest) -> Result<String, ProviderError> {
        let model = request
            .model
            .clone()
            .or_else(|| self.model.clone())
            .ok_or(ProviderError::NoModel)?;
        let mut body = json!({
            "model": model,
            "messages": chat_messages(request),
            "stream": false,
        });
        if let Some(n) = self.num_ctx {
            body["options"] = json!({ "num_ctx": n });
        }
        let value = self
            .http
            .post_json(&url_for(&self.base_url, "/api/chat"), &body)
            .await?;
        value
            .get("message")
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_str())
            .ok_or_else(|| {
                ProviderError::Parse("Ollama /api/chat: missing `message.content`".into())
            })
            .map(str::to_string)
    }

    fn chat_stream(
        &self,
        request: &ChatRequest,
    ) -> Pin<Box<dyn Stream<Item = Result<StreamChunk, ProviderError>> + Send + 'static>> {
        let model = match request.model.clone().or_else(|| self.model.clone()) {
            Some(model) => model,
            None => return Box::pin(stream::once(async { Err(ProviderError::NoModel) })),
        };
        let mut body = json!({
            "model": model,
            "messages": chat_messages(request),
            "stream": true,
        });
        if let Some(n) = self.num_ctx {
            body["options"] = json!({ "num_ctx": n });
        }
        let url = url_for(&self.base_url, "/api/chat");
        let lines = LineStream::new(self.http.post_stream(&url, &body));
        let acc = UsageAcc::new(request);
        Box::pin(stream::unfold(
            (lines, acc, false, false),
            |state| async move {
                let (mut lines, mut acc, emitted_usage, done) = state;
                if done {
                    return None;
                }
                loop {
                    let Some(line) = lines.next().await else {
                        // EOF with no `done: true` line: still end the turn,
                        // yielding an (estimated) usage if none was emitted.
                        if acc.ended.is_none() {
                            acc.ended = clock_now();
                        }
                        if !emitted_usage {
                            return Some((
                                Ok(StreamChunk::Usage(acc.finish())),
                                (lines, acc, true, true),
                            ));
                        }
                        return None;
                    };
                    match line {
                        Err(e) => return Some((Err(e), (lines, acc, emitted_usage, true))),
                        Ok(line) => {
                            let parsed: Value = serde_json::from_str(&line).unwrap_or(Value::Null);
                            usage_fields(&parsed, &mut acc);
                            match parse_stream_line(&line) {
                                Ok(StreamLine::Delta(delta)) => {
                                    if acc.started.is_none() {
                                        acc.started = clock_now();
                                    }
                                    acc.text.push_str(&delta);
                                    return Some((
                                        Ok(StreamChunk::Delta(delta)),
                                        (lines, acc, emitted_usage, false),
                                    ));
                                }
                                Ok(StreamLine::Done) => {
                                    acc.ended = clock_now();
                                    if !emitted_usage {
                                        return Some((
                                            Ok(StreamChunk::Usage(acc.finish())),
                                            (lines, acc, true, true),
                                        ));
                                    }
                                    return None;
                                }
                                Ok(StreamLine::Skip) => continue,
                                Err(e) => {
                                    return Some((Err(e), (lines, acc, emitted_usage, true)));
                                }
                            }
                        }
                    }
                }
            },
        ))
    }

    async fn chat_tools(&self, request: &ChatRequest) -> Result<ChatCompletion, ProviderError> {
        let model = request
            .model
            .clone()
            .or_else(|| self.model.clone())
            .ok_or(ProviderError::NoModel)?;
        let mut body = json!({
            "model": model,
            "messages": ollama_tool_messages(request),
            "stream": false,
            "tools": tools_wire(&request.tools),
        });
        if let Some(n) = self.num_ctx {
            body["options"] = json!({ "num_ctx": n });
        }
        let mut acc = UsageAcc::new(request);
        acc.started = clock_now();
        let value = self
            .http
            .post_json(&url_for(&self.base_url, "/api/chat"), &body)
            .await?;
        acc.ended = clock_now();
        usage_fields(&value, &mut acc);
        let message = value
            .get("message")
            .ok_or_else(|| ProviderError::Parse("Ollama /api/chat: missing `message`".into()))?;
        if let Some(calls) = message.get("tool_calls").and_then(|v| v.as_array()) {
            let mut tool_calls = Vec::new();
            for (i, call) in calls.iter().enumerate() {
                let function = call.get("function").ok_or_else(|| {
                    ProviderError::Parse("Ollama tool call missing `function`".into())
                })?;
                let name = function
                    .get("name")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| ProviderError::Parse("Ollama tool call missing `name`".into()))?
                    .to_string();
                // Ollama returns `arguments` as a JSON object; normalize it to
                // the canonical JSON-string form.
                let arguments = function
                    .get("arguments")
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                let arguments = match arguments {
                    Value::String(s) => s,
                    other => other.to_string(),
                };
                acc.text.push_str(&name);
                acc.text.push_str(&arguments);
                tool_calls.push(ToolCall {
                    id: format!("call_{i}"),
                    name,
                    arguments,
                });
            }
            return Ok(ChatCompletion {
                response: ChatResponse::ToolCalls(tool_calls),
                usage: Some(acc.finish()),
            });
        }
        let content = message
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        acc.text.push_str(&content);
        Ok(ChatCompletion {
            response: ChatResponse::Text(content),
            usage: Some(acc.finish()),
        })
    }

    async fn context_limit(&self, model: Option<&str>) -> Result<Option<usize>, ProviderError> {
        let Some(model) = model.map(str::to_string).or_else(|| self.model.clone()) else {
            return Ok(None);
        };
        let value = self
            .http
            .post_json(
                &url_for(&self.base_url, "/api/show"),
                &json!({ "model": model }),
            )
            .await?;
        // A Modelfile `num_ctx` is the real runtime window Ollama loads the
        // model with. `model_info.<arch>.context_length` is the model's
        // trained maximum, not what Ollama actually runs at (its default
        // window is far smaller unless `num_ctx` is set), so it is
        // deliberately not used as a fallback here (user decision).
        if let Some(params) = value.get("parameters").and_then(|v| v.as_str()) {
            for line in params.lines() {
                if let Some(rest) = line.trim().strip_prefix("num_ctx")
                    && let Ok(n) = rest.trim().parse::<usize>()
                {
                    return Ok(Some(n));
                }
            }
        }
        Ok(None)
    }
}

/// Ollama wire format for messages, including tool calls and tool results.
///
/// Unlike the OpenAI-compatible API, Ollama's tool calls carry no `id` and
/// their `arguments` is a JSON object, and tool results carry no
/// `tool_call_id`.
fn ollama_tool_messages(request: &ChatRequest) -> Vec<Value> {
    let mut messages = Vec::new();
    if let Some(system) = &request.system_prompt
        && !system.is_empty()
    {
        messages.push(json!({ "role": "system", "content": system }));
    }
    for message in &request.messages {
        match message.role {
            Role::Tool => {
                messages.push(json!({ "role": "tool", "content": message.content }));
            }
            Role::Assistant if message.tool_calls.is_some() => {
                let calls: Vec<Value> = message
                    .tool_calls
                    .as_ref()
                    .unwrap()
                    .iter()
                    .map(|call| {
                        let args: Value =
                            serde_json::from_str(&call.arguments).unwrap_or_else(|_| json!({}));
                        json!({ "function": { "name": call.name, "arguments": args } })
                    })
                    .collect();
                let content = if message.content.is_empty() {
                    Value::Null
                } else {
                    Value::String(message.content.clone())
                };
                messages
                    .push(json!({ "role": "assistant", "content": content, "tool_calls": calls }));
            }
            _ => {
                messages.push(json!({ "role": message.role.as_str(), "content": message.content }))
            }
        }
    }
    messages
}

/// Parse one NDJSON line of Ollama's streaming `/api/chat` response.
fn parse_stream_line(line: &str) -> Result<StreamLine, ProviderError> {
    let value: Value = serde_json::from_str(line)
        .map_err(|e| ProviderError::Parse(format!("Ollama stream: invalid JSON: {e}")))?;
    if let Some(error) = stream_error(&value) {
        return Err(ProviderError::Http(error));
    }
    let content = value
        .get("message")
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str());
    let done = value.get("done").and_then(|d| d.as_bool()) == Some(true);
    // A line may carry both a final delta and `done: true`; the delta wins.
    match content {
        Some(content) if !content.is_empty() => Ok(StreamLine::Delta(content.to_string())),
        _ if done => Ok(StreamLine::Done),
        _ => Ok(StreamLine::Skip),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::fake::{FakeHttpClient, FakeState};
    use futures::executor::block_on;
    use openwebide_core::{ChatMessage, Role, ToolDefinition, TurnTelemetry};

    const BASE: &str = "http://localhost:11434";

    fn provider(http: FakeHttpClient) -> (OllamaProvider<FakeHttpClient>, Arc<FakeState>) {
        let state = http.state();
        (
            OllamaProvider::new(BASE, Some("qwen2.5-coder:7b".into()), http),
            state,
        )
    }

    fn message(role: Role, content: &str) -> ChatMessage {
        ChatMessage {
            id: 1,
            session_id: 1,
            role,
            content: content.into(),
            created_at: 0,
            tool_calls: None,
            tool_call_id: None,
            usage: None,
        }
    }

    fn request(model: Option<&str>, system: Option<&str>) -> ChatRequest {
        ChatRequest {
            connection_id: 1,
            system_prompt: system.map(str::to_string),
            model: model.map(str::to_string),
            messages: vec![message(Role::User, "hi")],
            tools: vec![],
        }
    }

    fn read_file_tool() -> ToolDefinition {
        ToolDefinition {
            name: "read_file".into(),
            description: "Read a file".into(),
            parameters: json!({
                "type": "object",
                "properties": { "path": { "type": "string" } },
                "required": ["path"]
            }),
        }
    }

    #[test]
    fn list_models_parses_tags() {
        let (provider, state) = provider(FakeHttpClient::new());
        state.push(Ok(json!({
            "models": [ { "name": "qwen2.5-coder:7b" }, { "name": "llama3.2" } ]
        })));

        let models = block_on(provider.list_models()).unwrap();

        assert_eq!(
            models,
            vec![
                ModelInfo {
                    name: "qwen2.5-coder:7b".into()
                },
                ModelInfo {
                    name: "llama3.2".into()
                },
            ]
        );
        assert_eq!(
            state.calls.lock().unwrap()[0].url,
            format!("{BASE}/api/tags")
        );
    }

    #[test]
    fn list_models_rejects_missing_models() {
        let (provider, state) = provider(FakeHttpClient::new());
        state.push(Ok(json!({})));

        assert!(matches!(
            block_on(provider.list_models()),
            Err(ProviderError::Parse(_))
        ));
    }

    #[test]
    fn chat_sends_model_messages_and_system_prompt() {
        let (provider, state) = provider(FakeHttpClient::new());
        state.push(Ok(json!({
            "message": { "role": "assistant", "content": "hello" },
            "done": true
        })));

        assert_eq!(
            block_on(provider.chat(&request(None, Some("be brief")))).unwrap(),
            "hello"
        );

        let calls = state.calls.lock().unwrap();
        assert_eq!(calls[0].url, format!("{BASE}/api/chat"));
        let body = calls[0].body.as_ref().unwrap();
        assert_eq!(body["model"], "qwen2.5-coder:7b");
        assert_eq!(body["stream"], false);
        assert_eq!(
            body["messages"][0],
            json!({ "role": "system", "content": "be brief" })
        );
        assert_eq!(
            body["messages"][1],
            json!({ "role": "user", "content": "hi" })
        );
    }

    #[test]
    fn chat_prefers_request_model_over_connection() {
        let (provider, state) = provider(FakeHttpClient::new());
        state.push(Ok(json!({ "message": { "content": "ok" } })));

        block_on(provider.chat(&request(Some("other-model"), None))).unwrap();

        let calls = state.calls.lock().unwrap();
        let body = calls[0].body.as_ref().unwrap();
        assert_eq!(body["model"], "other-model");
    }

    #[test]
    fn chat_without_any_model_is_an_error() {
        let http = FakeHttpClient::new();
        let state = http.state();
        let provider = OllamaProvider::new(BASE, None, http);

        assert!(matches!(
            block_on(provider.chat(&request(None, None))),
            Err(ProviderError::NoModel)
        ));
        assert!(state.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn http_errors_propagate() {
        let (provider, state) = provider(FakeHttpClient::new());
        state.push(Err(ProviderError::Http(
            "404 Not Found: model 'nope' not found".into(),
        )));

        assert!(matches!(
            block_on(provider.chat(&request(None, None))),
            Err(ProviderError::Http(_))
        ));
    }

    /// Run a stream to completion, splitting deltas from the (at most one)
    /// usage chunk.
    fn run_stream(
        provider: &OllamaProvider<FakeHttpClient>,
        req: &ChatRequest,
    ) -> (Vec<String>, Vec<TurnTelemetry>, Vec<ProviderError>) {
        let items: Vec<Result<StreamChunk, ProviderError>> =
            block_on(provider.chat_stream(req).collect());
        let mut deltas = Vec::new();
        let mut usages = Vec::new();
        let mut errors = Vec::new();
        for item in items {
            match item {
                Ok(StreamChunk::Delta(d)) => deltas.push(d),
                Ok(StreamChunk::Usage(u)) => usages.push(u),
                Err(e) => errors.push(e),
            }
        }
        (deltas, usages, errors)
    }

    #[test]
    fn chat_stream_yields_deltas_until_done() {
        let (provider, state) = provider(FakeHttpClient::new());
        state.push_stream(vec![
            r#"{"message":{"role":"assistant","content":"Hel"}}
"#,
            r#"{"message":{"role":"assistant","content":"lo"}}
"#,
            r#"{"message":{"role":"assistant","content":""},"done":false}
"#,
            r#"{"message":{"role":"assistant","content":""},"done":true,"prompt_eval_count":12,"eval_count":4,"eval_duration":2000000}
"#,
        ]);

        let (deltas, usages, errors) = run_stream(&provider, &request(None, None));

        assert_eq!(deltas, vec!["Hel", "lo"]);
        assert!(errors.is_empty());
        assert_eq!(usages.len(), 1);
        assert_eq!(usages[0].prompt_tokens, 12);
        assert_eq!(usages[0].completion_tokens, 4);
        assert_eq!(usages[0].eval_duration_ms, 2);
        assert!(!usages[0].estimated);
        let calls = state.calls.lock().unwrap();
        let body = calls[0].body.as_ref().unwrap();
        assert_eq!(body["stream"], true);
    }

    #[test]
    fn chat_stream_splits_lines_across_chunks() {
        let (provider, _state) = provider(FakeHttpClient::new());
        // One chunk holds a partial line, the next finishes it and adds more.
        _state.push_stream(vec![
            r#"{"message":{"role":"assistant","content":"a"},"done":fal"#,
            r#"se}
{"message":{"role":"assistant","content":"b"},"done":true}
"#,
        ]);

        let (deltas, _usages, _errors) = run_stream(&provider, &request(None, None));

        assert_eq!(deltas, vec!["a", "b"]);
    }

    #[test]
    fn chat_stream_with_no_done_line_still_yields_estimated_usage() {
        let (provider, state) = provider(FakeHttpClient::new());
        state.push_stream(vec![
            r#"{"message":{"role":"assistant","content":"partial"}}
"#,
        ]);

        let (deltas, usages, errors) = run_stream(&provider, &request(None, None));

        assert_eq!(deltas, vec!["partial"]);
        assert!(errors.is_empty());
        assert_eq!(usages.len(), 1);
        assert!(usages[0].estimated);
    }

    #[test]
    fn chat_stream_reports_provider_errors() {
        let (provider, state) = provider(FakeHttpClient::new());
        state.push_stream(vec![
            r#"{"error":"model 'nope' not found"}
"#,
        ]);

        let (deltas, usages, errors) = run_stream(&provider, &request(None, None));
        assert!(deltas.is_empty());
        assert!(usages.is_empty());
        assert_eq!(errors.len(), 1);
        assert!(matches!(&errors[0], ProviderError::Http(msg) if msg.contains("not found")));
    }

    #[test]
    fn chat_stream_propagates_transport_errors() {
        let (provider, state) = provider(FakeHttpClient::new());
        state.push_stream_error(ProviderError::Http("connection reset".into()));

        let (deltas, usages, errors) = run_stream(&provider, &request(None, None));
        assert!(deltas.is_empty());
        assert!(usages.is_empty());
        assert!(matches!(&errors[0], ProviderError::Http(msg) if msg.contains("connection reset")));
        assert_eq!(state.calls.lock().unwrap().len(), 1);
    }

    #[test]
    fn chat_stream_without_any_model_is_an_error() {
        let http = FakeHttpClient::new();
        let state = http.state();
        let provider = OllamaProvider::new(BASE, None, http);

        let (_deltas, _usages, errors) = run_stream(&provider, &request(None, None));

        assert!(matches!(errors[0], ProviderError::NoModel));
        assert!(state.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn chat_tools_returns_tool_calls() {
        let (provider, state) = provider(FakeHttpClient::new());
        state.push(Ok(json!({
            "message": {
                "role": "assistant",
                "content": "",
                "tool_calls": [
                    { "function": { "name": "read_file", "arguments": { "path": "src/main.rs" } } }
                ]
            },
            "done": true,
            "prompt_eval_count": 20,
            "eval_count": 5,
            "eval_duration": 1_000_000
        })));

        let mut req = request(None, None);
        req.tools = vec![read_file_tool()];
        let completion = block_on(provider.chat_tools(&req)).unwrap();

        assert_eq!(
            completion.response,
            ChatResponse::ToolCalls(vec![ToolCall {
                id: "call_0".into(),
                name: "read_file".into(),
                arguments: r#"{"path":"src/main.rs"}"#.into(),
            }])
        );
        let usage = completion.usage.unwrap();
        assert_eq!(usage.prompt_tokens, 20);
        assert_eq!(usage.completion_tokens, 5);
        assert_eq!(usage.eval_duration_ms, 1);
        assert!(!usage.estimated);
        // The request carries the tool definitions and stream is off.
        let calls = state.calls.lock().unwrap();
        let body = calls[0].body.as_ref().unwrap();
        assert_eq!(body["stream"], false);
        assert_eq!(
            body["tools"][0],
            json!({
                "type": "function",
                "function": {
                    "name": "read_file",
                    "description": "Read a file",
                    "parameters": {
                        "type": "object",
                        "properties": { "path": { "type": "string" } },
                        "required": ["path"]
                    }
                }
            })
        );
    }

    #[test]
    fn chat_tools_returns_text_when_no_calls() {
        let (provider, state) = provider(FakeHttpClient::new());
        state.push(Ok(json!({
            "message": { "role": "assistant", "content": "all done" },
            "done": true
        })));

        let completion = block_on(provider.chat_tools(&request(None, None))).unwrap();
        assert_eq!(completion.response, ChatResponse::Text("all done".into()));
        // No usage fields on the response: the estimator filled them in.
        assert!(completion.usage.unwrap().estimated);
    }

    #[test]
    fn chat_tools_sends_tool_call_and_result_messages() {
        let (provider, state) = provider(FakeHttpClient::new());
        state.push(Ok(json!({
            "message": { "role": "assistant", "content": "fixed it" },
            "done": true
        })));

        let assistant = ChatMessage {
            id: 2,
            session_id: 1,
            role: Role::Assistant,
            content: String::new(),
            created_at: 0,
            tool_calls: Some(vec![ToolCall {
                id: "call_0".into(),
                name: "read_file".into(),
                arguments: r#"{"path":"src/main.rs"}"#.into(),
            }]),
            tool_call_id: None,
            usage: None,
        };
        let tool_result = ChatMessage {
            id: 3,
            session_id: 1,
            role: Role::Tool,
            content: "fn main() {}".into(),
            created_at: 0,
            tool_calls: None,
            tool_call_id: Some("call_0".into()),
            usage: None,
        };
        let req = ChatRequest {
            connection_id: 1,
            system_prompt: None,
            model: None,
            messages: vec![message(Role::User, "fix it"), assistant, tool_result],
            tools: vec![read_file_tool()],
        };
        block_on(provider.chat_tools(&req)).unwrap();

        let calls = state.calls.lock().unwrap();
        let body = calls[0].body.as_ref().unwrap();
        // Assistant message carries tool_calls with object arguments (Ollama
        // wire format) and a null content.
        assert_eq!(
            body["messages"][1],
            json!({
                "role": "assistant",
                "content": null,
                "tool_calls": [
                    { "function": { "name": "read_file", "arguments": { "path": "src/main.rs" } } }
                ]
            })
        );
        // Tool result carries no tool_call_id (Ollama wire format).
        assert_eq!(
            body["messages"][2],
            json!({ "role": "tool", "content": "fn main() {}" })
        );
    }

    #[test]
    fn chat_tools_without_any_model_is_an_error() {
        let http = FakeHttpClient::new();
        let state = http.state();
        let provider = OllamaProvider::new(BASE, None, http);

        assert!(matches!(
            block_on(provider.chat_tools(&request(None, None))),
            Err(ProviderError::NoModel)
        ));
        assert!(state.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn with_num_ctx_adds_options_to_every_request_body() {
        let (provider, state) = provider(FakeHttpClient::new());
        let provider = provider.with_num_ctx(Some(8192));

        state.push(Ok(json!({ "message": { "content": "hi" } })));
        block_on(provider.chat(&request(None, None))).unwrap();
        assert_eq!(
            state.calls.lock().unwrap()[0].body.as_ref().unwrap()["options"]["num_ctx"],
            8192
        );

        state.push_stream(vec![
            r#"{"message":{"content":"a"},"done":true}
"#,
        ]);
        block_on(
            provider
                .chat_stream(&request(None, None))
                .collect::<Vec<_>>(),
        );
        assert_eq!(
            state.calls.lock().unwrap()[1].body.as_ref().unwrap()["options"]["num_ctx"],
            8192
        );

        state.push(Ok(json!({ "message": { "content": "hi" } })));
        block_on(provider.chat_tools(&request(None, None))).unwrap();
        assert_eq!(
            state.calls.lock().unwrap()[2].body.as_ref().unwrap()["options"]["num_ctx"],
            8192
        );
    }

    #[test]
    fn without_num_ctx_the_body_has_no_options_key() {
        let (provider, state) = provider(FakeHttpClient::new());

        state.push(Ok(json!({ "message": { "content": "hi" } })));
        block_on(provider.chat(&request(None, None))).unwrap();
        assert!(
            state.calls.lock().unwrap()[0]
                .body
                .as_ref()
                .unwrap()
                .get("options")
                .is_none()
        );
    }

    #[test]
    fn context_limit_reads_num_ctx_from_parameters() {
        let (provider, state) = provider(FakeHttpClient::new());
        state.push(Ok(json!({
            "parameters": "num_ctx 8192\nstop \"<|eot|>\"",
        })));

        let limit = block_on(provider.context_limit(None)).unwrap();
        assert_eq!(limit, Some(8192));
        let calls = state.calls.lock().unwrap();
        assert_eq!(calls[0].url, format!("{BASE}/api/show"));
        assert_eq!(calls[0].body.as_ref().unwrap()["model"], "qwen2.5-coder:7b");
    }

    #[test]
    fn context_limit_ignores_model_info_trained_max() {
        // `model_info.<arch>.context_length` is the trained maximum, not the
        // runtime window Ollama actually loads without a configured
        // `num_ctx`, so it must not be used as a fallback.
        let (provider, state) = provider(FakeHttpClient::new());
        state.push(Ok(json!({
            "parameters": "stop \"<|eot|>\"",
            "model_info": {
                "general.architecture": "qwen2",
                "qwen2.context_length": 32768,
            }
        })));

        let limit = block_on(provider.context_limit(Some("qwen2.5-coder:7b"))).unwrap();
        assert_eq!(limit, None);
    }

    #[test]
    fn context_limit_with_no_model_returns_none_without_a_request() {
        let http = FakeHttpClient::new();
        let state = http.state();
        let provider = OllamaProvider::new(BASE, None, http);

        assert_eq!(block_on(provider.context_limit(None)).unwrap(), None);
        assert!(state.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn missing_prompt_eval_count_is_estimated() {
        let (provider, state) = provider(FakeHttpClient::new());
        state.push(Ok(json!({
            "message": { "role": "assistant", "content": "hi there" },
            "done": true,
            "eval_count": 3,
            "eval_duration": 1_000_000
        })));

        let completion = block_on(provider.chat_tools(&request(None, None))).unwrap();
        assert!(completion.usage.unwrap().estimated);
    }
}
