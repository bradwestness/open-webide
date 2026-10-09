use super::{
    project_git::{
        Http, gitCalls, gitChange, gitHistoryFixture, gitHttp, gitRelease, restore_token, token,
    },
    support::{mount_test, settle, wait_until},
};
use leptos::prelude::*;
use openwebide_core::{FileEntry, WorkspaceMode, git::*};
use openwebide_frontend::{
    project_git::ProjectGit,
    state_actions::{
        git::{GitActionContext, GitActions},
        git_history::GitHistoryActions,
        workspace::WorkspaceActions,
    },
    util::sleep_ms,
};
use std::{cell::Cell, rc::Rc};
use wasm_bindgen::{JsCast, prelude::*};
use wasm_bindgen_test::*;

#[wasm_bindgen(inline_js = r#"
export function reviewFolder() {
    const gate = {list:false, read:false};
    const file = name => Object.defineProperties(Object.create(FileSystemFileHandle.prototype), {name:{value:name},kind:{value:'file'},getFile:{value:async()=>{if(gate.read){gate.readStarted=true;await new Promise(resolve=>(gate.readResolvers??=[]).push(resolve));}return new File(['loaded'],name);}}});
    const folder = {name:'project',queryPermission:async()=> 'granted',values:async function*(){if(gate.list){gate.listStarted=true;await new Promise(resolve=>(gate.listResolvers??=[]).push(resolve));}yield file('old.rs');yield file('new.rs');},getFileHandle:async name=>file(name)};
    return {gate, folder};
}
export function reviewHandle(mock) {return mock.folder;}
export function reviewHold(mock, field) {mock.gate[field]=true;}
export function reviewStarted(mock, field) {return !!mock.gate[field+'Started'];}
export function reviewRelease(mock, field) {mock.gate[field]=false;for(const resolve of mock.gate[field+'Resolvers']||[]) resolve();mock.gate[field+'Resolvers']=[];}
"#)]
extern "C" {
    fn reviewFolder() -> JsValue;
    fn reviewHandle(mock: &JsValue) -> JsValue;
    fn reviewHold(mock: &JsValue, field: &str);
    fn reviewStarted(mock: &JsValue, field: &str) -> bool;
    fn reviewRelease(mock: &JsValue, field: &str);
}

fn entry() -> Vec<FileEntry> {
    vec![FileEntry {
        name: "old.rs".into(),
        path: "old.rs".into(),
        is_dir: false,
        size: 6,
    }]
}

#[wasm_bindgen_test]
async fn review_open_completion_survives_disposal_and_rejects_newer_selections_in_both_modes() {
    for mode in [WorkspaceMode::Remote, WorkspaceMode::Local] {
        for disposed in [true, false] {
            let folder = reviewFolder();
            let handle = reviewHandle(&folder);
            let slot = Rc::new(Cell::new(None));
            let output = slot.clone();
            let mounted = mount_test(move |state| {
                state.seed_project();
                state.projects.projects.update(|p| p[0].mode = mode);
                state.projects.local_handles.update(|h| {
                    h.insert(1, handle.unchecked_into());
                });
                output.set(Some(WorkspaceActions::new(
                    state.api,
                    state.projects,
                    state.workspace,
                    state.ui,
                    RwSignal::new(false),
                    Callback::new(|()| ()),
                )));
                view! {<div/>}
            });
            settle().await;
            let actions = slot.get().unwrap();
            let (send, receive) = futures::channel::oneshot::channel();
            if mode == WorkspaceMode::Remote {
                mounted
                    .state
                    .fake
                    .file_list_results
                    .borrow_mut()
                    .push_back(receive);
            } else {
                drop(receive);
                reviewHold(&folder, "list");
            }
            let owner = Owner::new();
            let done = owner.with(|| {
                let signal = RwSignal::new(false);
                Callback::new(move |_: Result<(), String>| signal.set(true))
            });
            actions.open_with_completion.run(("old.rs".into(), done));
            wait_until("held open listing", || {
                if mode == WorkspaceMode::Remote {
                    mounted.state.fake.file_list_results.borrow().is_empty()
                } else {
                    reviewStarted(&folder, "list")
                }
            })
            .await;
            if disposed {
                owner.cleanup();
            } else {
                actions.request_open.run("new.rs".into());
            }
            if mode == WorkspaceMode::Remote {
                send.send(Ok(entry())).unwrap();
            } else {
                reviewRelease(&folder, "list");
            }
            sleep_ms(20).await;
            settle().await;
            if !disposed {
                assert_eq!(
                    mounted.state.workspace.open_file.get_untracked().as_deref(),
                    Some("new.rs")
                );
            }
        }
    }
}

