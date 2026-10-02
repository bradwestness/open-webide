use leptos::prelude::*;
use openwebide_frontend::idb;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_test::*;

#[wasm_bindgen(inline_js = r#"
let originalNow;
let originalOpen;
let count;
let fixtureName;
let failName;
let failNext = false;
let originalPut;
let originalDelete;
let abortNext = false;
export function beginFixture() {
    originalNow = Date.now;
    originalOpen = indexedDB.open;
    count = 0;
    fixtureName = 'openwebide-idb-test-' + crypto.randomUUID();
    indexedDB.open = function(name, version) {
        if (name === 'openwebide') { name = failNext ? failName : fixtureName; failNext = false; count++; }
        return originalOpen.call(this, name, version);
    };
}
export function endFixture() {
    Date.now = originalNow;
    indexedDB.open = originalOpen;
    if (originalPut) IDBObjectStore.prototype.put = originalPut;
    originalPut = undefined;
    if (originalDelete) IDBObjectStore.prototype.delete = originalDelete;
    originalDelete = undefined;
    abortNext = false;
    failNext = false;
    if (failName) indexedDB.deleteDatabase(failName);
    indexedDB.deleteDatabase(fixtureName);
}
export async function failOpen() {
    failName = fixtureName + '-fail';
    await new Promise((resolve, reject) => {
        const request = originalOpen.call(indexedDB, failName, 2);
        request.onsuccess = () => { request.result.close(); resolve(); };
        request.onerror = () => reject(request.error);
    });
    failNext = true;
}
export function abortWrite() {
    if (!originalPut) {
        originalPut = IDBObjectStore.prototype.put;
        IDBObjectStore.prototype.put = function(...args) {
            const request = originalPut.apply(this, args);
            if (abortNext) {
                abortNext = false;
                const transaction = this.transaction;
                request.addEventListener('success', () => transaction.abort());
            }
            return request;
        };
    }
    abortNext = true;
}
export function abortDelete() {
    originalDelete = IDBObjectStore.prototype.delete;
    IDBObjectStore.prototype.delete = function(...args) {
        const request = originalDelete.apply(this, args);
        const transaction = this.transaction;
        request.addEventListener('success', () => transaction.abort(), {once: true});
        return request;
    };
}
export async function directoryHandle() { return await navigator.storage.getDirectory(); }
export function resetCount() { count = 0; }
export async function seedKeys(db) {
    await new Promise((resolve, reject) => {
        const tx = db.transaction('directories', 'readwrite');
        const store = tx.objectStore('directories');
        for (const id of [1, 2, 3]) store.put({projectId: id, userId: 1, revision: "seed"}, id);
        tx.oncomplete = resolve;
        tx.onabort = () => reject(tx.error);
    });
}
export async function hasKey(db, key) {
    return await new Promise((resolve, reject) => {
        const request = db.transaction('directories').objectStore('directories').getKey(key);
        request.onsuccess = () => resolve(request.result !== undefined);
        request.onerror = () => reject(request.error);
    });
}
export async function upgradeFixture() {
    await new Promise((resolve, reject) => {
        const request = originalOpen.call(indexedDB, fixtureName, 2);
        request.onsuccess = () => { request.result.close(); resolve(); };
        request.onerror = () => reject(request.error);
    });
}
export function correctClockBackward() { Date.now = () => originalNow() - 60000; }
export function restoreClock() { Date.now = originalNow; }
export function openCount() { return count; }
"#)]
extern "C" {
    #[wasm_bindgen(js_name = beginFixture)]
    fn begin_fixture();
    #[wasm_bindgen(js_name = endFixture)]
    fn end_fixture();
    #[wasm_bindgen(js_name = correctClockBackward)]
    fn correct_clock_backward();
    #[wasm_bindgen(js_name = restoreClock)]
    fn restore_clock();
    #[wasm_bindgen(js_name = openCount)]
    fn open_count() -> u32;
    #[wasm_bindgen(js_name = failOpen)]
    async fn fail_open();
    #[wasm_bindgen(js_name = abortWrite)]
    fn abort_write();
    #[wasm_bindgen(js_name = abortDelete)]
    fn abort_delete();
    #[wasm_bindgen(js_name = directoryHandle)]
    async fn directory_handle() -> JsValue;
    #[wasm_bindgen(js_name = resetCount)]
    fn reset_count();
    #[wasm_bindgen(js_name = seedKeys)]
    async fn seed_keys(db: &web_sys::IdbDatabase);
    #[wasm_bindgen(js_name = hasKey)]
    async fn has_key(db: &web_sys::IdbDatabase, key: u32) -> JsValue;
    #[wasm_bindgen(js_name = upgradeFixture)]
    async fn upgrade_fixture();
}

