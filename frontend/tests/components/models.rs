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
    assert!(!menu.contains("Default model"), "{menu}");
    for name in ["qwen3:8b", "llama3"] {
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

#[wasm_bindgen_test]
async fn connection_tree_switches_existing_session_in_both_modes() {
    for mode in [
        openwebide_core::WorkspaceMode::Remote,
        openwebide_core::WorkspaceMode::Local,
    ] {
        let mounted = mount_test(move |state| {
            state.seed_project();
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            state.seed_connection();
            state.seed_session();
            let mut second = state.settings.connections.get_untracked()[0].clone();
            second.id = 2;
            second.name = "llama.cpp".into();
            second.model = Some("second-model".into());
            state.fake.connections.borrow_mut().push(second.clone());
            state
                .settings
                .connections
                .update(|connections| connections.push(second));
            *state.fake.models.borrow_mut() = vec![ModelInfo {
                name: "qwen3:8b".into(),
            }];
            chat_view(state)
        });
        settle().await;
        mounted.click(".tui-model-name");
        settle().await;
        assert!(
            mounted
                .element(".recent-menu")
                .text_content()
                .unwrap()
                .contains("llama.cpp")
        );
        assert_eq!(&*mounted.state.fake.model_requests.borrow(), &[1]);
        *mounted.state.fake.models.borrow_mut() = vec![ModelInfo {
            name: "second-model".into(),
        }];
        mounted.click("[data-connection-id='2'] .tui-connection-heading");
        settle().await;
        assert_eq!(&*mounted.state.fake.model_requests.borrow(), &[1, 2]);
        mounted.click("[data-connection-id='2'] .tui-connection-model");
        settle().await;
        assert_eq!(
            mounted.state.fake.sessions.borrow()[0].connection_id,
            Some(2)
        );
        assert_eq!(
            mounted.state.chat.sessions.get_untracked()[0].connection_id,
            Some(2)
        );
        assert_eq!(mounted.state.chat.active_session.get_untracked(), Some(1));
        assert_eq!(
            mounted.state.chat.selected_model.get_untracked().as_deref(),
            Some("second-model")
        );
        assert_eq!(
            mounted.state.settings.default_connection.get_untracked(),
            Some(1)
        );
        assert!(
            mounted
                .root
                .query_selector(".recent-menu")
                .unwrap()
                .is_none()
        );
        // A fresh session goes back to the default connection and configured model.
        mounted.state.chat.active_session.set(None);
        settle().await;
        assert!(mounted.state.chat.selected_model.get_untracked().is_none());
        assert_eq!(
            mounted
                .element(".tui-model-name")
                .text_content()
                .unwrap()
                .trim(),
            "qwen3:8b"
        );
    }
}

#[wasm_bindgen_test]
async fn connection_tree_choice_carries_into_new_session() {
    let mounted = mount_test(|state| {
        state.seed_project();
        state.seed_connection();
        let mut second = state.settings.connections.get_untracked()[0].clone();
        second.id = 2;
        second.name = "llama.cpp".into();
        second.model = Some("second-model".into());
        state.fake.connections.borrow_mut().push(second.clone());
        state
            .settings
            .connections
            .update(|connections| connections.push(second));
        *state.fake.models.borrow_mut() = vec![ModelInfo {
            name: "second-model".into(),
        }];
        chat_view(state)
    });
    settle().await;
    mounted.click(".tui-model-name");
    settle().await;
    mounted.click("[data-connection-id='2'] .tui-connection-heading");
    settle().await;
    mounted.click("[data-connection-id='2'] .tui-connection-model");
    settle().await;
    mounted.input("new session");
    mounted.key("Enter", "Enter", false);
    settle().await;
    assert_eq!(
        mounted.state.fake.sessions.borrow()[0].connection_id,
        Some(2)
    );
    assert!(
        mounted
            .state
            .fake
            .calls
            .borrow()
            .contains(&Call::SendMessage {
                session: 1,
                content: "new session".into(),
                model: Some("second-model".into()),
            })
    );
    assert_eq!(
        mounted.state.settings.default_connection.get_untracked(),
        Some(1)
    );
}

#[wasm_bindgen_test]
async fn user_model_defaults_exclude_shared_model_configuration() {
    use openwebide_frontend::components::model_setup::ModelSetupPanel;
    use wasm_bindgen::JsCast;
    let mounted = mount_test(|state| {
        state.seed_connection();
        *state.fake.models.borrow_mut() = vec![ModelInfo {
            name: "qwen3:8b".into(),
        }];
        view! { <ModelSetupPanel defaults_only=true /> }
    });
    settle().await;
    assert!(
        mounted
            .root
            .query_selector(".model-settings-editor")
            .unwrap()
            .is_none()
    );
    assert!(
        mounted
            .root
            .query_selector(".model-setup input[type=number]")
            .unwrap()
            .is_none()
    );
    let primary = mounted
        .element(".model-setup select")
        .dyn_into::<web_sys::HtmlSelectElement>()
        .unwrap();
    primary.set_value(
        &serde_json::to_string(&openwebide_core::ModelSelection {
            server_id: 1,
            model: "qwen3:8b".into(),
        })
        .unwrap(),
    );
    primary
        .dispatch_event(&web_sys::Event::new("change").unwrap())
        .unwrap();
    mounted.click_text("Save model defaults");
    settle().await;
    let setup = mounted.state.fake.model_setup.borrow().clone();
    assert_eq!(setup.defaults.primary.as_ref().unwrap().model, "qwen3:8b");
    assert!(setup.defaults.fast.is_none());
    assert_eq!(setup.resolve(1, "qwen3:8b").fast, setup.defaults.primary);
    assert_eq!(
        setup.resolve(1, "qwen3:8b").auto_compact_threshold,
        Some(85)
    );
}

#[wasm_bindgen_test]
async fn servers_open_shared_configuration_while_preferences_keep_model_defaults() {
    use openwebide_frontend::components::{Settings, Sidebar, model_setup::ModelSetupDialog};
    use wasm_bindgen::JsCast;
    for mode in [
        openwebide_core::WorkspaceMode::Remote,
        openwebide_core::WorkspaceMode::Local,
    ] {
        let mounted = mount_test(move |state| {
            state.seed_project();
            state.seed_connection();
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            *state.fake.models.borrow_mut() = vec![ModelInfo {
                name: "qwen3:8b".into(),
            }];
            state
                .fake
                .model_setup
                .borrow_mut()
                .profiles
                .push(openwebide_core::ModelProfile {
                    selection: openwebide_core::ModelSelection {
                        server_id: 1,
                        model: "qwen3:8b".into(),
                    },
                    settings: openwebide_core::ModelSettings {
                        context_limit: Some(8192),
                        ..Default::default()
                    },
                });
            let unit = Callback::new(|()| ());
            let id = Callback::new(|_: i64| ());
            view! {
                <Sidebar on_new_connection=unit on_edit_connection=id on_cancel_connection=unit on_delete_connection=id
                    on_select_session=id on_new_session=unit on_rename_session=id on_delete_session=id
                    on_new_prompt=unit on_edit_prompt=id on_save_prompt=unit on_cancel_prompt=unit on_delete_prompt=id />
                <Show when=move || state.settings.show_model_setup.get()><ModelSetupDialog /></Show>
                <Show when=move || state.settings.show_settings.get()><Settings on_set_theme=Callback::new(|_| ()) on_set_default_prompt=Callback::new(|_| ()) on_set_bridge_url=Callback::new(|_| ()) /></Show>
            }
        });
        settle().await;
        mounted.element("button[title='Configure models']").click();
        settle().await;
        assert_eq!(
            mounted.state.settings.model_setup_server.get_untracked(),
            Some(1)
        );
        let modal = mounted.element(".modal");
        let text = modal.text_content().unwrap();
        assert!(text.contains("Configuration is shared") && text.contains("Context tokens"));
        assert!(!text.contains("Default model") && !text.contains("Fast model"));
        let context: web_sys::HtmlInputElement = mounted
            .element(".model-settings-editor input[type=number]")
            .unchecked_into();
        assert_eq!(context.value(), "8192");
        assert!(text.contains("Server authentication and options"));
        context.set_value("16384");
        context
            .dispatch_event(&web_sys::Event::new("input").unwrap())
            .unwrap();
        mounted.click_text("Save model settings");
        settle().await;
        assert_eq!(
            mounted.state.fake.model_setup.borrow().profiles[0]
                .settings
                .context_limit,
            Some(16384)
        );
        mounted.element(".modal button[title=Close]").click();
        settle().await;
        mounted.state.settings.show_settings.set(true);
        settle().await;
        let text = mounted.element(".modal").text_content().unwrap();
        assert!(
            text.contains("Default model")
                && text.contains("Fast model")
                && text.contains("Default system prompt")
        );
        assert!(
            !text.contains("Context tokens")
                && !text.contains("Auto-compact")
                && !text.contains("Discover local servers")
        );
    }
}

#[wasm_bindgen_test]
async fn setup_wizard_discovers_customizes_and_reruns_in_both_modes() {
    use openwebide_frontend::components::model_wizard::ModelSetupWizard;
    use wasm_bindgen::JsCast;
    for mode in [
        openwebide_core::WorkspaceMode::Local,
        openwebide_core::WorkspaceMode::Remote,
    ] {
        let mounted = mount_test(move |state| {
            state.seed_project();
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            state.seed_connection();
            state.settings.conn_edit_id.set(Some(1));
            state
                .settings
                .conn_base_url
                .set("http://models.test".into());
            *state.fake.models.borrow_mut() = vec![ModelInfo {
                name: "qwen3:8b".into(),
            }];
            let open = RwSignal::new(true);
            view! { <Show when=move || open.get()><ModelSetupWizard on_close=Callback::new(move |()| open.set(false)) /></Show> }
        });
        settle().await;
        assert!(mounted.root.text_content().unwrap().contains("Step 1 of 3"));
        mounted.click_text("Next: server");
        settle().await;
        let key: web_sys::HtmlInputElement =
            mounted.element("input[type=password]").unchecked_into();
        key.set_value("test-token");
        key.dispatch_event(&web_sys::Event::new("input").unwrap())
            .unwrap();
        mounted.click_text("Discover models");
        settle().await;
        assert!(mounted.root.text_content().unwrap().contains("Step 3 of 3"));
        assert!(mounted.state.fake.server_settings.borrow()[&1].has_api_key);
        let context: web_sys::HtmlInputElement = mounted
            .element(".model-settings-editor input[type=number]")
            .unchecked_into();
        assert_eq!(context.value(), "8192");
        context.set_value("16384");
        context
            .dispatch_event(&web_sys::Event::new("input").unwrap())
            .unwrap();
        mounted.click_text("Save model settings");
        settle().await;
        mounted.click_text("Re-run discovery");
        settle().await;
        let key: web_sys::HtmlInputElement =
            mounted.element("input[type=password]").unchecked_into();
        assert!(key.value().is_empty());
        mounted.click_text("Discover models");
        settle().await;
        let context: web_sys::HtmlInputElement = mounted
            .element(".model-settings-editor input[type=number]")
            .unchecked_into();
        assert_eq!(context.value(), "16384");
        assert_eq!(mounted.state.fake.connections.borrow().len(), 1);
        let context: web_sys::HtmlInputElement = mounted
            .element(".model-settings-editor input[type=number]")
            .unchecked_into();
        context.set_value("24576");
        context
            .dispatch_event(&web_sys::Event::new("input").unwrap())
            .unwrap();
        settle().await;
        mounted.click_text("Apply settings");
        settle().await;
        let setup = mounted.state.fake.model_setup.borrow();
        assert_eq!(setup.profiles[0].settings.context_limit, Some(24576));
        assert_eq!(setup.defaults.primary.as_ref().unwrap().model, "qwen3:8b");
        assert!(mounted.root.query_selector(".modal").unwrap().is_none());
    }
}

#[wasm_bindgen_test]
async fn setup_retry_keeps_one_server_and_closed_discovery_cannot_apply() {
    use openwebide_frontend::components::model_wizard::ModelSetupWizard;
    for close in [false, true] {
        let (sender, receiver) = futures::channel::oneshot::channel();
        let mounted = mount_test(move |state| {
            state.fake.model_results.borrow_mut().push_back(receiver);
            *state.fake.models.borrow_mut() = vec![ModelInfo {
                name: "chat".into(),
            }];
            let open = RwSignal::new(true);
            view! { <Show when=move || open.get()><ModelSetupWizard on_close=Callback::new(move |()| open.set(false)) /></Show> }
        });
        settle().await;
        mounted.click_text("Next: server");
        settle().await;
        mounted.click_text("Discover models");
        settle().await;
        assert_eq!(mounted.state.fake.connections.borrow().len(), 1);
        if close {
            mounted.element(".modal button[title=Close]").click();
            settle().await;
            sender
                .send(Ok(vec![ModelInfo {
                    name: "stale".into(),
                }]))
                .unwrap();
            settle().await;
            assert!(mounted.state.fake.model_setup.borrow().profiles.is_empty());
            assert!(
                mounted
                    .state
                    .fake
                    .model_setup
                    .borrow()
                    .defaults
                    .primary
                    .is_none()
            );
            assert!(mounted.root.query_selector(".modal").unwrap().is_none());
        } else {
            sender
                .send(Err("401: Authentication required".into()))
                .unwrap();
            settle().await;
            assert!(
                mounted
                    .root
                    .text_content()
                    .unwrap()
                    .contains("Authentication required")
            );
            mounted.click_text("Discover models");
            settle().await;
            assert!(mounted.root.text_content().unwrap().contains("Step 3 of 3"));
            assert_eq!(mounted.state.fake.connections.borrow().len(), 1);
        }
    }
}
