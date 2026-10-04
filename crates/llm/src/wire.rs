//! Provider message serialization; only protocol fields differ.

use openwebide_core::{ChatRequest, ProviderKind, Role, ToolDefinition};
use serde_json::{Value, json};

/// Provider wire format for chat messages: `role` + `content` pairs, with
/// the system prompt first when one is set.
pub(crate) fn chat_messages(request: &ChatRequest, kind: ProviderKind) -> Vec<serde_json::Value> {
    let mut messages = Vec::new();
    if let Some(system) = &request.system_prompt
        && !system.is_empty()
    {
        messages.push(json!({ "role": "system", "content": system }));
    }
    for message in &request.messages {
        messages.push(message_wire(message, kind));
    }
    messages
}

/// Provider wire format for the `tools` array, shared by both providers
/// (Ollama and the OpenAI-compatible llama.cpp API use the same shape).
pub(crate) fn tools_wire(tools: &[ToolDefinition]) -> serde_json::Value {
    serde_json::Value::Array(
        tools
            .iter()
            .map(|tool| {
                json!({
                    "type": "function",
                    "function": {
                        "name": tool.name,
                        "description": tool.description,
                        "parameters": tool.parameters,
                    }
                })
            })
            .collect(),
    )
}

/// Serialize tool history once, adapting only provider-specific wire fields.
pub(crate) fn tool_messages(request: &ChatRequest, kind: ProviderKind) -> Vec<Value> {
    let mut messages = Vec::new();
    if let Some(system) = &request.system_prompt
        && !system.is_empty()
    {
        messages.push(json!({ "role": "system", "content": system }));
    }
    for message in &request.messages {
        let mut value = message_wire(message, kind);
        if message.role == Role::Tool && kind == ProviderKind::LlamaCpp {
            value["tool_call_id"] = json!(message.tool_call_id.as_deref().unwrap_or_default());
        } else if message.role == Role::Assistant
            && let Some(calls) = &message.tool_calls
        {
            if message.content.is_empty() {
                value["content"] = Value::Null;
            }
            value["tool_calls"] = Value::Array(
                calls
                    .iter()
                    .map(|call| match kind {
                        ProviderKind::Ollama => {
                            let args: Value =
                                serde_json::from_str(&call.arguments).unwrap_or_else(|_| json!({}));
                            json!({ "function": { "name": call.name, "arguments": args } })
                        }
                        ProviderKind::LlamaCpp => json!({
                            "id": call.id, "type": "function",
                            "function": { "name": call.name, "arguments": call.arguments }
                        }),
                    })
                    .collect(),
            );
        }
        messages.push(value);
    }
    messages
}

