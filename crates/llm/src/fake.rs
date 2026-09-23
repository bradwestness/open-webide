//! Scripted in-memory [`HttpClient`] for native tests.

use std::collections::VecDeque;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use futures::{Stream, stream};

use crate::{HttpClient, ProviderError};

/// One recorded provider request.
#[derive(Debug, Clone, PartialEq)]
pub struct Call {
    pub url: String,
    pub body: Option<serde_json::Value>,
}

/// Shared state so tests can queue responses and assert on calls after the
/// fake has been boxed into a provider.
#[derive(Default)]
pub struct FakeState {
    pub calls: Mutex<Vec<Call>>,
    responses: Mutex<VecDeque<Result<serde_json::Value, ProviderError>>>,
    stream_chunks: Mutex<VecDeque<Result<Vec<Bytes>, ProviderError>>>,
}

impl FakeState {
    pub fn push(&self, response: Result<serde_json::Value, ProviderError>) {
        self.responses.lock().unwrap().push_back(response);
    }

    /// Queue a streaming response. Each string is delivered as a separate
    /// byte chunk, so tests can split lines across chunk boundaries.
    pub fn push_stream(&self, chunks: Vec<&str>) {
        let chunks = chunks
            .into_iter()
            .map(|chunk| Bytes::from(chunk.to_string()))
            .collect();
        self.stream_chunks.lock().unwrap().push_back(Ok(chunks));
    }

    pub fn push_stream_error(&self, error: ProviderError) {
        self.stream_chunks.lock().unwrap().push_back(Err(error));
    }

    /// Queue a streaming response as raw byte chunks, so tests can split a
    /// multi-byte UTF-8 character (or line) across chunk boundaries.
    pub fn push_stream_bytes(&self, chunks: Vec<Vec<u8>>) {
        let chunks = chunks.into_iter().map(Bytes::from).collect();
        self.stream_chunks.lock().unwrap().push_back(Ok(chunks));
    }
}

pub struct FakeHttpClient {
    state: Arc<FakeState>,
}

impl FakeHttpClient {
    pub fn new() -> Self {
        Self {
            state: Arc::new(FakeState::default()),
        }
    }

    /// Handle for queueing responses and asserting on recorded calls.
    pub fn state(&self) -> Arc<FakeState> {
        self.state.clone()
    }
}

impl HttpClient for FakeHttpClient {
    async fn get_json(&self, url: &str) -> Result<serde_json::Value, ProviderError> {
        self.state.calls.lock().unwrap().push(Call {
            url: url.into(),
            body: None,
        });
        self.next_response()
    }

    async fn post_json(
        &self,
        url: &str,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value, ProviderError> {
        self.state.calls.lock().unwrap().push(Call {
            url: url.into(),
            body: Some(body.clone()),
        });
        self.next_response()
    }

    fn post_stream(
        &self,
        url: &str,
        body: &serde_json::Value,
    ) -> Pin<Box<dyn Stream<Item = Result<Bytes, ProviderError>> + Send + 'static>> {
        self.state.calls.lock().unwrap().push(Call {
            url: url.into(),
            body: Some(body.clone()),
        });
        let queued = self.state.stream_chunks.lock().unwrap().pop_front();
        let chunks: Vec<Result<Bytes, ProviderError>> = match queued {
            Some(Ok(chunks)) => chunks.into_iter().map(Ok).collect(),
            Some(Err(error)) => vec![Err(error)],
            None => vec![Err(ProviderError::Parse(
                "FakeHttpClient: no stream response queued".into(),
            ))],
        };
        Box::pin(stream::iter(chunks))
    }
}

impl FakeHttpClient {
    fn next_response(&self) -> Result<serde_json::Value, ProviderError> {
        self.state
            .responses
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| {
                Err(ProviderError::Parse(
                    "FakeHttpClient: no response queued".into(),
                ))
            })
    }
}
