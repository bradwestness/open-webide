//! Per-run model admission shared by children and their descendants.
use futures::{Stream, StreamExt, channel::mpsc, lock::Mutex, stream};
use openwebide_core::{
    ChatCompletion, ChatRequest, ModelInfo, ProviderKind, tasks::MAX_PARALLEL_TASKS,
};
use openwebide_llm::{LlmProvider, ProviderError, StreamChunk, ToolStreamChunk, ToolStreamMemo};
use std::{pin::Pin, sync::Arc};

#[derive(Clone)]
pub struct ModelBudget(Arc<Admission>);
struct Admission {
    sender: mpsc::UnboundedSender<()>,
    receiver: Mutex<mpsc::UnboundedReceiver<()>>,
}
pub struct ModelPermit(mpsc::UnboundedSender<()>);
impl Drop for ModelPermit {
    fn drop(&mut self) {
        let _ = self.0.unbounded_send(());
    }
}
impl Default for ModelBudget {
    fn default() -> Self {
        let (sender, receiver) = mpsc::unbounded();
        for _ in 0..MAX_PARALLEL_TASKS {
            sender
                .unbounded_send(())
                .expect("New admission queue is open");
        }
        Self(Arc::new(Admission {
            sender,
            receiver: Mutex::new(receiver),
        }))
    }
}
impl ModelBudget {
    /// Hold a permit for model I/O only, releasing it before tools/approvals/delegation.
    pub async fn acquire(&self) -> ModelPermit {
        self.0
            .receiver
            .lock()
            .await
            .next()
            .await
            .expect("Admission keeps its sender alive");
        ModelPermit(self.0.sender.clone())
    }
}

/// Acquire before calling the provider, including providers that eagerly start a stream.
pub struct LimitedProvider<P> {
    provider: Arc<P>,
    budget: ModelBudget,
}
impl<P> LimitedProvider<P> {
    pub fn new(provider: P, budget: ModelBudget) -> Self {
        Self {
            provider: Arc::new(provider),
            budget,
        }
    }
}
fn hold_stream<T: Send + 'static>(
    source: Pin<Box<dyn Stream<Item = T> + Send>>,
    permit: ModelPermit,
) -> Pin<Box<dyn Stream<Item = T> + Send>> {
    Box::pin(stream::unfold(
        (source, permit),
        |(mut source, permit)| async move {
            let item = source.next().await?;
            Some((item, (source, permit)))
        },
    ))
}
impl<P: LlmProvider + 'static> LlmProvider for LimitedProvider<P> {
    fn kind(&self) -> ProviderKind {
        self.provider.kind()
    }
    fn tool_stream_memo(&self) -> Option<ToolStreamMemo> {
        self.provider.tool_stream_memo()
    }
    async fn list_models(&self) -> Result<Vec<ModelInfo>, ProviderError> {
        self.provider.list_models().await
    }
    async fn request_tokens(&self, request: &ChatRequest) -> Option<usize> {
        self.provider.request_tokens(request).await
    }
    async fn context_limit(&self, model: Option<&str>) -> Result<Option<usize>, ProviderError> {
        self.provider.context_limit(model).await
    }
    async fn chat(&self, request: &ChatRequest) -> Result<String, ProviderError> {
        let _permit = self.budget.acquire().await;
        self.provider.chat(request).await
    }
    async fn chat_tools(&self, request: &ChatRequest) -> Result<ChatCompletion, ProviderError> {
        let _permit = self.budget.acquire().await;
        self.provider.chat_tools(request).await
    }
    fn chat_stream(
        &self,
        request: &ChatRequest,
    ) -> Pin<Box<dyn Stream<Item = Result<StreamChunk, ProviderError>> + Send>> {
        let provider = self.provider.clone();
        let budget = self.budget.clone();
        let request = request.clone();
        Box::pin(
            stream::once(async move {
                let permit = budget.acquire().await;
                hold_stream(provider.chat_stream(&request), permit)
            })
            .flatten(),
        )
    }
    fn chat_tools_stream(
        &self,
        request: &ChatRequest,
    ) -> Pin<Box<dyn Stream<Item = Result<ToolStreamChunk, ProviderError>> + Send>> {
        let provider = self.provider.clone();
        let budget = self.budget.clone();
        let request = request.clone();
        Box::pin(
            stream::once(async move {
                let permit = budget.acquire().await;
                hold_stream(provider.chat_tools_stream(&request), permit)
            })
            .flatten(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::FutureExt;
    #[test]
    fn admission_is_shared_bounded_and_a_cancelled_waiter_does_not_lose_capacity() {
        futures::executor::block_on(async {
            let budget = ModelBudget::default();
            let mut permits = Vec::new();
            for _ in 0..MAX_PARALLEL_TASKS {
                permits.push(budget.acquire().await);
            }
            assert!(budget.clone().acquire().now_or_never().is_none());
            permits.pop();
            let replacement = budget.acquire().await;
            assert!(budget.acquire().now_or_never().is_none());
            drop(replacement);
            drop(permits);
            let mut restored = Vec::new();
            for _ in 0..MAX_PARALLEL_TASKS {
                restored.push(budget.acquire().await);
            }
            assert!(budget.acquire().now_or_never().is_none());
            drop(restored);
        });
    }
}
