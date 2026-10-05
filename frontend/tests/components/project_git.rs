use leptos::prelude::*;
use openwebide_core::{FileEntry, GitRepoStatus, WorkspaceMode};
use openwebide_frontend::{
    components::FileTree, project_git::ProjectGit, state::auth::AuthState,
    state_actions::git::GitActions, util::sleep_ms,
};
use wasm_bindgen::{JsCast, prelude::*};
use wasm_bindgen_test::*;

use super::{
    local_bridge::probe_folder,
    support::{mount_test, settle},
};

#[wasm_bindgen(inline_js = r#"
export function gitHttp() {
    const original = window.fetch;
    const mock = { calls: [], found: true, clean: false, invalid: false, branches: [], currentBranch: "local-branch", checkoutError: false, branchesError: false };
    window.fetch = async request => {
        if (!request.url.startsWith('http://git.test:3001/')) return original(request);
        const body = JSON.parse(await request.text());
        const path = new URL(request.url).pathname;
        mock.calls.push({path, body, authorization: request.headers.get('Authorization')});
        const json = (value, status = 200) => new Response(JSON.stringify(value), {status});
        if (path === '/exec') {
            const probe = body.command.match(/\.openwebide-probe-[0-9a-f]+/)?.[0];
            return json({exit_code: mock.found ? 0 : 1, stdout: body.command.startsWith('find ') && mock.found ? `./repos/local/${probe}\n` : '', stderr: ''});
        }
        if (mock.invalid) return json({error:'cwd does not exist: repos/local'}, 400);
        if (path === '/git/status') return json({branch:mock.currentBranch,commit_hash:'abc',commit_message:null,upstream:null,ahead:0,behind:0,is_clean:mock.clean,line_stats:{insertions:0,deletions:0},files:mock.clean?{}:{'main.rs':'modified'}});
        if (path === '/git/show') return json({content:'committed'});
        if (path === '/git/diff') return json({diff:'local diff'});
        if (path === '/git/branches') return mock.branchesError ? json({error:'cannot list branches'}, 400) : json(mock.branches);
        if (path === '/git/checkout') { if (mock.checkoutError) return json({error:'uncommitted changes'}, 400); const previous = mock.currentBranch; mock.currentBranch = body.branch; return json({branch:body.branch,previous_branch:previous,switched:true}); }
        if (path === '/git/commit') return json({commit_hash:'abc',summary:'saved',is_signed:false});
        if (path === '/git/sync') return json({remote:'origin',branch:'main',pulled_commits:0,pushed_commits:0,output:''});
        throw new Error(path);
    };
    mock.restore = () => { window.fetch = original; };
    return mock;
}
export function gitBranches(mock, branches) { mock.branches = JSON.parse(branches); mock.currentBranch = "main"; }
export function gitCalls(mock) { return JSON.stringify(mock.calls); }
export function gitChange(mock, field, value) { mock[field] = value; }
export function gitRestore(mock) { mock.restore(); }
"#)]
extern "C" {
    fn gitHttp() -> JsValue;
    fn gitBranches(mock: &JsValue, branches: &str);
    fn gitCalls(mock: &JsValue) -> String;
    fn gitChange(mock: &JsValue, field: &str, value: bool);
    fn gitRestore(mock: &JsValue);
}

struct Http(JsValue);
impl Drop for Http {
    fn drop(&mut self) {
        gitRestore(&self.0);
    }
}

async fn token() -> Option<String> {
    let old = openwebide_frontend::idb::get_bridge_pairing_token()
        .await
        .unwrap();
    openwebide_frontend::idb::set_bridge_pairing_token("git-test-token")
        .await
        .unwrap();
    old
}
async fn restore_token(old: Option<String>) {
    if let Some(old) = old {
        openwebide_frontend::idb::set_bridge_pairing_token(&old)
            .await
            .unwrap();
    } else {
        openwebide_frontend::idb::delete_bridge_pairing_token()
            .await
            .unwrap();
    }
}

