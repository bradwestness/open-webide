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

#[wasm_bindgen::prelude::wasm_bindgen(inline_js = r#"
export function attachTestImage(element, method, invalid, webp) {
    const transfer = new DataTransfer();
    let data = 'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jG1sAAAAASUVORK5CYII=';
    { const canvas=document.createElement('canvas');canvas.width=2;canvas.height=2;data=canvas.toDataURL(webp ? 'image/webp' : 'image/png').split(',')[1]; }
    const bytes=invalid ? new Uint8Array([1,2,3]) : Uint8Array.from(atob(data), c=>c.charCodeAt(0));
    transfer.items.add(new File([bytes], webp ? 'test.webp' : 'test.png', {type:webp?'image/webp':'image/png'}));
    if (method==='picker') { element.files=transfer.files;element.dispatchEvent(new Event('change',{bubbles:true})); }
    else if (method==='paste') element.dispatchEvent(new ClipboardEvent('paste',{clipboardData:transfer,bubbles:true,cancelable:true}));
    else element.dispatchEvent(new DragEvent('drop',{dataTransfer:transfer,bubbles:true,cancelable:true}));
}
"#)]
extern "C" {
    #[wasm_bindgen(js_name = attachTestImage)]
    fn attach_test_image(element: &web_sys::HtmlElement, method: &str, invalid: bool, webp: bool);
}

#[wasm_bindgen_test]
async fn images_paste_drop_pick_remove_normalize_and_send_in_projectless_chat() {
    for method in ["paste", "drop", "picker"] {
        let mounted = mount_test(|state| {
            state.seed_connection();
            chat_view(state)
        });
        settle().await;
        let selector = if method == "picker" {
            ".prompt-attachments input"
        } else if method == "drop" {
            ".tui-empty-state"
        } else {
            ".composer-input"
        };
        attach_test_image(&mounted.element(selector), method, false, method == "drop");
        for _ in 0..100 {
            sleep_ms(5).await;
            if !mounted.state.chat.reading_images.get_untracked() {
                break;
            }
        }
        settle().await;
        assert_eq!(mounted.state.chat.prompt_images.get_untracked().len(), 1);
        assert_eq!(
            mounted.state.chat.prompt_images.get_untracked()[0].mime,
            "image/png"
        );
        assert!(
            mounted
                .root
                .query_selector(".prompt-image img")
                .unwrap()
                .is_some()
        );
        mounted.click(".prompt-image button");
        settle().await;
        assert!(mounted.state.chat.prompt_images.get_untracked().is_empty());
        attach_test_image(&mounted.element(selector), method, false, false);
        for _ in 0..100 {
            sleep_ms(5).await;
            if !mounted.state.chat.reading_images.get_untracked() {
                break;
            }
        }
        mounted.key("Enter", "Enter", false);
        settle().await;
        let calls = mounted.state.fake.calls.borrow();
        let content = calls
            .iter()
            .find_map(|call| match call {
                Call::SendMessage { content, .. } => Some(content),
                _ => None,
            })
            .unwrap();
        let prompt = openwebide_core::PromptContent::parse(content).unwrap();
        assert_eq!(prompt.images.len(), 1);
        assert!(prompt.text.is_empty());
        assert!(mounted.state.chat.prompt_images.get_untracked().is_empty());
    }
}

