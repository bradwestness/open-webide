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
    /// A transient notice generated locally by the frontend.
    Notice { nonce: u64, text: String },
}

/// A fresh "stopped" marker for the conversation list.
static STOP_NONCE: AtomicU64 = AtomicU64::new(0);

pub fn stopped_marker() -> ConversationItem {
    ConversationItem::Stopped {
        nonce: STOP_NONCE.fetch_add(1, Ordering::Relaxed),
    }
}

/// A fresh local notice for the conversation list.
pub fn notice(text: impl Into<String>) -> ConversationItem {
    ConversationItem::Notice {
        nonce: NEXT_NOTICE_NONCE.fetch_add(1, Ordering::Relaxed),
        text: text.into(),
    }
}

static NEXT_NOTICE_NONCE: AtomicU64 = AtomicU64::new(0);

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

pub fn merge_snapshot(items: &mut Vec<ConversationItem>, snapshot: &openwebide_core::RunSnapshot) {
    use openwebide_core::RunItem;
    let placeholder = if matches!(items.last(), Some(ConversationItem::Message(m)) if m.id <= 0 && m.role == Role::Assistant)
    {
        items.pop()
    } else {
        None
    };
    for item in &snapshot.items {
        match item {
            RunItem::Message(message) => {
                if let Some(ConversationItem::Message(existing)) = items
                    .iter_mut()
                    .find(|item| matches!(item, ConversationItem::Message(m) if m.id == message.id))
                {
                    *existing = message.clone();
                } else {
                    items.push(ConversationItem::Message(message.clone()));
                }
            }
            RunItem::Step(step) => {
                let key = items
                    .iter()
                    .find_map(|item| match item {
                        ConversationItem::ToolStep { id, key, .. } if *id == step.id => Some(*key),
                        _ => None,
                    })
                    .unwrap_or_else(next_item_nonce);
                let merged = ConversationItem::ToolStep {
                    key,
                    id: step.id.clone(),
                    name: step.name.clone(),
                    summary: step.summary.clone(),
                    awaiting_permission: step.awaiting_permission,
                    result: step.result.as_ref().map(|result| ToolStepResult {
                        ok: result.ok,
                        summary: result.summary.clone(),
                        diff: result.diff.clone(),
                    }),
                };
                if let Some(existing) = items.iter_mut().find(
                    |item| matches!(item, ConversationItem::ToolStep { id, .. } if *id == step.id),
                ) {
                    *existing = merged;
                } else {
                    items.push(merged);
                }
            }
        }
    }
    if !snapshot.text.is_empty() {
        let session_id = snapshot
            .items
            .iter()
            .find_map(|item| match item {
                RunItem::Message(m) => Some(m.session_id),
                _ => None,
            })
            .unwrap_or_default();
        let mut placeholder =
            placeholder.unwrap_or_else(|| local_message(session_id, &snapshot.text));
        if let ConversationItem::Message(message) = &mut placeholder {
            message.session_id = session_id;
            message.content.clone_from(&snapshot.text);
        }
        items.push(placeholder);
    }
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
        ConversationItem::Notice { nonce, .. } => format!("n-{nonce}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshots_upsert_messages_steps_and_live_text_idempotently() {
        use openwebide_core::{RunItem, RunSnapshot, RunStep, ToolStepResultWire};
        let mut message = match local_message(1, "before") {
            ConversationItem::Message(m) => m,
            _ => unreachable!(),
        };
        message.id = 10;
        let mut items = vec![
            ConversationItem::Message(message.clone()),
            ConversationItem::ToolStep {
                key: 55,
                id: "t".into(),
                name: "write_file".into(),
                summary: "old".into(),
                awaiting_permission: false,
                result: None,
            },
        ];
        message.content = "updated".into();
        let mut snapshot = RunSnapshot {
            items: vec![
                RunItem::Message(message.clone()),
                RunItem::Step(RunStep {
                    id: "t".into(),
                    name: "write_file".into(),
                    summary: "new".into(),
                    awaiting_permission: true,
                    result: None,
                }),
            ],
            text: "live".into(),
            ..Default::default()
        };
        for _ in 0..2 {
            merge_snapshot(&mut items, &snapshot);
            assert_eq!(items.len(), 3);
            assert!(matches!(&items[0], ConversationItem::Message(m) if m.content == "updated"));
            assert!(
                matches!(&items[1], ConversationItem::ToolStep { key: 55, awaiting_permission: true, summary, .. } if summary == "new")
            );
            assert!(matches!(&items[2], ConversationItem::Message(m) if m.content == "live"));
        }
        if let RunItem::Step(step) = &mut snapshot.items[1] {
            step.awaiting_permission = false;
            step.result = Some(ToolStepResultWire {
                ok: true,
                summary: "written".into(),
                diff: None,
            });
        }
        snapshot.text.clear();
        for _ in 0..2 {
            merge_snapshot(&mut items, &snapshot);
            assert_eq!(items.len(), 2);
            assert!(
                matches!(&items[1], ConversationItem::ToolStep { key: 55, awaiting_permission: false, result: Some(result), .. } if result.ok)
            );
        }
    }

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

    #[test]
    fn notice_items_have_distinct_keys() {
        let a = notice("Saved.");
        let b = notice("Saved.");
        assert_ne!(item_key(&a), item_key(&b));
    }
}