#[wasm_bindgen_test]
async fn local_git_badges_and_all_actions_share_verified_bridge_repository() {
    let old = token().await;
    let http = Http(gitHttp());
    let folder = probe_folder();
    let handle = folder.clone();
    let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
    let facade_slot = slot.clone();
    let mounted = mount_test(move |state| {
        state.seed_project();
        state
            .projects
            .projects
            .update(|projects| projects[0].mode = WorkspaceMode::Local);
        state.projects.local_handles.update(|handles| {
            handles.insert(1, handle.unchecked_into());
        });
        state.settings.bridge_url.set("ws://git.test:3001".into());
        state.workspace.entries.update(|entries| {
            entries.insert(
                String::new(),
                vec![FileEntry {
                    name: "main.rs".into(),
                    path: "main.rs".into(),
                    is_dir: false,
                    size: 1,
                }],
            );
        });
        let project_git = expect_context::<ProjectGit>();
        *facade_slot.borrow_mut() = Some(project_git);
        let refresh = GitActions::refresh(
            project_git,
            state.projects,
            state.git,
            expect_context::<AuthState>(),
        );
        view! {
                   <button id="refresh" on:click=move |_| refresh.run(())>"Refresh"</button>
                   <FileTree on_toggle=Callback::new(|_| ()) on_open=Callback::new(|_| ())
                       on_new_file=Callback::new(|()| ()) on_new_dir=Callback::new(|()| ())
        />
               }
    });
    settle().await;
    mounted.click("#refresh");
    for _ in 0..100 {
        sleep_ms(5).await;
        settle().await;
        if mounted.state.git.status.get_untracked().is_some() {
            break;
        }
    }
    assert_eq!(
        mounted.state.git.status.get_untracked().unwrap().branch,
        "local-branch"
    );
    assert_eq!(
        mounted
            .element(".git-badge-modified")
            .text_content()
            .unwrap(),
        "M"
    );
    let git = slot.borrow().unwrap();
    let repo = git.repository(Some(1)).await.unwrap();
    assert_eq!(repo.file_head("main.rs").await.unwrap(), "committed");
    assert_eq!(repo.diff(Some("main.rs")).await.unwrap(), "local diff");
    assert!(repo.branches().await.unwrap().is_empty());
    repo.commit(&openwebide_core::GitCommitRequest {
        message: "save".into(),
        paths: None,
        include_untracked: false,
    })
    .await
    .unwrap();
    repo.checkout(&openwebide_core::GitCheckoutRequest {
        branch: "other".into(),
        create_if_missing: false,
    })
    .await
    .unwrap();
    repo.sync(&openwebide_core::GitSyncRequest {
        action: "sync".into(),
        remote: None,
        branch: None,
    })
    .await
    .unwrap();
    // Resolved access is shared by UI refreshes and all Git actions.
    let before: Vec<serde_json::Value> = serde_json::from_str(&gitCalls(&http.0)).unwrap();
    assert_eq!(
        before.iter().filter(|call| call["path"] == "/exec").count(),
        1
    );
    gitChange(&http.0, "clean", true);
    mounted.click("#refresh");
    for _ in 0..100 {
        sleep_ms(5).await;
        settle().await;
        if mounted
            .state
            .git
            .status
            .get_untracked()
            .is_some_and(|s| s.is_clean)
        {
            break;
        }
    }
    assert!(
        mounted
            .root
            .query_selector(".git-badge-modified")
            .unwrap()
            .is_none()
    );
    assert!(mounted.state.fake.git_status_requests.borrow().is_empty());
    let calls: Vec<serde_json::Value> = serde_json::from_str(&gitCalls(&http.0)).unwrap();
    for call in calls
        .iter()
        .filter(|call| call["path"].as_str().unwrap().starts_with("/git/"))
    {
        assert_eq!(call["body"]["cwd"], "repos/local");
        assert_eq!(call["authorization"], "Bearer git-test-token");
    }
    assert_eq!(
        js_sys::Reflect::get(&folder, &"files".into())
            .unwrap()
            .unchecked_into::<js_sys::Map>()
            .size(),
        0
    );
    // A vanished cwd invalidates the adapter; the next access re-probes instead of
    // falling back to the backend's unrelated workspace.
    gitChange(&http.0, "invalid", true);
    assert!(repo.status().await.is_err());
    assert!(git.repository(Some(1)).await.is_err());
    gitChange(&http.0, "invalid", false);
    let recovered = git.repository(Some(1)).await.unwrap();
    assert_eq!(recovered.status().await.unwrap().branch, "other");
    let after: Vec<serde_json::Value> = serde_json::from_str(&gitCalls(&http.0)).unwrap();
    assert_eq!(
        after.iter().filter(|call| call["path"] == "/exec").count(),
        2
    );

    // Replacing a browser handle also invalidates cached host access.
    mounted.state.projects.local_handles.update(|handles| {
        handles.insert(1, probe_folder().unchecked_into());
    });
    settle().await;
    git.repository(Some(1)).await.unwrap();
    let after: Vec<serde_json::Value> = serde_json::from_str(&gitCalls(&http.0)).unwrap();
    assert_eq!(
        after.iter().filter(|call| call["path"] == "/exec").count(),
        3
    );
    drop(mounted);
    restore_token(old).await;
}

