//! One child-loop implementation over provider, execution and permission primitives.
use super::{
    budget::TaskBudget,
    executor::{TaskExecutor, TaskGate},
    model_budget::LimitedProvider,
    mutations::MutationExecutor,
    scheduler::{ChildStream, TaskHost},
};
use crate::{AgentConfig, CancelCheck, PermissionGate, ToolExecutor, model::ModelSource};
use openwebide_core::{ChatCompletion, ChatRequest, ModelRuntime, ModelSelection};
use openwebide_llm::LlmProvider;
use std::future::Future;

pub type TaskPrimitives<F> = (
    <F as TaskFactory>::Provider,
    <F as TaskFactory>::Executor,
    <F as TaskFactory>::Gate,
);

pub trait TaskFactory: Clone + Send + Sync + 'static {
    type Provider: LlmProvider + Send + Sync + 'static;
    type Executor: ToolExecutor + Sync + 'static;
    type Gate: PermissionGate + Sync + 'static;
    fn now_ms(&self) -> u64;
    fn prepare(
        &self,
        request: &ChatRequest,
    ) -> impl Future<Output = Result<TaskPrimitives<Self>, String>> + Send;
}
#[derive(Clone)]
pub struct ChildHost<F, C, S> {
    factory: F,
    cancel: C,
    source: S,
    anchor: i64,
    config: AgentConfig,
}
impl<F: TaskFactory, C: CancelCheck + Clone + Sync + 'static, S: ModelSource + Clone + 'static>
    CancelCheck for ChildHost<F, C, S>
{
    fn check(&self) -> impl Future<Output = bool> + Send {
        self.cancel.check()
    }
    fn cancelled(&self) -> impl Future<Output = ()> + Send {
        self.cancel.cancelled()
    }
}
impl<F: TaskFactory, C: CancelCheck + Clone + Sync + 'static, S: ModelSource + Clone + 'static>
    ModelSource for ChildHost<F, C, S>
{
    fn available(&self) -> bool {
        self.source.available()
    }
    fn runtime(
        &self,
        selection: &ModelSelection,
    ) -> impl Future<Output = Result<ModelRuntime, String>> + Send {
        self.source.runtime(selection)
    }
    fn complete(
        &self,
        request: &ChatRequest,
    ) -> impl Future<Output = Result<ChatCompletion, String>> + Send {
        self.source.complete(request)
    }
    fn context_limit(&self, request: &ChatRequest) -> impl Future<Output = Option<usize>> + Send {
        self.source.context_limit(request)
    }
    fn tokens(&self, request: &ChatRequest) -> impl Future<Output = Option<usize>> + Send {
        self.source.tokens(request)
    }
}
#[derive(Clone)]
struct BudgetSource<S> {
    source: S,
    budget: TaskBudget,
}
impl<S: ModelSource> ModelSource for BudgetSource<S> {
    fn available(&self) -> bool {
        self.source.available()
    }
    fn runtime(
        &self,
        selection: &ModelSelection,
    ) -> impl Future<Output = Result<ModelRuntime, String>> + Send {
        self.source.runtime(selection)
    }
    async fn complete(&self, request: &ChatRequest) -> Result<ChatCompletion, String> {
        let _permit = self.budget.models.acquire().await;
        self.source.complete(request).await
    }
    async fn complete_with_timeout(
        &self,
        request: &ChatRequest,
        timeout_seconds: u32,
    ) -> Result<ChatCompletion, String> {
        let _permit = self.budget.models.acquire().await;
        self.source
            .complete_with_timeout(request, timeout_seconds)
            .await
    }
    fn context_limit(&self, request: &ChatRequest) -> impl Future<Output = Option<usize>> + Send {
        self.source.context_limit(request)
    }
    fn tokens(&self, request: &ChatRequest) -> impl Future<Output = Option<usize>> + Send {
        self.source.tokens(request)
    }
}
impl<F: TaskFactory, C: CancelCheck + Clone + Sync + 'static, S: ModelSource + Clone + 'static>
    TaskHost for ChildHost<F, C, S>
{
    fn now_ms(&self) -> u64 {
        self.factory.now_ms()
    }
    async fn run_child(
        &self,
        request: ChatRequest,
        scope: String,
        depth: usize,
        budget: TaskBudget,
    ) -> Result<ChildStream, String> {
        let (provider, executor, gate) = self.factory.prepare(&request).await?;
        Ok(Box::pin(crate::run_scoped(
            LimitedProvider::new(provider, budget.models.clone()),
            TaskExecutor::new(
                MutationExecutor::new(executor, budget.clone()),
                self.clone(),
                request.clone(),
                depth,
                budget.clone(),
            ),
            request,
            AgentConfig {
                first_turn: 1,
                max_turns: self.config.max_turns.min(64),
                max_tool_calls: self.config.max_tool_calls.min(128),
            },
            self.cancel.clone(),
            TaskGate(gate, budget.models.clone()),
            self.anchor,
            BudgetSource {
                source: self.source.clone(),
                budget,
            },
            Some(scope),
        )))
    }
}

/// Root and descendants share delegation policy, model capacity and checkpoint coordination.
#[allow(clippy::too_many_arguments)]
pub fn run_tree<F, C, S, P>(
    factory: F,
    source: S,
    cancel: C,
    provider: P,
    executor: F::Executor,
    gate: F::Gate,
    request: ChatRequest,
    config: AgentConfig,
    anchor: i64,
) -> impl futures::Stream<Item = crate::AgentEvent> + Send
where
    P: LlmProvider + Send + Sync + 'static,
    F: TaskFactory,
    C: CancelCheck + Clone + Sync + 'static,
    S: ModelSource + Clone + 'static,
{
    let budget = TaskBudget::new(config.max_tool_calls);
    let host = ChildHost {
        factory,
        cancel: cancel.clone(),
        source: source.clone(),
        anchor,
        config,
    };
    crate::run_scoped(
        LimitedProvider::new(provider, budget.models.clone()),
        TaskExecutor::new(
            MutationExecutor::new(executor, budget.clone()),
            host,
            request.clone(),
            0,
            budget.clone(),
        ),
        request,
        config,
        cancel,
        TaskGate(gate, budget.models.clone()),
        anchor,
        BudgetSource { source, budget },
        None,
    )
}
