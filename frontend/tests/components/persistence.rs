use leptos::prelude::*;
use openwebide_core::{User, UserRole};
use openwebide_frontend::{
    state::{auth::AuthState, layout::LayoutState},
    state_actions::lifecycle::{ProjectEffectContext, install_project_effects},
    testing::fake_backend::Call,
};
use wasm_bindgen::{JsCast, prelude::*};
use wasm_bindgen_test::*;

use super::support::{TestState, chat_view, mount_test, settle};

#[wasm_bindgen(inline_js = r#"
export function setViewport(width) {
    const original = Object.getOwnPropertyDescriptor(window, 'innerWidth');
    Object.defineProperty(window, 'innerWidth', { configurable: true, value: width });
    return () => {
        if (original) Object.defineProperty(window, 'innerWidth', original);
        else delete window.innerWidth;
    };
}
"#)]
extern "C" {
    #[wasm_bindgen(js_name = setViewport)]
    fn set_viewport(width: f64) -> js_sys::Function;
}

struct Viewport(js_sys::Function);
impl Viewport {
    fn new(width: f64) -> Self {
        Self(set_viewport(width))
    }
}
impl Drop for Viewport {
    fn drop(&mut self) {
        self.0.call0(&JsValue::NULL).unwrap();
    }
}

fn install(state: &TestState) {
    let auth = expect_context::<AuthState>();
    auth.set_user(User {
        id: openwebide_core::UserId::new(1),
        username: "test".into(),
        role: UserRole::User,
        created_at: 0,
    });
    install_project_effects(ProjectEffectContext {
        api: state.api,
        health: RwSignal::new(None),
        auth,
        settings: state.settings,
        projects: state.projects,
        chat: state.chat,
        layout: expect_context::<LayoutState>(),
        select_project: Callback::new(|_| ()),
    });
}