#[wasm_bindgen_test]
async fn review_session_switch_settles_the_current_editor_read_in_both_modes() {
    for mode in [WorkspaceMode::Remote, WorkspaceMode::Local] {
        let folder = reviewFolder();
        let handle = reviewHandle(&folder);
        let slot = Rc::new(Cell::new(None));
        let output = slot.clone();
        let mounted = mount_test(move |state| {
            state.seed_project();
            state.projects.projects.update(|p| p[0].mode = mode);
            state.projects.local_handles.update(|h| {
                h.insert(1, handle.unchecked_into());
            });
            output.set(Some(WorkspaceActions::new(
                state.api,
                state.projects,
                state.workspace,
                state.ui,
                RwSignal::new(false),
                Callback::new(|()| ()),
            )));
            view! {<div/>}
        });
        settle().await;
        let (send, receive) = futures::channel::oneshot::channel();
        if mode == WorkspaceMode::Remote {
            mounted
                .state
                .fake
                .file_read_results
                .borrow_mut()
                .push_back(receive);
        } else {
            drop(receive);
            reviewHold(&folder, "read");
        }
        slot.get().unwrap().request_open.run("old.rs".into());
        wait_until("held file read", || {
            if mode == WorkspaceMode::Remote {
                mounted.state.fake.file_read_results.borrow().is_empty()
            } else {
                reviewStarted(&folder, "read")
            }
        })
        .await;
        assert!(mounted.state.workspace.editor_loading.get_untracked());
        mounted.state.workspace.active_session.set(Some(2));
        if mode == WorkspaceMode::Remote {
            send.send(Ok("loaded".into())).unwrap();
        } else {
            reviewRelease(&folder, "read");
        }
        sleep_ms(20).await;
        settle().await;
        wait_until(
            &format!("{mode:?} editor read settles after session switch"),
            || !mounted.state.workspace.editor_loading.get_untracked(),
        )
        .await;
        assert!(!mounted.state.workspace.editor_loading.get_untracked());
        assert_eq!(
            mounted.state.workspace.content.get_untracked().as_str(),
            "loaded"
        );
    }
}

