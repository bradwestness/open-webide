use super::support::{mount_test, settle};
use leptos::prelude::*;
use openwebide_core::{SessionPreferences, WorkspaceMode};
use openwebide_frontend::{components::SessionList, state_actions::sessions::SessionActions};
use wasm_bindgen_test::*;

#[wasm_bindgen_test]
async fn session_archive_restore_and_pin_share_every_mode() {
    for mode in [
        Some(WorkspaceMode::Local),
        Some(WorkspaceMode::Remote),
        None,
    ] {
        let captured = std::rc::Rc::new(std::cell::Cell::new(None));
        let slot = captured.clone();
        let mounted = mount_test(move |state| {
            if let Some(mode) = mode {
                state.seed_project();
                state
                    .projects
                    .projects
                    .update(|projects| projects[0].mode = mode);
            }
            state.seed_session();
            let project = state.projects.active_project.get_untracked();
            state
                .chat
                .sessions
                .update(|sessions| sessions[0].project_id = project);
            state.fake.sessions.borrow_mut()[0].project_id = project;
            slot.set(Some(SessionActions::from_context()));
            view! { <SessionList on_select=Callback::new(|_| {}) on_new=Callback::new(|()| {}) on_rename=Callback::new(|_| {}) on_delete=Callback::new(|_| {})/> }
        });
        settle().await;
        let state = &mounted.state;
        let id = state.chat.sessions.get_untracked()[0].id;
        let actions = captured.get().unwrap();
        actions.preferences.run((
            id,
            SessionPreferences {
                pinned: Some(true),
                archived: None,
            },
        ));
        settle().await;
        assert!(state.chat.sessions.get_untracked()[0].pinned);
        actions.preferences.run((
            id,
            SessionPreferences {
                pinned: None,
                archived: Some(true),
            },
        ));
        settle().await;
        assert!(state.session_management.visible.get_untracked().is_empty());
        state.session_management.archived.set(true);
        settle().await;
        assert_eq!(state.session_management.visible.get_untracked().len(), 1);
        actions.preferences.run((
            id,
            SessionPreferences {
                pinned: None,
                archived: Some(false),
            },
        ));
        settle().await;
        assert!(state.session_management.visible.get_untracked().is_empty());
        state.session_management.archived.set(false);
        settle().await;
        assert!(state.session_management.visible.get_untracked()[0].pinned);
    }
}

#[wasm_bindgen_test]
async fn late_title_cannot_overwrite_manual_name_or_another_account() {
    for change_account in [false, true] {
        let (send, receive) = futures::channel::oneshot::channel();
        let fake =
            std::rc::Rc::new(openwebide_frontend::testing::fake_backend::FakeBackend::default());
        fake.title_results.borrow_mut().push_back(receive);
        let mounted = super::support::mount_test_with_backend(fake, |state| {
            state.seed_project();
            state.seed_session();
            state
                .chat
                .sessions
                .update(|sessions| sessions[0].auto_title = true);
            SessionActions::from_context();
            view! { <div/> }
        });
        settle().await;
        let mut generated = mounted.state.chat.sessions.get_untracked()[0].clone();
        generated.name = "Generated title".into();
        generated.auto_title = false;
        generated.title_revision += 1;
        mounted.state.chat.sessions.update(|sessions| {
            sessions[0].name = "My title".into();
            if !change_account {
                sessions[0].auto_title = false;
                sessions[0].title_revision += 1;
            }
        });
        if change_account {
            mounted.state.auth.generation.update(|epoch| *epoch += 1);
        }
        send.send(Ok(Some(generated))).unwrap();
        settle().await;
        assert_eq!(
            mounted.state.chat.sessions.get_untracked()[0].name,
            "My title"
        );
    }
}