#[wasm_bindgen_test]
async fn stored_history_is_recalled_and_submissions_are_persisted() {
    let mounted = mount_test(|state| {
        state
            .fake
            .settings
            .borrow_mut()
            .insert("prompt_history".into(), r#"["a","b"]"#.into());
        state.seed_connection();
        state.seed_session();
        install(&state);
        chat_view(state)
    });
    settle().await;
    mounted.key("ArrowUp", "ArrowUp", false);
    settle().await;
    assert_eq!(mounted.state.chat.draft.get_untracked(), "b");
    let input: web_sys::HtmlTextAreaElement = mounted.element(".composer-input").unchecked_into();
    input.set_selection_start(Some(0)).unwrap();
    mounted.key("ArrowUp", "ArrowUp", false);
    settle().await;
    assert_eq!(mounted.state.chat.draft.get_untracked(), "a");
    mounted.input("new prompt");
    mounted.key("Enter", "Enter", false);
    settle().await;
    assert!(
        mounted
            .state
            .fake
            .calls
            .borrow()
            .contains(&Call::SetSetting {
                key: "prompt_history".into(),
                value: r#"["a","b","new prompt"]"#.into(),
            })
    );
    let calls = mounted.state.fake.calls.borrow();
    let settings = calls
        .iter()
        .position(|call| {
            *call
                == Call::Request {
                    method: "get_settings",
                }
        })
        .unwrap();
    let projects = calls
        .iter()
        .position(|call| {
            *call
                == Call::Request {
                    method: "list_projects",
                }
        })
        .unwrap();
    assert!(settings < projects);
}

#[wasm_bindgen_test]
async fn resize_shrinks_chat_first_without_persisting_fitted_widths() {
    let _wide = Viewport::new(2000.0);
    let mounted = mount_test(|state| {
        for (key, value) in [
            ("panel_sidebar_width", "200"),
            ("panel_tree_width", "180"),
            ("panel_chat_width", "600"),
        ] {
            state
                .fake
                .settings
                .borrow_mut()
                .insert(key.into(), value.into());
        }
        install(&state);
        chat_view(state)
    });
    settle().await;
    assert!(
        mounted
            .element(".chat-pane")
            .get_attribute("style")
            .unwrap()
            .contains("600px")
    );
    let _small = Viewport::new(900.0);
    web_sys::window()
        .unwrap()
        .dispatch_event(&web_sys::Event::new("resize").unwrap())
        .unwrap();
    settle().await;
    assert!(
        mounted
            .element(".chat-pane")
            .get_attribute("style")
            .unwrap()
            .contains("260px")
    );
    let settings = mounted.state.fake.settings.borrow();
    assert_eq!(settings["panel_chat_width"], "600");
    assert_eq!(settings["panel_tree_width"], "180");
    assert_eq!(settings["panel_sidebar_width"], "200");
}

#[wasm_bindgen_test]
async fn legacy_history_is_imported_once_and_legacy_keys_are_removed() {
    let storage = web_sys::window().unwrap().local_storage().unwrap().unwrap();
    storage
        .set_item("owide-prompt-history", r#"["a","a","b"]"#)
        .unwrap();
    storage.set_item("owide-theme", "light").unwrap();
    let mounted = mount_test(|state| {
        install(&state);
        chat_view(state)
    });
    settle().await;
    assert_eq!(
        mounted.state.chat.prompt_history.get_untracked(),
        ["a", "b"]
    );
    assert_eq!(
        mounted.state.fake.settings.borrow()["prompt_history"],
        r#"["a","b"]"#
    );
    assert!(storage.get_item("owide-prompt-history").unwrap().is_none());
    assert!(storage.get_item("owide-theme").unwrap().is_none());
    drop(mounted);
    storage
        .set_item("owide-prompt-history", r#"["legacy"]"#)
        .unwrap();
    let mounted = mount_test(|state| {
        state
            .fake
            .settings
            .borrow_mut()
            .insert("prompt_history".into(), r#"["server"]"#.into());
        install(&state);
        chat_view(state)
    });
    settle().await;
    assert_eq!(
        mounted.state.chat.prompt_history.get_untracked(),
        ["server"]
    );
    assert!(storage.get_item("owide-prompt-history").unwrap().is_none());
}

#[wasm_bindgen_test]
async fn failed_settings_load_does_not_overwrite_history() {
    let mounted = mount_test(|state| {
        state
            .fake
            .settings
            .borrow_mut()
            .insert("prompt_history".into(), r#"["saved prompt"]"#.into());
        *state.fake.settings_load_error.borrow_mut() = Some("unavailable".into());
        install(&state);
        chat_view(state)
    });
    settle().await;
    assert!(mounted.state.projects.projects_loaded.get_untracked());
    mounted.state.chat.prompt_history.set(vec!["new".into()]);
    settle().await;
    assert_eq!(
        mounted.state.fake.settings.borrow()["prompt_history"],
        r#"["saved prompt"]"#
    );
    assert!(
        !mounted
            .state
            .fake
            .calls
            .borrow()
            .iter()
            .any(|call| matches!(call,
        Call::SetSetting { key, .. } if key == "prompt_history"))
    );
}

#[wasm_bindgen_test]
async fn legacy_history_survives_a_failed_save_and_remount() {
    let storage = web_sys::window().unwrap().local_storage().unwrap().unwrap();
    storage
        .set_item("owide-prompt-history", r#"["only copy"]"#)
        .unwrap();
    let (release, pending) = futures::channel::oneshot::channel();
    let mounted = mount_test(move |state| {
        state
            .fake
            .history_save_results
            .borrow_mut()
            .push_back(pending);
        install(&state);
        chat_view(state)
    });
    settle().await;
    assert_eq!(
        storage.get_item("owide-prompt-history").unwrap().as_deref(),
        Some(r#"["only copy"]"#)
    );
    release.send(Err("unavailable".into())).unwrap();
    settle().await;
    assert_eq!(
        storage.get_item("owide-prompt-history").unwrap().as_deref(),
        Some(r#"["only copy"]"#)
    );
    drop(mounted);
    let mounted = mount_test(|state| {
        install(&state);
        chat_view(state)
    });
    settle().await;
    assert_eq!(
        mounted.state.fake.settings.borrow()["prompt_history"],
        r#"["only copy"]"#
    );
    assert!(storage.get_item("owide-prompt-history").unwrap().is_none());
}

#[wasm_bindgen_test]
async fn history_saves_wait_for_older_writes_and_coalesce_changes() {
    let mounted = mount_test(|state| {
        install(&state);
        chat_view(state)
    });
    settle().await;
    mounted.state.fake.calls.borrow_mut().clear();
    let (release, pending) = futures::channel::oneshot::channel();
    mounted
        .state
        .fake
        .history_save_results
        .borrow_mut()
        .push_back(pending);
    mounted.state.chat.prompt_history.set(vec!["old".into()]);
    settle().await;
    mounted
        .state
        .chat
        .prompt_history
        .set(vec!["old".into(), "new".into()]);
    settle().await;
    mounted
        .state
        .chat
        .prompt_history
        .set(vec!["old".into(), "new".into(), "newest".into()]);
    settle().await;
    let history_calls = || {
        mounted
            .state
            .fake
            .calls
            .borrow()
            .iter()
            .filter(|call| matches!(call, Call::SetSetting { key, .. } if key == "prompt_history"))
            .count()
    };
    assert_eq!(history_calls(), 1);
    release.send(Ok(())).unwrap();
    settle().await;
    assert_eq!(history_calls(), 2);
    assert_eq!(
        mounted.state.fake.settings.borrow()["prompt_history"],
        r#"["old","new","newest"]"#
    );
}

#[wasm_bindgen_test]
async fn sidebar_drag_does_not_save_fitted_chat_width() {
    use openwebide_frontend::{components::PanelResizer, state::layout::ActiveResizer};
    let _wide = Viewport::new(2000.0);
    let mounted = mount_test(|state| {
        for (key, value) in [
            ("panel_sidebar_width", "200"),
            ("panel_tree_width", "180"),
            ("panel_chat_width", "600"),
        ] {
            state
                .fake
                .settings
                .borrow_mut()
                .insert(key.into(), value.into());
        }
        install(&state);
        let layout = expect_context::<LayoutState>();
        view! {
            <PanelResizer kind=ActiveResizer::Sidebar />
            <PanelResizer kind=ActiveResizer::Tree />
            <PanelResizer kind=ActiveResizer::Chat />
            <button on:click=move |_| {
                layout.sidebar_width.set(layout.sidebar_width.get_untracked() - 10.0);
                layout.active_resizer.set(ActiveResizer::Sidebar);
            }>"Drag sidebar"</button>
        }
    });
    settle().await;
    let _small = Viewport::new(900.0);
    web_sys::window()
        .unwrap()
        .dispatch_event(&web_sys::Event::new("resize").unwrap())
        .unwrap();
    settle().await;
    mounted.click("button");
    web_sys::window()
        .unwrap()
        .dispatch_event(&web_sys::PointerEvent::new("pointerup").unwrap())
        .unwrap();
    settle().await;
    let settings = mounted.state.fake.settings.borrow();
    assert_eq!(settings["panel_sidebar_width"], "190");
    assert_eq!(settings["panel_tree_width"], "180");
    assert_eq!(settings["panel_chat_width"], "600");
}

#[wasm_bindgen_test]
async fn logout_preserves_saved_history() {
    let auth_slot = std::rc::Rc::new(std::cell::Cell::new(None));
    let slot = auth_slot.clone();
    let mounted = mount_test(move |state| {
        state
            .fake
            .settings
            .borrow_mut()
            .insert("prompt_history".into(), r#"["saved prompt"]"#.into());
        install(&state);
        slot.set(Some(expect_context::<AuthState>()));
        chat_view(state)
    });
    settle().await;
    mounted.state.fake.calls.borrow_mut().clear();
    let state = &mounted.state;
    auth_slot.get().unwrap().reset_user_state(
        state.projects,
        state.workspace,
        state.git,
        state.chat,
        state.settings,
        state.ui,
    );
    settle().await;
    assert_eq!(
        state.fake.settings.borrow()["prompt_history"],
        r#"["saved prompt"]"#
    );
    assert!(!state.fake.calls.borrow().iter().any(|call| matches!(call,
        Call::SetSetting { key, .. } if key == "prompt_history")));
}

#[wasm_bindgen_test]
async fn old_save_loop_cannot_drain_a_later_login_queue() {
    let auth_slot = std::rc::Rc::new(std::cell::Cell::new(None));
    let slot = auth_slot.clone();
    let mounted = mount_test(move |state| {
        install(&state);
        slot.set(Some(expect_context::<AuthState>()));
        chat_view(state)
    });
    settle().await;
    let state = &mounted.state;
    state.fake.calls.borrow_mut().clear();
    let (release_old, pending_old) = futures::channel::oneshot::channel();
    state
        .fake
        .history_save_results
        .borrow_mut()
        .push_back(pending_old);
    state.chat.prompt_history.set(vec!["old".into()]);
    settle().await;
    state.chat.prompt_history.set(vec!["queued old".into()]);
    settle().await;
    let auth = auth_slot.get().unwrap();
    auth.reset_user_state(
        state.projects,
        state.workspace,
        state.git,
        state.chat,
        state.settings,
        state.ui,
    );
    state
        .fake
        .settings
        .borrow_mut()
        .insert("prompt_history".into(), r#"["other account"]"#.into());
    let (release_new, pending_new) = futures::channel::oneshot::channel();
    state
        .fake
        .history_save_results
        .borrow_mut()
        .push_back(pending_new);
    auth.set_user(User {
        id: openwebide_core::UserId::new(2),
        username: "other".into(),
        role: UserRole::User,
        created_at: 0,
    });
    settle().await;
    state
        .chat
        .prompt_history
        .set(vec!["new account update".into()]);
    settle().await;
    release_old.send(Err("session expired".into())).unwrap();
    settle().await;
    let history_values = || {
        state
            .fake
            .calls
            .borrow()
            .iter()
            .filter_map(|call| match call {
                Call::SetSetting { key, value } if key == "prompt_history" => Some(value.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(history_values(), [r#"["old"]"#, r#"["other account"]"#]);
    release_new.send(Ok(())).unwrap();
    settle().await;
    assert_eq!(
        state.fake.settings.borrow()["prompt_history"],
        r#"["new account update"]"#
    );
}

#[wasm_bindgen_test]
async fn stale_settings_load_cannot_enable_history_for_a_later_login() {
    let auth_slot = std::rc::Rc::new(std::cell::Cell::new(None));
    let slot = auth_slot.clone();
    let (release, pending) = futures::channel::oneshot::channel();
    let mounted = mount_test(move |state| {
        state
            .fake
            .settings_load_results
            .borrow_mut()
            .push_back(pending);
        install(&state);
        slot.set(Some(expect_context::<AuthState>()));
        chat_view(state)
    });
    settle().await;
    let state = &mounted.state;
    let auth = auth_slot.get().unwrap();
    auth.reset_user_state(
        state.projects,
        state.workspace,
        state.git,
        state.chat,
        state.settings,
        state.ui,
    );
    *state.fake.settings_load_error.borrow_mut() = Some("unavailable".into());
    auth.set_user(User {
        id: openwebide_core::UserId::new(2),
        username: "other".into(),
        role: UserRole::User,
        created_at: 0,
    });
    settle().await;
    release
        .send(Ok(std::collections::BTreeMap::from([(
            "prompt_history".into(),
            r#"["old account"]"#.into(),
        )])))
        .unwrap();
    settle().await;
    assert!(state.chat.prompt_history.get_untracked().is_empty());
    state.chat.prompt_history.set(vec!["unsaved".into()]);
    settle().await;
    assert!(!state.fake.calls.borrow().iter().any(|call| matches!(call,
        Call::SetSetting { key, .. } if key == "prompt_history")));
}

#[wasm_bindgen_test]
async fn startup_latency_measurement() {
    for fail in [false, true] {
        let start = js_sys::Date::now();
        let mounted = mount_test(move |state| {
            *state.fake.endpoint_latency_ms.borrow_mut() = 200;
            state.seed_project();
            if fail {
                *state.fake.settings_load_error.borrow_mut() = Some("unavailable".into());
            }
            install(&state);
            view! { <div /> }
        });
        for _ in 0..200 {
            if mounted.state.projects.projects_loaded.get_untracked() {
                break;
            }
            openwebide_frontend::util::sleep_ms(10).await;
        }
        assert!(mounted.state.projects.projects_loaded.get_untracked());
        for method in [
            "get_settings",
            "list_projects",
            "list_connections",
            "list_sessions",
            "list_system_prompts",
        ] {
            assert_eq!(
                mounted
                    .state
                    .fake
                    .calls
                    .borrow()
                    .iter()
                    .filter(|call| **call == Call::Request { method })
                    .count(),
                1
            );
        }
        wasm_bindgen_test::console_log!(
            "startup fail={}: {:.0} ms; calls={:?}",
            fail,
            js_sys::Date::now() - start,
            mounted.state.fake.calls.borrow()
        );
    }
}

#[wasm_bindgen_test]
async fn startup_reads_start_together_and_wait_for_settings_and_projects() {
    let (settings_release, settings_pending) = futures::channel::oneshot::channel();
    let (projects_release, projects_pending) = futures::channel::oneshot::channel();
    let mounted = mount_test(move |state| {
        state.seed_project();
        state
            .fake
            .settings_load_results
            .borrow_mut()
            .push_back(settings_pending);
        state
            .fake
            .project_results
            .borrow_mut()
            .push_back(projects_pending);
        install(&state);
        view! { <div /> }
    });
    settle().await;
    for method in [
        "get_settings",
        "list_projects",
        "list_connections",
        "list_sessions",
        "list_system_prompts",
    ] {
        assert!(
            mounted
                .state
                .fake
                .calls
                .borrow()
                .contains(&Call::Request { method })
        );
    }
    assert!(!mounted.state.projects.projects_loaded.get_untracked());
    assert!(
        mounted
            .state
            .projects
            .open_tab_ids
            .get_untracked()
            .is_empty()
    );
    settings_release
        .send(Ok(std::collections::BTreeMap::from([
            ("open_tabs".into(), "[99,1,1]".into()),
            ("active_project".into(), "99".into()),
        ])))
        .unwrap();
    settle().await;
    assert!(!mounted.state.projects.projects_loaded.get_untracked());
    projects_release
        .send(Ok(mounted.state.fake.projects.borrow().clone()))
        .unwrap();
    settle().await;
    assert!(mounted.state.projects.projects_loaded.get_untracked());
    assert_eq!(mounted.state.projects.open_tab_ids.get_untracked(), [1]);
}

#[wasm_bindgen_test]
async fn logout_discards_deferred_startup_lists() {
    let auth_slot = std::rc::Rc::new(std::cell::Cell::new(None));
    let slot = auth_slot.clone();
    let (release, pending) = futures::channel::oneshot::channel();
    let mounted = mount_test(move |state| {
        state.fake.project_results.borrow_mut().push_back(pending);
        install(&state);
        slot.set(Some(expect_context::<AuthState>()));
        view! { <div /> }
    });
    settle().await;
    auth_slot.get().unwrap().logout();
    release
        .send(Ok(vec![openwebide_core::Project {
            id: 7,
            name: "stale".into(),
            mode: openwebide_core::WorkspaceMode::Remote,
            path: None,
            user_id: None,
            created_at: 0,
        }]))
        .unwrap();
    settle().await;
    assert!(mounted.state.projects.projects.get_untracked().is_empty());
    assert!(!mounted.state.projects.projects_loaded.get_untracked());
    assert!(
        mounted
            .state
            .projects
            .open_tab_ids
            .get_untracked()
            .is_empty()
    );
}