fn page() -> GitHistoryPage {
    serde_json::from_str(r#"{"commits":[{"hash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","parents":[],"author":"A","author_email":"a@b","authored_at":"2026-01-01T00:00:00Z","committer":"A","committer_email":"a@b","committed_at":"2026-01-01T00:00:00Z","subject":"Old","message":"Old","refs":[]},{"hash":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","parents":[],"author":"B","author_email":"b@b","authored_at":"2026-10-01T00:00:00Z","committer":"B","committer_email":"b@b","committed_at":"2026-10-01T00:00:00Z","subject":"New","message":"New","refs":[]}],"refs":[],"has_more":false}"#).unwrap()
}
fn diff() -> GitCommitDiff {
    GitCommitDiff {
        files: vec![],
        diff: String::new(),
        truncated: false,
    }
}

#[wasm_bindgen_test]
async fn review_empty_query_invalidates_pending_selection_and_stops_spinner_in_both_modes() {
    let old = token().await;
    for mode in [WorkspaceMode::Remote, WorkspaceMode::Local] {
        let http = Http(gitHttp());
        gitHistoryFixture(
            &http.0,
            &serde_json::to_string(&page()).unwrap(),
            &serde_json::to_string(&diff()).unwrap(),
        );
        let handle = super::local_bridge::probe_folder();
        let slot = Rc::new(Cell::new(None));
        let output = slot.clone();
        let mounted = mount_test(move |state| {
            state.seed_project();
            state.projects.projects.update(|p| p[0].mode = mode);
            state.projects.local_handles.update(|h| {
                h.insert(1, handle.unchecked_into());
            });
            state.settings.bridge_url.set("ws://git.test:3001".into());
            *state.fake.git_history.borrow_mut() = page();
            *state.fake.git_commit_diff.borrow_mut() = Some(diff());
            output.set(Some(GitHistoryActions::new()));
            view! {<div/>}
        });
        let actions = slot.get().unwrap();
        wait_until("initial page", || {
            actions.commits.get_untracked().len() == 2
        })
        .await;
        let (send, receive) = futures::channel::oneshot::channel();
        if mode == WorkspaceMode::Remote {
            mounted
                .state
                .fake
                .git_commit_diff_results
                .borrow_mut()
                .push_back(receive);
        } else {
            drop(receive);
            gitChange(&http.0, "deferDiff", true);
        }
        actions.select.run((page().commits.remove(0), None, None));
        wait_until("held selection", || {
            if mode == WorkspaceMode::Remote {
                !mounted
                    .state
                    .fake
                    .git_commit_diff_requests
                    .borrow()
                    .is_empty()
            } else {
                gitCalls(&http.0).contains("/git/commit-diff")
            }
        })
        .await;
        *mounted.state.fake.git_history.borrow_mut() = GitHistoryPage {
            commits: vec![],
            refs: vec![],
            has_more: false,
        };
        gitHistoryFixture(
            &http.0,
            &serde_json::to_string(&GitHistoryPage {
                commits: vec![],
                refs: vec![],
                has_more: false,
            })
            .unwrap(),
            &serde_json::to_string(&diff()).unwrap(),
        );
        actions.search.set("missing".into());
        wait_until("empty search published", || {
            actions.displayed.get_untracked().search == "missing"
        })
        .await;
        assert!(!actions.diff_loading.get_untracked());
        if mode == WorkspaceMode::Remote {
            send.send(Ok(diff())).unwrap();
        } else {
            gitRelease(&http.0, "deferDiff");
        }
        sleep_ms(20).await;
        settle().await;
        assert!(actions.selected.get_untracked().is_none());
        assert!(actions.diff.get_untracked().is_none());
    }
    restore_token(old).await;
}

#[wasm_bindgen_test]
async fn history_pagination_preserves_pending_selection_in_both_modes() {
    let old = token().await;
    for mode in [WorkspaceMode::Remote, WorkspaceMode::Local] {
        let http = Http(gitHttp());
        let mut initial = page();
        initial.has_more = true;
        gitHistoryFixture(
            &http.0,
            &serde_json::to_string(&initial).unwrap(),
            &serde_json::to_string(&diff()).unwrap(),
        );
        let handle = super::local_bridge::probe_folder();
        let slot = Rc::new(Cell::new(None));
        let output = slot.clone();
        let mounted = mount_test(move |state| {
            state.seed_project();
            state.projects.projects.update(|p| p[0].mode = mode);
            state.projects.local_handles.update(|h| {
                h.insert(1, handle.unchecked_into());
            });
            state.settings.bridge_url.set("ws://git.test:3001".into());
            let mut initial = page();
            initial.has_more = true;
            *state.fake.git_history.borrow_mut() = initial;
            *state.fake.git_commit_diff.borrow_mut() = Some(diff());
            output.set(Some(GitHistoryActions::new()));
            view! {<div/>}
        });
        let actions = slot.get().unwrap();
        wait_until("initial page", || {
            actions.commits.get_untracked().len() == 2
        })
        .await;
        let (send, receive) = futures::channel::oneshot::channel();
        if mode == WorkspaceMode::Remote {
            mounted
                .state
                .fake
                .git_commit_diff_results
                .borrow_mut()
                .push_back(receive);
        } else {
            drop(receive);
            gitChange(&http.0, "deferDiff", true);
        }
        actions.select.run((page().commits.remove(0), None, None));
        wait_until("held selection", || {
            if mode == WorkspaceMode::Remote {
                !mounted
                    .state
                    .fake
                    .git_commit_diff_requests
                    .borrow()
                    .is_empty()
            } else {
                gitCalls(&http.0).contains("/git/commit-diff")
            }
        })
        .await;
        *mounted.state.fake.git_history.borrow_mut() = GitHistoryPage {
            commits: vec![],
            refs: vec![],
            has_more: false,
        };
        gitHistoryFixture(
            &http.0,
            &serde_json::to_string(&GitHistoryPage {
                commits: vec![],
                refs: vec![],
                has_more: false,
            })
            .unwrap(),
            &serde_json::to_string(&diff()).unwrap(),
        );
        actions.load.run(true);
        wait_until("final history page appended", || {
            !actions.loading.get_untracked() && !actions.has_more.get_untracked()
        })
        .await;
        assert!(actions.diff_loading.get_untracked());
        if mode == WorkspaceMode::Remote {
            send.send(Ok(diff())).unwrap();
        } else {
            gitRelease(&http.0, "deferDiff");
        }
        wait_until("pending selection published after pagination", || {
            !actions.diff_loading.get_untracked() && actions.selected.get_untracked().is_some()
        })
        .await;
        assert_eq!(
            actions.selected.get_untracked().unwrap().hash,
            page().commits[0].hash
        );
        assert_eq!(actions.diff.get_untracked(), Some(diff()));
    }
    restore_token(old).await;
}

#[wasm_bindgen_test]
async fn review_duplicate_project_and_stale_sync_release_all_sync_actions_in_both_modes() {
    let old = token().await;
    for mode in [WorkspaceMode::Remote, WorkspaceMode::Local] {
        let http = Http(gitHttp());
        let handle = super::local_bridge::probe_folder();
        let slot = Rc::new(Cell::new(None));
        let output = slot.clone();
        let mounted = mount_test(move |state| {
            state.seed_project();
            state.projects.projects.update(|p| p[0].mode = mode);
            state.projects.local_handles.update(|h| {
                h.insert(1, handle.unchecked_into());
            });
            state.settings.bridge_url.set("ws://git.test:3001".into());
            output.set(Some((
                GitActions::new(GitActionContext {
                    project_git: expect_context::<ProjectGit>(),
                    projects: state.projects,
                    workspace: state.workspace,
                    git: state.git,
                    chat: state.chat,
                    ui: state.ui,
                    workspace_for: Callback::new(|_| None),
                    refresh: Callback::new(|()| ()),
                }),
                expect_context::<ProjectGit>(),
            )));
            view! {<div/>}
        });
        settle().await;
        for action in ["sync", "fetch", "pull", "push"] {
            for stale in [false, true] {
                let (send, receive) = futures::channel::oneshot::channel();
                if mode == WorkspaceMode::Remote {
                    mounted
                        .state
                        .fake
                        .git_sync_results
                        .borrow_mut()
                        .push_back(receive);
                } else {
                    drop(receive);
                    gitChange(&http.0, "deferSync", true);
                }
                let before = if mode == WorkspaceMode::Remote {
                    mounted.state.fake.git_sync_requests.borrow().len()
                } else {
                    serde_json::from_str::<Vec<serde_json::Value>>(&gitCalls(&http.0))
                        .unwrap()
                        .iter()
                        .filter(|c| c["path"] == "/git/sync")
                        .count()
                };
                slot.get().unwrap().0.on_sync.run(action.into());
                wait_until("held sync", || {
                    if mode == WorkspaceMode::Remote {
                        mounted.state.fake.git_sync_requests.borrow().len() > before
                    } else {
                        serde_json::from_str::<Vec<serde_json::Value>>(&gitCalls(&http.0))
                            .unwrap()
                            .iter()
                            .filter(|c| c["path"] == "/git/sync")
                            .count()
                            > before
                    }
                })
                .await;
                let facade = slot.get().unwrap().1;
                let revision = facade.revision();
                let project = mounted.state.projects.project(1).unwrap();
                assert!(!mounted.state.projects.add_project(project));
                settle().await;
                assert_eq!(facade.revision(), revision);
                if stale {
                    mounted
                        .state
                        .settings
                        .bridge_url
                        .set(format!("ws://git.test:3001/{action}"));
                    settle().await;
                }
                if mode == WorkspaceMode::Remote {
                    send.send(Err("held failure".into())).unwrap();
                } else {
                    gitRelease(&http.0, "deferSync");
                }
                sleep_ms(20).await;
                settle().await;
                assert!(mounted.state.git.sync_busy.get_untracked().is_none());
                mounted
                    .state
                    .settings
                    .bridge_url
                    .set("ws://git.test:3001".into());
                settle().await;
            }
        }
    }
    restore_token(old).await;
}

#[wasm_bindgen_test]
async fn review_timeline_edge_popovers_and_buttons_stay_inside_ci_viewport() {
    for width in [360, 768, 1280] {
        let mounted = mount_test(move |state| {
            let layout = expect_context::<openwebide_frontend::state::layout::LayoutState>();
            provide_context(
                openwebide_frontend::state_actions::layout::LayoutActions::new(
                    state.api, layout, state.auth, state.ui,
                ),
            );
            state.seed_project();
            *state.fake.git_history.borrow_mut() = page();
            *state.fake.git_commit_diff.borrow_mut() = Some(diff());
            WorkspaceActions::new(
                state.api,
                state.projects,
                state.workspace,
                state.ui,
                RwSignal::new(false),
                Callback::new(|()| ()),
            );
            view! {<style>{include_str!("../../styles.css")}</style><div style=format!("position:fixed;left:0;top:0;width:{width}px;max-width:100vw")><openwebide_frontend::components::git_history::FileHistory path="old.rs".to_owned() on_close=Callback::new(|()|()) /></div>}
        });
        wait_until("timeline extremes", || {
            mounted
                .root
                .query_selector_all(".git-timeline-cluster")
                .unwrap()
                .length()
                == 2
        })
        .await;
        mounted
            .element(".modal-overlay")
            .set_attribute(
                "style",
                &format!("left:0;right:auto;width:{width}px;max-width:100vw"),
            )
            .unwrap();
        sleep_ms(20).await;
        settle().await;
        let clusters = mounted
            .root
            .query_selector_all(".git-timeline-cluster")
            .unwrap();
        for index in [0, 1] {
            let cluster = clusters
                .item(index)
                .unwrap()
                .unchecked_into::<web_sys::Element>();
            cluster.set_attribute("open", "").unwrap();
            settle().await;
            for selector in [".ui-feedback-overlay", "button"] {
                let rect = cluster
                    .query_selector(selector)
                    .unwrap()
                    .unwrap()
                    .get_bounding_client_rect();
                assert!(rect.width() > 0.0);
                assert!(
                    rect.right() <= f64::from(width),
                    "{width}: container right {}",
                    rect.right()
                );
                assert!(rect.left() >= 0.0, "{width}: left {}", rect.left());
                assert!(
                    rect.right()
                        <= web_sys::window()
                            .unwrap()
                            .inner_width()
                            .unwrap()
                            .as_f64()
                            .unwrap(),
                    "{width}: right {}",
                    rect.right()
                );
            }
            cluster.remove_attribute("open").unwrap();
        }
    }
}
