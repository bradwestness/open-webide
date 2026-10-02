use leptos::prelude::*;
use openwebide_core::ModelInfo;
use openwebide_core::RunEvent;
use openwebide_frontend::testing::fake_backend::Call;
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
            .push_back(vec![RunEvent::Delta {
                content: "answer".into(),
            }]);
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

#[wasm_bindgen_test]
async fn model_request_measurement() {
    let mounted = mount_test(|state| {
        state.seed_connection();
        state.seed_session();
        let mut second = state.chat.sessions.get_untracked()[0].clone();
        second.id = 2;
        state.chat.sessions.update(|sessions| sessions.push(second));
        chat_view(state)
    });
    settle().await;
    mounted.state.fake.calls.borrow_mut().clear();
    mounted.state.fake.model_requests.borrow_mut().clear();
    mounted.state.fake.context_requests.borrow_mut().clear();
    for i in 0..10 {
        mounted
            .state
            .chat
            .active_session
            .set(Some(if i % 2 == 0 { 2 } else { 1 }));
        settle().await;
        mounted
            .state
            .chat
            .sessions
            .update(|sessions| sessions[0].name = format!("rename {i}"));
        settle().await;
    }
    assert!(mounted.state.fake.model_requests.borrow().is_empty());
    assert!(mounted.state.fake.context_requests.borrow().is_empty());
    wasm_bindgen_test::console_log!(
        "ten switches + renames: models={} context={}",
        mounted.state.fake.model_requests.borrow().len(),
        mounted.state.fake.context_requests.borrow().len()
    );
}

#[wasm_bindgen_test]
async fn request_identity_ignores_names_but_tracks_connection_configuration() {
    let mounted = mount_test(|state| {
        state.seed_connection();
        state.seed_session();
        chat_view(state)
    });
    settle().await;
    mounted.state.fake.model_requests.borrow_mut().clear();
    mounted.state.fake.context_requests.borrow_mut().clear();
    mounted
        .state
        .settings
        .connections
        .update(|connections| connections[0].name = "renamed".into());
    settle().await;
    assert!(mounted.state.fake.model_requests.borrow().is_empty());
    assert!(mounted.state.fake.context_requests.borrow().is_empty());
    for change in 0..3 {
        mounted
            .state
            .settings
            .connections
            .update(|connections| match change {
                0 => connections[0].base_url = "http://changed".into(),
                1 => connections[0].kind = openwebide_core::ProviderKind::LlamaCpp,
                _ => connections[0].enabled = false,
            });
        settle().await;
    }
    assert_eq!(mounted.state.fake.model_requests.borrow().len(), 3);
    assert_eq!(mounted.state.fake.context_requests.borrow().len(), 3);
    mounted
        .state
        .settings
        .connections
        .update(|connections| connections[0].context_limit = Some(16384));
    settle().await;
    mounted
        .state
        .settings
        .connections
        .update(|connections| connections[0].model = Some("new default".into()));
    settle().await;
    mounted
        .state
        .chat
        .selected_model
        .set(Some("selected".into()));
    settle().await;
    assert_eq!(mounted.state.fake.model_requests.borrow().len(), 3);
    assert_eq!(mounted.state.fake.context_requests.borrow().len(), 6);
    assert_eq!(
        mounted
            .state
            .fake
            .context_requests
            .borrow()
            .last()
            .unwrap()
            .1
            .as_deref(),
        Some("selected")
    );
}

#[wasm_bindgen_test]
async fn stale_model_and_context_results_cannot_win_after_switching_back() {
    let (model_release, model_pending) = futures::channel::oneshot::channel();
    let (context_release, context_pending) = futures::channel::oneshot::channel();
    let mounted = mount_test(move |state| {
        state.seed_connection();
        state.seed_session();
        state
            .fake
            .model_results
            .borrow_mut()
            .push_back(model_pending);
        state
            .fake
            .context_results
            .borrow_mut()
            .push_back(context_pending);
        chat_view(state)
    });
    settle().await;
    mounted.state.settings.connections.update(|connections| {
        let mut second = connections[0].clone();
        second.id = 2;
        connections.push(second);
    });
    mounted
        .state
        .chat
        .sessions
        .update(|sessions| sessions[0].connection_id = Some(2));
    settle().await;
    *mounted.state.fake.models.borrow_mut() = vec![ModelInfo {
        name: "current".into(),
    }];
    mounted.state.fake.connections.borrow_mut()[0].context_limit = Some(8192);
    mounted
        .state
        .chat
        .sessions
        .update(|sessions| sessions[0].connection_id = Some(1));
    settle().await;
    assert_eq!(mounted.state.chat.models.get_untracked()[0].name, "current");
    assert_eq!(
        mounted
            .state
            .chat
            .session_telemetry
            .get_untracked()
            .context_limit,
        8192
    );
    model_release
        .send(Ok(vec![ModelInfo {
            name: "stale".into(),
        }]))
        .unwrap();
    context_release.send(Ok(Some(123))).unwrap();
    settle().await;
    assert_eq!(mounted.state.chat.models.get_untracked()[0].name, "current");
    assert_eq!(
        mounted
            .state
            .chat
            .session_telemetry
            .get_untracked()
            .context_limit,
        8192
    );
}

#[wasm_bindgen_test]
async fn logout_discards_model_and_context_results() {
    use openwebide_frontend::state::auth::AuthState;
    let auth_slot = std::rc::Rc::new(std::cell::Cell::new(None));
    let slot = auth_slot.clone();
    let (model_release, model_pending) = futures::channel::oneshot::channel();
    let (context_release, context_pending) = futures::channel::oneshot::channel();
    let mounted = mount_test(move |state| {
        state.seed_connection();
        state.seed_session();
        state
            .fake
            .model_results
            .borrow_mut()
            .push_back(model_pending);
        state
            .fake
            .context_results
            .borrow_mut()
            .push_back(context_pending);
        slot.set(Some(expect_context::<AuthState>()));
        chat_view(state)
    });
    settle().await;
    auth_slot.get().unwrap().logout();
    settle().await;
    model_release
        .send(Ok(vec![ModelInfo {
            name: "stale".into(),
        }]))
        .unwrap();
    context_release.send(Ok(Some(123))).unwrap();
    settle().await;
    assert!(mounted.state.chat.models.get_untracked().is_empty());
    assert_ne!(
        mounted
            .state
            .chat
            .session_telemetry
            .get_untracked()
            .context_limit,
        123
    );
}
