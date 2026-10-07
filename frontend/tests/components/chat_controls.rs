use super::support::{Mounted, chat_view, mount_test, settle, wait_until};
use leptos::prelude::*;
use openwebide_core::{
    ChatCompletion, ChatMessage, ChatResponse, GoalStatus, Role, RunEvent, StopReason, ToolCall,
    ToolTiming, WorkspaceMode,
};
use openwebide_frontend::conversation::{ConversationItem, ToolStepResult};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

fn fixture(mode: WorkspaceMode) -> Mounted {
    mount_test(move |state| {
        state.seed_project();
        state.seed_connection();
        state.seed_session();
        state
            .projects
            .projects
            .update(|projects| projects[0].mode = mode);
        if mode == WorkspaceMode::Local {
            state.projects.local_handles.update(|handles| {
                handles.insert(1, super::local_bridge::probe_folder().unchecked_into());
            });
        }
        view! { <style>{include_str!("../../styles.css")}</style> {chat_view(state)} }
    })
}
fn reply(mounted: &Mounted, mode: WorkspaceMode) {
    if mode == WorkspaceMode::Local {
        mounted
            .state
            .fake
            .scripted_completions
            .borrow_mut()
            .push_back(ChatCompletion {
                response: ChatResponse::Text("Verified work; ready for review".into()),
                preamble: String::new(),
                reasoning: String::new(),
                usage: None,
                stop_reason: StopReason::Complete,
            });
    } else {
        mounted
            .state
            .fake
            .scripted_events
            .borrow_mut()
            .push_back(vec![RunEvent::Delta {
                content: "Verified work; ready for review".into(),
            }]);
    }
}
async fn idle(mounted: &Mounted) {
    wait_until("idle chat controls", || {
        !mounted.state.chat.streaming.get_untracked()
            && !mounted.state.chat.compacting.get_untracked()
            && !mounted.state.chat.goal_busy.get_untracked()
    })
    .await;
    settle().await;
}

#[wasm_bindgen_test]
async fn chat_controls_commands_complete_search_and_preserve_drafts_in_both_modes() {
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        let mounted = fixture(mode);
        settle().await;
        mounted.input("/comp");
        settle().await;
        assert_eq!(
            mounted
                .element("[role='listbox']")
                .query_selector_all("[role='option']")
                .unwrap()
                .length(),
            1
        );
        mounted.key("Tab", "Tab", false);
        settle().await;
        assert_eq!(mounted.state.chat.draft.get_untracked(), "/compact");
        assert!(
            mounted
                .state
                .fake
                .calls
                .borrow()
                .iter()
                .all(|call| !matches!(
                    call,
                    openwebide_frontend::testing::fake_backend::Call::Request {
                        method: "compact_session"
                    }
                ))
        );
        mounted.input("/help goal");
        mounted.key("Enter", "Enter", false);
        settle().await;
        let text = mounted.root.text_content().unwrap();
        assert!(text.contains("Start and manage a saved objective"));
        assert!(!text.contains("List or switch models"));
        mounted.input("/");
        settle().await;
        mounted.key("ArrowDown", "ArrowDown", false);
        settle().await;
        assert_eq!(
            mounted
                .element(".composer-input")
                .get_attribute("aria-activedescendant")
                .as_deref(),
            Some("slash-option-1")
        );
        mounted.key("Escape", "Escape", false);
        settle().await;
        assert_eq!(mounted.state.chat.draft.get_untracked(), "/");
        assert!(
            mounted
                .root
                .query_selector("[role='listbox']")
                .unwrap()
                .is_none()
        );
        mounted.input("/goal ");
        settle().await;
        assert!(
            mounted
                .element(".slash-hint")
                .text_content()
                .unwrap()
                .contains("objective")
        );
    }
}