struct Fixture;
impl Fixture {
    fn new() -> Self {
        idb::reset_connection();
        begin_fixture();
        Self
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        idb::reset_connection();
        end_fixture();
    }
}

#[wasm_bindgen_test]
async fn handle_load_measurement() {
    let _fixture = Fixture::new();
    let handle = directory_handle().await.unchecked_into();
    for id in 1..=20 {
        idb::save_handle(id, Some(openwebide_core::UserId::new(1)), &handle)
            .await
            .unwrap();
    }
    idb::reset_connection();
    reset_count();
    let mut timings = Vec::new();
    for _ in 0..5 {
        let start = js_sys::Date::now();
        for id in 1..=20 {
            assert!(idb::load_handle(id).await.unwrap().is_some());
        }
        for _ in 0..10 {
            assert!(idb::get_bridge_pairing_token().await.unwrap().is_none());
        }
        timings.push(js_sys::Date::now() - start);
    }
    wasm_bindgen_test::console_log!(
        "IDB 20 handle loads + 10 settings reads: opens={}, elapsed_ms={:?}",
        open_count(),
        timings
    );
}

#[wasm_bindgen_test]
async fn concurrent_opens_and_repeated_reads_share_one_connection() {
    let _fixture = Fixture::new();
    let databases = futures::future::join_all((0..20).map(|_| idb::open_db())).await;
    let first = databases[0].as_ref().unwrap();
    for database in &databases {
        assert_eq!(database.as_ref().unwrap(), first);
    }
    idb::get_bridge_pairing_token().await.unwrap();
    assert_eq!(open_count(), 1);
}

#[wasm_bindgen_test]
async fn failed_open_retries_and_cancelled_waiter_does_not_poison_cache() {
    let _fixture = Fixture::new();
    fail_open().await;
    assert!(idb::open_db().await.is_err());
    let mut abandoned = Box::pin(idb::open_db());
    assert!(futures::poll!(abandoned.as_mut()).is_pending());
    drop(abandoned);
    idb::open_db().await.unwrap();
    assert_eq!(open_count(), 2);
}

#[wasm_bindgen_test]
async fn close_invalidates_and_versionchange_unblocks_upgrade() {
    let _fixture = Fixture::new();
    let database = idb::open_db().await.unwrap();
    database
        .dispatch_event(&web_sys::Event::new("close").unwrap())
        .unwrap();
    super::support::settle().await;
    idb::open_db().await.unwrap();
    assert_eq!(open_count(), 2);
    upgrade_fixture().await;
    super::support::settle().await;
    // Version 2 belongs to another build; this build must reopen rather than use a closed DB.
    assert!(idb::open_db().await.is_err());
    assert_eq!(open_count(), 3);
}

#[wasm_bindgen_test]
async fn directory_deletion_and_orphan_cleanup_keep_the_pairing_token() {
    let _fixture = Fixture::new();
    let database = idb::open_db().await.unwrap();
    seed_keys(&database).await;
    idb::set_bridge_pairing_token("paired").await.unwrap();
    idb::delete_handle(1).await.unwrap();
    idb::delete_handle(1).await.unwrap();
    let user = openwebide_core::UserId::new(1);
    let candidates = idb::orphan_candidates(user).await.unwrap();
    idb::delete_orphan_handles(user, &candidates, &[2], || true)
        .await
        .unwrap();
    idb::reset_connection();
    let database = idb::open_db().await.unwrap();
    assert_eq!(has_key(&database, 1).await.as_bool(), Some(false));
    assert_eq!(has_key(&database, 2).await.as_bool(), Some(true));
    assert_eq!(has_key(&database, 3).await.as_bool(), Some(false));
    assert_eq!(
        idb::get_bridge_pairing_token().await.unwrap().as_deref(),
        Some("paired")
    );
}

#[wasm_bindgen_test]
async fn abort_after_request_success_is_reported_and_not_committed() {
    let _fixture = Fixture::new();
    idb::set_bridge_pairing_token("original").await.unwrap();
    abort_write();
    assert!(idb::set_bridge_pairing_token("aborted").await.is_err());
    assert_eq!(
        idb::get_bridge_pairing_token().await.unwrap().as_deref(),
        Some("original")
    );
}

fn project_actions(
    state: &super::support::TestState,
) -> openwebide_frontend::state_actions::projects::ProjectsActions {
    use openwebide_frontend::state_actions::projects::{
        ProjectsActionContext, build_projects_actions,
    };
    build_projects_actions(ProjectsActionContext {
        api: state.api,
        projects: state.projects,
        workspace: state.workspace,
        git: state.git,
        chat: state.chat,
        ui: state.ui,
        ensure_root: Callback::new(|_| ()),
        refresh_git: Callback::new(|()| ()),
    })
}

