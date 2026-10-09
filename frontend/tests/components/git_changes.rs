use super::{
    local_bridge::probe_folder,
    project_git::{Http, gitCalls, gitChange, gitHttp, gitRelease, restore_token, token},
    support::{mount_test, settle, wait_until},
};
use leptos::prelude::*;
use openwebide_core::{WorkspaceMode, git::*};
use openwebide_frontend::{
    components::{ConfirmDialog, git_changes::GitChanges},
    project_git::ProjectGit,
    state_actions::{
        file_tree::FileTreeActions, git_assistance::GitAssistance, workspace::WorkspaceActions,
    },
};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

fn paths() -> GitPathChanges {
    GitPathChanges {
        has_head: true,
        staged: ["a.txt".into()].into(),
        unstaged: ["a.txt".into()].into(),
        ..Default::default()
    }
}
#[wasm_bindgen_test]
async fn staged_and_unstaged_controls_route_through_both_adapters() {
    let old = token().await;
    for mode in [WorkspaceMode::Remote, WorkspaceMode::Local] {
        let http = Http(gitHttp());
        let handle = probe_folder();
        let slot = std::rc::Rc::new(std::cell::Cell::new(None));
        let capture = slot.clone();
        let mounted = mount_test(move |state| {
            state.seed_project();
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            state.projects.local_handles.update(|handles| {
                handles.insert(1, handle.unchecked_into());
            });
            state.settings.bridge_url.set("ws://git.test:3001".into());
            for _ in 0..50 {
                let (send, receive) = futures::channel::oneshot::channel();
                send.send(Ok(paths())).unwrap();
                state.fake.git_path_results.borrow_mut().push_back(receive);
            }
            WorkspaceActions::new(
                state.api,
                state.projects,
                state.workspace,
                state.ui,
                RwSignal::new(false),
                Callback::new(|()| ()),
            );
            capture.set(Some(expect_context::<FileTreeActions>()));
            view! {<GitChanges on_open=Callback::new(|_|()) /><ConfirmDialog />}
        });
        wait_until("both change sections", || {
            mounted
                .root
                .query_selector_all(".git-change-file")
                .unwrap()
                .length()
                == 2
        })
        .await;
        for label in [
            "Stage all changes",
            "Unstage all changes",
            "Stage file",
            "Unstage file",
        ] {
            let button = mounted.element(&format!("[aria-label=\"{label}\"]"));
            assert_eq!(button.get_attribute("title").as_deref(), Some(label));
        }
        mounted.click("[aria-label=\"Stage all changes\"]");
        wait_until("bulk staging", || {
            if mode == WorkspaceMode::Remote {
                !mounted.state.fake.git_path_requests.borrow().is_empty()
            } else {
                serde_json::from_str::<Vec<serde_json::Value>>(&gitCalls(&http.0))
                    .unwrap()
                    .iter()
                    .any(|call| call["path"] == "/git/path")
            }
        })
        .await;
        settle().await;
        for label in ["Stage file", "Unstage file", "Unstage all changes"] {
            wait_until("previous staging settled", || {
                !slot.get().unwrap().busy.get_untracked()
            })
            .await;
            mounted.click(&format!("[aria-label=\"{label}\"]"));
            settle().await;
        }
        wait_until("all staging actions settled", || {
            !slot.get().unwrap().busy.get_untracked()
        })
        .await;
        let requests: Vec<GitPathRequest> = if mode == WorkspaceMode::Remote {
            mounted
                .state
                .fake
                .git_path_requests
                .borrow()
                .iter()
                .map(|(_, request)| request.clone())
                .collect()
        } else {
            serde_json::from_str::<Vec<serde_json::Value>>(&gitCalls(&http.0))
                .unwrap()
                .into_iter()
                .filter(|call| call["path"] == "/git/path")
                .map(|call| serde_json::from_value(call["body"].clone()).unwrap())
                .collect()
        };
        assert_eq!(requests[0].action, GitPathAction::StageAll);
        assert!(requests[0].path.is_empty());
        assert_eq!(
            requests
                .iter()
                .map(|request| request.action)
                .collect::<Vec<_>>(),
            [
                GitPathAction::StageAll,
                GitPathAction::Stage,
                GitPathAction::Unstage,
                GitPathAction::UnstageAll
            ]
        );
        assert_eq!(requests[1].path, "a.txt");
        assert_eq!(requests[2].path, "a.txt");
        assert!(requests[3].path.is_empty());
        mounted.click("[aria-label=\"Toggle staged changes\"]");
        settle().await;
        assert!(
            mounted
                .element("[aria-label=\"Staged changes\"]")
                .has_attribute("hidden")
        );
        let actions = slot.get().unwrap();
        actions.stash(GitStashRequest {
            action: GitStashAction::Drop,
            hash: Some("stash-hash".into()),
            message: None,
        });
        settle().await;
        assert!(
            mounted
                .root
                .query_selector("[role=dialog]")
                .unwrap()
                .is_some()
        );
        assert!(
            mounted
                .state
                .fake
                .git_stash_requests
                .borrow()
                .iter()
                .all(|request| request.action != GitStashAction::Drop)
        );
        mounted.state.ui.confirm.set(None);
        drop(mounted);
        drop(http);
    }
    restore_token(old).await;
}
#[wasm_bindgen_test]
async fn automatic_commit_summary_uses_index_and_keeps_user_edits_in_both_modes() {
    let old = token().await;
    for mode in [WorkspaceMode::Remote, WorkspaceMode::Local] {
        for edit in [false, true] {
            let http = Http(gitHttp());
            let handle = probe_folder();
            let slot = std::rc::Rc::new(std::cell::Cell::new(None));
            let capture = slot.clone();
            let (send, receive) = futures::channel::oneshot::channel();
            let mounted = mount_test(move |state| {
                state.seed_project();
                state.seed_connection();
                state.seed_session();
                state
                    .projects
                    .projects
                    .update(|projects| projects[0].mode = mode);
                state.projects.local_handles.update(|handles| {
                    handles.insert(1, handle.unchecked_into());
                });
                state.settings.bridge_url.set("ws://git.test:3001".into());
                state.settings.default_connection.set(None);
                state.git.path_changes.set(Some(paths()));
                *state.fake.git_index_diff.borrow_mut() = "staged diff".into();
                for _ in 0..2 {
                    let (send, receive) = futures::channel::oneshot::channel();
                    send.send(Ok(openwebide_core::GitRepoStatus {
                        branch: "main".into(),
                        ..Default::default()
                    }))
                    .unwrap();
                    state.fake.git_statuses.borrow_mut().push_back(receive);
                }
                state
                    .fake
                    .assistance_results
                    .borrow_mut()
                    .push_back(receive);
                let actions = GitAssistance::new(
                    state.api,
                    expect_context::<ProjectGit>(),
                    state.projects,
                    state.auth,
                    state.chat,
                    state.settings,
                );
                actions.auto_commit(Signal::derive(|| true));
                capture.set(Some(actions));
                view! {<div/>}
            });
            wait_until("automatic staged summary", || {
                !mounted.state.fake.assistance_requests.borrow().is_empty()
            })
            .await;
            assert!(
                mounted.state.fake.assistance_requests.borrow()[0]
                    .input
                    .contains("staged diff")
            );
            if edit {
                mounted.state.git.commit_message.set("My message".into());
            }
            send.send(Ok(Some("Summarize staged changes".into())))
                .unwrap();
            wait_until("summary complete", || {
                !slot.get().unwrap().busy.get_untracked()
            })
            .await;
            assert_eq!(
                mounted.state.git.commit_message.get_untracked(),
                if edit {
                    "My message"
                } else {
                    "Summarize staged changes"
                }
            );
            assert!(mounted.state.fake.git_commit_requests.borrow().is_empty());
            drop(mounted);
            drop(http);
        }
    }
    restore_token(old).await;
}

