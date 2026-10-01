#![cfg(target_arch = "wasm32")]

use openwebide_core::{ChatMessage, ConversationEntry, Role, RunEvent};
use openwebide_frontend::{backend::Backend, testing::fake_backend::FakeBackend};
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

#[wasm_bindgen_test]
async fn listings_discover_immediate_directories_and_preserve_project_boundaries() {
    let fake = FakeBackend::default();
    for (project, path) in [
        (1, "src/components/chat.rs"),
        (1, "src/components/editor.rs"),
        (1, "src/main.rs"),
        (1, "src-other/hidden.rs"),
        (2, "foreign.rs"),
    ] {
        fake.files
            .borrow_mut()
            .insert((project, path.into()), "code".into());
    }
    fake.create_file(1, "empty", true).await.unwrap();
    fake.create_file(1, "empty", true).await.unwrap();
    let root = fake.list_files(1, "").await.unwrap();
    assert_eq!(root.len(), 3);
    assert!(root.iter().all(|entry| entry.is_dir));
    assert_eq!(root[0].path, "empty");
    assert_eq!(root[1].path, "src");
    let src = fake.list_files(1, "src").await.unwrap();
    assert_eq!(src.len(), 2);
    assert_eq!(src[0].path, "src/components");
    assert!(src[0].is_dir);
    assert_eq!(src[1].path, "src/main.rs");
    assert!(!src[1].is_dir);
    let components = fake.list_files(1, "src/components").await.unwrap();
    assert_eq!(components.len(), 2);
    assert!(components.iter().all(|entry| !entry.is_dir));
    assert!(fake.list_files(1, "empty").await.unwrap().is_empty());
}

#[wasm_bindgen_test]
async fn creating_existing_file_preserves_contents() {
    let fake = FakeBackend::default();
    fake.files
        .borrow_mut()
        .insert((1, "main.rs".into()), "original".into());
    assert!(fake.create_file(1, "main.rs", false).await.is_err());
    assert_eq!(fake.read_file(1, "main.rs").await.unwrap(), "original");
    fake.create_file(2, "main.rs", false).await.unwrap();
    assert_eq!(fake.read_file(2, "main.rs").await.unwrap(), "");
    fake.create_file(1, "folder", true).await.unwrap();
    assert!(fake.create_file(1, "folder", false).await.is_err());
}

#[wasm_bindgen_test]
async fn sent_prompts_and_delta_replies_survive_history_reload() {
    let fake = FakeBackend::default();
    fake.scripted_events.borrow_mut().push_back(vec![
        RunEvent::Delta {
            content: "hello ".into(),
        },
        RunEvent::Delta {
            content: "world".into(),
        },
    ]);
    fake.send_message(1, "question", None, None, None, Box::new(|_| {}))
        .await
        .unwrap();
    let history = fake.list_messages(1).await.unwrap();
    assert_eq!(history.len(), 2);
    let ConversationEntry::Message(user) = &history[0] else {
        panic!("expected user message");
    };
    let ConversationEntry::Message(assistant) = &history[1] else {
        panic!("expected assistant message");
    };
    assert_eq!(user.role, Role::User);
    assert_eq!(user.content, "question");
    assert_eq!(user.session_id, 1);
    assert_eq!(assistant.role, Role::Assistant);
    assert_eq!(assistant.content, "hello world");
    assert_eq!(assistant.session_id, 1);
    assert_ne!(user.id, assistant.id);
    assert!(fake.list_messages(2).await.unwrap().is_empty());
}

#[wasm_bindgen_test]
async fn scripted_messages_are_persisted_once_with_final_metadata() {
    let fake = FakeBackend::default();
    let user = ChatMessage {
        id: 7,
        session_id: 1,
        role: Role::User,
        content: "question".into(),
        created_at: 123,
        tool_calls: None,
        tool_call_id: None,
        usage: None,
    };
    let assistant = ChatMessage {
        id: 8,
        role: Role::Assistant,
        content: "answer".into(),
        ..user.clone()
    };
    fake.scripted_events.borrow_mut().push_back(vec![
        RunEvent::Message {
            message: user.clone(),
        },
        RunEvent::Delta {
            content: "partial".into(),
        },
        RunEvent::Message {
            message: assistant.clone(),
        },
        RunEvent::Done {
            message: assistant.clone(),
        },
    ]);
    let mut events = Vec::new();
    fake.send_message(
        1,
        "question",
        None,
        None,
        None,
        Box::new(|event| events.push(event)),
    )
    .await
    .unwrap();
    assert_eq!(events.len(), 4);
    assert_eq!(
        fake.list_messages(1).await.unwrap(),
        vec![
            ConversationEntry::Message(user),
            ConversationEntry::Message(assistant),
        ]
    );
}
