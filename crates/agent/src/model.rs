//! Model I/O primitives shared by background workflows.
use openwebide_core::{ChatCompletion, ChatRequest, ModelRuntime, ModelSelection};
use std::future::Future;

pub trait ModelSource: Send + Sync {
    fn available(&self) -> bool {
        false
    }
    fn runtime(
        &self,
        _selection: &ModelSelection,
    ) -> impl Future<Output = Result<ModelRuntime, String>> + Send {
        async { Err("Model runtime unavailable".into()) }
    }
    fn complete(
        &self,
        _request: &ChatRequest,
    ) -> impl Future<Output = Result<ChatCompletion, String>> + Send {
        async { Err("Model completion unavailable".into()) }
    }
    /// Hosts enforce this transport deadline for best-effort background work.
    fn complete_with_timeout(
        &self,
        request: &ChatRequest,
        _timeout_seconds: u32,
    ) -> impl Future<Output = Result<ChatCompletion, String>> + Send {
        self.complete(request)
    }
    fn context_limit(&self, _request: &ChatRequest) -> impl Future<Output = Option<usize>> + Send {
        async { None }
    }
    fn tokens(&self, _request: &ChatRequest) -> impl Future<Output = Option<usize>> + Send {
        async { None }
    }
}
