use std::pin::Pin;

use futures::{Stream, StreamExt, stream};
use openwebide_core::{ChatRequest, ChatResponse, ModelInfo, ProviderKind, Role, ToolCall};
use serde_json::{Value, json};

use crate::{
    HttpClient, LineStream, LlmProvider, ProviderError, StreamLine, chat_messages, stream_error,
    tools_wire, url_for,
};

/// Provider for a [llama.cpp](https://github.com/ggml-org/llama.cpp) server
/// (`llama-server`, OpenAI-compatible API).
pub struct LlamaCppProvider<C: HttpClient> {
    base_url: String,
    model: Option<String>,
    http: C,
}

impl<C: HttpClient> LlamaCppProvider<C> {
    pub fn new(base_url: impl Into<String>, model: Option<String>, http: C) -> Self {
        Self {
            base_url: base_url.into(),
            model,
            http,
        }
    }
}

impl<C: HttpClient> LlmProvider for LlamaCppProvider<C> {
    fn kind(&self) -> ProviderKind {
        ProviderKind::LlamaCpp
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>, ProviderError> {
        let value = self
            .http
            .get_json(&url_for(&self.base_url, "/v1/models"))
            .await?;
        let data = value
            .get("data")
            .and_then(|d| d.as_array())
            .ok_or_else(|| {
                ProviderError::Parse("llama.cpp /v1/models: missing `data` array".into())
            })?;
        Ok(data
            .iter()
            .filter_map(|m| {
                m.get("id").and_then(|i| i.as_str()).map(|id| ModelInfo {
                    name: id.to_string(),
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
            .post_json(&url_for(&self.base_url, "/v1/chat/completions"), &body)
            .await?;
        value
            .get("choices")
            .and_then(|c| c.as_array())
            .and_then(|c| c.first())
            .and_then(|c| c.get("message"))
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_str())
            .ok_or_else(|| {
                ProviderError::Parse(
                    "llama.cpp /v1/chat/completions: missing `choices[0].message.content`".into(),
                )
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
        let url = url_for(&self.base_url, "/v1/chat/completions");
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
            "messages": llamacpp_tool_messages(request),
            "stream": false,
            "tools": tools_wire(&request.tools),
        });
        let value = self
            .http
            .post_json(&url_for(&self.base_url, "/v1/chat/completions"), &body)
            .await?;
        let message = value
            .get("choices")
            .and_then(|c| c.as_array())
            .and_then(|c| c.first())
            .and_then(|c| c.get("message"))
            .ok_or_else(|| {
                ProviderError::Parse(
                    "llama.cpp /v1/chat/completions: missing `choices[0].message`".into(),
                )
            })?;
        if let Some(calls) = message.get("tool_calls").and_then(|v| v.as_array()) {
            let mut tool_calls = Vec::new();
            for call in calls {
                let function = call.get("function").ok_or_else(|| {
                    ProviderError::Parse("llama.cpp tool call missing `function`".into())
                })?;
                let name = function
                    .get("name")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        ProviderError::Parse("llama.cpp tool call missing `name`".into())
                    })?
                    .to_string();
                // The OpenAI-compatible API returns `arguments` as a JSON
                // string, which is already the canonical form.
                let arguments = function
                    .get("arguments")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                let id = call
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                tool_calls.push(ToolCall {
                    id,
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

/// OpenAI-compatible wire format for messages, including tool calls and tool
/// results. Tool calls carry an `id` and a string `arguments`; tool results
/// carry a `tool_call_id`.
fn llamacpp_tool_messages(request: &ChatRequest) -> Vec<Value> {
    let mut messages = Vec::new();
    if let Some(system) = &request.system_prompt
        && !system.is_empty()
    {
        messages.push(json!({ "role": "system", "content": system }));
    }
    for message in &request.messages {
        match message.role {
            Role::Tool => {
                messages.push(json!({
                    "role": "tool",
                    "tool_call_id": message.tool_call_id.clone().unwrap_or_default(),
                    "content": message.content,
                }));
            }
            Role::Assistant if message.tool_calls.is_some() => {
                let calls: Vec<Value> = message
                    .tool_calls
                    .as_ref()
                    .unwrap()
                    .iter()
                    .map(|call| {
                        json!({
                            "id": call.id,
                            "type": "function",
                            "function": { "name": call.name, "arguments": call.arguments }
                        })
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

/// Parse one SSE line of llama.cpp's streaming `/v1/chat/completions`
/// response.
fn parse_stream_line(line: &str) -> Result<StreamLine, ProviderError> {
    let data = line.strip_prefix("data:").unwrap_or(line);
    let data = data.trim();
    if data == "[DONE]" {
        return Ok(StreamLine::Done);
    }
    if data.is_empty() || data.starts_with(':') {
        return Ok(StreamLine::Skip);
    }
    let value: Value = serde_json::from_str(data)
        .map_err(|e| ProviderError::Parse(format!("llama.cpp stream: invalid JSON: {e}")))?;
    if let Some(error) = stream_error(&value) {
        return Err(ProviderError::Http(error));
    }
    let content = value
        .get("choices")
        .and_then(|c| c.as_array())
        .and_then(|c| c.first())
        .and_then(|c| c.get("delta"))
        .and_then(|d| d.get("content"))
        .and_then(|c| c.as_str());
    match content {
        Some(content) if !content.is_empty() => Ok(StreamLine::Delta(content.to_string())),
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

    const BASE: &str = "http://localhost:8080";

    fn provider(http: FakeHttpClient) -> (LlamaCppProvider<FakeHttpClient>, Arc<FakeState>) {
        let state = http.state();
        (
            LlamaCppProvider::new(BASE, Some("qwen2.5-coder".into()), http),
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
    fn list_models_parses_openai_data() {
        let (provider, state) = provider(FakeHttpClient::new());
        state.push(Ok(json!({
            "data": [ { "id": "qwen2.5-coder" }, { "id": "mistral" } ]
        })));

        let models = block_on(provider.list_models()).unwrap();

        assert_eq!(
            models,
            vec![
                ModelInfo {
                    name: "qwen2.5-coder".into()
                },
                ModelInfo {
                    name: "mistral".into()
                },
            ]
        );
        assert_eq!(
            state.calls.lock().unwrap()[0].url,
            format!("{BASE}/v1/models")
        );
    }

    #[test]
    fn list_models_rejects_missing_data() {
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
            "choices": [ { "message": { "role": "assistant", "content": "hello" } } ]
        })));

        assert_eq!(
            block_on(provider.chat(&request(None, Some("be brief")))).unwrap(),
            "hello"
        );

        let calls = state.calls.lock().unwrap();
        assert_eq!(calls[0].url, format!("{BASE}/v1/chat/completions"));
        let body = calls[0].body.as_ref().unwrap();
        assert_eq!(body["model"], "qwen2.5-coder");
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
    fn chat_without_any_model_is_an_error() {
        let http = FakeHttpClient::new();
        let state = http.state();
        let provider = LlamaCppProvider::new(BASE, None, http);

        assert!(matches!(
            block_on(provider.chat(&request(None, None))),
            Err(ProviderError::NoModel)
        ));
        assert!(state.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn http_errors_propagate() {
        let (provider, state) = provider(FakeHttpClient::new());
        state.push(Err(ProviderError::Http("500 Internal Server Error".into())));

        assert!(matches!(
            block_on(provider.chat(&request(None, None))),
            Err(ProviderError::Http(_))
        ));
    }

    #[test]
    fn chat_stream_yields_deltas_until_done() {
        let (provider, state) = provider(FakeHttpClient::new());
        state.push_stream(vec![
            "data: {\"choices\":[{\"delta\":{\"role\":\"assistant\"}}]}\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"Hel\"}}]}\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"lo\"}}]}\n",
            "data: [DONE]\n",
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
    fn chat_stream_ignores_comments_and_meta_deltas() {
        let (provider, state) = provider(FakeHttpClient::new());
        state.push_stream(vec![
            ": keep-alive\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"a\"}}]}\n",
            "data: {\"choices\":[{\"finish_reason\":\"stop\"}]}\n",
            "data: [DONE]\n",
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

        assert_eq!(deltas, vec!["a"]);
    }

    #[test]
    fn chat_stream_reports_provider_errors() {
        let (provider, state) = provider(FakeHttpClient::new());
        state.push_stream(vec![
            "data: {\"choices\":[{\"delta\":{\"content\":\"a\"}}]}\n",
            "data: {\"error\":{\"message\":\"context length exceeded\"}}\n",
        ]);

        let result: Vec<Result<String, _>> =
            block_on(provider.chat_stream(&request(None, None)).collect());

        let items: Vec<_> = result.into_iter().collect();
        assert_eq!(items.len(), 2);
        assert!(matches!(items[0], Ok(ref s) if s == "a"));
        assert!(matches!(
            items[1],
            Err(ProviderError::Http(ref msg)) if msg.contains("context length")
        ));
    }

    #[test]
    fn chat_stream_without_any_model_is_an_error() {
        let http = FakeHttpClient::new();
        let state = http.state();
        let provider = LlamaCppProvider::new(BASE, None, http);

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
            "choices": [ {
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [
                        {
                            "id": "call_abc",
                            "type": "function",
                            "function": { "name": "read_file", "arguments": "{\"path\":\"src/main.rs\"}" }
                        }
                    ]
                }
            } ]
        })));

        let mut req = request(None, None);
        req.tools = vec![read_file_tool()];
        let response = block_on(provider.chat_tools(&req)).unwrap();

        // The provider-supplied id and string arguments are preserved as-is.
        assert_eq!(
            response,
            ChatResponse::ToolCalls(vec![ToolCall {
                id: "call_abc".into(),
                name: "read_file".into(),
                arguments: r#"{"path":"src/main.rs"}"#.into(),
            }])
        );
        let calls = state.calls.lock().unwrap();
        let body = calls[0].body.as_ref().unwrap();
        assert_eq!(body["stream"], false);
        assert_eq!(body["tools"][0]["function"]["name"], "read_file");
    }

    #[test]
    fn chat_tools_returns_text_when_no_calls() {
        let (provider, state) = provider(FakeHttpClient::new());
        state.push(Ok(json!({
            "choices": [ { "message": { "role": "assistant", "content": "all done" } } ]
        })));

        let response = block_on(provider.chat_tools(&request(None, None))).unwrap();
        assert_eq!(response, ChatResponse::Text("all done".into()));
    }

    #[test]
    fn chat_tools_sends_tool_call_and_result_messages() {
        let (provider, state) = provider(FakeHttpClient::new());
        state.push(Ok(json!({
            "choices": [ { "message": { "role": "assistant", "content": "fixed it" } } ]
        })));

        let assistant = ChatMessage {
            id: 2,
            session_id: 1,
            role: Role::Assistant,
            content: String::new(),
            created_at: 0,
            tool_calls: Some(vec![ToolCall {
                id: "call_abc".into(),
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
            tool_call_id: Some("call_abc".into()),
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
        // Assistant message carries id, type, and string arguments (OpenAI
        // wire format).
        assert_eq!(
            body["messages"][1],
            json!({
                "role": "assistant",
                "content": null,
                "tool_calls": [
                    {
                        "id": "call_abc",
                        "type": "function",
                        "function": { "name": "read_file", "arguments": "{\"path\":\"src/main.rs\"}" }
                    }
                ]
            })
        );
        // Tool result carries the tool_call_id (OpenAI wire format).
        assert_eq!(
            body["messages"][2],
            json!({ "role": "tool", "tool_call_id": "call_abc", "content": "fn main() {}" })
        );
    }

    #[test]
    fn chat_tools_without_any_model_is_an_error() {
        let http = FakeHttpClient::new();
        let state = http.state();
        let provider = LlamaCppProvider::new(BASE, None, http);

        assert!(matches!(
            block_on(provider.chat_tools(&request(None, None))),
            Err(ProviderError::NoModel)
        ));
        assert!(state.calls.lock().unwrap().is_empty());
    }
}
