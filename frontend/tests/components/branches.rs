use super::support::{Mounted, chat_view, mount_test, settle};
use leptos::prelude::*;
use openwebide_core::{
    ApprovalMode, ChatCompletion, ChatMessage, ChatResponse, ConversationEntry, PromptContent,
    PromptImage, Role, StopReason, WorkspaceMode,
};
use openwebide_frontend::{testing::fake_backend::Call, util::sleep_ms};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

fn message(id: i64, role: Role, content: String) -> ConversationEntry {
    ConversationEntry::Message(ChatMessage {
        id,
        session_id: 1,
        role,
        content,
        created_at: 0,
        tool_calls: None,
        tool_call_id: None,
        usage: None,
    })
}
fn original_prompt() -> String {
    PromptContent {
        text: "<active_editor_context>\nFile: a.txt\n</active_editor_context>\n\noriginal prompt"
            .into(),
        images: vec![PromptImage::from_bytes("image.png".into(), b"\x89PNG\r\n\x1a\n").unwrap()],
        ..Default::default()
    }
    .encode()
    .unwrap()
}
fn fixture(mode: Option<WorkspaceMode>) -> Mounted {
    mount_test(move |state| {
        if let Some(mode) = mode {
            state.seed_project();
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            if mode == WorkspaceMode::Local {
                state.projects.local_handles.update(|handles| {
                    handles.insert(1, super::local_bridge::probe_folder().unchecked_into());
                });
            }
        }
        state.seed_connection();
        state.seed_session();
        state.fake.connections.borrow_mut()[0].context_limit = Some(32768);
        state
            .settings
            .connections
            .update(|connections| connections[0].context_limit = Some(32768));
        if mode.is_none() {
            state
                .chat
                .sessions
                .update(|sessions| sessions[0].project_id = None);
            state.fake.sessions.borrow_mut()[0].project_id = None;
        }
        state.chat.set_approval_mode(1, ApprovalMode::Yolo);
        state
            .fake
            .settings
            .borrow_mut()
            .insert(ApprovalMode::setting_key(1), "\"yolo\"".into());
        state.chat.session_model.update(|models| {
            models.insert(1, Some("chosen".into()));
        });
        state.fake.messages.borrow_mut().insert(
            1,
            vec![
                message(1, Role::User, "earlier".into()),
                message(2, Role::Assistant, "earlier answer".into()),
                message(3, Role::User, original_prompt()),
                message(4, Role::Assistant, "original answer".into()),
            ],
        );
        state
            .fake
            .scripted_completions
            .borrow_mut()
            .push_back(ChatCompletion {
                response: ChatResponse::Text("new reply".into()),
                preamble: String::new(),
                reasoning: String::new(),
                stop_reason: StopReason::Complete,
                usage: None,
            });
        chat_view(state)
    })
}
async fn idle(mounted: &Mounted) {
    for _ in 0..400 {
        sleep_ms(5).await;
        settle().await;
        if !mounted.state.chat.streaming.get_untracked()
            && !mounted.state.chat.branching.get_untracked()
        {
            return;
        }
    }
    panic!(
        "branch did not settle: {:?}",
        mounted.state.chat.error.get_untracked()
    );
}

