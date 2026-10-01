use std::{
    collections::HashMap,
    sync::{Arc, Mutex, atomic::AtomicBool},
};

use leptos::prelude::*;
use openwebide_agent::policy::ApprovalMode;
use openwebide_core::{ChatSession, FileDiff, ModelInfo, RunEvent, SessionTelemetry};

use crate::conversation::{
    ConversationItem, ToolStepResult, local_message, next_item_nonce, notice, stopped_marker,
};

/// Side effects that need a browser or another feature store to execute.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChatEffect {
    /// Automatically approve a tool request under the active session policy.
    ApprovePermission { id: String },
    /// Merge the edit into the owning project's pending diff state.
    ToolDiff(FileDiff),
}

/// Signals shared by the chat pane and run handlers.
#[derive(Clone, Copy)]
pub struct ChatState {
    pub sessions: RwSignal<Vec<ChatSession>>,
    pub active_session: RwSignal<Option<i64>>,
    pub has_session: Memo<bool>,
    pub messages: RwSignal<Vec<ConversationItem>>,
    pub history_gen: StoredValue<u64>,
    pub skip_history_load: StoredValue<Option<i64>>,
    pub streaming: RwSignal<bool>,
    pub active_run: RwSignal<Option<(i64, String, u64)>>,
    pub notice: RwSignal<Option<String>>,
    pub error: RwSignal<Option<String>>,
    pub draft: RwSignal<String>,
    pub show_terminal: RwSignal<bool>,
    #[cfg(target_arch = "wasm32")]
    pub abort: RwSignal<Option<web_sys::AbortController>>,
    pub streaming_session: RwSignal<Option<i64>>,
    pub streaming_is_local: RwSignal<bool>,
    pub local_cancel_flag: StoredValue<Arc<AtomicBool>>,
    pub local_permissions: StoredValue<Arc<Mutex<HashMap<String, bool>>>>,
    pub models: RwSignal<Vec<ModelInfo>>,
    pub session_model: RwSignal<HashMap<i64, Option<String>>>,
    pub selected_model: RwSignal<Option<String>>,
    pub session_telemetry: RwSignal<SessionTelemetry>,
    pub approval_mode: RwSignal<HashMap<i64, ApprovalMode>>,
    pub current_run_anchor: RwSignal<Option<i64>>,
    pub active_editor_context: RwSignal<Option<openwebide_core::EditorContext>>,
    pub awaiting_step_id: Memo<Option<String>>,
}

impl ChatState {
    pub fn new() -> Self {
        Self::with_active_session_and_toast(RwSignal::new(None), RwSignal::new(None))
    }

    pub fn with_active_session(active_session: RwSignal<Option<i64>>) -> Self {
        Self::with_active_session_and_toast(active_session, RwSignal::new(None))
    }

    pub fn with_active_session_and_toast(
        active_session: RwSignal<Option<i64>>,
        error: RwSignal<Option<String>>,
    ) -> Self {
        let messages = RwSignal::new(Vec::new());
        let current_run_anchor = RwSignal::new(None);
        let has_session = Memo::new(move |_| active_session.get().is_some());
        let awaiting_step_id = Memo::new(move |_| {
            let anchor = current_run_anchor.get()?;
            let prefix = openwebide_agent::step_id_prefix(anchor);
            messages.get().into_iter().rev().find_map(|item| {
                if let ConversationItem::ToolStep {
                    id,
                    awaiting_permission: true,
                    result: None,
                    ..
                } = item
                    && id.starts_with(&prefix)
                {
                    return Some(id);
                }
                None
            })
        });

        Self {
            sessions: RwSignal::new(Vec::new()),
            active_session,
            has_session,
            messages,
            history_gen: StoredValue::new(0),
            skip_history_load: StoredValue::new(None),
            streaming: RwSignal::new(false),
            active_run: RwSignal::new(None),
            notice: RwSignal::new(None),
            error,
            draft: RwSignal::new(String::new()),
            show_terminal: RwSignal::new(false),
            #[cfg(target_arch = "wasm32")]
            abort: RwSignal::new(None),
            streaming_session: RwSignal::new(None),
            streaming_is_local: RwSignal::new(false),
            local_cancel_flag: StoredValue::new(Arc::new(AtomicBool::new(false))),
            local_permissions: StoredValue::new(Arc::new(Mutex::new(HashMap::new()))),
            models: RwSignal::new(Vec::new()),
            session_model: RwSignal::new(HashMap::new()),
            selected_model: RwSignal::new(None),
            session_telemetry: RwSignal::new(SessionTelemetry::default()),
            approval_mode: RwSignal::new(HashMap::new()),
            current_run_anchor,
            active_editor_context: RwSignal::new(None),
            awaiting_step_id,
        }
    }

