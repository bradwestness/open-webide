use super::support::{mount_test_with_backend, settle, wait_until};
use leptos::prelude::*;
use openwebide_core::{ChatMessage, Role, WorkspaceMode};
use openwebide_frontend::{
    conversation::ConversationItem, state_actions::assistance::ChatAssistance,
    testing::fake_backend::FakeBackend,
};
use std::{cell::Cell, rc::Rc};
use wasm_bindgen_test::*;

#[wasm_bindgen_test]
async fn context_assistance_rejects_invented_ids_and_attaches_only_after_selection() {
    use openwebide_frontend::state_actions::context_assistance::{
        ContextAssistance, ContextCandidate,
    };
    for mode in [
        Some(WorkspaceMode::Local),
        Some(WorkspaceMode::Remote),
        None,
    ] {
        let fake = Rc::new(FakeBackend::default());
        let (send, receive) = futures::channel::oneshot::channel();
        send.send(Ok(Some("session:2\nsession:999\nfile:invented.rs".into())))
            .unwrap();
        fake.assistance_results.borrow_mut().push_back(receive);
        let captured = Rc::new(Cell::new(None));
        let slot = captured.clone();
        let mounted = mount_test_with_backend(fake, move |state| {
            if let Some(mode) = mode {
                state.seed_project();
                state
                    .projects
                    .projects
                    .update(|projects| projects[0].mode = mode);
            }
            state.seed_session();
            let project = state.projects.active_project.get_untracked();
            state.chat.sessions.update(|sessions| {
                sessions[0].project_id = project;
                let mut previous = sessions[0].clone();
                previous.id = 2;
                previous.name = "Menu icons".into();
                sessions.push(previous);
            });
            state.fake.messages.borrow_mut().insert(
                2,
                vec![openwebide_core::ConversationEntry::Message(ChatMessage {
                    id: 2,
                    session_id: 2,
                    role: Role::Assistant,
                    content: "<think>private</think>Fixed menu icons".into(),
                    created_at: 0,
                    tool_calls: None,
                    tool_call_id: None,
                    usage: None,
                })],
            );
            state.chat.draft.set("Explain the menu icon changes".into());
            slot.set(Some(ContextAssistance::from_context()));
            view! { <div/> }
        });
        wait_until("context suggestions", || {
            !captured
                .get()
                .unwrap()
                .suggestions
                .get_untracked()
                .is_empty()
        })
        .await;
        let actions = captured.get().unwrap();
        assert_eq!(
            actions.suggestions.get_untracked(),
            [ContextCandidate::Session(2, "Menu icons".into())]
        );
        assert_eq!(
            mounted.state.chat.draft.get_untracked(),
            "Explain the menu icon changes"
        );
        actions
            .attach
            .run(actions.suggestions.get_untracked()[0].clone());
        wait_until("selected chat attached", || {
            mounted
                .state
                .chat
                .draft
                .get_untracked()
                .contains("Fixed menu icons")
        })
        .await;
        assert!(!mounted.state.chat.draft.get_untracked().contains("private"));
        let old = ContextCandidate::Session(2, "Menu icons".into());
        mounted.state.auth.generation.update(|epoch| *epoch += 1);
        settle().await;
        let original = mounted.state.chat.draft.get_untracked();
        actions.attach.run(old);
        settle().await;
        assert_eq!(mounted.state.chat.draft.get_untracked(), original);
    }
}

#[wasm_bindgen_test]
async fn assistance_shows_grounded_recaps_and_at_most_two_suggestions() {
    let fake = Rc::new(FakeBackend::default());
    for result in [
        "Fixed menu icons; tests passed.",
        "Review changes\nDraft a commit\nIgnored third suggestion",
        "Fixed menu icons; tests passed.",
    ] {
        let (send, receive) = futures::channel::oneshot::channel();
        send.send(Ok(Some(result.into()))).unwrap();
        fake.assistance_results.borrow_mut().push_back(receive);
    }
    let captured = Rc::new(Cell::new(None));
    let slot = captured.clone();
    let mounted = mount_test_with_backend(fake, move |state| {
        state.seed_project();
        state.seed_session();
        state
            .chat
            .messages
            .set(vec![ConversationItem::Message(ChatMessage {
                id: 1,
                session_id: 1,
                role: Role::Assistant,
                content: "<think>private reasoning</think>Fixed icons; tests passed".into(),
                created_at: 0,
                tool_calls: None,
                tool_call_id: None,
                usage: None,
            })]);
        slot.set(Some(ChatAssistance::new(
            state.api,
            state.auth,
            state.chat,
            state.projects,
        )));
        view! { <div/> }
    });
    wait_until("recap and suggestions", || {
        captured.get().unwrap().next_actions.get_untracked().len() == 2
            && captured.get().unwrap().completion.get_untracked().is_some()
    })
    .await;
    let assistance = captured.get().unwrap();
    assert_eq!(
        assistance.recap.get_untracked().as_deref(),
        Some("Fixed menu icons; tests passed.")
    );
    assert_eq!(
        assistance.next_actions.get_untracked(),
        ["Review changes", "Draft a commit"]
    );
    assert!(
        mounted
            .state
            .fake
            .assistance_requests
            .borrow()
            .iter()
            .all(|request| !request.input.contains("private reasoning"))
    );
    mounted.state.chat.streaming.set(true);
    settle().await;
    assert!(assistance.recap.get_untracked().is_none());
    assert!(assistance.completion.get_untracked().is_none());
    assert!(assistance.next_actions.get_untracked().is_empty());
}

