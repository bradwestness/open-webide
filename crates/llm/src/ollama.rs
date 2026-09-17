use openwebide_core::{ChatRequest, ModelInfo, ProviderKind};
use serde_json::json;

use crate::{HttpClient, LlmProvider, ProviderError, chat_messages, url_for};

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
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::fake::{FakeHttpClient, FakeState};
    use futures::executor::block_on;
    use openwebide_core::{ChatMessage, Role};

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
}