    /// Append a transient notice to the active conversation.
    pub fn notify(&self, text: impl Into<String>) {
        self.messages.update(|items| items.push(notice(text)));
    }

    pub fn set_approval_mode(&self, session_id: i64, mode: ApprovalMode) {
        self.approval_mode.update(|modes| {
            modes.insert(session_id, mode);
        });
    }

    /// Mark prompts owned by this run as cancelled.
    pub fn cancel_run_prompts(&self, anchor: i64) {
        self.messages
            .update(|items| cancel_run_prompts(items, anchor));
    }

    pub fn mark_stopped(&self) {
        if let Some(anchor) = self.current_run_anchor.get_untracked() {
            self.cancel_run_prompts(anchor);
        }
        self.messages.update(|items| {
            if !matches!(items.last(), Some(ConversationItem::Stopped { .. })) {
                items.push(stopped_marker());
            }
        });
        self.current_run_anchor.set(None);
    }

    /// Apply one stream event to chat state and return effects owned by the
    /// browser or another feature store.
    pub fn apply_event(&self, event: RunEvent) -> Vec<ChatEffect> {
        self.apply_event_for_run(self.streaming_session.get_untracked(), event)
    }

    /// Apply an event from a specific stream run, dropping state changes once
    /// the user has switched sessions while the request was in flight.
    pub fn apply_event_for_session(&self, session_id: i64, event: RunEvent) -> Vec<ChatEffect> {
        self.apply_event_for_run(Some(session_id), event)
    }