#[wasm_bindgen_test]
async fn chat_controls_goals_start_pause_continue_complete_and_reload_in_both_modes() {
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        let mounted = fixture(mode);
        settle().await;
        reply(&mounted, mode);
        mounted.input("/goal Fix the failing test");
        mounted.key("Enter", "Enter", false);
        settle().await;
        idle(&mounted).await;
        assert!(
            mounted.state.chat.error.get_untracked().is_none(),
            "{:?}",
            mounted.state.chat.error.get_untracked()
        );
        let goal = mounted.state.chat.goal.get_untracked().expect("saved goal");
        assert_eq!(goal.status, GoalStatus::Active);
        assert_eq!(goal.objective, "Fix the failing test");
        assert!(
            mounted
                .element(".tui-goal")
                .text_content()
                .unwrap()
                .contains("review or continue")
        );
        mounted.click_text("Pause");
        settle().await;
        idle(&mounted).await;
        assert_eq!(
            mounted.state.fake.goals.borrow()[&1].status,
            GoalStatus::Paused
        );
        mounted.state.chat.draft.set("Keep my draft".into());
        reply(&mounted, mode);
        mounted.click_text("Continue");
        settle().await;
        idle(&mounted).await;
        assert_eq!(mounted.state.chat.draft.get_untracked(), "Keep my draft");
        assert_eq!(
            mounted.state.fake.goals.borrow()[&1].status,
            GoalStatus::Active
        );
        mounted.click_text("Mark complete");
        settle().await;
        idle(&mounted).await;
        assert_eq!(
            mounted.state.fake.goals.borrow()[&1].status,
            GoalStatus::Completed
        );
        mounted.state.chat.active_session.set(None);
        settle().await;
        mounted.state.chat.active_session.set(Some(1));
        settle().await;
        assert_eq!(
            mounted.state.chat.goal.get_untracked().unwrap().status,
            GoalStatus::Completed
        );
    }
}

#[wasm_bindgen_test]
async fn chat_controls_compact_failures_and_stale_results_preserve_other_sessions_in_both_modes() {
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        let mounted = fixture(mode);
        settle().await;
        let (done, pending) = futures::channel::oneshot::channel();
        mounted
            .state
            .fake
            .compact_results
            .borrow_mut()
            .push_back(pending);
        mounted.input("/compact");
        mounted.key("Enter", "Enter", false);
        settle().await;
        assert!(mounted.state.chat.compacting.get_untracked());
        mounted.input("Do not send during compaction");
        mounted.key("Enter", "Enter", false);
        settle().await;
        assert!(!mounted.state.chat.streaming.get_untracked());
        assert_eq!(
            mounted.state.chat.draft.get_untracked(),
            "Do not send during compaction"
        );
        mounted.state.chat.active_session.set(Some(2));
        settle().await;
        done.send(Ok(ChatMessage {
            id: 999,
            session_id: 1,
            role: Role::System,
            content: "stale summary".into(),
            created_at: 1,
            tool_calls: None,
            tool_call_id: None,
            usage: None,
        }))
        .unwrap();
        settle().await;
        assert!(
            !mounted
                .root
                .text_content()
                .unwrap()
                .contains("stale summary")
        );
        mounted.state.chat.active_session.set(Some(1));
        settle().await;
        *mounted.state.fake.compact_error.borrow_mut() = Some("model unavailable".into());
        mounted.input("/compact");
        mounted.key("Enter", "Enter", false);
        settle().await;
        assert!(!mounted.state.chat.compacting.get_untracked());
        assert!(
            mounted
                .root
                .text_content()
                .unwrap()
                .contains("model unavailable")
        );
        *mounted.state.fake.compact_error.borrow_mut() = None;
        mounted.input("/compact");
        mounted.key("Enter", "Enter", false);
        settle().await;
        assert!(
            mounted
                .root
                .text_content()
                .unwrap()
                .contains("Original messages are retained")
        );
    }
}

