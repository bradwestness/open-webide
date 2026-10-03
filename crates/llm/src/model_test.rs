//! Read-only model checks. The probe tool is never executed.
use crate::{LlmProvider, ProviderError, StreamChunk, ToolStreamChunk};
use futures::StreamExt;
use openwebide_core::{ChatRequest, ChatResponse, ModelTestResult, ToolDefinition};

pub async fn test(
    provider: &impl LlmProvider,
    mut request: ChatRequest,
) -> Result<ModelTestResult, ProviderError> {
    request.model_settings.max_output_tokens = Some(128);
    request.model_settings.tools = Some(true);
    request.messages.clear();
    request.system_prompt = Some("Call the setup_probe tool with value ok. This checks tool calling; do not answer with prose.".into());
    request.tools = vec![ToolDefinition {
        name: "setup_probe".into(),
        description: "Confirm tool calling without performing any operation.".into(),
        parameters: serde_json::json!({"type":"object","properties":{"value":{"type":"string","enum":["ok"]}},"required":["value"]}),
    }];
    let valid = |response: &ChatResponse| matches!(response, ChatResponse::ToolCalls(calls) if calls.iter().any(|call| call.name == "setup_probe" && serde_json::from_str::<serde_json::Value>(&call.arguments).ok().is_some_and(|arguments| arguments.get("value").and_then(serde_json::Value::as_str) == Some("ok"))));
    let structured = match provider.chat_tools(&request).await {
        Ok(completion) => valid(&completion.response),
        Err(ProviderError::Authentication) => return Err(ProviderError::Authentication),
        Err(_) => false,
    };
    let mut result = ModelTestResult {
        structured_tools: structured,
        ..Default::default()
    };
    if structured {
        let mut stream = provider.chat_tools_stream(&request);
        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(ToolStreamChunk::Response(response)) => {
                    result.streamed_tools = valid(&response)
                        && !provider
                            .tool_stream_memo()
                            .is_some_and(|memo| memo.unsupported());
                }
                Err(ProviderError::Authentication) => return Err(ProviderError::Authentication),
                Err(_) => break,
                _ => {}
            }
        }
    } else {
        result.notice = Some("Tool check failed; this model will use plain chat after Save. Detect settings or Test model can recheck it.".into());
    }
    request.tools.clear();
    request.system_prompt = Some("Reply with ok.".into());
    let started = crate::clock_now();
    let mut stream = provider.chat_stream(&request);
    let mut stopped = false;
    while let Some(chunk) = stream.next().await {
        match chunk? {
            StreamChunk::Delta(text) | StreamChunk::Reasoning(text) => {
                if !text.is_empty() && result.first_token_ms.is_none() {
                    result.first_token_ms = started
                        .map(|time| u64::try_from(time.elapsed().as_millis()).unwrap_or(u64::MAX));
                }
            }
            StreamChunk::Usage(usage) => {
                if usage.eval_duration_ms > 0 {
                    result.tokens_per_second = Some(
                        usage.completion_tokens as f64 * 1000.0 / usage.eval_duration_ms as f64,
                    );
                    result.estimated = usage.estimated;
                }
            }
            StreamChunk::Stop(_) => stopped = true,
        }
    }
    if !stopped {
        return Err(ProviderError::Incomplete);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{fake::FakeHttpClient, registry::Provider};
    use openwebide_core::{Connection, ProviderKind};
    use serde_json::json;

    fn request() -> ChatRequest {
        ChatRequest {
            connection_id: 1,
            model: Some("main".into()),
            system_prompt: None,
            messages: vec![],
            tools: vec![],
            model_settings: Default::default(),
        }
    }
    fn connection(kind: ProviderKind) -> Connection {
        Connection {
            id: 1,
            name: String::new(),
            kind,
            base_url: "http://model".into(),
            model: Some("main".into()),
            enabled: true,
            context_limit: None,
            tool_stream_unsupported: false,
            tool_stream_revision: 0,
        }
    }
    #[test]
    fn checks_both_tool_protocols_and_measures_plain_chat_without_executing_tools() {
        futures::executor::block_on(async {
            for kind in [ProviderKind::Ollama, ProviderKind::LlamaCpp] {
                let http = FakeHttpClient::new();
                let state = http.state();
                if kind == ProviderKind::Ollama {
                    let completion = json!({"message":{"role":"assistant","content":"","tool_calls":[{"function":{"name":"setup_probe","arguments":{"value":"ok"}}}]},"done":true});
                    state.push(Ok(completion.clone()));
                    state.push_stream(vec![&format!("{completion}\n")]);
                    state.push_stream(vec!["{\"message\":{\"content\":\"ok\"},\"done\":true,\"eval_count\":4,\"prompt_eval_count\":10,\"eval_duration\":1000000000}\n"]);
                } else {
                    state.push(Ok(json!({"choices":[{"message":{"content":"","tool_calls":[{"id":"probe","type":"function","function":{"name":"setup_probe","arguments":"{\"value\":\"ok\"}"}}]},"finish_reason":"tool_calls"}]})));
                    state.push_stream(vec!["data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"probe\",\"function\":{\"name\":\"setup_probe\",\"arguments\":\"{\\\"value\\\":\\\"ok\\\"}\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\ndata: [DONE]\n\n"]);
                    state.push_stream(vec!["data: {\"choices\":[{\"delta\":{\"content\":\"ok\"},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":4}}\n\ndata: [DONE]\n\n"]);
                }
                let provider = Provider::for_connection(&connection(kind), http);
                let result = test(&provider, request()).await.unwrap();
                assert!(
                    result.structured_tools && result.streamed_tools,
                    "{result:?}"
                );
                assert!(result.first_token_ms.is_some());
                let calls = state.calls.lock().unwrap();
                assert_eq!(calls.len(), 3);
                assert!(calls[2].body.as_ref().unwrap().get("tools").is_none());
            }
        });
    }
    #[test]
    fn rejected_tools_fall_back_only_after_plain_chat_succeeds_and_auth_does_not() {
        futures::executor::block_on(async {
            for kind in [ProviderKind::Ollama, ProviderKind::LlamaCpp] {
                let http = FakeHttpClient::new();
                let state = http.state();
                state.push(Err(ProviderError::Http("tools unsupported".into())));
                if kind == ProviderKind::Ollama {
                    state.push_stream(vec!["{\"message\":{\"content\":\"ok\"},\"done\":true}\n"]);
                } else {
                    state.push_stream(vec!["data: {\"choices\":[{\"delta\":{\"content\":\"ok\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n"]);
                }
                let result = test(
                    &Provider::for_connection(&connection(kind), http),
                    request(),
                )
                .await
                .unwrap();
                assert!(!result.structured_tools);
                assert!(result.notice.is_some());
                assert_eq!(state.calls.lock().unwrap().len(), 2);
                let http = FakeHttpClient::new();
                http.state().push(Err(ProviderError::Authentication));
                assert!(matches!(
                    test(
                        &Provider::for_connection(&connection(kind), http),
                        request()
                    )
                    .await,
                    Err(ProviderError::Authentication)
                ));
            }
        });
    }
}