    fn apply_event_for_run(&self, run_session: Option<i64>, event: RunEvent) -> Vec<ChatEffect> {
        let mut effects = Vec::new();
        if let RunEvent::ToolResult {
            diff: Some(diff), ..
        } = &event
        {
            effects.push(ChatEffect::ToolDiff(diff.clone()));
        }

        if run_session.is_some() && self.active_session.get_untracked() != run_session {
            return effects;
        }
        let session_id = run_session
            .or_else(|| self.active_session.get_untracked())
            .unwrap_or_default();

        match event {
            RunEvent::Message { message: msg } => {
                if msg.role == openwebide_core::Role::User {
                    self.current_run_anchor.set(Some(msg.id));
                }
                self.messages.update(|items| {
                    if let Some(ConversationItem::Message(existing)) = items.iter_mut().find(|item| matches!(item, ConversationItem::Message(m) if m.id == msg.id)) {
                        *existing = msg;
                    } else {
                        items.push(ConversationItem::Message(msg));
                    }
                });
            }
            RunEvent::Delta { content: delta } => self.messages.update(|items| {
                let extends_assistant = items.last().is_some_and(|item| {
                    matches!(
                        item,
                        ConversationItem::Message(message)
                            if message.role == openwebide_core::Role::Assistant && message.id <= 0
                    )
                });
                if extends_assistant {
                    if let Some(ConversationItem::Message(message)) = items.last_mut() {
                        message.content.push_str(&delta);
                    }
                } else {
                    items.push(local_message(session_id, delta));
                }
            }),
            RunEvent::PermissionRequest { id, name, summary } => {
                let mode = self
                    .approval_mode
                    .get_untracked()
                    .get(&session_id)
                    .copied()
                    .unwrap_or_default();
                if mode.auto_approves(&name) {
                    effects.push(ChatEffect::ApprovePermission { id });
                } else {
                    self.messages.update(|items| {
                        items.push(ConversationItem::ToolStep {
                            key: next_item_nonce(),
                            id,
                            name,
                            summary,
                            result: None,
                            awaiting_permission: true,
                        });
                    });
                }
            }
            RunEvent::ToolCall { id, name, summary } => {
                self.session_telemetry
                    .update(|telemetry| telemetry.tool_calls_count += 1);
                self.messages.update(|items| {
                    let idx = items.iter().rposition(|item| {
                        matches!(item, ConversationItem::ToolStep { id: tool_id, .. } if *tool_id == id)
                    });
                    match idx {
                        Some(i) => {
                            if let ConversationItem::ToolStep {
                                summary: step_summary,
                                awaiting_permission,
                                ..
                            } = &mut items[i]
                            {
                                *step_summary = summary;
                                *awaiting_permission = false;
                            }
                        }
                        None => items.push(ConversationItem::ToolStep {
                            key: next_item_nonce(),
                            id,
                            name,
                            summary,
                            result: None,
                            awaiting_permission: false,
                        }),
                    }
                });
            }
            RunEvent::ToolResult {
                id,
                name: _,
                ok,
                summary,
                diff,
            } => self.messages.update(|items| {
                let idx = items.iter().rposition(|item| {
                    matches!(item, ConversationItem::ToolStep { id: tool_id, .. } if *tool_id == id)
                });
                match idx {
                    Some(i) => {
                        if let ConversationItem::ToolStep {
                            result,
                            awaiting_permission,
                            ..
                        } = &mut items[i]
                        {
                            *result = Some(ToolStepResult {
                                ok,
                                summary: summary.clone(),
                                diff: diff.clone(),
                            });
                            *awaiting_permission = false;
                        }
                    }
                    None => items.push(ConversationItem::ToolStep {
                        key: next_item_nonce(),
                        id,
                        name: String::new(),
                        summary: summary.clone(),
                        result: Some(ToolStepResult { ok, summary, diff }),
                        awaiting_permission: false,
                    }),
                }
            }),
            RunEvent::Interim { message } | RunEvent::Done { message } => self.messages.update(|items| {
                if message.id > 0
                    && let Some(ConversationItem::Message(existing)) = items.iter_mut().find(|item| {
                        matches!(item, ConversationItem::Message(existing) if existing.id == message.id)
                    })
                {
                    *existing = message;
                    if matches!(
                        items.last(),
                        Some(ConversationItem::Message(last))
                            if last.role == openwebide_core::Role::Assistant && last.id <= 0
                    ) {
                        items.pop();
                    }
                    return;
                }
                let replace_assistant = items.last().is_some_and(|item| {
                    matches!(
                        item,
                        ConversationItem::Message(last)
                            if last.role == openwebide_core::Role::Assistant && last.id <= 0
                    )
                });
                if replace_assistant {
                    if let Some(ConversationItem::Message(last)) = items.last_mut() {
                        *last = message;
                    }
                } else {
                    items.push(ConversationItem::Message(message));
                }
            }),
            RunEvent::Telemetry { usage: telemetry } => {
                self.session_telemetry
                    .update(|session| session.record_turn(&telemetry));
            }
            RunEvent::Cancelled => {
                self.messages.update(|items| {
                    if let Some(anchor) = self.current_run_anchor.get_untracked() {
                        cancel_run_prompts(items, anchor);
                    }
                    if !matches!(items.last(), Some(ConversationItem::Stopped { .. })) {
                        items.push(stopped_marker());
                    }
                });
            }
            RunEvent::Error { message: error } => {
                if let Some(anchor) = self.current_run_anchor.get_untracked() {
                    self.cancel_run_prompts(anchor);
                }
                self.error.set(Some(error));
            }
        }

        effects
    }
}

impl Default for ChatState {
    fn default() -> Self {
        Self::new()
    }
}

