use std::rc::Rc;

use leptos::prelude::*;
use openwebide_core::{
    BridgeClientMessage, BridgeServerMessage, ChatMessage, ConversationEntry, Role, RunEvent,
};
use openwebide_frontend::{
    bridge::{BridgeConfig, BridgeConn},
    components::ChatPane,
    state_actions::projects::{ProjectsActionContext, build_projects_actions},
    testing::{fake_backend::Call, fake_transport::FakeTransport},
    util::sleep_ms,
};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

use super::support::{chat_view, mount_test, settle};

#[wasm_bindgen_test]
async fn composer_creates_a_projectless_session_and_sends() {
    let mounted = mount_test(|state| {
        state.seed_connection();
        chat_view(state)
    });
    settle().await;
    let textarea: web_sys::HtmlTextAreaElement =
        mounted.element(".composer-input").unchecked_into();
    assert!(!textarea.disabled());
    assert!(mounted.root.query_selector(".chat-hint").unwrap().is_none());
    mounted.input("hello");
    mounted.key("Enter", "Enter", false);
    settle().await;
    let sessions = mounted.state.fake.sessions.borrow();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].project_id, None);
    assert_eq!(sessions[0].name, "hello");
    assert_eq!(
        mounted
            .state
            .chat
            .approval_mode
            .get_untracked()
            .get(&sessions[0].id),
        Some(&openwebide_core::ApprovalMode::Auto)
    );
    assert_eq!(
        mounted
            .state
            .fake
            .settings
            .borrow()
            .get(&openwebide_core::ApprovalMode::setting_key(sessions[0].id))
            .map(String::as_str),
        Some("\"auto\"")
    );
    assert!(
        mounted
            .state
            .fake
            .calls
            .borrow()
            .iter()
            .any(|call| matches!(call, Call::SendMessage { .. }))
    );
}

#[wasm_bindgen_test]
async fn selected_sessions_remain_viewable_without_a_project() {
    for project_id in [Some(1), None] {
        let mounted = mount_test(move |state| {
            state.seed_session();
            state
                .chat
                .sessions
                .update(|sessions| sessions[0].project_id = project_id);
            state.fake.messages.borrow_mut().insert(
                1,
                vec![ConversationEntry::Message(ChatMessage {
                    id: 1,
                    session_id: 1,
                    role: Role::Assistant,
                    content: "Saved answer".into(),
                    tool_calls: None,
                    tool_call_id: None,
                    usage: None,
                    created_at: 0,
                })],
            );
            chat_view(state)
        });
        settle().await;
        assert!(
            mounted
                .root
                .text_content()
                .unwrap()
                .contains("Saved answer")
        );

        assert!(!mounted.element(".composer-input").has_attribute("disabled"));
        assert!(mounted.element(".tui-btn-send").has_attribute("disabled"));
    }
}

#[wasm_bindgen_test]
async fn projectless_chat_supports_permission_shortcuts() {
    let permissions = RwSignal::new(0);
    let mounted = mount_test(move |state| {
        state.chat.current_run_anchor.set(Some(1));
        state
            .chat
            .apply_event(openwebide_core::RunEvent::PermissionRequest {
                id: "a1t1c0".into(),
                name: "write_file".into(),
                summary: "write".into(),
                diff: None,
                note: None,
            });
        view! {
            <ChatPane
                on_select_connection_model=Callback::new(|_| ())
                on_send=Callback::new(|()| panic!("sent without a project"))
                on_resume_run=Callback::new(|()| ())
                on_stop=Callback::new(|()| ())
                on_permission=Callback::new(move |_| permissions.update(|n| *n += 1))
                on_permission_always=Callback::new(move |_| permissions.update(|n| *n += 1))
                on_slash_command=Callback::new(|_| panic!("command without a project"))
            />
        }
    });
    settle().await;
    assert!(
        mounted
            .root
            .query_selector(".tui-permission-prompt")
            .unwrap()
            .is_some()
    );
    for (key, code) in [("y", "KeyY"), ("n", "KeyN"), ("a", "KeyA")] {
        mounted.key(key, code, true);
    }
    assert_eq!(permissions.get_untracked(), 3);
}

