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
        let (plugins, actions) = captured.get().unwrap();
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
        actions.install.run(());
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

#[wasm_bindgen_test]
async fn plugins_marketplace_install_and_explicit_project_activation() {
    use openwebide_core::plugins::{
        ProjectPlugin,
        testing::{catalog, package},
    };
    use openwebide_frontend::project_plugins::CatalogSelection;
    let fake = Rc::new(FakeBackend::default());
    let catalog = catalog();
    *fake.marketplaces.borrow_mut() = openwebide_core::plugins::marketplace::MarketplaceSettings {
        revision: 1,
        sources: vec![catalog.source.clone()],
        catalogs: vec![catalog.clone()],
    };
    let (send, receive) = futures::channel::oneshot::channel();
    send.send(Ok(package().prepared)).unwrap();
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
    let (plugins, actions) = captured.get().unwrap();
    assert!(
        mounted
            .root
            .text_content()
            .unwrap()
            .contains("Test marketplace")
    );
    let listing = &catalog.catalog.plugins[0];
    actions.install_release.run(CatalogSelection {
        marketplace: catalog.source,
        publisher: listing.publisher.clone(),
        name: listing.name.clone(),
        version: listing.releases[0].version.clone(),
    });
    settle().await;
    assert_eq!(
        fake.plugin_requests.borrow()[0].1.repository,
        "https://git.example.org/plugins.git"
    );
    assert!(fake.plugin_commands.borrow().is_empty());
    let installation = plugins.installations.get_untracked()[0].clone();
    let (send, receive) = futures::channel::oneshot::channel();
    send.send(Ok(package())).unwrap();
    fake.plugin_packages.borrow_mut().push_back(receive);
    let binding = ProjectPlugin {
        id: 1,
        revision: 1,
        prepared: package().prepared,
        enabled: true,
    };
    let (send, receive) = futures::channel::oneshot::channel();
    send.send(Ok(vec![binding])).unwrap();
    fake.project_plugin_results.borrow_mut().push_back(receive);
    actions.enable.run(installation);
    settle().await;
    assert_eq!(fake.plugin_commands.borrow().len(), 1);
    let openwebide_core::plugins::ProjectPluginCommand::Enable {
        package: activated, ..
    } = &fake.plugin_commands.borrow()[0].1
    else {
        panic!("Expected activation")
    };
    assert!(activated.skills[0].instructions.contains("review"));
    assert!(!activated.skills[0].resources.is_empty());
    assert!(
        mounted
            .root
            .text_content()
            .unwrap()
            .contains("Enabled in this project")
    );
    assert!(plugins.error.get_untracked().is_none());
}

#[wasm_bindgen_test]
async fn plugins_reject_mismatched_catalog_identity_and_stale_activation_packages() {
    use openwebide_core::plugins::testing::{catalog, package};
    use openwebide_frontend::project_plugins::CatalogSelection;
    for boundary in ["mismatch", "project", "account", "host"] {
        let fake = Rc::new(FakeBackend::default());
        let catalog = catalog();
        *fake.marketplaces.borrow_mut() =
            openwebide_core::plugins::marketplace::MarketplaceSettings {
                revision: 1,
                sources: vec![catalog.source.clone()],
                catalogs: vec![catalog.clone()],
            };
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
            view! {<div/>}
        });
        settle().await;
        let (plugins, actions) = captured.get().unwrap();
        if boundary == "mismatch" {
            let (send, receive) = futures::channel::oneshot::channel();
            let mut prepared = package().prepared;
            prepared.manifest.version = "0.2.0".into();
            send.send(Ok(prepared)).unwrap();
            fake.plugin_preparations.borrow_mut().push_back(receive);
            let listing = &catalog.catalog.plugins[0];
            actions.install_release.run(CatalogSelection {
                marketplace: catalog.source,
                publisher: listing.publisher.clone(),
                name: listing.name.clone(),
                version: listing.releases[0].version.clone(),
            });
            settle().await;
            assert!(fake.plugin_records.borrow().is_empty());
            assert!(
                plugins
                    .error
                    .get_untracked()
                    .unwrap()
                    .contains("does not match")
            );
            continue;
        }
        let entries = openwebide_core::plugins::record_installation(
            Vec::new(),
            &openwebide_core::plugins::RecordPlugin {
                prepared: package().prepared,
                revision: None,
            },
            1,
        )
        .unwrap();
        *fake.plugins.borrow_mut() = entries.clone();
        plugins.installations.set(entries.clone());
        let (send, receive) = futures::channel::oneshot::channel();
        fake.plugin_packages.borrow_mut().push_back(receive);
        actions.enable.run(entries[0].clone());
        settle().await;
        match boundary {
            "project" => mounted.state.projects.active_project.set(None),
            "account" => mounted.state.auth.logout(),
            _ => mounted
                .state
                .settings
                .bridge_url
                .set("ws://other-host:3001".into()),
        }
        send.send(Ok(package())).unwrap();
        settle().await;
        assert!(fake.plugin_records.borrow().is_empty());
        assert!(fake.plugin_commands.borrow().is_empty());
        assert!(plugins.project_plugins.get_untracked().is_empty());
        assert!(!plugins.busy.get_untracked());
    }
}

#[wasm_bindgen_test]
async fn plugins_custom_marketplace_cache_failure_and_account_guard() {
    use openwebide_core::plugins::{marketplace::*, testing::catalog};
    let fake = Rc::new(FakeBackend::default());
    let captured = Rc::new(Cell::new(None));
    let slot = captured.clone();
    let mounted = mount_test_with_backend(fake.clone(), move |state| {
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
        view! {<div/>}
    });
    settle().await;
    let (plugins, actions) = captured.get().unwrap();
    let cache = catalog();
    actions
        .save_sources
        .run(vec![MarketplaceSource::official(), cache.source.clone()]);
    settle().await;
    assert_eq!(fake.marketplaces.borrow().sources.len(), 2);
    let settings =
        cache_catalogs(fake.marketplaces.borrow().clone(), 1, vec![cache.clone()]).unwrap();
    let (send, receive) = futures::channel::oneshot::channel();
    send.send(Ok(MarketplaceRefresh {
        settings: settings.clone(),
        failures: vec![MarketplaceFailure {
            source: MarketplaceSource::official(),
            message: "Offline".into(),
        }],
    }))
    .unwrap();
    fake.marketplace_results.borrow_mut().push_back(receive);
    actions.refresh_catalogs.run(());
    settle().await;
    assert_eq!(
        plugins.marketplaces.get_untracked().catalogs,
        vec![cache.clone()]
    );
    assert_eq!(plugins.failures.get_untracked().len(), 1);
    let (send, receive) = futures::channel::oneshot::channel();
    fake.marketplace_results.borrow_mut().push_back(receive);
    actions.refresh_catalogs.run(());
    settle().await;
    mounted.state.auth.logout();
    send.send(Ok(MarketplaceRefresh {
        settings,
        failures: Vec::new(),
    }))
    .unwrap();
    settle().await;
    assert!(plugins.marketplaces.get_untracked().catalogs.is_empty());
    assert!(plugins.failures.get_untracked().is_empty());
    assert!(!plugins.busy.get_untracked());
}