fn cancel_run_prompts(items: &mut [ConversationItem], anchor: i64) {
    let prefix = openwebide_agent::step_id_prefix(anchor);
    for item in items {
        if let ConversationItem::ToolStep {
            id,
            awaiting_permission,
            result,
            ..
        } = item
            && id.starts_with(&prefix)
            && *awaiting_permission
            && result.is_none()
        {
            *awaiting_permission = false;
            *result = Some(ToolStepResult {
                ok: false,
                summary: "cancelled".into(),
                diff: None,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use leptos::prelude::Owner;
    use openwebide_agent::policy::ApprovalMode;
    use openwebide_core::{ChatMessage, FileDiff, Role, TurnTelemetry};

    use super::*;

    fn message(id: i64, role: Role, content: &str) -> ChatMessage {
        ChatMessage {
            id,
            session_id: 7,
            role,
            content: content.into(),
            created_at: 0,
            tool_calls: None,
            tool_call_id: None,
            usage: None,
        }
    }

    #[test]
    fn interim_completes_placeholder_and_next_delta_starts_reply() {
        Owner::new().with(|| {
            let chat = ChatState::new();
            chat.active_session.set(Some(7));
            chat.apply_event(RunEvent::Message { message: message(90, Role::User, "go") });
            chat.apply_event(RunEvent::Delta { content: "checking".into() });
            chat.apply_event(RunEvent::Interim { message: message(91, Role::Assistant, "checking") });
            chat.apply_event(RunEvent::Delta { content: "finished".into() });
            chat.apply_event(RunEvent::Done { message: message(92, Role::Assistant, "finished") });
            let items = chat.messages.get_untracked();
            assert_eq!(items.len(), 3);
            for (item, (id, text)) in items.iter().zip([(90, "go"), (91, "checking"), (92, "finished")]) {
                assert!(matches!(item, ConversationItem::Message(msg) if msg.id == id && msg.content == text));
            }
        });
    }

    #[test]
    fn persisted_reply_events_reconcile_messages_loaded_from_history() {
        Owner::new().with(|| {
            for (interim, queued_delta) in
                [(true, false), (false, false), (true, true), (false, true)]
            {
                let chat = ChatState::new();
                chat.active_session.set(Some(7));
                let existing = message(91, Role::Assistant, "checking");
                let later = message(92, Role::Assistant, "finished");
                chat.messages.set(vec![
                    ConversationItem::Message(message(90, Role::User, "go")),
                    ConversationItem::Message(existing.clone()),
                    ConversationItem::Message(later.clone()),
                ]);
                let mut incoming = existing;
                incoming.usage = Some(TurnTelemetry {
                    prompt_tokens: 100,
                    ..Default::default()
                });
                if queued_delta {
                    chat.apply_event(RunEvent::Delta {
                        content: "check".into(),
                    });
                    chat.apply_event(RunEvent::Delta {
                        content: "ing".into(),
                    });
                }
                chat.apply_event(if interim {
                    RunEvent::Interim {
                        message: incoming.clone(),
                    }
                } else {
                    RunEvent::Done {
                        message: incoming.clone(),
                    }
                });
                let items = chat.messages.get_untracked();
                assert_eq!(items.len(), 3);
                assert!(matches!(&items[1], ConversationItem::Message(msg) if msg == &incoming));
                assert!(matches!(&items[2], ConversationItem::Message(msg) if msg == &later));
            }
        });
    }

    #[test]
    fn applies_message_delta_tool_permission_result_and_done_events() {
        Owner::new().with(|| {
            let chat = ChatState::new();
            chat.active_session.set(Some(7));
            chat.streaming_session.set(Some(7));
            chat.apply_event(RunEvent::Message { message: message(90, Role::User, "edit this") });
            chat.apply_event(RunEvent::Delta { content: "working".into() });
            chat.apply_event(RunEvent::ToolCall {
                id: "a90t0c0".into(),
                name: "write_file".into(),
                summary: "draft.txt".into(),
            });
            chat.apply_event(RunEvent::PermissionRequest {
                id: "a90t1c0".into(),
                name: "write_file".into(),
                summary: "draft.txt".into(),
            });
            chat.apply_event(RunEvent::ToolResult {
                id: "a90t1c0".into(),
                name: "write_file".into(),
                ok: true,
                summary: "wrote draft.txt".into(),
                diff: Some(FileDiff {
                    path: "draft.txt".into(),
                    old: None,
                    new: "done".into(),
                    old_unavailable: false,
                    backup_path: None,
                }),
            });
            chat.apply_event(RunEvent::Done { message: message(91, Role::Assistant, "finished") });

            let items = chat.messages.get_untracked();
            assert_eq!(items.len(), 5);
            assert!(matches!(&items[0], ConversationItem::Message(msg) if msg.id == 90));
            assert!(matches!(&items[1], ConversationItem::Message(msg) if msg.content == "working"));
            assert!(matches!(&items[2], ConversationItem::ToolStep { id, result: None, awaiting_permission: false, .. } if id == "a90t0c0"));
            assert!(matches!(&items[3], ConversationItem::ToolStep { id, result: Some(result), awaiting_permission: false, .. } if id == "a90t1c0" && result.ok));
            assert!(matches!(&items[4], ConversationItem::Message(msg) if msg.id == 91));
            assert_eq!(chat.session_telemetry.get_untracked().tool_calls_count, 1);
        });
    }

    #[test]
    fn cancelled_event_marks_anchored_prompts_and_adds_one_stop_marker() {
        Owner::new().with(|| {
            let chat = ChatState::new();
            chat.active_session.set(Some(7));
            chat.streaming_session.set(Some(7));
            chat.current_run_anchor.set(Some(90));
            chat.messages.set(vec![ConversationItem::ToolStep {
                key: 1,
                id: "a90t0c0".into(),
                name: "write_file".into(),
                summary: "draft.txt".into(),
                result: None,
                awaiting_permission: true,
            }]);

            chat.apply_event(RunEvent::Cancelled);
            chat.apply_event(RunEvent::Cancelled);

            let items = chat.messages.get_untracked();
            assert_eq!(items.len(), 2);
            assert!(matches!(&items[0], ConversationItem::ToolStep { awaiting_permission: false, result: Some(result), .. } if result.summary == "cancelled"));
            assert!(matches!(items[1], ConversationItem::Stopped { .. }));
        });
    }

    #[test]
    fn late_events_from_a_finished_session_do_not_restore_chat_state() {
        Owner::new().with(|| {
            let chat = ChatState::new();
            let effects = chat.apply_event_for_session(
                7,
                RunEvent::ToolResult {
                    id: "a90t0c0".into(),
                    name: "write_file".into(),
                    ok: true,
                    summary: "wrote draft.txt".into(),
                    diff: Some(FileDiff {
                        path: "draft.txt".into(),
                        old: None,
                        new: "done".into(),
                        old_unavailable: false,
                        backup_path: None,
                    }),
                },
            );

            assert!(matches!(effects.as_slice(), [ChatEffect::ToolDiff(_)]));
            assert!(chat.messages.get_untracked().is_empty());
        });
    }

    #[test]
    fn approval_mode_is_scoped_to_each_session() {
        Owner::new().with(|| {
            let chat = ChatState::new();
            chat.set_approval_mode(7, ApprovalMode::AlwaysForSession);

            assert_eq!(
                chat.approval_mode.get_untracked().get(&7),
                Some(&ApprovalMode::AlwaysForSession)
            );
            assert_eq!(chat.approval_mode.get_untracked().get(&8), None);
        });
    }

    #[test]
    fn always_approval_is_reported_as_an_effect_for_its_session() {
        Owner::new().with(|| {
            let chat = ChatState::new();
            chat.active_session.set(Some(7));
            chat.streaming_session.set(Some(7));
            chat.set_approval_mode(7, ApprovalMode::AlwaysForSession);

            let effects = chat.apply_event(RunEvent::PermissionRequest {
                id: "a90t0c0".into(),
                name: "write_file".into(),
                summary: "draft.txt".into(),
            });

            assert_eq!(
                effects,
                vec![ChatEffect::ApprovePermission {
                    id: "a90t0c0".into()
                }]
            );
            assert!(chat.messages.get_untracked().is_empty());
        });
    }

    #[test]
    fn telemetry_events_update_the_session_counters() {
        Owner::new().with(|| {
            let chat = ChatState::new();
            chat.active_session.set(Some(7));
            chat.apply_event(RunEvent::Telemetry {
                usage: TurnTelemetry {
                    prompt_tokens: 3,
                    completion_tokens: 2,
                    estimated: false,
                    eval_duration_ms: 1000,
                },
            });

            let telemetry = chat.session_telemetry.get_untracked();
            assert_eq!(telemetry.total_prompt_tokens, 3);
            assert_eq!(telemetry.total_completion_tokens, 2);
        });
    }
}