#[wasm_bindgen_test]
async fn cancel_shortcuts_work_after_closing_the_last_project() {
    for (key, code, ctrl, meta) in [
        ("Escape", "Escape", false, false),
        ("c", "KeyC", true, false),
        ("c", "KeyC", false, true),
    ] {
        let fake = Rc::new(FakeTransport::default());
        let transport = fake.clone();
        let close_project = RwSignal::new(None);
        let mounted = mount_test(move |state| {
            state.seed_project();
            state.projects.open_tab(1);
            state.seed_connection();
            state.seed_session();
            state.bridge.set(Some(BridgeConn::with_transport(
                BridgeConfig::new("ws://test"),
                transport,
                Rc::new(|| Box::pin(async { Ok("token".into()) })),
            )));
            close_project.set(Some(
                build_projects_actions(ProjectsActionContext {
                    api: state.api,
                    projects: state.projects,
                    workspace: state.workspace,
                    git: state.git,
                    chat: state.chat,
                    ui: state.ui,
                    ensure_root: Callback::new(|_| ()),
                    refresh_git: Callback::new(|()| ()),
                })
                .close_project,
            ));
            chat_view(state)
        });
        settle().await;
        fake.reply(BridgeServerMessage::HelloOk {
            user_id: Some(1),
            protocol: 1,
            runs: true,
        });
        sleep_ms(25).await;
        settle().await;
        fake.reply(BridgeServerMessage::Runs {
            session_id: 1,
            runs: vec![],
        });
        mounted.input("hello");
        mounted.key("Enter", "Enter", false);
        settle().await;
        let run_id = fake
            .sent()
            .into_iter()
            .find_map(|message| match message {
                BridgeClientMessage::RunStart { run_id, .. } => Some(run_id),
                _ => None,
            })
            .unwrap();
        fake.reply(BridgeServerMessage::RunEvent {
            run_id: run_id.clone(),
            seq: 1,
            event: RunEvent::Delta {
                content: "reply".into(),
            },
        });
        settle().await;
        assert!(mounted.state.chat.streaming.get_untracked());
        let state = &mounted.state;
        close_project.get_untracked().unwrap().run(1);
        settle().await;
        assert!(state.projects.active_project.get_untracked().is_none());
        assert!(state.chat.streaming.get_untracked());
        let textarea: web_sys::HtmlTextAreaElement =
            mounted.element(".composer-input").unchecked_into();
        assert!(!textarea.disabled());
        textarea.set_selection_start(Some(0)).unwrap();
        textarea.set_selection_end(Some(0)).unwrap();
        let init = web_sys::KeyboardEventInit::new();
        init.set_key(key);
        init.set_code(code);
        init.set_ctrl_key(ctrl);
        init.set_meta_key(meta);
        init.set_bubbles(true);
        init.set_cancelable(true);
        textarea
            .dispatch_event(
                &web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init)
                    .unwrap(),
            )
            .unwrap();
        settle().await;
        assert!(fake.sent().contains(&BridgeClientMessage::RunCancel {
            run_id: run_id.clone(),
        }));
        fake.reply(BridgeServerMessage::RunEvent {
            run_id,
            seq: 2,
            event: RunEvent::Cancelled,
        });
        settle().await;
        assert!(!state.chat.streaming.get_untracked());
        assert!(!textarea.disabled());
        state.bridge.get_untracked().unwrap().close();
    }
}

#[wasm_bindgen_test]
async fn new_projectless_session_preserves_an_explicit_manual_choice() {
    let mounted = mount_test(|state| {
        state.seed_connection();
        state
            .chat
            .draft_approval_mode
            .set(openwebide_core::ApprovalMode::Default);
        chat_view(state)
    });
    settle().await;
    assert_eq!(
        mounted.element(".tui-mode-badge").text_content().as_deref(),
        Some("[MANUAL]")
    );
    mounted.input("hello");
    mounted.key("Enter", "Enter", false);
    settle().await;
    let session = mounted.state.fake.sessions.borrow()[0].id;
    assert_eq!(
        mounted
            .state
            .fake
            .settings
            .borrow()
            .get(&openwebide_core::ApprovalMode::setting_key(session))
            .map(String::as_str),
        Some("\"default\"")
    );
}

#[wasm_bindgen_test]
async fn context_command_shows_saved_breakdown_in_both_modes_and_projectless_chat() {
    use openwebide_core::{ContextBreakdown, TurnTelemetry, WorkspaceMode};
    for mode in [
        Some(WorkspaceMode::Local),
        Some(WorkspaceMode::Remote),
        None,
    ] {
        let mounted = mount_test(move |state| {
            state.seed_connection();
            state.seed_session();
            if let Some(mode) = mode {
                state.seed_project();
                state
                    .projects
                    .projects
                    .update(|projects| projects[0].mode = mode);
            } else {
                state
                    .chat
                    .sessions
                    .update(|sessions| sessions[0].project_id = None);
            }
            state.fake.messages.borrow_mut().insert(
                1,
                vec![ConversationEntry::Message(ChatMessage {
                    id: 2,
                    session_id: 1,
                    role: Role::Assistant,
                    content: "Saved reply".into(),
                    created_at: 0,
                    tool_calls: None,
                    tool_call_id: None,
                    usage: Some(TurnTelemetry {
                        context: Some(ContextBreakdown {
                            system: 100,
                            files: 50,
                            tool_output: 25,
                            history: 200,
                            tools: 125,
                        }),
                        prompt_tokens: 500,
                        completion_tokens: 100,
                        ..Default::default()
                    }),
                })],
            );
            chat_view(state)
        });
        settle().await;
        mounted.input("/context");
        mounted.key("Enter", "Enter", false);
        settle().await;
        let modal = mounted.element(".context-usage");
        let text = modal.text_content().unwrap();
        for label in [
            "System instructions",
            "Files",
            "Tool output",
            "History",
            "Tool schemas",
            "Generated reply",
            "~200",
            "600",
        ] {
            assert!(text.contains(label), "{text}");
        }
        assert!(
            modal
                .query_selector(".context-breakdown-bar")
                .unwrap()
                .is_some()
        );
        assert!(
            !mounted
                .state
                .fake
                .calls
                .borrow()
                .iter()
                .any(|call| matches!(call, Call::SendMessage { .. }))
        );
        if mode.is_some() {
            mounted.state.projects.active_project.set(None);
            settle().await;
            assert!(
                mounted
                    .root
                    .query_selector(".context-usage")
                    .unwrap()
                    .is_none()
            );
            mounted.input("/context");
            mounted.key("Enter", "Enter", false);
            settle().await;
        }
        mounted.state.auth.generation.update(|value| *value += 1);
        settle().await;
        assert!(
            mounted
                .root
                .query_selector(".context-usage")
                .unwrap()
                .is_none()
        );
    }
}
