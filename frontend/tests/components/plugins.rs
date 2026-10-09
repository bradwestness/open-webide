use super::support::{mount_test_with_backend, settle};
use leptos::prelude::*;
use openwebide_core::{User, UserId, UserRole, WorkspaceMode, plugins::testing::receipt};
use openwebide_frontend::{
    project_host::ProjectHost, project_plugins::ProjectPluginActions, state::plugins::PluginsState,
    testing::fake_backend::FakeBackend,
};
use std::{cell::Cell, rc::Rc};
use wasm_bindgen_test::*;

#[wasm_bindgen_test]
async fn plugins_guard_preparation_before_recording_on_account_project_and_host_changes() {
    for boundary in ["account", "project", "host", "success", "unpaired"] {
        let fake = Rc::new(FakeBackend::default());
        let (send, receive) = futures::channel::oneshot::channel();
        fake.plugin_preparations.borrow_mut().push_back(receive);
        let captured = Rc::new(Cell::new(None));
        let slot = captured.clone();
        let mounted = mount_test_with_backend(fake.clone(), move |state| {
            state.seed_project();
            state.auth.set_user(User {
                id: UserId::new(1),
                username: "test".into(),
                role: UserRole::User,
                created_at: 0,
            });
            let plugins = PluginsState::default();
            let host = ProjectHost::new(state.api, state.projects, state.settings, state.auth);
            let actions = ProjectPluginActions::new(
                state.api,
                plugins,
                host,
                state.auth,
                state.projects,
                state.chat,
                state.settings,
            );
            slot.set(Some((plugins, actions)));
            provide_context(plugins);
            provide_context(actions);
            view! {<openwebide_frontend::components::Plugins/>}
        });
        settle().await;
        let (plugins, _actions) = captured.get().unwrap();
        if boundary == "unpaired" {
            mounted
                .state
                .projects
                .projects
                .update(|projects| projects[0].mode = WorkspaceMode::Local);
            settle().await;
        }
        let prepared = receipt();
        plugins.repository.set(prepared.source.repository.clone());
        plugins.commit.set(prepared.source.commit.clone());
        plugins.path.set(prepared.source.path.clone());
        mounted.click("button");
        settle().await;
        if boundary == "unpaired" {
            assert!(fake.plugin_requests.borrow().is_empty());
            assert!(plugins.error.get_untracked().is_some());
            continue;
        }
        assert_eq!(fake.plugin_requests.borrow().len(), 1);
        match boundary {
            "account" => mounted.state.auth.logout(),
            "project" => mounted.state.projects.active_project.set(None),
            "host" => mounted
                .state
                .settings
                .bridge_url
                .set("ws://other-host:3001".into()),
            _ => {}
        }
        // Exercise the synchronous guard before reactive invalidation also runs.
        send.send(Ok(prepared)).unwrap();
        settle().await;
        if boundary == "success" {
            assert_eq!(fake.plugin_records.borrow().len(), 1);
            assert_eq!(plugins.installations.get_untracked().len(), 1);
            assert!(mounted.root.text_content().unwrap().contains("PR Review"));
        } else {
            assert!(fake.plugin_records.borrow().is_empty());
            assert!(plugins.installations.get_untracked().is_empty());
        }
        assert!(!plugins.busy.get_untracked());
    }
}

#[wasm_bindgen_test]
async fn plugins_discard_an_old_accounts_installation_list() {
    let fake = Rc::new(FakeBackend::default());
    let (send, receive) = futures::channel::oneshot::channel();
    fake.plugin_loads.borrow_mut().push_back(receive);
    let captured = Rc::new(Cell::new(None));
    let slot = captured.clone();
    let mounted = mount_test_with_backend(fake, move |state| {
        state.auth.set_user(User {
            id: UserId::new(1),
            username: "test".into(),
            role: UserRole::User,
            created_at: 0,
        });
        let plugins = PluginsState::default();
        let host = ProjectHost::new(state.api, state.projects, state.settings, state.auth);
        ProjectPluginActions::new(
            state.api,
            plugins,
            host,
            state.auth,
            state.projects,
            state.chat,
            state.settings,
        );
        slot.set(Some(plugins));
        view! {<div/>}
    });
    settle().await;
    mounted.state.auth.logout();
    let entries = openwebide_core::plugins::record_installation(
        Vec::new(),
        &openwebide_core::plugins::RecordPlugin {
            prepared: receipt(),
            revision: None,
        },
        1,
    )
    .unwrap();
    send.send(Ok(entries)).unwrap();
    settle().await;
    let plugins = captured.get().unwrap();
    assert!(plugins.installations.get_untracked().is_empty());
    assert!(!plugins.busy.get_untracked());
}