#[wasm_bindgen_test]
async fn chat_controls_activity_groups_reasoning_and_tools_but_keeps_approvals_and_failures_visible()
 {
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        let mounted = fixture(mode);
        settle().await;
        mounted
            .state
            .chat
            .messages
            .push(ConversationItem::Message(ChatMessage {
                id: 1,
                session_id: 1,
                role: Role::Assistant,
                content: "<think>Check the file</think>".into(),
                created_at: 0,
                tool_calls: Some(vec![ToolCall {
                    id: "call".into(),
                    name: "read_file".into(),
                    arguments: "{}".into(),
                }]),
                tool_call_id: None,
                usage: None,
            }));
        mounted
            .state
            .chat
            .messages
            .push(ConversationItem::ToolStep {
                key: 1,
                id: "call".into(),
                name: "read_file".into(),
                summary: "main.rs".into(),
                result: Some(ToolStepResult {
                    ok: true,
                    summary: "file".into(),
                    diff: None,
                }),
                diff: None,
                note: None,
                awaiting_permission: false,
                timing: Some(ToolTiming {
                    started_at_ms: 1,
                    elapsed_ms: 1234,
                    finished: true,
                }),
            });
        settle().await;
        let group = mounted.element(".tui-tool-group");
        assert!(group.text_content().unwrap().contains("2 steps"));
        assert!(group.text_content().unwrap().contains("1.2s"));
        assert_eq!(
            group
                .query_selector(".ui-disclosure-toggle")
                .unwrap()
                .unwrap()
                .get_attribute("aria-expanded")
                .as_deref(),
            Some("false")
        );
        mounted
            .state
            .chat
            .messages
            .push(ConversationItem::ToolStep {
                key: 2,
                id: "failed".into(),
                name: "read_file".into(),
                summary: "missing.rs".into(),
                result: Some(ToolStepResult {
                    ok: false,
                    summary: "File missing".into(),
                    diff: None,
                }),
                diff: None,
                note: None,
                awaiting_permission: false,
                timing: None,
            });
        settle().await;
        assert_eq!(
            group
                .query_selector(".ui-disclosure-toggle")
                .unwrap()
                .unwrap()
                .get_attribute("aria-expanded")
                .as_deref(),
            Some("true")
        );
        assert!(
            !group
                .query_selector(".ui-disclosure-content")
                .unwrap()
                .unwrap()
                .has_attribute("hidden")
        );
    }
}

#[wasm_bindgen_test]
async fn chat_controls_delayed_goal_loads_cannot_cross_account_project_or_session_changes() {
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        for change in ["account", "project", "session"] {
            let mounted = fixture(mode);
            settle().await;
            let (done, pending) = futures::channel::oneshot::channel();
            mounted
                .state
                .fake
                .goal_load_results
                .borrow_mut()
                .push_back(pending);
            mounted.input("/goal status");
            mounted.key("Enter", "Enter", false);
            settle().await;
            match change {
                "account" => mounted
                    .state
                    .auth
                    .generation
                    .update(|generation| *generation += 1),
                "project" => mounted.state.projects.active_project.set(Some(2)),
                _ => mounted.state.chat.active_session.set(Some(2)),
            }
            settle().await;
            done.send(Ok(Some(openwebide_core::Goal {
                session_id: 1,
                objective: "stale objective".into(),
                status: GoalStatus::Active,
                revision: 42,
                updated_at: 1,
            })))
            .unwrap();
            settle().await;
            assert!(
                !mounted
                    .root
                    .text_content()
                    .unwrap()
                    .contains("stale objective")
            );
        }
    }
}

