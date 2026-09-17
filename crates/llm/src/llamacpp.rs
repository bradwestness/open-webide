use openwebide_core::{ChatRequest, ModelInfo, ProviderKind};
use serde_json::json;

use crate::{HttpClient, LlmProvider, ProviderError, chat_messages, url_for};

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
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::fake::{FakeHttpClient, FakeState};
    use futures::executor::block_on;
    use openwebide_core::{ChatMessage, Role};

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
        }
    }

    fn request(model: Option<&str>, system: Option<&str>) -> ChatRequest {
        ChatRequest {
            connection_id: 1,
            system_prompt: system.map(str::to_string),
            model: model.map(str::to_string),
            messages: vec![message(Role::User, "hi")],
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
}
