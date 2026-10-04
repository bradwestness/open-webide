use super::support::{chat_view, mount_test, settle};
use leptos::prelude::*;
use openwebide_core::{RunEvent, WorkspaceMode};
use wasm_bindgen::prelude::*;
use wasm_bindgen_test::*;

#[wasm_bindgen(inline_js = r#"
export function clipboardProbe() {
    const fixture = {written:[], reject:false, legacy:false};
    const clipboard = Object.getOwnPropertyDescriptor(navigator, 'clipboard');
    const command = Object.getOwnPropertyDescriptor(document, 'execCommand');
    Object.defineProperty(navigator, 'clipboard', {configurable:true, value:{writeText: async text => {
        if (fixture.reject) throw new Error('clipboard denied');
        fixture.written.push(text);
    }}});
    Object.defineProperty(document, 'execCommand', {configurable:true, value:command => {
        if (command !== 'copy') throw new Error('unexpected command');
        const input = document.querySelector('.tui-copy-buffer');
        if (!input) throw new Error('missing copy input');
        fixture.written.push(input.value);
        return true;
    }});
    fixture.restore = () => {
        if (clipboard) Object.defineProperty(navigator,'clipboard',clipboard); else delete navigator.clipboard;
        if (command) Object.defineProperty(document,'execCommand',command); else delete document.execCommand;
    };
    return fixture;
}
export function clipboardText(fixture) { return fixture.written.at(-1) || ''; }
export function clipboardDeny(fixture) { fixture.reject = true; }
export function clipboardUnavailable() { Object.defineProperty(navigator,'clipboard',{configurable:true,value:undefined}); }
export function clipboardRestore(fixture) { fixture.restore(); }
"#)]
extern "C" {
    #[wasm_bindgen(js_name = clipboardProbe)]
    fn clipboard_probe() -> JsValue;
    #[wasm_bindgen(js_name = clipboardText)]
    fn clipboard_text(fixture: &JsValue) -> String;
    #[wasm_bindgen(js_name = clipboardDeny)]
    fn clipboard_deny(fixture: &JsValue);
    #[wasm_bindgen(js_name = clipboardUnavailable)]
    fn clipboard_unavailable();
    #[wasm_bindgen(js_name = clipboardRestore)]
    fn clipboard_restore(fixture: &JsValue);
}
struct ClipboardProbe(JsValue);
impl Drop for ClipboardProbe {
    fn drop(&mut self) {
        clipboard_restore(&self.0);
    }
}

#[wasm_bindgen_test]
async fn ansi_tool_output_expands_and_copies_full_plain_text_in_every_mode() {
    for mode in [
        Some(WorkspaceMode::Local),
        Some(WorkspaceMode::Remote),
        None,
    ] {
        let clipboard = ClipboardProbe(clipboard_probe());
        let mounted = mount_test(move |state| {
            if let Some(mode) = mode {
                state.seed_project();
                state
                    .projects
                    .projects
                    .update(|projects| projects[0].mode = mode);
            }
            state.seed_connection();
            state.seed_session();
            if mode.is_none() {
                state
                    .chat
                    .sessions
                    .update(|sessions| sessions[0].project_id = None);
            }
            chat_view(state)
        });
        settle().await;
        mounted.state.chat.apply_event(RunEvent::ToolCall {
            id: "step".into(),
            name: "shell".into(),
            summary: "test command".into(),
        });
        mounted.state.chat.apply_event(RunEvent::ToolResult {
            id: "step".into(),
            name: "shell".into(),
            ok: false,
            summary: format!(
                "\x1b[31m<script>& danger\x1b[0m\n{}last line",
                "line\n".repeat(30)
            ),
            diff: None,
        });
        settle().await;
        let output = mounted.element(".tui-tool-summary-out");
        assert!(output.query_selector(".term-red").unwrap().is_some());
        assert!(output.query_selector("script").unwrap().is_none());
        assert!(!output.text_content().unwrap().contains("last line"));
        mounted.click(".tui-output-expand");
        settle().await;
        assert!(output.text_content().unwrap().contains("last line"));
        mounted.click(".tui-output-expand");
        settle().await;
        mounted.click(".tui-output-copy");
        settle().await;
        let text = clipboard_text(&clipboard.0);
        assert!(text.starts_with("<script>& danger\n"));
        assert!(text.ends_with("last line"));
        assert!(!text.contains('\x1b'));
        assert_eq!(
            mounted
                .element(".tui-output-copy")
                .text_content()
                .as_deref(),
            Some("Copied")
        );
        clipboard_deny(&clipboard.0);
        mounted.click(".tui-output-copy");
        settle().await;
        assert_eq!(
            mounted
                .element(".tui-output-copy")
                .text_content()
                .as_deref(),
            Some("Copy failed")
        );
        clipboard_unavailable();
        mounted.click(".tui-output-copy");
        settle().await;
        assert_eq!(
            mounted
                .element(".tui-output-copy")
                .text_content()
                .as_deref(),
            Some("Copied")
        );
        assert_eq!(clipboard_text(&clipboard.0), text);
        assert!(
            web_sys::window()
                .unwrap()
                .document()
                .unwrap()
                .query_selector(".tui-copy-buffer")
                .unwrap()
                .is_none()
        );
    }
}