#[wasm_bindgen_test]
async fn unresolved_local_git_never_falls_back_to_remote_workspace() {
    let old = token().await;
    let http = Http(gitHttp());
    gitChange(&http.0, "found", false);
    let folder = probe_folder();
    let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
    let facade_slot = slot.clone();
    let mounted = mount_test(move |state| {
        *facade_slot.borrow_mut() = Some(expect_context::<ProjectGit>());
        state.seed_project();
        state
            .projects
            .projects
            .update(|projects| projects[0].mode = WorkspaceMode::Local);
        state.projects.local_handles.update(|handles| {
            handles.insert(1, folder.unchecked_into());
        });
        state.settings.bridge_url.set("ws://git.test:3001".into());
        view! { <div /> }
    });
    let git = slot.borrow().unwrap();
    settle().await;
    assert!(git.repository(Some(1)).await.is_err());
    assert!(mounted.state.fake.git_status_requests.borrow().is_empty());
    let calls: Vec<serde_json::Value> = serde_json::from_str(&gitCalls(&http.0)).unwrap();
    assert!(calls.iter().all(|call| call["path"] == "/exec"));
    drop(mounted);
    restore_token(old).await;
}

#[wasm_bindgen_test]
async fn remote_git_refresh_ignores_results_after_project_switch_or_logout() {
    for logout in [false, true] {
        let (tx, rx) = futures::channel::oneshot::channel();
        let auth = std::rc::Rc::new(std::cell::RefCell::new(None));
        let auth_slot = auth.clone();
        let mounted = mount_test(move |state| {
            state.seed_project();
            state.fake.git_statuses.borrow_mut().push_back(rx);
            let session_auth = expect_context::<AuthState>();
            *auth_slot.borrow_mut() = Some(session_auth);
            let refresh = GitActions::refresh(
                expect_context::<ProjectGit>(),
                state.projects,
                state.git,
                session_auth,
            );
            view! { <button on:click=move |_| refresh.run(())>"Refresh"</button> }
        });
        settle().await;
        mounted.click("button");
        settle().await;
        assert_eq!(
            *mounted.state.fake.git_status_requests.borrow(),
            vec![Some(1)]
        );
        if logout {
            auth.borrow().unwrap().logout();
        } else {
            mounted.state.projects.active_project.set(Some(2));
        }
        tx.send(Ok(GitRepoStatus {
            branch: "stale".into(),
            ..Default::default()
        }))
        .unwrap();
        settle().await;
        assert!(mounted.state.git.status.get_untracked().is_none());
        drop(mounted);
    }
}

#[wasm_bindgen_test]
async fn remote_git_refresh_deduplicates_and_publishes_current_status() {
    let (tx, rx) = futures::channel::oneshot::channel();
    let mounted = mount_test(move |state| {
        state.seed_project();
        state.fake.git_statuses.borrow_mut().push_back(rx);
        let refresh = GitActions::refresh(
            expect_context::<ProjectGit>(),
            state.projects,
            state.git,
            expect_context::<AuthState>(),
        );
        view! { <button on:click=move |_| refresh.run(())>"Refresh"</button> }
    });
    settle().await;
    mounted.click("button");
    mounted.click("button");
    settle().await;
    assert_eq!(
        *mounted.state.fake.git_status_requests.borrow(),
        vec![Some(1)]
    );
    tx.send(Ok(GitRepoStatus {
        branch: "remote-branch".into(),
        ..Default::default()
    }))
    .unwrap();
    settle().await;
    assert_eq!(
        mounted.state.git.status.get_untracked().unwrap().branch,
        "remote-branch"
    );
}

