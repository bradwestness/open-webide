//! The agent-maintained checklist shared by every session host.
use std::collections::HashSet;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TodoStatus {
    Pending,
    InProgress,
    Completed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TodoItem {
    pub id: String,
    pub content: String,
    pub status: TodoStatus,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TodoPlan {
    pub todos: Vec<TodoItem>,
}

impl TodoPlan {
    /// A tool update replaces the checklist; an empty list clears it.
    pub fn validate(&self) -> Result<(), String> {
        if self.todos.len() > 64 {
            return Err("A plan may contain at most 64 items".into());
        }
        let mut ids = HashSet::new();
        let mut total = 0;
        let mut active = 0;
        for todo in &self.todos {
            if todo.id.trim().is_empty()
                || todo.id.len() > 64
                || todo.id.chars().any(char::is_control)
            {
                return Err(
                    "Each plan item needs an ID of 1–64 bytes without control characters".into(),
                );
            }
            if !ids.insert(&todo.id) {
                return Err("Plan item IDs must be unique".into());
            }
            if todo.content.trim().is_empty()
                || todo.content.len() > 2048
                || todo.content.contains('\0')
            {
                return Err("Each plan item needs text of 1–2048 bytes".into());
            }
            total += todo.content.len();
            active += usize::from(todo.status == TodoStatus::InProgress);
        }
        if total > 32 * 1024 {
            return Err("Plan text must fit within 32 KiB".into());
        }
        if active > 1 {
            return Err("Only one plan item may be in progress at a time".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_updates_validate_identity_status_and_size_without_mutating_the_input() {
        let mut plan = TodoPlan {
            todos: vec![
                TodoItem {
                    id: "inspect".into(),
                    content: "Inspect the failing test".into(),
                    status: TodoStatus::InProgress,
                },
                TodoItem {
                    id: "fix".into(),
                    content: "Fix the failure".into(),
                    status: TodoStatus::Pending,
                },
            ],
        };
        assert!(plan.validate().is_ok());
        let stored = serde_json::to_string(&plan).unwrap();
        assert!(stored.contains("in_progress"));
        assert_eq!(serde_json::from_str::<TodoPlan>(&stored).unwrap(), plan);
        assert!(
            serde_json::from_str::<TodoPlan>(&stored.replace("in_progress", "unknown")).is_err()
        );
        plan.todos[1].status = TodoStatus::InProgress;
        let invalid = plan.clone();
        assert!(plan.validate().is_err());
        assert_eq!(plan, invalid);
        plan.todos[1].status = TodoStatus::Completed;
        plan.todos[1].id = "inspect".into();
        assert!(plan.validate().is_err());
        plan.todos[1].id = "fix".into();
        plan.todos[1].content = "x".repeat(2049);
        assert!(plan.validate().is_err());
        plan.todos[1].content = "  ".into();
        assert!(plan.validate().is_err());
        assert!(TodoPlan::default().validate().is_ok());
    }
}

/// A persisted checklist revision, anchored to the prompt that produced it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TodoUpdate {
    pub id: i64,
    pub session_id: i64,
    pub anchor_message_id: i64,
    pub plan: TodoPlan,
    pub created_at: i64,
}
