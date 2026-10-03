use super::*;
use crate::fake::FakeHttpClient;
use futures::{StreamExt, executor::block_on};
use openwebide_core::{ChatResponse, StopReason};

fn connection(kind: ProviderKind, model: Option<&str>) -> Connection {
    Connection {
        id: 1,
        name: "test".into(),
        kind,
        base_url: "http://model.test".into(),
        model: model.map(str::to_string),
        enabled: true,
        context_limit: None,
        tool_stream_unsupported: false,
        tool_stream_revision: 0,
    }
}
fn request(model: Option<&str>) -> ChatRequest {
    ChatRequest {
        connection_id: 1,
        model: model.map(str::to_string),
        system_prompt: None,
        messages: vec![],
        tools: vec![],
        model_settings: Default::default(),
    }
}

#[test]
fn missing_model_contract_for_every_completion_path() {
    block_on(async {
        for kind in [ProviderKind::Ollama, ProviderKind::LlamaCpp] {
            let http = FakeHttpClient::new();
            let state = http.state();
            let provider = Provider::for_connection(&connection(kind, None), http);
            let request = request(None);
            assert!(matches!(
                provider.chat(&request).await,
                Err(ProviderError::NoModel)
            ));
            assert!(matches!(
                provider.chat_tools(&request).await,
                Err(ProviderError::NoModel)
            ));
            let chunks: Vec<_> = provider.chat_stream(&request).collect().await;
            assert!(matches!(chunks.as_slice(), [Err(ProviderError::NoModel)]));
            let chunks: Vec<_> = provider.chat_tools_stream(&request).collect().await;
            assert!(matches!(chunks.as_slice(), [Err(ProviderError::NoModel)]));
            assert!(state.calls.lock().unwrap().is_empty());
        }
    });
}

#[test]
fn delta_stream_contract_for_both_protocols() {
    block_on(async {
        for (kind, wire) in [
            (
                ProviderKind::Ollama,
                concat!(
                    "{\"message\":{\"thinking\":\"why\",\"content\":\"世界\"}}\r\n",
                    "{\"done\":true,\"done_reason\":\"length\",\"message\":{\"content\":\"!\"}}\n"
                ),
            ),
            (
                ProviderKind::LlamaCpp,
                concat!(
                    "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"why\",\"content\":\"世界\"}}]}\r\n",
                    "data: {\"choices\":[{\"delta\":{\"content\":\"!\"},\"finish_reason\":\"length\"}]}\n",
                    "data: [DONE]\n"
                ),
            ),
        ] {
            let http = FakeHttpClient::new();
            let state = http.state();
            state.push_stream_bytes(wire.as_bytes().chunks(1).map(<[u8]>::to_vec).collect());
            let provider = Provider::for_connection(&connection(kind, Some("saved")), http);
            let chunks: Vec<_> = provider
                .chat_stream(&request(Some("override")))
                .collect()
                .await;
            assert!(matches!(chunks.as_slice(), [
                Ok(StreamChunk::Reasoning(reasoning)),
                Ok(StreamChunk::Delta(first)),
                Ok(StreamChunk::Delta(last)),
                Ok(StreamChunk::Stop(StopReason::Length)),
                Ok(StreamChunk::Usage(usage)),
            ] if reasoning == "why" && first == "世界" && last == "!" && usage.estimated));
            assert_eq!(
                state.calls.lock().unwrap()[0].body.as_ref().unwrap()["model"],
                "override"
            );
        }
    });
}

#[test]
fn incomplete_and_transport_failure_contract_for_both_protocols() {
    block_on(async {
        for (kind, wire) in [
            (
                ProviderKind::Ollama,
                "{\"message\":{\"content\":\"partial\"}}\n",
            ),
            (
                ProviderKind::LlamaCpp,
                "data: {\"choices\":[{\"delta\":{\"content\":\"partial\"}}]}\n",
            ),
        ] {
            let http = FakeHttpClient::new();
            let state = http.state();
            state.push_stream(vec![wire]);
            state.push_stream_error(ProviderError::Http("connection reset".into()));
            let provider = Provider::for_connection(&connection(kind, Some("saved")), http);
            let request = request(None);
            let chunks: Vec<_> = provider.chat_stream(&request).collect().await;
            assert!(
                matches!(chunks.as_slice(), [Ok(StreamChunk::Delta(text)), Err(ProviderError::Incomplete)] if text == "partial")
            );
            let chunks: Vec<_> = provider.chat_stream(&request).collect().await;
            assert!(
                matches!(chunks.as_slice(), [Err(ProviderError::Http(message))] if message == "connection reset")
            );
        }
    });
}

#[test]
fn tool_stream_contract_for_both_protocols() {
    block_on(async {
        for (kind, wire) in [
            (
                ProviderKind::Ollama,
                concat!(
                    "{\"message\":{\"thinking\":\"why\",\"content\":\"checking\"}}\n",
                    "{\"done\":true,\"message\":{\"tool_calls\":[{\"function\":{\"name\":\"read_file\",\"arguments\":{\"path\":\"a.txt\"}}}]}}\n"
                ),
            ),
            (
                ProviderKind::LlamaCpp,
                concat!(
                    "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"why\",\"content\":\"checking\"}}]}\n",
                    "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call\",\"function\":{\"name\":\"read_file\",\"arguments\":\"{\\\"path\\\":\"}}]}}]}\n",
                    "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"\\\"a.txt\\\"}\"}}]},\"finish_reason\":\"tool_calls\"}]}\n",
                    "data: [DONE]\n"
                ),
            ),
        ] {
            let http = FakeHttpClient::new();
            let state = http.state();
            state.push_stream_bytes(wire.as_bytes().chunks(1).map(<[u8]>::to_vec).collect());
            let provider = Provider::for_connection(&connection(kind, Some("saved")), http);
            let chunks: Vec<_> = provider
                .chat_tools_stream(&request(Some("override")))
                .collect()
                .await;
            assert!(matches!(chunks.as_slice(), [
                Ok(ToolStreamChunk::Reasoning(reasoning)),
                Ok(ToolStreamChunk::Delta(text)),
                Ok(ToolStreamChunk::Stop(StopReason::Complete)),
                Ok(ToolStreamChunk::Usage(usage)),
                Ok(ToolStreamChunk::Response(ChatResponse::ToolCalls(calls))),
            ] if reasoning == "why" && text == "checking" && usage.estimated && calls.len() == 1
                && calls[0].name == "read_file"
                && serde_json::from_str::<serde_json::Value>(&calls[0].arguments).unwrap() == serde_json::json!({"path":"a.txt"})));
        }
    });
}