#[wasm_bindgen_test]
async fn assistance_discards_stale_account_project_and_session_results_in_every_mode() {
    for mode in [
        Some(WorkspaceMode::Local),
        Some(WorkspaceMode::Remote),
        None,
    ] {
        for change in ["account", "project", "session", "streaming"] {
            let fake = Rc::new(FakeBackend::default());
            let (send, receive) = futures::channel::oneshot::channel();
            fake.assistance_results.borrow_mut().push_back(receive);
            let captured = Rc::new(Cell::new(None));
            let slot = captured.clone();
            let mounted = mount_test_with_backend(fake, move |state| {
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
                state
                    .chat
                    .messages
                    .set(vec![ConversationItem::Message(ChatMessage {
                        id: 1,
                        session_id: 1,
                        role: Role::Assistant,
                        content: "Fixed icons; tests passed".into(),
                        created_at: 0,
                        tool_calls: None,
                        tool_call_id: None,
                        usage: None,
                    })]);
                slot.set(Some(ChatAssistance::new(
                    state.api,
                    state.auth,
                    state.chat,
                    state.projects,
                )));
                view! { <div/> }
            });
            wait_until("idle assistance request", || {
                !mounted.state.fake.assistance_requests.borrow().is_empty()
            })
            .await;
            let request = mounted.state.fake.assistance_requests.borrow()[0].clone();
            assert_eq!(
                request.project_id,
                mounted.state.projects.active_project.get_untracked()
            );
            match change {
                "account" => mounted
                    .state
                    .auth
                    .generation
                    .update(|generation| *generation += 1),
                "project" => mounted.state.projects.active_project.set(Some(99)),
                "session" => mounted.state.chat.active_session.set(Some(99)),
                _ => mounted.state.chat.streaming.set(true),
            }
            settle().await;
            send.send(Ok(Some("Old account result".into()))).unwrap();
            settle().await;
            let assistance = captured.get().unwrap();
            assert!(assistance.recap.get_untracked().is_none());
            assert!(assistance.next_actions.get_untracked().is_empty());
        }
    }
}

#[wasm_bindgen_test]
async fn assistance_git_drafts_keep_edits_and_reject_changed_sources_in_both_modes() {
    use super::project_git::{Http, gitDiff, gitHttp, restore_token, token};
    use openwebide_core::{AssistanceKind, GitRepoStatus};
    use openwebide_frontend::{
        project_git::ProjectGit, state_actions::git_assistance::GitAssistance,
    };
    use wasm_bindgen::JsCast;
    let old = token().await;
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        for outcome in ["stable", "edit", "diff", "account", "foreground"] {
            let http = Http(gitHttp());
            let folder = super::local_bridge::probe_folder();
            let captured = Rc::new(Cell::new(None));
            let slot = captured.clone();
            let fake = Rc::new(FakeBackend::default());
            let (send, receive) = futures::channel::oneshot::channel();
            fake.assistance_results.borrow_mut().push_back(receive);
            if mode == WorkspaceMode::Remote {
                for _ in 0..2 {
                    let (send, receive) = futures::channel::oneshot::channel();
                    send.send(Ok(GitRepoStatus {
                        branch: "main".into(),
                        ..Default::default()
                    }))
                    .unwrap();
                    fake.git_statuses.borrow_mut().push_back(receive);
                }
                fake.git_diffs.borrow_mut().extend([
                    Ok("actual remote diff".into()),
                    Ok(if outcome == "diff" {
                        "changed diff"
                    } else {
                        "actual remote diff"
                    }
                    .into()),
                ]);
            }
            let mounted = mount_test_with_backend(fake, move |state| {
                state.seed_project();
                state.seed_session();
                state
                    .projects
                    .projects
                    .update(|projects| projects[0].mode = mode);
                if mode == WorkspaceMode::Local {
                    state.projects.local_handles.update(|handles| {
                        handles.insert(1, folder.unchecked_into());
                    });
                    state.settings.bridge_url.set("ws://git.test:3001".into());
                }
                slot.set(Some(GitAssistance::new(
                    state.api,
                    expect_context::<ProjectGit>(),
                    state.projects,
                    state.auth,
                    state.chat,
                    state.settings,
                )));
                view! { <div/> }
            });
            settle().await;
            let actions = captured.get().unwrap();
            actions.generate.run(AssistanceKind::Commit);
            wait_until("diff grounded draft request", || {
                !mounted.state.fake.assistance_requests.borrow().is_empty()
            })
            .await;
            assert!(
                mounted.state.fake.assistance_requests.borrow()[0]
                    .input
                    .contains(if mode == WorkspaceMode::Local {
                        "local diff"
                    } else {
                        "actual remote diff"
                    })
            );
            match outcome {
                "edit" => actions.draft.set("My edited message".into()),
                "diff" if mode == WorkspaceMode::Local => gitDiff(&http.0, "changed diff"),
                "account" => mounted.state.auth.generation.update(|epoch| *epoch += 1),
                "foreground" => mounted.state.chat.streaming.set(true),
                _ => (),
            }
            settle().await;
            send.send(Ok(Some("Fix menu icons".into()))).unwrap();
            wait_until("draft resolved", || {
                !actions.busy.get_untracked() || outcome == "foreground"
            })
            .await;
            settle().await;
            assert_eq!(
                actions.draft.get_untracked(),
                match outcome {
                    "stable" => "Fix menu icons",
                    "edit" => "My edited message",
                    _ => "",
                }
            );
            if outcome == "diff" {
                assert!(actions.error.get_untracked().unwrap().contains("Try again"));
            }
            assert!(
                !mounted
                    .state
                    .fake
                    .calls
                    .borrow()
                    .iter()
                    .any(|call| matches!(
                        call,
                        openwebide_frontend::testing::fake_backend::Call::Request {
                            method: "git_commit"
                        }
                    ))
            );
            drop(mounted);
            drop(http);
        }
    }
    restore_token(old).await;
}