#[wasm_bindgen_test]
async fn edit_and_resend_branches_the_prefix_preserving_source_images_editor_context_and_model_in_all_modes()
 {
    for mode in [
        Some(WorkspaceMode::Local),
        Some(WorkspaceMode::Remote),
        None,
    ] {
        let mounted = fixture(mode);
        settle().await;
        let original = mounted.state.fake.messages.borrow()[&1].clone();
        mounted.click(".tui-edit-prompt[data-message-id='3']");
        settle().await;
        assert_eq!(mounted.state.chat.draft.get_untracked(), "original prompt");
        assert_eq!(mounted.state.chat.prompt_images.get_untracked().len(), 1);
        assert_eq!(mounted.state.chat.prompt_edit.get_untracked(), Some((1, 3)));
        assert_eq!(mounted.state.fake.sessions.borrow().len(), 1);
        mounted.input("edited prompt");
        mounted.key("Enter", "Enter", false);
        idle(&mounted).await;
        assert_eq!(mounted.state.fake.sessions.borrow().len(), 2);
        let branch = mounted.state.chat.active_session.get_untracked().unwrap();
        assert_ne!(branch, 1);
        assert!(mounted.state.chat.prompt_edit.get_untracked().is_none());
        assert_eq!(mounted.state.fake.messages.borrow()[&1], original);
        let copied = mounted.state.fake.messages.borrow()[&branch].clone();
        let user = copied
            .iter()
            .filter_map(|entry| match entry {
                ConversationEntry::Message(message) if message.role == Role::User => Some(message),
                _ => None,
            })
            .next_back()
            .unwrap();
        let prompt = PromptContent::decode(&user.content);
        assert_eq!(prompt.images.len(), 1);
        // Projectless planning excludes editor injection, just as normal sends do.
        if mode.is_some() {
            assert!(prompt.text.starts_with("<active_editor_context>"));
        }
        assert!(prompt.text.ends_with("edited prompt"));
        assert!(copied.iter().any(|entry| matches!(entry, ConversationEntry::Message(message) if message.content == "earlier answer")));
        assert!(copied.iter().all(|entry| !matches!(entry, ConversationEntry::Message(message) if message.content == "original answer")));
        assert_eq!(
            mounted.state.chat.approval_mode.get_untracked()[&branch],
            ApprovalMode::Yolo
        );
        assert_eq!(
            mounted.state.chat.session_model.get_untracked()[&branch].as_deref(),
            Some("chosen")
        );
        assert!(
            mounted
                .state
                .fake
                .calls
                .borrow()
                .iter()
                .all(|call| !matches!(call, Call::WriteFile { .. }))
        );
        assert!(
            mounted.state.chat.error.get_untracked().is_none(),
            "{:?}",
            mounted.state.chat.error.get_untracked()
        );
    }
}

#[wasm_bindgen_test]
async fn fork_restores_the_selected_draft_and_images_without_sending_or_changing_the_source() {
    for mode in [
        Some(WorkspaceMode::Local),
        Some(WorkspaceMode::Remote),
        None,
    ] {
        let mounted = fixture(mode);
        settle().await;
        let original = mounted.state.fake.messages.borrow()[&1].clone();
        mounted.click(".tui-fork-prompt[data-message-id='3']");
        idle(&mounted).await;
        let branch = mounted.state.chat.active_session.get_untracked().unwrap();
        assert_ne!(branch, 1);
        assert_eq!(mounted.state.chat.draft.get_untracked(), "original prompt");
        assert_eq!(mounted.state.chat.prompt_images.get_untracked().len(), 1);
        assert_eq!(mounted.state.fake.messages.borrow()[&1], original);
        assert_eq!(mounted.state.fake.messages.borrow()[&branch].len(), 2);
        assert!(
            mounted
                .state
                .fake
                .calls
                .borrow()
                .iter()
                .all(|call| !matches!(call, Call::SendMessage { .. } | Call::WriteFile { .. }))
        );
        assert!(mounted.state.fake.completion_requests.borrow().is_empty());
    }
}

