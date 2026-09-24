//! Conversation list items and their `<For>` keys.
//!
//! Lives in the lib target (no wasm-only APIs) so the keying rules are
//! natively testable.

use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};

use openwebide_core::{ChatMessage, FileDiff, Role};

/// One item in the conversation: a chat message, an agent tool step, or a
/// stop marker.
#[derive(Debug, Clone)]
pub enum ConversationItem {
    /// A user or assistant chat message.
    Message(ChatMessage),
    /// An agent tool step: the call (always known) and, once it finishes, its
    /// result (which may carry a file diff for edits).
    ToolStep {
        /// A fresh per-item nonce, used as the `<For>` key so two tool calls
        /// sharing a legacy `id` never collide.
        key: u64,
        id: String,
        name: String,
        summary: String,
        result: Option<ToolStepResult>,
        /// A gated call (e.g. a file write) waiting for the user's approval.
        awaiting_permission: bool,
    },
    /// A marker that the user stopped the run. The nonce keeps the item key
    /// unique when a conversation has several stops.
    Stopped { nonce: u64 },
}

/// A fresh "stopped" marker for the conversation list.
static STOP_NONCE: AtomicU64 = AtomicU64::new(0);

pub fn stopped_marker() -> ConversationItem {
    ConversationItem::Stopped {
        nonce: STOP_NONCE.fetch_add(1, Ordering::Relaxed),
    }
}

/// A fresh per-item nonce for a new `ToolStep`, unique within the process.
static NEXT_ITEM_NONCE: AtomicU64 = AtomicU64::new(0);

pub fn next_item_nonce() -> u64 {
    NEXT_ITEM_NONCE.fetch_add(1, Ordering::Relaxed)
}

/// Locally invented message ids: strictly decreasing negatives, so they can
/// never collide with server-assigned (positive) ids.
static NEXT_LOCAL_ID: AtomicI64 = AtomicI64::new(0);

pub fn next_local_id() -> i64 {
    NEXT_LOCAL_ID.fetch_sub(1, Ordering::Relaxed) - 1
}

/// A locally generated assistant message (e.g. a slash-command notice) that
/// never round-trips through the server, so it gets a local negative id.
pub fn local_message(session_id: i64, text: impl Into<String>) -> ConversationItem {
    ConversationItem::Message(ChatMessage {
        id: next_local_id(),
        session_id,
        role: Role::Assistant,
        content: text.into(),
        created_at: 0,
        tool_calls: None,
        tool_call_id: None,
        usage: None,
    })
}

/// The outcome of a finished tool step.
#[derive(Debug, Clone)]
pub struct ToolStepResult {
    pub ok: bool,
    pub summary: String,
    pub diff: Option<FileDiff>,
}

/// A key for a conversation item that changes when the item's content changes
/// (a streamed delta, a tool result arriving) so Leptos' `For` re-renders it.
pub fn item_key(item: &ConversationItem) -> String {
    match item {
        ConversationItem::Message(m) => format!("m-{}-{}", m.id, m.content.len()),
        ConversationItem::ToolStep {
            key,
            result,
            awaiting_permission,
            ..
        } => format!(
            "t-{}-{}-{}",
            key,
            if result.is_some() { 1 } else { 0 },
            if *awaiting_permission { 1 } else { 0 }
        ),
        ConversationItem::Stopped { nonce } => format!("s-{nonce}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_messages_get_distinct_keys() {
        let a = local_message(1, "/help text");
        let b = local_message(1, "/help text");
        assert_ne!(item_key(&a), item_key(&b));
    }

    #[test]
    fn tool_steps_with_same_id_but_distinct_keys_get_distinct_keys() {
        let a = ConversationItem::ToolStep {
            id: "same".into(),
            name: "write_file".into(),
            summary: "a.txt".into(),
            result: None,
            awaiting_permission: false,
            key: next_item_nonce(),
        };
        let b = ConversationItem::ToolStep {
            id: "same".into(),
            name: "write_file".into(),
            summary: "b.txt".into(),
            result: None,
            awaiting_permission: false,
            key: next_item_nonce(),
        };
        assert_ne!(item_key(&a), item_key(&b));
    }

    #[test]
    fn next_local_id_is_negative_and_strictly_decreasing() {
        let a = next_local_id();
        let b = next_local_id();
        let c = next_local_id();
        assert!(a < 0);
        assert!(b < a);
        assert!(c < b);
    }

    #[test]
    fn message_key_changes_when_content_grows() {
        let mut item = local_message(1, "hello");
        let before = item_key(&item);
        if let ConversationItem::Message(m) = &mut item {
            m.content.push_str(" world");
        } else {
            unreachable!("local_message returns a Message");
        }
        assert_ne!(before, item_key(&item));
    }
}