#[wasm_bindgen_test]
async fn branch_selectors_share_checkout_creation_and_failures_in_both_modes() {
    use super::support::wait_until;
    use openwebide_core::{GitBranchInfo, GitCheckoutResult};
    use openwebide_frontend::{
        components::{GitPane, StatusBar},
        state_actions::git::GitActionContext,
    };
    let old = token().await;
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        let http = Http(gitHttp());
        let branches = vec![
            GitBranchInfo {
                name: "feature".into(),
                is_current: false,
                is_remote: false,
                upstream: None,
            },
            GitBranchInfo {
                name: "main".into(),
                is_current: true,
                is_remote: false,
                upstream: None,
            },
        ];
        gitBranches(&http.0, &serde_json::to_string(&branches).unwrap());
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
            *state.fake.git_branches.borrow_mut() = branches;
            state.git.status.set(Some(GitRepoStatus {
                branch: "main".into(),
                ..GitRepoStatus::default()
            }));
            let actions = GitActions::new(GitActionContext {
                project_git: expect_context::<ProjectGit>(),
                projects: state.projects,
                workspace: state.workspace,
                git: state.git,
                chat: state.chat,
                ui: state.ui,
                workspace_for: Callback::new(|_: i64| None),
                refresh: Callback::new(|()| ()),
            });
            view! {
                <div id="changes-branch"><GitPane on_open=Callback::new(|_: String| ()) on_load_git_diff=Callback::new(|()| ()) on_discard_git_diff=Callback::new(|()| ()) on_load_branches=actions.on_load_branches on_select_branch=actions.on_select_branch on_new_branch=actions.on_branch_click /></div>
                <div id="footer-branch"><StatusBar health=RwSignal::new(None).read_only() on_toggle_terminal=|| () on_load_branches=actions.on_load_branches on_select_branch=actions.on_select_branch on_branch_click=actions.on_branch_click /></div>
            }
        });
        wait_until("branch options", || {
            mounted
                .state
                .git
                .branches
                .with_untracked(|branches| branches.len() == 2)
        })
        .await;
        let choose = |selector, value: &str| {
            let select: web_sys::HtmlSelectElement = mounted.element(selector).unchecked_into();
            select.set_value(value);
            select
                .dispatch_event(&web_sys::Event::new("change").unwrap())
                .unwrap();
        };
        let (send, receive) = futures::channel::oneshot::channel();
        mounted
            .state
            .fake
            .git_checkout_results
            .borrow_mut()
            .push_back(receive);
        choose("#changes-branch select", "branch:feature");
        if mode == WorkspaceMode::Remote {
            wait_until("checkout request", || {
                !mounted.state.fake.git_checkout_requests.borrow().is_empty()
            })
            .await;
            assert_eq!(
                mounted.state.git.status.get_untracked().unwrap().branch,
                "main"
            );
            send.send(Ok(GitCheckoutResult {
                branch: "feature".into(),
                previous_branch: Some("main".into()),
                switched: true,
            }))
            .unwrap();
        }
        wait_until("shared branch selection", || {
            mounted.state.git.status.with_untracked(|status| {
                status
                    .as_ref()
                    .is_some_and(|status| status.branch == "feature")
            })
        })
        .await;
        for selector in ["#changes-branch select", "#footer-branch select"] {
            assert_eq!(
                mounted
                    .element(selector)
                    .unchecked_into::<web_sys::HtmlSelectElement>()
                    .value(),
                "branch:feature"
            );
        }
        if mode == WorkspaceMode::Remote {
            assert!(
                !mounted.state.fake.git_checkout_requests.borrow()[0]
                    .1
                    .create_if_missing
            );
        } else {
            assert!(gitCalls(&http.0).contains("checkout"));
        }
        choose("#footer-branch select", "new");
        settle().await;
        let prompt = mounted.state.ui.prompt.get_untracked().unwrap();
        assert_eq!(prompt.title, "New Git Branch");
        let (send, receive) = futures::channel::oneshot::channel();
        if mode == WorkspaceMode::Remote {
            mounted
                .state
                .fake
                .git_checkout_results
                .borrow_mut()
                .push_back(receive);
        }
        prompt.on_submit.run("new-feature".into());
        if mode == WorkspaceMode::Remote {
            send.send(Ok(GitCheckoutResult {
                branch: "new-feature".into(),
                previous_branch: Some("feature".into()),
                switched: true,
            }))
            .unwrap();
        }
        wait_until("new branch selected", || {
            mounted.state.git.status.with_untracked(|status| {
                status
                    .as_ref()
                    .is_some_and(|status| status.branch == "new-feature")
            })
        })
        .await;
        if mode == WorkspaceMode::Remote {
            assert!(
                mounted
                    .state
                    .fake
                    .git_checkout_requests
                    .borrow()
                    .last()
                    .unwrap()
                    .1
                    .create_if_missing
            );
        }
        gitChange(&http.0, "checkoutError", true);
        let (send, receive) = futures::channel::oneshot::channel();
        if mode == WorkspaceMode::Remote {
            mounted
                .state
                .fake
                .git_checkout_results
                .borrow_mut()
                .push_back(receive);
        }
        choose("#changes-branch select", "branch:main");
        if mode == WorkspaceMode::Remote {
            send.send(Err("uncommitted changes".into())).unwrap();
        }
        wait_until("checkout failure", || {
            mounted.state.ui.toast.with_untracked(|toast| {
                toast
                    .as_ref()
                    .is_some_and(|toast| toast.contains("Git checkout failed"))
            })
        })
        .await;
        assert_eq!(
            mounted.state.git.status.get_untracked().unwrap().branch,
            "new-feature"
        );
        assert!(!mounted.state.git.branch_busy.get_untracked());
        gitChange(&http.0, "branchesError", true);
        let (send, receive) = futures::channel::oneshot::channel();
        if mode == WorkspaceMode::Remote {
            mounted
                .state
                .fake
                .git_branches_results
                .borrow_mut()
                .push_back(receive);
        }
        mounted
            .element("#changes-branch select")
            .dispatch_event(&web_sys::PointerEvent::new("pointerdown").unwrap())
            .unwrap();
        if mode == WorkspaceMode::Remote {
            send.send(Err("cannot list branches".into())).unwrap();
        }
        wait_until("branch discovery failure", || {
            mounted.state.git.branches_error.get_untracked().is_some()
        })
        .await;
        assert_eq!(
            mounted.state.git.status.get_untracked().unwrap().branch,
            "new-feature"
        );
        gitChange(&http.0, "branchesError", false);
        mounted
            .element("#footer-branch select")
            .dispatch_event(&web_sys::PointerEvent::new("pointerdown").unwrap())
            .unwrap();
        wait_until("branch discovery retry", || {
            !mounted.state.git.branches_loading.get_untracked()
                && mounted.state.git.branches_error.get_untracked().is_none()
        })
        .await;
    }
    restore_token(old).await;
}

