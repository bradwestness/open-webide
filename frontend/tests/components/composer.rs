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
async fn composer_requires_a_project_and_reenables_when_one_opens() {
    let mounted = mount_test(|state| {
        state.seed_connection();
        chat_view(state)
    });
    settle().await;
    let textarea: web_sys::HtmlTextAreaElement =
        mounted.element(".composer-input").unchecked_into();
    let send: web_sys::HtmlButtonElement = mounted.element(".tui-btn-send").unchecked_into();
    assert!(textarea.disabled());
    assert!(send.disabled());
    let text = mounted.root.text_content().unwrap();
    for label in [
        "Open a project to start a session",
        "Open local",
        "Open remote",
    ] {
        assert!(text.contains(label));
    }
    mounted.input("ignored");
    assert!(mounted.state.chat.draft.get_untracked().is_empty());
    for draft in ["hello", "/help"] {
        mounted.state.chat.draft.set(draft.into());
        settle().await;
        assert!(send.disabled());
        mounted.key("Enter", "Enter", false);
        mounted.key("Enter", "Enter", true);
        mounted.click(".tui-btn-send");
        settle().await;
        assert_eq!(mounted.state.chat.draft.get_untracked(), draft);
        assert!(mounted.state.chat.messages.get_untracked().is_empty());
        assert!(mounted.state.chat.prompt_history.get_untracked().is_empty());
    }
    assert!(
        !mounted
            .state
            .fake
            .calls
            .borrow()
            .iter()
            .any(|call| matches!(call, Call::SendMessage { .. }))
    );
    mounted.state.seed_project();
    mounted.state.chat.draft.set("hello".into());
    settle().await;
    assert!(!textarea.disabled());
    assert!(!send.disabled());
    assert!(mounted.root.query_selector(".chat-hint").unwrap().is_none());
    mounted.key("Enter", "Enter", false);
    settle().await;
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
        assert!(
            mounted
                .root
                .text_content()
                .unwrap()
                .contains("Open a project to start a session")
        );
        assert!(mounted.element(".composer-input").has_attribute("disabled"));
        assert!(mounted.element(".tui-btn-send").has_attribute("disabled"));
        mounted.state.chat.draft.set("hello".into());
        mounted.key("Enter", "Enter", false);
        settle().await;
        assert!(
            !mounted
                .state
                .fake
                .calls
                .borrow()
                .iter()
                .any(|call| matches!(call, Call::SendMessage { .. }))
        );
    }
}

#[wasm_bindgen_test]
async fn project_open_buttons_call_their_callbacks_and_alt_shortcuts_are_blocked() {
    let local = RwSignal::new(0);
    let remote = RwSignal::new(0);
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
                on_open_local=Callback::new(move |()| local.update(|n| *n += 1))
                on_open_remote=Callback::new(move |()| remote.update(|n| *n += 1))
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
    assert_eq!(permissions.get_untracked(), 0);
    mounted.click_text("Open local");
    mounted.click_text("Open remote");
    assert_eq!(local.get_untracked(), 1);
    assert_eq!(remote.get_untracked(), 1);
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
        let draft = state.chat.draft.get_untracked();
        let history = state.chat.prompt_history.get_untracked();
        for (blocked_key, blocked_code) in [
            ("x", "KeyX"),
            ("Enter", "Enter"),
            ("ArrowUp", "ArrowUp"),
            ("ArrowDown", "ArrowDown"),
        ] {
            let init = web_sys::KeyboardEventInit::new();
            init.set_key(blocked_key);
            init.set_code(blocked_code);
            init.set_bubbles(true);
            init.set_cancelable(true);
            let event = web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init)
                .unwrap();
            textarea.dispatch_event(&event).unwrap();
            assert!(event.default_prevented());
        }
        for input in ["ignored", "/help"] {
            mounted.input(input);
            mounted.key("Enter", "Enter", false);
            settle().await;
            assert_eq!(state.chat.draft.get_untracked(), draft);
        }
        assert_eq!(state.chat.prompt_history.get_untracked(), history);
        assert_eq!(
            fake.sent()
                .iter()
                .filter(|message| matches!(message, BridgeClientMessage::RunStart { .. }))
                .count(),
            1
        );
        assert!(
            !state
                .fake
                .calls
                .borrow()
                .iter()
                .any(|call| matches!(call, Call::SendMessage { .. }))
        );
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
        assert!(textarea.disabled());
        state.bridge.get_untracked().unwrap().close();
    }
}