#[wasm_bindgen_test]
async fn search_discards_a_response_after_switching_project() {
    let (send, receive) = futures::channel::oneshot::channel();
    let fake = std::rc::Rc::new(openwebide_frontend::testing::fake_backend::FakeBackend::default());
    fake.session_search_results.borrow_mut().push_back(receive);
    let mounted = super::support::mount_test_with_backend(fake, |state| {
        state.seed_project();
        state.seed_session();
        SessionActions::from_context();
        view! { <div/> }
    });
    settle().await;
    mounted.state.session_management.query.set("test".into());
    openwebide_frontend::util::sleep_ms(250).await;
    assert!(mounted.state.session_management.searching.get_untracked());
    let result = mounted.state.chat.sessions.get_untracked();
    mounted.state.projects.active_project.set(None);
    send.send(Ok(result)).unwrap();
    settle().await;
    assert!(
        mounted
            .state
            .session_management
            .matches
            .get_untracked()
            .is_none()
    );
    assert!(
        mounted
            .state
            .session_management
            .visible
            .get_untracked()
            .is_empty()
    );
}

#[derive(Default)]
struct Downloads(std::cell::RefCell<Vec<openwebide_core::SessionExport>>);
impl openwebide_frontend::state_actions::sessions::SessionDownload for Downloads {
    fn save(&self, export: &openwebide_core::SessionExport) -> Result<(), String> {
        self.0.borrow_mut().push(export.clone());
        Ok(())
    }
}

#[wasm_bindgen_test]
async fn export_is_single_dispatch_and_cannot_download_after_project_switch() {
    for switch_project in [false, true] {
        let (send, receive) = futures::channel::oneshot::channel();
        let fake =
            std::rc::Rc::new(openwebide_frontend::testing::fake_backend::FakeBackend::default());
        fake.session_export_results.borrow_mut().push_back(receive);
        let downloads = std::rc::Rc::new(Downloads::default());
        let host = downloads.clone();
        let captured = std::rc::Rc::new(std::cell::Cell::new(None));
        let slot = captured.clone();
        let mounted = super::support::mount_test_with_backend(fake, move |state| {
            state.seed_project();
            state.seed_session();
            let actions = SessionActions::new(
                state.api,
                state.auth,
                state.chat,
                state.projects,
                state.session_management,
                state.ui,
                host,
            );
            slot.set(Some(actions));
            view! { <div/> }
        });
        settle().await;
        let actions = captured.get().unwrap();
        actions.export.run(1);
        actions.export.run(1);
        settle().await;
        assert_eq!(
            mounted
                .state
                .fake
                .calls
                .borrow()
                .iter()
                .filter(|call| matches!(
                    call,
                    openwebide_frontend::testing::fake_backend::Call::Request {
                        method: "export_session"
                    }
                ))
                .count(),
            1
        );
        if switch_project {
            mounted.state.projects.active_project.set(None);
        }
        send.send(Ok(openwebide_core::SessionExport {
            filename: "test.md".into(),
            markdown: "# Test".into(),
        }))
        .unwrap();
        settle().await;
        assert_eq!(downloads.0.borrow().len(), usize::from(!switch_project));
        assert!(
            mounted
                .state
                .session_management
                .exporting
                .get_untracked()
                .is_empty()
        );
    }
}

#[wasm_bindgen_test]
async fn pending_titles_run_one_at_a_time_and_resume_after_completion() {
    let (send, receive) = futures::channel::oneshot::channel();
    let fake = std::rc::Rc::new(openwebide_frontend::testing::fake_backend::FakeBackend::default());
    fake.title_results.borrow_mut().push_back(receive);
    let mounted = super::support::mount_test_with_backend(fake, |state| {
        state.seed_project();
        state.seed_session();
        state.chat.sessions.update(|sessions| {
            sessions[0].auto_title = true;
            let mut second = sessions[0].clone();
            second.id = 2;
            sessions.push(second);
        });
        SessionActions::from_context();
        view! { <div/> }
    });
    settle().await;
    let count = || {
        mounted
            .state
            .fake
            .calls
            .borrow()
            .iter()
            .filter(|call| {
                matches!(
                    call,
                    openwebide_frontend::testing::fake_backend::Call::Request {
                        method: "session_title"
                    }
                )
            })
            .count()
    };
    assert_eq!(count(), 1);
    let mut first = mounted.state.chat.sessions.get_untracked()[0].clone();
    first.name = "Generated title".into();
    first.auto_title = false;
    first.title_revision += 1;
    send.send(Ok(Some(first))).unwrap();
    settle().await;
    assert_eq!(count(), 2);
    assert_eq!(
        mounted.state.chat.sessions.get_untracked()[0].name,
        "Generated title"
    );
}
