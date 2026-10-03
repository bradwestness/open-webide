//! Plain-chat streaming and persistence shared by browser, Spin and bridge hosts.
use super::RunPersistence;
use crate::CancelCheck;
use futures::{
    Stream, StreamExt,
    future::{Either, select},
    stream,
};
use openwebide_core::{
    REPLY_CUT_OFF_MARKER, REPLY_TRUNCATED_MARKER, Role, RunEvent, StopReason, TurnTelemetry,
    with_reasoning,
};
use openwebide_llm::{ProviderError, StreamChunk};
use std::pin::Pin;

pub trait ChatPersistence: Send + Sync {
    fn save_reply(
        &self,
        content: &str,
        usage: Option<&TurnTelemetry>,
    ) -> impl std::future::Future<Output = Result<openwebide_core::ChatMessage, String>> + Send;
}
impl<P: RunPersistence> ChatPersistence for P {
    async fn save_reply(
        &self,
        content: &str,
        usage: Option<&TurnTelemetry>,
    ) -> Result<openwebide_core::ChatMessage, String> {
        self.message(Role::Assistant, content, usage, None).await
    }
}

type ChatStream<'a> = Pin<Box<dyn Stream<Item = Result<StreamChunk, ProviderError>> + Send + 'a>>;
struct State<'a, P, C> {
    persistence: P,
    cancel: C,
    chunks: ChatStream<'a>,
    text: String,
    reasoning: String,
    usage: Option<TurnTelemetry>,
    done: bool,
}

pub fn chat_events<'a, P: ChatPersistence + 'a, C: CancelCheck + Sync + 'a>(
    persistence: P,
    chunks: ChatStream<'a>,
    cancel: C,
) -> impl Stream<Item = RunEvent> + Send + 'a {
    stream::unfold(
        State {
            persistence,
            cancel,
            chunks,
            text: String::new(),
            reasoning: String::new(),
            usage: None,
            done: false,
        },
        |mut state| async move {
            if state.done {
                return None;
            }
            loop {
                let received =
                    match select(state.chunks.next(), Box::pin(state.cancel.cancelled())).await {
                        Either::Left((chunk, _)) => Some(chunk),
                        Either::Right(((), _)) => None,
                    };
                let Some(chunk) = received else {
                    state.done = true;
                    return Some((RunEvent::Cancelled, state));
                };
                let reasoning_chunk = matches!(&chunk, Some(Ok(StreamChunk::Reasoning(_))));
                match chunk {
                    Some(Ok(StreamChunk::Delta(content) | StreamChunk::Reasoning(content))) => {
                        // Distinguish the variant after cancellation, without duplicating accumulation policy.
                        let reasoning = reasoning_chunk;
                        if state.cancel.check().await {
                            state.done = true;
                            return Some((RunEvent::Cancelled, state));
                        }
                        let event = if reasoning {
                            state.reasoning.push_str(&content);
                            RunEvent::ReasoningDelta { content }
                        } else {
                            state.text.push_str(&content);
                            RunEvent::Delta { content }
                        };
                        return Some((event, state));
                    }
                    Some(Ok(StreamChunk::Stop(reason))) => {
                        if reason == StopReason::Length {
                            state.text.push_str(REPLY_CUT_OFF_MARKER);
                        }
                    }
                    Some(Ok(StreamChunk::Usage(usage))) => {
                        state.usage = Some(usage);
                        return Some((RunEvent::Telemetry { usage }, state));
                    }
                    None | Some(Err(ProviderError::Incomplete)) => {
                        let incomplete = chunk.is_some();
                        state.done = true;
                        if state.cancel.check().await {
                            return Some((RunEvent::Cancelled, state));
                        }
                        if incomplete && state.text.is_empty() && state.reasoning.is_empty() {
                            return Some((
                                RunEvent::Error {
                                    message: ProviderError::Incomplete.to_string(),
                                },
                                state,
                            ));
                        }
                        let mut content = with_reasoning(&state.reasoning, &state.text);
                        if incomplete {
                            content.push_str(REPLY_TRUNCATED_MARKER);
                        }
                        let event = match state
                            .persistence
                            .save_reply(&content, state.usage.as_ref())
                            .await
                        {
                            Ok(message) => RunEvent::Done { message },
                            Err(error) => RunEvent::Error {
                                message: format!("failed to save reply: {error}"),
                            },
                        };
                        return Some((event, state));
                    }
                    Some(Err(error)) => {
                        state.done = true;
                        return Some((
                            RunEvent::Error {
                                message: error.to_string(),
                            },
                            state,
                        ));
                    }
                }
            }
        },
    )
}