#[wasm_bindgen_test]
async fn tool_duration_uses_host_samples_freezes_and_restores_history_in_every_mode() {
    use openwebide_core::ToolTiming;
    use openwebide_frontend::conversation::{ConversationItem, ToolStepResult, next_item_nonce};
    for mode in [
        Some(WorkspaceMode::Local),
        Some(WorkspaceMode::Remote),
        None,
    ] {
        let mounted = mount_test(move |state| {
            if let Some(mode) = mode {
                state.seed_project();
                state
                    .projects
                    .projects
                    .update(|projects| projects[0].mode = mode);
            }
            state.seed_connection();
            state.seed_session();
            if mode.is_none() {
                state
                    .chat
                    .sessions
                    .update(|sessions| sessions[0].project_id = None);
            }
            chat_view(state)
        });
        settle().await;
        let chat = mounted.state.chat;
        chat.streaming.set(true);
        chat.streaming_session.set(Some(1));
        chat.apply_event_for_session(
            1,
            RunEvent::ToolCall {
                id: "tool".into(),
                name: "host_info".into(),
                summary: "host".into(),
            },
        );
        let started = ToolTiming::start(1000);
        chat.apply_event_for_session(
            1,
            RunEvent::ToolTiming {
                id: "tool".into(),
                timing: started.sample(6000, false),
            },
        );
        settle().await;
        let initial = mounted
            .element(".tui-tool-duration")
            .text_content()
            .unwrap();
        let seconds: f64 = initial.trim_end_matches('s').parse().unwrap();
        assert!(
            (5.0..30.0).contains(&seconds),
            "host/browser clock skew leaked into display: {initial}"
        );
        openwebide_frontend::util::sleep_ms(220).await;
        chat.apply_event_for_session(
            1,
            RunEvent::ToolTiming {
                id: "tool".into(),
                timing: started,
            },
        );
        settle().await;
        let live = mounted
            .element(".tui-tool-duration")
            .text_content()
            .unwrap();
        assert_ne!(live, initial);
        let finished = started.sample(8600, true);
        chat.apply_event_for_session(
            1,
            RunEvent::ToolResult {
                id: "tool".into(),
                name: "host_info".into(),
                ok: true,
                summary: "host info".into(),
                diff: None,
            },
        );
        chat.apply_event_for_session(
            1,
            RunEvent::ToolTiming {
                id: "tool".into(),
                timing: finished,
            },
        );
        settle().await;
        assert_eq!(
            mounted
                .element(".tui-tool-duration")
                .text_content()
                .as_deref(),
            Some("7.6s")
        );
        chat.apply_event_for_session(
            99,
            RunEvent::ToolTiming {
                id: "tool".into(),
                timing: started.sample(99999, true),
            },
        );
        chat.apply_event_for_session(
            1,
            RunEvent::ToolTiming {
                id: "tool".into(),
                timing: started,
            },
        );
        settle().await;
        openwebide_frontend::util::sleep_ms(220).await;
        assert_eq!(
            mounted
                .element(".tui-tool-duration")
                .text_content()
                .as_deref(),
            Some("7.6s")
        );
        chat.messages.set(vec![ConversationItem::ToolStep {
            timing: Some(finished),
            key: next_item_nonce(),
            id: "tool".into(),
            name: "host_info".into(),
            summary: "host".into(),
            result: Some(ToolStepResult {
                ok: true,
                summary: "host info".into(),
                diff: None,
            }),
            awaiting_permission: false,
            diff: None,
            note: None,
        }]);
        settle().await;
        assert_eq!(
            mounted
                .element(".tui-tool-duration")
                .text_content()
                .as_deref(),
            Some("7.6s")
        );
        // If only the start was saved, completed history must not present it
        // as a running timer or invent a final duration.
        chat.messages.update(|items| {
            if let ConversationItem::ToolStep { timing, .. } = &mut items[0] {
                *timing = Some(started);
            }
        });
        settle().await;
        assert!(
            mounted
                .root
                .query_selector(".tui-tool-duration")
                .unwrap()
                .is_none()
        );
        chat.streaming.set(false);
        chat.messages.update(|items| {
            if let ConversationItem::ToolStep { result, .. } = &mut items[0] {
                *result = None;
            }
        });
        settle().await;
        assert!(
            mounted
                .root
                .query_selector(".tui-tool-duration")
                .unwrap()
                .is_none()
        );
        assert_eq!(
            mounted
                .element(".tui-tool-status-badge")
                .text_content()
                .as_deref(),
            Some("[⏹ stopped]")
        );
    }
}