#[wasm_bindgen_test]
async fn assistance_context_files_and_memories_require_selection_and_reject_changed_memories() {
    use openwebide_core::{FileEntry, ProjectMemories, ProjectMemory};
    use openwebide_frontend::state_actions::context_assistance::{
        ContextAssistance, ContextCandidate,
    };
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        let fake = Rc::new(FakeBackend::default());
        for _ in 0..2 {
            let (send, receive) = futures::channel::oneshot::channel();
            send.send(Ok(Some("file:main.rs\nmemory:1\nfile:main.rs".into())))
                .unwrap();
            fake.assistance_results.borrow_mut().push_back(receive);
        }
        let captured = Rc::new(Cell::new(None));
        let slot = captured.clone();
        let entry = ProjectMemory {
            id: 1,
            title: "Build instructions".into(),
            content: "cargo test".into(),
            revision: 1,
            updated_at: 0,
            auto_title: false,
        };
        let setup_entry = entry.clone();
        let mounted = mount_test_with_backend(fake, move |state| {
            state.seed_project();
            state.seed_session();
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            state.workspace.entries.update(|entries| {
                entries.insert(
                    String::new(),
                    vec![FileEntry {
                        name: "main.rs".into(),
                        path: "main.rs".into(),
                        is_dir: false,
                        size: 5,
                    }],
                );
            });
            state.fake.memories.borrow_mut().insert(
                1,
                ProjectMemories {
                    enabled: true,
                    entries: vec![setup_entry.clone()],
                },
            );
            state.memories.data.set(Some(ProjectMemories {
                enabled: true,
                entries: vec![setup_entry],
            }));
            state.chat.draft.set("Explain the build for main.rs".into());
            slot.set(Some(ContextAssistance::from_context()));
            view! { <div/> }
        });
        let actions = captured.get().unwrap();
        wait_until("file and memory candidates", || {
            actions.suggestions.get_untracked().len() == 2
        })
        .await;
        assert_eq!(
            mounted.state.chat.draft.get_untracked(),
            "Explain the build for main.rs"
        );
        actions.attach.run(ContextCandidate::File("main.rs".into()));
        wait_until("selected file mention", || {
            mounted.state.chat.draft.get_untracked().contains("@file:")
        })
        .await;
        wait_until("refreshed memory candidate", || {
            actions
                .suggestions
                .get_untracked()
                .iter()
                .any(|candidate| matches!(candidate, ContextCandidate::Memory(_)))
        })
        .await;
        actions.attach.run(ContextCandidate::Memory(entry.clone()));
        wait_until("selected memory contents", || {
            mounted
                .state
                .chat
                .draft
                .get_untracked()
                .contains("cargo test")
        })
        .await;
        let original = mounted.state.chat.draft.get_untracked();
        mounted.state.memories.data.update(|data| {
            data.as_mut().unwrap().entries[0].revision += 1;
        });
        actions.attach.run(ContextCandidate::Memory(entry));
        settle().await;
        assert_eq!(mounted.state.chat.draft.get_untracked(), original);
    }
}