#[wasm_bindgen_test]
async fn branch_results_and_new_branch_dialog_ignore_project_and_account_changes() {
    use super::support::wait_until;
    use openwebide_core::{GitBranchInfo, GitCheckoutResult, User, UserId, UserRole};
    use openwebide_frontend::{components::BranchPicker, state_actions::git::GitActionContext};
    let (load_send, load_receive) = futures::channel::oneshot::channel();
    let slot = std::rc::Rc::new(std::cell::Cell::new(None));
    let read = slot.clone();
    let mounted = mount_test(move |state| {
        state.seed_project();
        state
            .fake
            .git_branches_results
            .borrow_mut()
            .push_back(load_receive);
        state.git.status.set(Some(GitRepoStatus {
            branch: "main".into(),
            ..GitRepoStatus::default()
        }));
        let actions = GitActions::new(GitActionContext {
            project_git: expect_context::<ProjectGit>(),
            projects: state.projects,
            workspace: state.workspace,
            git: state.git,
            chat: state.chat,
            ui: state.ui,
            workspace_for: Callback::new(|_: i64| None),
            refresh: Callback::new(|()| ()),
        });
        read.set(Some(actions));
        view! { <BranchPicker on_load=actions.on_load_branches on_select=actions.on_select_branch on_new=actions.on_branch_click /> }
    });
    wait_until("pending branch listing", || {
        mounted.state.git.branches_loading.get_untracked()
    })
    .await;
    let actions = slot.get().unwrap();
    actions.on_branch_click.run(());
    let prompt = mounted.state.ui.prompt.get_untracked().unwrap();
    mounted.state.projects.active_project.set(None);
    settle().await;
    load_send
        .send(Ok(vec![GitBranchInfo {
            name: "stale".into(),
            is_current: false,
            is_remote: false,
            upstream: None,
        }]))
        .unwrap();
    prompt.on_submit.run("wrong-project".into());
    settle().await;
    assert!(mounted.state.git.branches.get_untracked().is_empty());
    assert!(mounted.state.fake.git_checkout_requests.borrow().is_empty());
    mounted.state.projects.active_project.set(Some(1));
    settle().await;
    let (checkout_send, checkout_receive) = futures::channel::oneshot::channel();
    mounted
        .state
        .fake
        .git_checkout_results
        .borrow_mut()
        .push_back(checkout_receive);
    actions.on_select_branch.run("feature".into());
    wait_until("pending branch checkout", || {
        !mounted.state.fake.git_checkout_requests.borrow().is_empty()
    })
    .await;
    mounted.state.auth.set_user(User {
        id: UserId::new(2),
        username: "other".into(),
        role: UserRole::User,
        created_at: 0,
    });
    settle().await;
    checkout_send
        .send(Ok(GitCheckoutResult {
            branch: "feature".into(),
            previous_branch: Some("main".into()),
            switched: true,
        }))
        .unwrap();
    settle().await;
    assert_eq!(
        mounted.state.git.status.get_untracked().unwrap().branch,
        "main"
    );
    assert!(!mounted.state.git.branch_busy.get_untracked());
    assert!(mounted.state.ui.toast.get_untracked().is_none());
}
