use std::pin::Pin;

use futures::{Stream, StreamExt, stream};
use openwebide_core::{ChatRequest, ChatResponse, ModelInfo, ProviderKind, Role, ToolCall};
use serde_json::{Value, json};

use crate::{
    HttpClient, LineStream, LlmProvider, ProviderError, StreamLine, chat_messages, stream_error,
    tools_wire, url_for,
};

/// Provider for [Ollama](https://ollama.com).
///
/// Talks to the Ollama HTTP API (`/api/tags`, `/api/chat`).
pub struct OllamaProvider<C: HttpClient> {
    base_url: String,
    model: Option<String>,
    http: C,
}

impl<C: HttpClient> OllamaProvider<C> {
    pub fn new(base_url: impl Into<String>, model: Option<String>, http: C) -> Self {
        Self {
            base_url: base_url.into(),
            model,
            http,
        }
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
        let body = json!({
            "model": model,
            "messages": chat_messages(request),
            "stream": false,
        });
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
    ) -> Pin<Box<dyn Stream<Item = Result<String, ProviderError>> + Send + 'static>> {
        let model = match request.model.clone().or_else(|| self.model.clone()) {
            Some(model) => model,
            None => return Box::pin(stream::once(async { Err(ProviderError::NoModel) })),
        };
        let body = json!({
            "model": model,
            "messages": chat_messages(request),
            "stream": true,
        });
        let url = url_for(&self.base_url, "/api/chat");
        let lines = LineStream::new(self.http.post_stream(&url, &body));
        Box::pin(stream::unfold((lines, false), |state| async move {
            let (mut lines, done) = state;
            if done {
                return None;
            }
            loop {
                let line = lines.next().await?;
                match line {
                    Err(e) => return Some((Err(e), (lines, true))),
                    Ok(line) => match parse_stream_line(&line) {
                        Ok(StreamLine::Delta(delta)) => return Some((Ok(delta), (lines, false))),
                        Ok(StreamLine::Done) => return None,
                        Ok(StreamLine::Skip) => continue,
                        Err(e) => return Some((Err(e), (lines, true))),
                    },
                }
            }
        }))
    }

    async fn chat_tools(&self, request: &ChatRequest) -> Result<ChatResponse, ProviderError> {
        let model = request
            .model
            .clone()
            .or_else(|| self.model.clone())
            .ok_or(ProviderError::NoModel)?;
        let body = json!({
            "model": model,
            "messages": ollama_tool_messages(request),
            "stream": false,
            "tools": tools_wire(&request.tools),
        });
        let value = self
            .http
            .post_json(&url_for(&self.base_url, "/api/chat"), &body)
            .await?;
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
                tool_calls.push(ToolCall {
                    id: format!("call_{i}"),
                    name,
                    arguments,
                });
            }
            return Ok(ChatResponse::ToolCalls(tool_calls));
        }
        let content = message
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        Ok(ChatResponse::Text(content))
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
    use openwebide_core::{ChatMessage, Role, ToolDefinition};

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
            r#"{"message":{"role":"assistant","content":""},"done":true}
"#,
        ]);

        let deltas: Vec<String> = block_on(async {
            provider
                .chat_stream(&request(None, None))
                .collect::<Vec<_>>()
                .await
        })
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

        assert_eq!(deltas, vec!["Hel", "lo"]);
        let calls = state.calls.lock().unwrap();
        let body = calls[0].body.as_ref().unwrap();
        assert_eq!(body["stream"], true);
    }

    #[test]
    fn chat_stream_splits_lines_across_chunks() {
        let (provider, state) = provider(FakeHttpClient::new());
        // One chunk holds a partial line, the next finishes it and adds more.
        state.push_stream(vec![
            r#"{"message":{"role":"assistant","content":"a"},"done":fal"#,
            r#"se}
{"message":{"role":"assistant","content":"b"},"done":true}
"#,
        ]);

        let deltas: Vec<String> = block_on(async {
            provider
                .chat_stream(&request(None, None))
                .collect::<Vec<_>>()
                .await
        })
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

        assert_eq!(deltas, vec!["a", "b"]);
    }

    #[test]
    fn chat_stream_reports_provider_errors() {
        let (provider, state) = provider(FakeHttpClient::new());
        state.push_stream(vec![
            r#"{"error":"model 'nope' not found"}
"#,
        ]);

        let result: Vec<Result<String, _>> =
            block_on(provider.chat_stream(&request(None, None)).collect());

        assert!(matches!(
            result.into_iter().next().unwrap(),
            Err(ProviderError::Http(ref msg)) if msg.contains("not found")
        ));
    }

    #[test]
    fn chat_stream_propagates_transport_errors() {
        let (provider, state) = provider(FakeHttpClient::new());
        state.push_stream_error(ProviderError::Http("connection reset".into()));

        let result: Vec<Result<String, _>> =
            block_on(provider.chat_stream(&request(None, None)).collect());

        assert!(matches!(
            result.into_iter().next().unwrap(),
            Err(ProviderError::Http(ref msg)) if msg.contains("connection reset")
        ));
        assert_eq!(state.calls.lock().unwrap().len(), 1);
    }

    #[test]
    fn chat_stream_without_any_model_is_an_error() {
        let http = FakeHttpClient::new();
        let state = http.state();
        let provider = OllamaProvider::new(BASE, None, http);

        let result: Vec<Result<String, _>> =
            block_on(provider.chat_stream(&request(None, None)).collect());

        assert!(matches!(
            result.into_iter().next().unwrap(),
            Err(ProviderError::NoModel)
        ));
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
            "done": true
        })));

        let mut req = request(None, None);
        req.tools = vec![read_file_tool()];
        let response = block_on(provider.chat_tools(&req)).unwrap();

        assert_eq!(
            response,
            ChatResponse::ToolCalls(vec![ToolCall {
                id: "call_0".into(),
                name: "read_file".into(),
                arguments: r#"{"path":"src/main.rs"}"#.into(),
            }])
        );
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

        let response = block_on(provider.chat_tools(&request(None, None))).unwrap();
        assert_eq!(response, ChatResponse::Text("all done".into()));
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
        };
        let tool_result = ChatMessage {
            id: 3,
            session_id: 1,
            role: Role::Tool,
            content: "fn main() {}".into(),
            created_at: 0,
            tool_calls: None,
            tool_call_id: Some("call_0".into()),
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
}