#[wasm_bindgen_test]
async fn chat_controls_starting_a_goal_creates_a_session_when_needed() {
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        let mounted = fixture(mode);
        settle().await;
        mounted.state.chat.active_session.set(None);
        settle().await;
        mounted.state.fake.sessions.borrow_mut().clear();
        mounted.state.chat.sessions.set(Vec::new());
        reply(&mounted, mode);
        mounted.input("/goal Fix startup");
        mounted.key("Enter", "Enter", false);
        settle().await;
        idle(&mounted).await;
        let session = mounted
            .state
            .chat
            .active_session
            .get_untracked()
            .expect("created session");
        assert_eq!(
            mounted.state.fake.goals.borrow()[&session].objective,
            "Fix startup"
        );
        assert_eq!(
            mounted.state.chat.goal.get_untracked().unwrap().objective,
            "Fix startup"
        );
    }
}

#[wasm_bindgen_test]
async fn chat_controls_stop_cancels_compaction_and_ignores_its_late_result() {
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        let mounted = fixture(mode);
        settle().await;
        let (done, pending) = futures::channel::oneshot::channel();
        mounted
            .state
            .fake
            .compact_results
            .borrow_mut()
            .push_back(pending);
        mounted.input("/compact");
        mounted.key("Enter", "Enter", false);
        settle().await;
        mounted.click_text("Stop");
        settle().await;
        assert!(!mounted.state.chat.compacting.get_untracked());
        assert!(
            mounted
                .state
                .fake
                .calls
                .borrow()
                .iter()
                .any(|call| matches!(
                    call,
                    openwebide_frontend::testing::fake_backend::Call::CancelSession { session: 1 }
                ))
        );
        done.send(Ok(ChatMessage {
            id: 999,
            session_id: 1,
            role: Role::System,
            content: "cancelled summary".into(),
            created_at: 1,
            tool_calls: None,
            tool_call_id: None,
            usage: None,
        }))
        .unwrap();
        settle().await;
        assert!(
            !mounted
                .root
                .text_content()
                .unwrap()
                .contains("cancelled summary")
        );
    }
}

#[wasm_bindgen_test]
async fn chat_controls_tool_spinners_follow_execution_and_permission_states() {
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        let mounted = fixture(mode);
        settle().await;
        let chat = mounted.state.chat;
        chat.streaming.set(true);
        chat.streaming_session.set(Some(1));
        chat.current_run_anchor.set(Some(1));
        chat.apply_event(RunEvent::PermissionRequest {
            id: "a1t1c0".into(),
            name: "write_file".into(),
            summary: "main.rs".into(),
            diff: None,
            note: None,
        });
        settle().await;
        let group = mounted.element(".tui-tool-group");
        assert!(group.query_selector(".tui-spinner").unwrap().is_none());
        assert_eq!(
            group
                .query_selector(".ui-disclosure-toggle")
                .unwrap()
                .unwrap()
                .get_attribute("aria-expanded")
                .as_deref(),
            Some("true")
        );
        chat.apply_event(RunEvent::ToolCall {
            id: "a1t1c0".into(),
            name: "write_file".into(),
            summary: "main.rs".into(),
        });
        settle().await;
        assert!(
            group
                .query_selector(".ui-disclosure-toggle .tui-spinner")
                .unwrap()
                .is_some()
        );
        assert!(
            group
                .query_selector(".tui-tool-topbar .tui-spinner")
                .unwrap()
                .is_some()
        );
        chat.apply_event(RunEvent::ToolResult {
            id: "a1t1c0".into(),
            name: "write_file".into(),
            ok: true,
            summary: "written".into(),
            diff: None,
        });
        settle().await;
        assert!(group.query_selector(".tui-spinner").unwrap().is_none());
        chat.apply_event(RunEvent::ToolCall {
            id: "a1t1c1".into(),
            name: "read_file".into(),
            summary: "main.rs".into(),
        });
        settle().await;
        assert!(
            group
                .query_selector(".ui-disclosure-toggle .tui-spinner")
                .unwrap()
                .is_some()
        );
        chat.apply_event(RunEvent::Cancelled);
        settle().await;
        assert!(group.query_selector(".tui-spinner").unwrap().is_none());
    }
}