fn message_wire(message: &openwebide_core::ChatMessage, kind: ProviderKind) -> Value {
    let prompt = if message.role == Role::User {
        openwebide_core::PromptContent::decode(&message.content)
    } else {
        openwebide_core::PromptContent {
            text: message.content.clone(),
            ..Default::default()
        }
    };
    let mut value = json!({ "role": message.role.as_str(), "content": prompt.model_text() });
    if !prompt.images.is_empty() {
        match kind {
            ProviderKind::Ollama => {
                value["images"] = json!(
                    prompt
                        .images
                        .iter()
                        .map(|image| &image.data)
                        .collect::<Vec<_>>()
                );
            }
            ProviderKind::LlamaCpp => {
                let mut content = vec![json!({"type":"text","text":prompt.model_text()})];
                content.extend(
                    prompt
                        .images
                        .iter()
                        .map(|image| json!({"type":"image_url","image_url":{"url":image.url()}})),
                );
                value["content"] = json!(content);
            }
        }
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    use openwebide_core::{
        PromptContent, PromptImage,
        prompt::{Mention, MentionKind, PromptReference},
    };
    #[test]
    fn both_providers_send_images_and_reference_context_in_plain_and_tool_requests() {
        let image = PromptImage::from_bytes("image.png".into(), b"\x89PNG\r\n\x1a\n").unwrap();
        let prompt = PromptContent {
            text: "Explain".into(),
            images: vec![image.clone()],
            references: vec![PromptReference {
                mention: Mention {
                    kind: MentionKind::File,
                    path: "file.rs".into(),
                },
                content: "let value = 1;".into(),
            }],
        };
        let request: ChatRequest = serde_json::from_value(json!({"connection_id":1,"messages":[{"id":1,"session_id":1,"role":"user","content":prompt.encode().unwrap(),"created_at":0}]})).unwrap();
        for kind in [ProviderKind::Ollama, ProviderKind::LlamaCpp] {
            let plain = chat_messages(&request, kind);
            let tools = tool_messages(&request, kind);
            assert_eq!(plain, tools);
            if kind == ProviderKind::Ollama {
                assert_eq!(plain[0]["images"][0], image.data);
                assert!(
                    plain[0]["content"]
                        .as_str()
                        .unwrap()
                        .contains("let value = 1;")
                );
                assert!(!plain[0]["content"].as_str().unwrap().contains(&image.data));
            } else {
                assert_eq!(plain[0]["content"][1]["image_url"]["url"], image.url());
                assert!(
                    plain[0]["content"][0]["text"]
                        .as_str()
                        .unwrap()
                        .contains("let value = 1;")
                );
            }
        }
    }
}

#[cfg(test)]
mod provider_contract {
    use crate::{
        HttpClient, LlmProvider, ProviderError, llamacpp::LlamaCppProvider, ollama::OllamaProvider,
    };
    use bytes::Bytes;
    use futures::{Stream, StreamExt, stream};
    use openwebide_core::{ChatRequest, PromptContent, PromptImage};
    use serde_json::{Value, json};
    use std::{
        pin::Pin,
        sync::{Arc, Mutex},
    };
    #[derive(Clone, Default)]
    struct Http(Arc<Mutex<Vec<Value>>>);
    impl HttpClient for Http {
        async fn get_json(&self, _: &str) -> Result<Value, ProviderError> {
            Ok(json!({}))
        }
        async fn post_json(&self, url: &str, body: &Value) -> Result<Value, ProviderError> {
            self.0.lock().unwrap().push(body.clone());
            if url.ends_with("/apply-template") {
                return Ok(json!({"prompt":"text template"}));
            }
            if url.ends_with("/tokenize") {
                return Ok(json!({"tokens":[1,2,3]}));
            }
            Ok(
                json!({"message":{"content":"seen"},"choices":[{"message":{"content":"seen"},"finish_reason":"stop"}]}),
            )
        }
        fn post_stream(
            &self,
            url: &str,
            body: &Value,
        ) -> Pin<Box<dyn Stream<Item = Result<Bytes, ProviderError>> + Send>> {
            self.0.lock().unwrap().push(body.clone());
            let content = if url.contains("/api/chat") {
                "{\"message\":{\"content\":\"seen\"},\"done\":true}\n"
            } else {
                "data: {\"choices\":[{\"delta\":{\"content\":\"seen\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n"
            };
            Box::pin(stream::once(async move { Ok(Bytes::from(content)) }))
        }
    }
    async fn contract(provider: impl LlmProvider, http: Http, ollama: bool) {
        let image = PromptImage::from_bytes("test.png".into(), b"\x89PNG\r\n\x1a\n").unwrap();
        let prompt = PromptContent {
            text: "Describe".into(),
            images: vec![image.clone()],
            ..Default::default()
        };
        let mut request: ChatRequest = serde_json::from_value(json!({"connection_id":1,"model":"vision","messages":[{"id":1,"session_id":1,"role":"user","content":prompt.encode().unwrap(),"created_at":0}]})).unwrap();
        assert_eq!(provider.chat(&request).await.unwrap(), "seen");
        provider.chat_tools(&request).await.unwrap();
        for chunk in provider.chat_stream(&request).collect::<Vec<_>>().await {
            chunk.unwrap();
        }
        for chunk in provider
            .chat_tools_stream(&request)
            .collect::<Vec<_>>()
            .await
        {
            chunk.unwrap();
        }
        let bodies = http.0.lock().unwrap().clone();
        assert_eq!(bodies.len(), 4);
        for body in bodies {
            if ollama {
                assert_eq!(body["messages"][0]["images"][0], image.data);
            } else {
                assert_eq!(
                    body["messages"][0]["content"][1]["image_url"]["url"],
                    image.url()
                );
            }
        }
        let expected_calls = if ollama {
            4
        } else {
            assert_eq!(
                provider.request_tokens(&request).await,
                Some(3 + 32 + prompt.image_tokens())
            );
            let bodies = http.0.lock().unwrap();
            assert_eq!(bodies[4]["messages"][0]["content"], "Describe");
            assert!(!bodies[4].to_string().contains(&image.data));
            6
        };
        request.model_settings.vision = Some(false);
        assert!(provider.chat(&request).await.is_err());
        assert!(provider.chat_tools(&request).await.is_err());
        assert!(
            provider
                .chat_stream(&request)
                .next()
                .await
                .unwrap()
                .is_err()
        );
        assert!(
            provider
                .chat_tools_stream(&request)
                .next()
                .await
                .unwrap()
                .is_err()
        );
        assert_eq!(
            http.0.lock().unwrap().len(),
            expected_calls,
            "Known unsupported images must not reach HTTP"
        );
    }
    #[test]
    fn image_inputs_and_known_unsupported_models_cover_every_provider_entrypoint() {
        futures::executor::block_on(async {
            let http = Http::default();
            contract(
                OllamaProvider::new("http://server", None, http.clone()),
                http,
                true,
            )
            .await;
            let http = Http::default();
            contract(
                LlamaCppProvider::new("http://server", None, http.clone()),
                http,
                false,
            )
            .await;
        });
    }
}