#[wasm_bindgen_test]
async fn invalid_images_and_projectless_mentions_preserve_draft_and_do_not_send() {
    let mounted = mount_test(|state| {
        state.seed_connection();
        chat_view(state)
    });
    settle().await;
    attach_test_image(&mounted.element(".composer-input"), "paste", true, false);
    for _ in 0..100 {
        sleep_ms(5).await;
        if !mounted.state.chat.reading_images.get_untracked() {
            break;
        }
    }
    assert!(mounted.state.chat.prompt_images.get_untracked().is_empty());
    assert!(mounted.state.chat.error.get_untracked().is_some());
    mounted.input("Explain @file:secret.rs");
    mounted.key("Enter", "Enter", false);
    settle().await;
    assert_eq!(
        mounted.state.chat.draft.get_untracked(),
        "Explain @file:secret.rs"
    );
    assert!(mounted.state.fake.sessions.borrow().is_empty());
    assert!(!mounted.state.chat.streaming.get_untracked());
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

#[wasm_bindgen_test]
async fn mention_completion_selects_directory_and_file_before_send() {
    let mounted = mount_test(|state| {
        state.seed_project();
        state.seed_connection();
        chat_view(state)
    });
    let files = openwebide_frontend::workspace::Workspace::for_project(
        mounted.state.api,
        mounted.state.projects,
        1,
    )
    .unwrap();
    files.write("src/main.rs", "fn main() {}").await.unwrap();
    settle().await;
    mounted.input("@fi");
    settle().await;
    assert!(mounted.root.text_content().unwrap().contains("file"));
    mounted.key("Enter", "Enter", false);
    settle().await;
    assert_eq!(mounted.state.chat.draft.get_untracked(), "@file:");
    mounted.click(".mention-choices button");
    settle().await;
    assert_eq!(mounted.state.chat.draft.get_untracked(), "@file:src/");
    mounted.key("Tab", "Tab", false);
    settle().await;
    assert_eq!(
        mounted.state.chat.draft.get_untracked(),
        "@file:src/main.rs "
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
    mounted.input("Explain @file:src/main.rs");
    mounted.key("Enter", "Enter", false);
    settle().await;
    let calls = mounted.state.fake.calls.borrow();
    let content = calls
        .iter()
        .find_map(|call| match call {
            Call::SendMessage { content, .. } => Some(content),
            _ => None,
        })
        .unwrap();
    let prompt = openwebide_core::PromptContent::parse(content).unwrap();
    assert_eq!(prompt.references[0].content, "fn main() {}");
}

#[wasm_bindgen_test]
async fn attachment_history_renders_and_rewind_restores_text_and_images() {
    let image =
        openwebide_core::PromptImage::from_bytes("image.png".into(), b"\x89PNG\r\n\x1a\n").unwrap();
    let prompt = openwebide_core::PromptContent {
        text: "Describe this".into(),
        images: vec![image.clone()],
        ..Default::default()
    };
    let content = prompt.encode().unwrap();
    let mounted = mount_test(move |state| {
        state.seed_session();
        state
            .chat
            .sessions
            .update(|sessions| sessions[0].project_id = None);
        state.fake.messages.borrow_mut().insert(
            1,
            vec![ConversationEntry::Message(ChatMessage {
                id: 1,
                session_id: 1,
                role: Role::User,
                content,
                created_at: 0,
                tool_calls: None,
                tool_call_id: None,
                usage: None,
            })],
        );
        view! { {chat_view(state)} <openwebide_frontend::components::ConfirmDialog/> }
    });
    settle().await;
    assert!(
        mounted
            .root
            .query_selector(".prompt-history-image")
            .unwrap()
            .is_some()
    );
    assert!(
        mounted
            .root
            .text_content()
            .unwrap()
            .contains("Describe this")
    );
    assert!(
        !mounted
            .root
            .text_content()
            .unwrap()
            .contains("[Open WebIDE prompt]")
    );
    super::support::click_action(&mounted, ".tui-rewind").await;
    settle().await;
    mounted.click(".modal-footer .danger");
    settle().await;
    assert_eq!(mounted.state.chat.draft.get_untracked(), "Describe this");
    assert_eq!(
        mounted.state.chat.prompt_images.get_untracked(),
        vec![image]
    );
}

#[wasm_bindgen_test]
async fn imported_images_cannot_cross_project_session_or_account_switches() {
    for change in ["project", "session", "account"] {
        let mounted = mount_test(|state| {
            state.seed_connection();
            chat_view(state)
        });
        settle().await;
        attach_test_image(&mounted.element(".composer-input"), "paste", false, false);
        match change {
            "project" => mounted.state.projects.active_project.set(Some(2)),
            "session" => mounted.state.chat.active_session.set(Some(2)),
            _ => mounted
                .state
                .auth
                .generation
                .update(|generation| *generation += 1),
        }
        sleep_ms(20).await;
        settle().await;
        assert!(mounted.state.chat.prompt_images.get_untracked().is_empty());
        assert!(!mounted.state.chat.reading_images.get_untracked());
    }
}

#[wasm_bindgen_test]
async fn stopping_mention_preparation_restores_the_prompt_without_starting_a_turn() {
    let (response, waiting) = futures::channel::oneshot::channel();
    let mounted = mount_test(move |state| {
        state.seed_project();
        state.seed_connection();
        state.fake.file_list_results.borrow_mut().push_back(waiting);
        chat_view(state)
    });
    settle().await;
    mounted.state.chat.draft.set("Attach @folder:src".into());
    settle().await;
    mounted.click_text("Send");
    settle().await;
    assert!(mounted.state.chat.streaming.get_untracked());
    mounted.click_text("Stop");
    response.send(Ok(vec![])).unwrap();
    settle().await;
    assert_eq!(
        mounted.state.chat.draft.get_untracked(),
        "Attach @folder:src"
    );
    assert!(mounted.state.fake.sessions.borrow().is_empty());
    assert!(!mounted.state.chat.streaming.get_untracked());
    assert!(mounted.state.chat.error.get_untracked().is_none());
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

#[wasm_bindgen_test]
async fn composer_grows_shrinks_and_caps_height_in_every_workspace() {
    use super::support::wait_until;
    for mode in [
        Some(openwebide_core::WorkspaceMode::Local),
        Some(openwebide_core::WorkspaceMode::Remote),
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
            view! { <style>{include_str!("../../styles.css")}</style><div style="height:600px;display:flex;width:600px">{chat_view(state)}</div> }
        });
        settle().await;
        let pane = mounted.element(".chat-pane");
        let input: web_sys::HtmlTextAreaElement =
            mounted.element(".composer-input").unchecked_into();
        wait_until("short composer height", || input.client_height() >= 42).await;
        let short = input.get_bounding_client_rect().height();
        let send = mounted.element(".tui-btn-send");
        assert!((short - send.get_bounding_client_rect().height()).abs() < 1.0);
        mounted.input(&"more content\n".repeat(40));
        wait_until("expanded composer", || {
            input.get_bounding_client_rect().height() > short
        })
        .await;
        assert!(
            input.get_bounding_client_rect().height()
                <= pane.get_bounding_client_rect().height() * 0.8 + 1.0
        );
        mounted.state.chat.draft.set(String::new());
        wait_until("cleared composer", || {
            input.get_bounding_client_rect().height() <= short
        })
        .await;
    }
}

#[wasm_bindgen_test]
async fn compact_chat_centers_prompt_actions_and_keeps_composer_actions_inline_in_both_modes() {
    use openwebide_core::WorkspaceMode;
    use openwebide_frontend::{
        components::ToolPanel,
        state::{
            auth::AuthState,
            layout::{LayoutState, Panel},
        },
        state_actions::layout::LayoutActions,
    };
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        let mounted = mount_test(move |state| {
            state.seed_project();
            state.seed_session();
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            state.fake.messages.borrow_mut().insert(
                1,
                vec![ConversationEntry::Message(ChatMessage {
                    id: 1,
                    session_id: 1,
                    role: Role::User,
                    content: "short prompt".into(),
                    created_at: 0,
                    tool_calls: None,
                    tool_call_id: None,
                    usage: None,
                })],
            );
            let layout = expect_context::<LayoutState>();
            layout.chat_width.set(360.0);
            let auth = expect_context::<AuthState>();
            provide_context(LayoutActions::new(state.api, layout, auth, state.ui));
            let chat = chat_view(state).into_any();
            view! { <style>{include_str!("../../styles.css")}</style><div style="height:600px;display:flex;width:360px"><ToolPanel panel=Panel::Chat>{chat}</ToolPanel></div> }
        });
        settle().await;
        super::support::wait_until("prompt row", || {
            mounted.root.query_selector(".tui-user").unwrap().is_some()
        })
        .await;
        let row = mounted.element(".tui-user").get_bounding_client_rect();
        let text = mounted.element(".tui-user-text").get_bounding_client_rect();
        let menu = mounted
            .element(".tui-prompt-actions .ui-dropdown-trigger")
            .get_bounding_client_rect();
        assert!(
            ((row.top() + row.bottom()) / 2.0 - (text.top() + text.bottom()) / 2.0).abs() < 1.0
        );
        assert!(
            ((row.top() + row.bottom()) / 2.0 - (menu.top() + menu.bottom()) / 2.0).abs() < 1.0
        );
        mounted.click(".tui-prompt-actions .ui-dropdown-trigger");
        settle().await;
        let popup = mounted.element(".tui-prompt-actions .ui-dropdown-menu");
        let popup_rect = popup.get_bounding_client_rect();
        // The shared dropdown flips upward when the viewport has less room
        // below. Either placement must stay anchored to the same trigger.
        assert!(
            (popup_rect.top() - menu.bottom() - 4.0).abs() < 1.0
                || (menu.top() - popup_rect.bottom() - 4.0).abs() < 1.0,
            "popup={}..{}, trigger={}..{}",
            popup_rect.top(),
            popup_rect.bottom(),
            menu.top(),
            menu.bottom(),
        );
        let backdrop = mounted.element(".tui-prompt-actions .ui-dropdown-backdrop");
        let backdrop_rect = backdrop.get_bounding_client_rect();
        assert!(backdrop_rect.left().abs() < 1.0);
        assert!(backdrop_rect.top().abs() < 1.0);
        assert!(backdrop_rect.width() > 600.0);
        let open_row = mounted.element(".tui-user").get_bounding_client_rect();
        assert!((open_row.top() - row.top()).abs() < 1.0);
        assert!((open_row.height() - row.height()).abs() < 1.0);
        // Use actual hit testing outside the chat row, rather than invoking the
        // backdrop directly: a transformed ancestor used to trap it in the row.
        let outside = document().element_from_point(1.0, 1.0).unwrap();
        assert!(outside.is_same_node(Some(backdrop.as_ref())));
        outside.unchecked_ref::<web_sys::HtmlElement>().click();
        settle().await;
        assert!(
            mounted
                .root
                .query_selector(".ui-dropdown-menu")
                .unwrap()
                .is_none()
        );
        assert_eq!(
            mounted.element(".tui-user-text").text_content().as_deref(),
            Some("short prompt")
        );
        assert!(
            mounted
                .element(".tui-statusline")
                .get_bounding_client_rect()
                .height()
                < 40.0
        );
        assert!(
            mounted
                .element(".prompt-attachments")
                .get_bounding_client_rect()
                .height()
                < 1.0
        );
        let input = mounted.element(".prompt-attachments input");
        input.set_onclick(Some(&js_sys::Function::new_no_args(
            "this.setAttribute('data-opened', 'true')",
        )));
        mounted.click("#panel-chat .tool-panel-heading .ui-dropdown-trigger");
        settle().await;
        mounted.click("button[aria-label='Attach images']");
        settle().await;
        assert_eq!(input.get_attribute("data-opened").as_deref(), Some("true"));
        assert!(!mounted.state.chat.image_picker_requested.get_untracked());
        mounted.state.chat.streaming.set(true);
        settle().await;
        let composer_input = mounted
            .element(".composer-input")
            .get_bounding_client_rect();
        for class in [".tui-btn-queue", ".tui-btn-steer", ".tui-btn-stop"] {
            let action = mounted.element(class).get_bounding_client_rect();
            assert!((action.bottom() - composer_input.bottom()).abs() < 1.0);
            assert!(action.width() <= 44.0);
        }
        assert!(
            mounted
                .element(".tui-composer")
                .get_bounding_client_rect()
                .height()
                < 70.0
        );
    }
}
