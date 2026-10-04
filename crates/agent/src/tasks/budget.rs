//! One delegation and mutation budget shared by a run's child tree.
use super::model_budget::ModelBudget;
use futures::lock::Mutex;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

pub const MAX_TASKS_PER_RUN: usize = 32;
#[derive(Clone)]
pub struct TaskBudget {
    pub models: ModelBudget,
    pub(crate) mutations: Arc<Mutex<()>>,
    started: Arc<AtomicUsize>,
    tools: Arc<AtomicUsize>,
    max_tools: usize,
}
impl Default for TaskBudget {
    fn default() -> Self {
        Self::new(crate::AgentConfig::default().max_tool_calls)
    }
}
impl TaskBudget {
    pub fn new(max_tools: usize) -> Self {
        Self {
            models: ModelBudget::default(),
            mutations: Arc::default(),
            started: Arc::default(),
            tools: Arc::default(),
            max_tools,
        }
    }
    pub fn reserve_tool(&self) -> Result<(), String> {
        self.tools
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |count| {
                count
                    .checked_add(1)
                    .filter(|count| *count <= self.max_tools)
            })
            .map(|_| ())
            .map_err(|_| {
                format!(
                    "This run reached its limit of {} tool executions across the child tree.",
                    self.max_tools
                )
            })
    }

    pub fn reserve(&self, count: usize) -> Result<(), String> {
        self.started.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |started| {
            started.checked_add(count).filter(|total| *total <= MAX_TASKS_PER_RUN)
        }).map(|_| ()).map_err(|_| format!("This run reached its limit of {MAX_TASKS_PER_RUN} child tasks. Continue the remaining work directly."))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tools_share_the_parent_limit() {
        let budget = TaskBudget::new(2);
        budget.reserve_tool().unwrap();
        budget.clone().reserve_tool().unwrap();
        assert!(budget.reserve_tool().is_err());
    }
    #[test]
    fn descendants_share_a_budget_and_failed_reservations_do_not_consume_it() {
        let budget = TaskBudget::default();
        let descendant = budget.clone();
        budget.reserve(MAX_TASKS_PER_RUN - 1).unwrap();
        assert!(descendant.reserve(2).is_err());
        descendant.reserve(1).unwrap();
        assert!(budget.reserve(1).is_err());
        assert!(budget.reserve(usize::MAX).is_err());
    }
}