#[wasm_bindgen_test]
async fn cancelling_an_edit_keeps_the_source_and_sending_failure_restores_the_edited_draft() {
    let mounted = fixture(None);
    settle().await;
    mounted.click(".tui-edit-prompt[data-message-id='3']");
    settle().await;
    mounted.click(".tui-prompt-edit .btn");
    settle().await;
    assert!(mounted.state.chat.prompt_edit.get_untracked().is_none());
    assert_eq!(mounted.state.fake.sessions.borrow().len(), 1);
    mounted.state.chat.draft.set(String::new());
    mounted.state.chat.prompt_images.set(Vec::new());
    settle().await;
    mounted.click(".tui-edit-prompt[data-message-id='3']");
    settle().await;
    mounted
        .state
        .fake
        .fork_errors
        .borrow_mut()
        .push_back("offline".into());
    mounted.input("retry this edit");
    mounted.key("Enter", "Enter", false);
    idle(&mounted).await;
    assert_eq!(mounted.state.chat.draft.get_untracked(), "retry this edit");
    assert_eq!(mounted.state.chat.prompt_images.get_untracked().len(), 1);
    assert_eq!(mounted.state.chat.prompt_edit.get_untracked(), Some((1, 3)));
    assert_eq!(mounted.state.fake.sessions.borrow().len(), 1);
    assert_eq!(mounted.state.chat.active_session.get_untracked(), Some(1));
}

#[wasm_bindgen_test]
async fn stale_fork_response_cannot_switch_a_new_session_or_account() {
    for account_change in [false, true] {
        let mounted = fixture(None);
        settle().await;
        let (sender, receiver) = futures::channel::oneshot::channel();
        mounted
            .state
            .fake
            .fork_results
            .borrow_mut()
            .push_back(receiver);
        mounted.click(".tui-fork-prompt[data-message-id='3']");
        settle().await;
        let session = mounted.state.chat.sessions.get_untracked()[0].clone();
        if account_change {
            mounted.state.auth.generation.update(|value| *value += 1);
        } else {
            mounted.state.chat.active_session.set(Some(2));
        }
        settle().await;
        mounted.state.chat.draft.set("new draft".into());
        sender
            .send(Ok(openwebide_core::ForkedSession {
                session: openwebide_core::ChatSession { id: 99, ..session },
                prompt: original_prompt(),
                history: vec![],
            }))
            .unwrap();
        settle().await;
        assert_eq!(mounted.state.chat.draft.get_untracked(), "new draft");
        assert_ne!(mounted.state.chat.active_session.get_untracked(), Some(99));
        assert!(
            mounted
                .state
                .chat
                .sessions
                .get_untracked()
                .iter()
                .all(|session| session.id != 99)
        );
        assert!(!mounted.state.chat.branching.get_untracked());
    }
}

#[wasm_bindgen_test]
async fn fork_keeps_the_draft_images_and_editor_context_added_while_copying() {
    let mounted = fixture(Some(WorkspaceMode::Remote));
    settle().await;
    let (sender, receiver) = futures::channel::oneshot::channel();
    mounted
        .state
        .fake
        .fork_results
        .borrow_mut()
        .push_back(receiver);
    mounted.click(".tui-fork-prompt[data-message-id='3']");
    settle().await;
    mounted.input("next draft");
    let images = vec![PromptImage::from_bytes("next.png".into(), b"\x89PNG\r\n\x1a\n").unwrap()];
    mounted.state.chat.prompt_images.set(images.clone());
    let editor = openwebide_core::tui::EditorContext {
        file_path: "next.txt".into(),
        cursor_line: 2,
        cursor_col: 1,
        selection: None,
    };
    mounted
        .state
        .chat
        .active_editor_context
        .set(Some(editor.clone()));
    let session = mounted.state.chat.sessions.get_untracked()[0].clone();
    sender
        .send(Ok(openwebide_core::ForkedSession {
            session: openwebide_core::ChatSession { id: 99, ..session },
            prompt: original_prompt(),
            history: vec![],
        }))
        .unwrap();
    idle(&mounted).await;
    assert_eq!(mounted.state.chat.active_session.get_untracked(), Some(99));
    assert_eq!(mounted.state.chat.draft.get_untracked(), "next draft");
    assert_eq!(mounted.state.chat.prompt_images.get_untracked(), images);
    assert_eq!(
        mounted.state.chat.active_editor_context.get_untracked(),
        Some(editor)
    );
}