#[wasm_bindgen_test]
async fn project_deletion_cleans_up_only_after_backend_success_and_reports_cleanup_failure() {
    for outcome in ["success", "backend failure", "cleanup failure"] {
        let _fixture = Fixture::new();
        let database = idb::open_db().await.unwrap();
        seed_keys(&database).await;
        let mounted = super::support::mount_test(move |state| {
            state.seed_project();
            if outcome == "backend failure" {
                *state.fake.project_delete_error.borrow_mut() = Some("backend unavailable".into());
            }
            if outcome == "cleanup failure" {
                abort_delete();
            }
            project_actions(&state).on_delete_project.run(1);
            state.ui.confirm.get_untracked().unwrap().action.run(());
            view! { <div /> }
        });
        // A readonly transaction runs after the queued deletion, including its abort.
        super::support::settle().await;
        let database = idb::open_db().await.unwrap();
        let retained = has_key(&database, 1).await.as_bool().unwrap();
        assert_eq!(retained, outcome != "success");
        assert_eq!(
            mounted.state.projects.projects.get_untracked().is_empty(),
            outcome != "backend failure"
        );
        assert_eq!(
            mounted.state.ui.toast.get_untracked().is_some(),
            outcome != "success"
        );
    }
}

#[wasm_bindgen_test]
async fn startup_reconciles_only_a_successful_project_list() {
    use openwebide_frontend::{
        state::{auth::AuthState, layout::LayoutState},
        state_actions::lifecycle::{ProjectEffectContext, install_project_effects},
    };
    for fail in [false, true] {
        let _fixture = Fixture::new();
        let database = idb::open_db().await.unwrap();
        seed_keys(&database).await;
        let mounted = super::support::mount_test(move |state| {
            state.seed_project();
            if fail {
                let (tx, rx) = futures::channel::oneshot::channel();
                tx.send(Err("unavailable".into())).unwrap();
                state.fake.project_results.borrow_mut().push_back(rx);
            }
            let auth = expect_context::<AuthState>();
            auth.set_user(openwebide_core::User {
                id: openwebide_core::UserId::new(1),
                username: "test".into(),
                role: openwebide_core::UserRole::User,
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
            view! { <div /> }
        });
        for _ in 0..100 {
            if mounted.state.projects.projects_loaded.get_untracked() {
                break;
            }
            openwebide_frontend::util::sleep_ms(5).await;
        }
        assert!(mounted.state.projects.projects_loaded.get_untracked());
        let database = idb::open_db().await.unwrap();
        if !fail {
            for _ in 0..100 {
                if has_key(&database, 3).await.as_bool() == Some(false) {
                    break;
                }
                openwebide_frontend::util::sleep_ms(5).await;
            }
        }
        assert_eq!(has_key(&database, 1).await.as_bool(), Some(true));
        assert_eq!(has_key(&database, 2).await.as_bool(), Some(fail));
        assert_eq!(has_key(&database, 3).await.as_bool(), Some(fail));
    }
}

fn install_startup(
    state: &super::support::TestState,
) -> openwebide_frontend::state::auth::AuthState {
    use openwebide_frontend::{
        state::{auth::AuthState, layout::LayoutState},
        state_actions::lifecycle::{ProjectEffectContext, install_project_effects},
    };
    let auth = expect_context::<AuthState>();
    auth.set_user(openwebide_core::User {
        id: openwebide_core::UserId::new(1),
        username: "test".into(),
        role: openwebide_core::UserRole::User,
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
    auth
}

async fn wait_for_cleanup(database: &web_sys::IdbDatabase, deleted_key: u32) {
    for _ in 0..100 {
        if has_key(database, deleted_key).await.as_bool() == Some(false) {
            return;
        }
        openwebide_frontend::util::sleep_ms(5).await;
    }
    panic!("owned orphan was not deleted");
}

#[wasm_bindgen_test]
async fn account_switch_preserves_other_account_and_unknown_handles() {
    let _fixture = Fixture::new();
    let handle = directory_handle().await.unchecked_into();
    idb::save_handle(2, Some(openwebide_core::UserId::new(2)), &handle)
        .await
        .unwrap();
    idb::save_handle(3, None, &handle).await.unwrap();
    idb::save_handle(4, Some(openwebide_core::UserId::new(1)), &handle)
        .await
        .unwrap();
    idb::set_bridge_pairing_token("paired").await.unwrap();
    openwebide_frontend::util::sleep_ms(5).await;
    let auth_slot = std::rc::Rc::new(std::cell::Cell::new(None));
    let slot = auth_slot.clone();
    let mounted = super::support::mount_test(move |state| {
        state.seed_project();
        let mut alice_project = state.fake.projects.borrow()[0].clone();
        alice_project.id = 2;
        *state.fake.projects.borrow_mut() = vec![alice_project];
        let auth = install_startup(&state);
        auth.set_user(openwebide_core::User {
            id: openwebide_core::UserId::new(2),
            username: "alice".into(),
            role: openwebide_core::UserRole::User,
            created_at: 0,
        });
        slot.set(Some(auth));
        view! { <div /> }
    });
    for _ in 0..100 {
        if mounted.state.projects.projects_loaded.get_untracked() {
            break;
        }
        openwebide_frontend::util::sleep_ms(5).await;
    }
    assert!(mounted.state.projects.projects_loaded.get_untracked());
    assert!(idb::load_handle(2).await.unwrap().is_some());
    let mut bob_project = mounted.state.fake.projects.borrow()[0].clone();
    bob_project.id = 1;
    *mounted.state.fake.projects.borrow_mut() = vec![bob_project];
    let auth = auth_slot.get().unwrap();
    auth.logout();
    super::support::settle().await;
    auth.set_user(openwebide_core::User {
        id: openwebide_core::UserId::new(1),
        username: "bob".into(),
        role: openwebide_core::UserRole::User,
        created_at: 0,
    });
    let database = idb::open_db().await.unwrap();
    wait_for_cleanup(&database, 4).await;
    assert!(mounted.state.projects.projects_loaded.get_untracked());
    idb::reset_connection();
    assert!(idb::load_handle(2).await.unwrap().is_some());
    assert!(idb::load_handle(3).await.unwrap().is_some());
    assert_eq!(
        idb::get_bridge_pairing_token().await.unwrap().as_deref(),
        Some("paired")
    );
}

#[wasm_bindgen_test]
async fn delayed_startup_list_preserves_new_and_replaced_handles() {
    for (replace, clock_correction) in [(false, false), (true, false), (false, true), (true, true)]
    {
        let _fixture = Fixture::new();
        let user = Some(openwebide_core::UserId::new(1));
        let handle = directory_handle().await.unchecked_into();
        if replace {
            idb::save_handle(2, user, &handle).await.unwrap();
        }
        idb::save_handle(3, user, &handle).await.unwrap();
        openwebide_frontend::util::sleep_ms(5).await;
        let (tx, rx) = futures::channel::oneshot::channel();
        let mounted = super::support::mount_test(move |state| {
            state.seed_project();
            state.fake.project_results.borrow_mut().push_back(rx);
            install_startup(&state);
            view! { <div /> }
        });
        for _ in 0..100 {
            if mounted.state.fake.calls.borrow().iter().any(|call| {
                matches!(
                    call,
                    openwebide_frontend::testing::fake_backend::Call::Request {
                        method: "list_projects"
                    }
                )
            }) {
                break;
            }
            openwebide_frontend::util::sleep_ms(5).await;
        }
        assert!(mounted.state.fake.project_results.borrow().is_empty());
        let snapshot = mounted.state.fake.projects.borrow().clone();
        let mut project = snapshot[0].clone();
        project.id = 2;
        project.mode = openwebide_core::WorkspaceMode::Local;
        project.user_id = user;
        mounted.state.fake.projects.borrow_mut().push(project);
        // An independent connection models a second tab saving after the request began.
        idb::reset_connection();
        if clock_correction {
            correct_clock_backward();
        }
        idb::save_handle(2, user, &handle).await.unwrap();
        restore_clock();
        tx.send(Ok(snapshot)).unwrap();
        let database = idb::open_db().await.unwrap();
        wait_for_cleanup(&database, 3).await;
        assert!(
            mounted
                .state
                .fake
                .projects
                .borrow()
                .iter()
                .any(|project| project.id == 2)
        );
        idb::reset_connection();
        assert!(idb::load_handle(2).await.unwrap().is_some());
    }
}

#[wasm_bindgen_test]
async fn auth_generation_change_during_reconciliation_retains_handle() {
    let _fixture = Fixture::new();
    let user = openwebide_core::UserId::new(1);
    let handle = directory_handle().await.unchecked_into();
    idb::save_handle(2, Some(user), &handle).await.unwrap();
    let candidates = idb::orphan_candidates(user).await.unwrap();
    let current = std::cell::Cell::new(true);
    let checks = std::cell::Cell::new(0);
    idb::delete_orphan_handles(user, &candidates, &[], || {
        checks.set(checks.get() + 1);
        if checks.get() == 2 {
            current.set(false);
        }
        current.get()
    })
    .await
    .unwrap();
    assert_eq!(checks.get(), 2);
    idb::reset_connection();
    assert!(idb::load_handle(2).await.unwrap().is_some());
}
