//! Scripted in-memory [`HttpClient`] for native tests.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

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
}

impl FakeState {
    pub fn push(&self, response: Result<serde_json::Value, ProviderError>) {
        self.responses.lock().unwrap().push_back(response);
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