#[wasm_bindgen_test]
async fn changes_refresh_and_feedback_keep_composer_and_rows_in_both_modes() {
    let old = token().await;
    for mode in [WorkspaceMode::Remote, WorkspaceMode::Local] {
        let http = Http(gitHttp());
        let handle = probe_folder();
        let mounted = mount_test(move |state| {
            state.seed_project();
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            state.projects.local_handles.update(|handles| {
                handles.insert(1, handle.unchecked_into());
            });
            state.settings.bridge_url.set("ws://git.test:3001".into());
            state.git.commit_message.set("Keep my message".into());
            for _ in 0..3 {
                let (send, receive) = futures::channel::oneshot::channel();
                send.send(Ok(paths())).unwrap();
                state.fake.git_path_results.borrow_mut().push_back(receive);
            }
            WorkspaceActions::new(
                state.api,
                state.projects,
                state.workspace,
                state.ui,
                RwSignal::new(false),
                Callback::new(|()| ()),
            );
            view! {<style>{include_str!("../../styles.css")}</style><div style="width:420px;height:700px"><openwebide_frontend::components::GitPane on_open=Callback::new(|_|()) on_load_git_diff=Callback::new(|()|()) on_discard_git_diff=Callback::new(|()|()) /></div>}
        });
        wait_until("staged rows", || {
            mounted
                .root
                .query_selector(".git-change-file")
                .unwrap()
                .is_some()
        })
        .await;
        let textarea = mounted.element("textarea");
        let first = mounted.element(".git-change-file");
        let before = textarea.get_bounding_client_rect();
        let staged_before = first.get_bounding_client_rect();
        let (send, receive) = futures::channel::oneshot::channel();
        if mode == WorkspaceMode::Remote {
            mounted.state.fake.git_path_results.borrow_mut().clear();
            mounted
                .state
                .fake
                .git_path_results
                .borrow_mut()
                .push_back(receive);
        } else {
            gitChange(&http.0, "deferPaths", true);
        }
        mounted
            .state
            .git
            .changes_revision
            .update(|revision| *revision += 1);
        wait_until("reserved progress slot", || {
            mounted
                .root
                .query_selector(".ui-progress-slot .ui-spinner")
                .unwrap()
                .is_some()
        })
        .await;
        assert!(textarea.is_same_node(Some(&mounted.element("textarea"))));
        assert!((textarea.get_bounding_client_rect().top() - before.top()).abs() < 1.0);
        assert!((first.get_bounding_client_rect().top() - staged_before.top()).abs() < 1.0);
        mounted.state.git.sync_busy.set(Some("sync".into()));
        if mode == WorkspaceMode::Remote {
            send.send(Err("index unavailable".into())).unwrap();
        } else {
            gitChange(&http.0, "pathsError", true);
            gitRelease(&http.0, "deferPaths");
        }
        wait_until("visible index failure", || {
            mounted
                .root
                .query_selector(".ui-feedback-overlay [role=alert]")
                .unwrap()
                .is_some()
        })
        .await;
        assert!(
            mounted
                .root
                .query_selector(".ui-progress-slot .ui-spinner")
                .unwrap()
                .is_some()
        );
        assert!(first.is_same_node(Some(&mounted.element(".git-change-file"))));
        assert!((textarea.get_bounding_client_rect().top() - before.top()).abs() < 1.0);
        assert!((first.get_bounding_client_rect().top() - staged_before.top()).abs() < 1.0);
        assert_eq!(
            mounted.state.git.commit_message.get_untracked(),
            "Keep my message"
        );
        mounted.state.git.sync_busy.set(None);
        mounted
            .state
            .git
            .path_changes
            .set(Some(GitPathChanges::default()));
        settle().await;
        assert!(
            mounted
                .element("[aria-label=\"Draft staged commit message\"]")
                .has_attribute("disabled")
        );
        assert_eq!(
            mounted
                .element(".ui-disabled-help")
                .get_attribute("title")
                .as_deref(),
            Some("Stage changes to draft a commit message")
        );
        drop(mounted);
        drop(http);
    }
    restore_token(old).await;
}