#[wasm_bindgen_test]
async fn turn_summary_counts_unique_tools_and_shell_changes_without_crossing_prompts_or_sessions() {
    use openwebide_core::{
        ChatMessage, FileDiff, Role, RunChange, ToolTiming, TurnTelemetry, rewind::RewindFile,
    };
    use openwebide_frontend::state::reviews::ReviewsState;
    use std::{cell::Cell, rc::Rc};
    fn message(id: i64, role: Role, elapsed: u64) -> ChatMessage {
        ChatMessage {
            id,
            session_id: 1,
            role,
            content: "text".into(),
            created_at: 0,
            tool_calls: None,
            tool_call_id: None,
            usage: (elapsed > 0).then_some(TurnTelemetry {
                eval_duration_ms: elapsed,
                ..Default::default()
            }),
        }
    }
    fn change(session: i64, prompt: i64, path: &str) -> RunChange {
        RunChange::new(
            session,
            prompt,
            RewindFile::from_bytes(
                path.into(),
                Some(b"before".to_vec()),
                Some(b"after".to_vec()),
            ),
        )
        .unwrap()
    }
    for mode in [
        Some(WorkspaceMode::Local),
        Some(WorkspaceMode::Remote),
        None,
    ] {
        let records = Rc::new(Cell::new(None));
        let capture = records.clone();
        let mounted = mount_test(move |state| {
            if let Some(mode) = mode {
                state.seed_project();
                state
                    .projects
                    .projects
                    .update(|projects| projects[0].mode = mode);
            }
            state.seed_connection();
            state.seed_session();
            if mode.is_none() {
                state
                    .chat
                    .sessions
                    .update(|sessions| sessions[0].project_id = None);
            }
            let reviews = ReviewsState::new();
            provide_context(reviews);
            capture.set(Some(reviews));
            chat_view(state)
        });
        settle().await;
        let chat = mounted.state.chat;
        chat.apply_event(RunEvent::Message {
            message: message(1, Role::User, 0),
        });
        chat.apply_event(RunEvent::Interim {
            message: message(2, Role::Assistant, 7000),
        });
        chat.apply_event(RunEvent::ToolCall {
            id: "one".into(),
            name: "host_info".into(),
            summary: "host".into(),
        });
        chat.apply_event(RunEvent::ToolTiming {
            id: "one".into(),
            timing: ToolTiming::start(1000).sample(2300, true),
        });
        let result = RunEvent::ToolResult {
            id: "one".into(),
            name: "host_info".into(),
            ok: true,
            summary: "done".into(),
            diff: mode.map(|_| FileDiff {
                path: "a".into(),
                old: Some("before".into()),
                new: "after".into(),
                old_unavailable: false,
                backup_path: None,
            }),
        };
        chat.apply_event(result.clone());
        chat.apply_event(result);
        if mode.is_some() {
            records.get().unwrap().records.set(vec![
                change(1, 1, "a"),
                change(1, 1, "shell-file"),
                change(99, 1, "foreign"),
                change(1, 99, "other-prompt"),
            ]);
        }
        chat.apply_event(RunEvent::Done {
            message: message(3, Role::Assistant, 4000),
        });
        settle().await;
        let expected = format!(
            "1 tool · {} files changed · 12.3s",
            if mode.is_some() { 2 } else { 0 }
        );
        assert_eq!(
            mounted
                .element(".tui-turn-summary[data-message-id='1']")
                .text_content()
                .unwrap(),
            expected
        );
        chat.apply_event(RunEvent::Message {
            message: message(9, Role::User, 0),
        });
        chat.apply_event(RunEvent::ToolCall {
            id: "two".into(),
            name: "host_info".into(),
            summary: "host".into(),
        });
        chat.apply_event(RunEvent::ToolTiming {
            id: "two".into(),
            timing: ToolTiming::start(3000).sample(3500, true),
        });
        chat.apply_event(RunEvent::ToolResult {
            id: "two".into(),
            name: "host_info".into(),
            ok: false,
            summary: "failed".into(),
            diff: None,
        });
        chat.apply_event(RunEvent::Done {
            message: message(10, Role::Assistant, 0),
        });
        settle().await;
        assert_eq!(
            mounted
                .element(".tui-turn-summary[data-message-id='1']")
                .text_content()
                .unwrap(),
            expected
        );
        assert_eq!(
            mounted
                .element(".tui-turn-summary[data-message-id='9']")
                .text_content()
                .as_deref(),
            Some("1 tool · 0 files changed · ≥0.5s")
        );
    }
}
