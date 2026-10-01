use leptos::prelude::*;
use openwebide_core::ModelInfo;
use openwebide_frontend::{sse::SseEvent, testing::fake_backend::Call};
use wasm_bindgen_test::*;

use super::support::{chat_view, mount_test, settle};

#[wasm_bindgen_test]
async fn model_dropdown_uses_default_and_sends_selected_model() {
    let mounted = mount_test(|state| {
        state.seed_project();
        state.seed_connection();
        state.seed_session();
        *state.fake.models.borrow_mut() = vec![
            ModelInfo {
                name: "qwen3:8b".into(),
            },
            ModelInfo {
                name: "llama3".into(),
            },
        ];
        state
            .fake
            .scripted_events
            .borrow_mut()
            .push_back(vec![SseEvent::Delta("answer".into())]);
        chat_view(state)
    });
    settle().await;
    assert_eq!(
        mounted
            .element(".tui-model-name")
            .text_content()
            .unwrap()
            .trim(),
        "qwen3:8b"
    );
    mounted.click(".tui-model-name");
    settle().await;
    let menu = mounted.element(".recent-menu").text_content().unwrap();
    for name in ["Default model", "qwen3:8b", "llama3"] {
        assert!(menu.contains(name), "{menu}");
    }
    mounted.click_text("llama3");
    settle().await;
    assert_eq!(
        mounted
            .state
            .chat
            .session_model
            .get_untracked()
            .get(&1)
            .cloned()
            .flatten()
            .as_deref(),
        Some("llama3")
    );
    mounted.input("hello");
    mounted.key("Enter", "Enter", false);
    settle().await;
    assert!(
        mounted
            .state
            .fake
            .calls
            .borrow()
            .contains(&Call::SendMessage {
                session: 1,
                content: "hello".into(),
                model: Some("llama3".into())
            })
    );
    assert!(mounted.root.text_content().unwrap().contains("answer"));
}

#[wasm_bindgen_test]
async fn model_choice_carries_into_auto_created_session() {
    let mounted = mount_test(|state| {
        state.seed_project();
        state.seed_connection();
        *state.fake.models.borrow_mut() = vec![ModelInfo {
            name: "llama3".into(),
        }];
        chat_view(state)
    });
    settle().await;
    mounted.click(".tui-model-name");
    settle().await;
    mounted.click_text("llama3");
    settle().await;
    mounted.input("new session");
    mounted.key("Enter", "Enter", false);
    settle().await;
    let session = mounted.state.chat.active_session.get_untracked().unwrap();
    assert_eq!(
        mounted
            .state
            .chat
            .session_model
            .get_untracked()
            .get(&session)
            .cloned()
            .flatten()
            .as_deref(),
        Some("llama3")
    );
    assert_eq!(mounted.state.fake.sessions.borrow().len(), 1);
    assert!(
        mounted
            .state
            .fake
            .calls
            .borrow()
            .contains(&Call::SendMessage {
                session,
                content: "new session".into(),
                model: Some("llama3".into())
            })
    );
}
