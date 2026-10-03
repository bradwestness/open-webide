use leptos::prelude::*;
use openwebide_core::{FileEntry, WorkspaceMode};
use openwebide_frontend::{state::auth::AuthState, testing::fake_backend::Call};
use wasm_bindgen::{JsCast, prelude::*};
use wasm_bindgen_test::*;

use super::support::{editor_view, mount_test, settle};

#[wasm_bindgen(inline_js = r#"
export function refreshTimers() {
    const set = window.setInterval, clear = window.clearInterval;
    const hidden = Object.getOwnPropertyDescriptor(document, 'hidden');
    const fixture = { callbacks: new Map(), next: -1, hidden: false };
    window.setInterval = (callback, delay, ...args) => {
        if (delay !== 2000) return set.call(window, callback, delay, ...args);
        const id = fixture.next--;
        fixture.callbacks.set(id, callback);
        return id;
    };
    window.clearInterval = id => {
        if (!fixture.callbacks.delete(id)) clear.call(window, id);
    };
    Object.defineProperty(document, 'hidden', { configurable: true, get: () => fixture.hidden });
    fixture.restore = () => {
        window.setInterval = set;
        window.clearInterval = clear;
        if (hidden) Object.defineProperty(document, 'hidden', hidden);
        else delete document.hidden;
    };
    return fixture;
}
export function refreshTick(fixture) { for (const callback of [...fixture.callbacks.values()]) callback(); }
export function refreshHidden(fixture, hidden) { fixture.hidden = hidden; }
export function refreshTimerCount(fixture) { return fixture.callbacks.size; }
export function restoreRefreshTimers(fixture) { fixture.restore(); }
export function localTree() {
    const tree = { entries: new Map(), calls: [], allowed: true };
    const directory = path => {
        const handle = Object.create(FileSystemDirectoryHandle.prototype);
        Object.defineProperties(handle, {
            name: { value: path.split('/').pop() || 'root' },
            kind: { value: 'directory' },
            queryPermission: { value: async () => tree.allowed ? 'granted' : 'prompt' },
            getDirectoryHandle: { value: async name => {
                const child = path ? path + '/' + name : name;
                if (tree.entries.get(child) !== 'directory') throw new DOMException('missing', 'NotFoundError');
                return directory(child);
            } },
            values: { value: () => {
                tree.calls.push(path);
                const prefix = path ? path + '/' : '';
                const children = [...tree.entries].filter(([name]) => name.startsWith(prefix) && !name.slice(prefix.length).includes('/'));
                const iterator = children.map(([name, kind]) => {
                    if (kind === 'directory') return directory(name);
                    const file = Object.create(FileSystemFileHandle.prototype);
                    Object.defineProperties(file, { name: { value: name.slice(prefix.length) }, kind: { value: 'file' }, getFile: { value: async () => new File([''], name) } });
                    return file;
                })[Symbol.iterator]();
                return { next: async () => iterator.next() };
            } }
        });
        return handle;
    };
    tree.root = directory('');
    return tree;
}
export function localRoot(tree) { return tree.root; }
export function localEntry(tree, path, kind) { if (kind) tree.entries.set(path, kind); else tree.entries.delete(path); }
export function localCalls(tree) { return JSON.stringify(tree.calls); }
export function localAllowed(tree, allowed) { tree.allowed = allowed; }
"#)]
extern "C" {
    fn refreshTimers() -> JsValue;
    fn refreshTick(fixture: &JsValue);
    fn refreshHidden(fixture: &JsValue, hidden: bool);
    fn refreshTimerCount(fixture: &JsValue) -> usize;
    fn restoreRefreshTimers(fixture: &JsValue);
    fn localTree() -> JsValue;
    fn localRoot(tree: &JsValue) -> JsValue;
    fn localEntry(tree: &JsValue, path: &str, kind: &str);
    fn localCalls(tree: &JsValue) -> String;
    fn localAllowed(tree: &JsValue, allowed: bool);
}

struct Timers(JsValue);
impl Drop for Timers {
    fn drop(&mut self) {
        restoreRefreshTimers(&self.0);
    }
}

fn file(path: &str) -> FileEntry {
    FileEntry {
        name: path.rsplit('/').next().unwrap().into(),
        path: path.into(),
        is_dir: false,
        size: 0,
    }
}

#[wasm_bindgen_test]
async fn remote_tree_refreshes_expanded_directories_and_preserves_editor() {
    let timers = Timers(refreshTimers());
    let updates = std::rc::Rc::new(std::cell::Cell::new(0));
    let observed = updates.clone();
    let mounted = mount_test(move |state| {
        state.seed_project();
        let entries = state.workspace.entries;
        Effect::new(move |_| {
            entries.track();
            observed.set(observed.get() + 1);
        });
        state
            .fake
            .files
            .borrow_mut()
            .insert((1, "src/old.rs".into()), "old".into());
        state
            .fake
            .files
            .borrow_mut()
            .insert((1, "closed/hidden.rs".into()), "hidden".into());
        state.workspace.expanded.update(|dirs| {
            dirs.insert("src".into());
        });
        editor_view(state)
    });
    settle().await;
    let workspace = mounted.state.workspace;
    workspace.open_file.set(Some("src/old.rs".into()));
    workspace.content.set("unsaved".into());
    workspace.dirty.set(true);
    refreshTick(&timers.0);
    settle().await;
    assert!(
        workspace.entries.with_untracked(
            |entries| entries.contains_key("src") && !entries.contains_key("closed")
        )
    );
    let before = updates.get();
    refreshTick(&timers.0);
    settle().await;
    assert_eq!(
        updates.get(),
        before,
        "unchanged listings must not notify the tree"
    );
    mounted
        .state
        .fake
        .files
        .borrow_mut()
        .remove(&(1, "src/old.rs".into()));
    mounted
        .state
        .fake
        .files
        .borrow_mut()
        .insert((1, "src/new.rs".into()), "new".into());
    refreshTick(&timers.0);
    settle().await;
    assert_eq!(
        workspace
            .entries
            .with_untracked(|entries| entries["src"].clone()),
        vec![file("src/new.rs")]
    );
    assert_eq!(workspace.content.get_untracked(), "unsaved");
    assert!(workspace.dirty.get_untracked());
    assert!(
        workspace
            .expanded
            .with_untracked(|dirs| dirs.contains("src"))
    );
    let (send, receive) = futures::channel::oneshot::channel();
    mounted
        .state
        .fake
        .file_list_results
        .borrow_mut()
        .push_back(receive);
    send.send(Err("temporarily unavailable".into())).unwrap();
    refreshTick(&timers.0);
    settle().await;
    assert_eq!(
        workspace
            .entries
            .with_untracked(|entries| entries["src"].clone()),
        vec![file("src/new.rs")]
    );
    mounted.state.fake.files.borrow_mut().clear();
    refreshTick(&timers.0);
    settle().await;
    assert!(
        !workspace
            .entries
            .with_untracked(|entries| entries.contains_key("src"))
    );
    drop(mounted);
    assert_eq!(refreshTimerCount(&timers.0), 0);
}

#[wasm_bindgen_test]
async fn local_tree_refreshes_external_changes_and_waits_for_permission() {
    let timers = Timers(refreshTimers());
    let tree = localTree();
    localEntry(&tree, "src", "directory");
    localEntry(&tree, "src/old.rs", "file");
    let root = localRoot(&tree).unchecked_into();
    let mounted = mount_test(move |state| {
        state.seed_project();
        state
            .projects
            .projects
            .update(|projects| projects[0].mode = WorkspaceMode::Local);
        state.projects.local_handles.update(|handles| {
            handles.insert(1, root);
        });
        state.workspace.expanded.update(|dirs| {
            dirs.insert("src".into());
        });
        editor_view(state)
    });
    settle().await;
    refreshTick(&timers.0);
    settle().await;
    assert_eq!(
        mounted
            .state
            .workspace
            .entries
            .with_untracked(|entries| entries["src"].clone()),
        vec![file("src/old.rs")]
    );
    localEntry(&tree, "src/old.rs", "");
    localEntry(&tree, "src/renamed.rs", "file");
    refreshTick(&timers.0);
    settle().await;
    assert_eq!(
        mounted
            .state
            .workspace
            .entries
            .with_untracked(|entries| entries["src"].clone()),
        vec![file("src/renamed.rs")]
    );
    assert!(
        !mounted
            .state
            .fake
            .calls
            .borrow()
            .iter()
            .any(|call| matches!(
                call,
                Call::Request {
                    method: "list_files"
                }
            ))
    );
    localAllowed(&tree, false);
    refreshTick(&timers.0);
    settle().await;
    assert!(
        mounted
            .state
            .projects
            .needs_grant
            .with_untracked(|ids| ids.contains(&1))
    );
    let before = localCalls(&tree);
    refreshTick(&timers.0);
    settle().await;
    assert_eq!(localCalls(&tree), before);
    localAllowed(&tree, true);
    mounted.state.projects.needs_grant.set(Default::default());
    localEntry(&tree, "src/new.rs", "file");
    refreshTick(&timers.0);
    settle().await;
    assert_eq!(
        mounted
            .state
            .workspace
            .entries
            .with_untracked(|entries| entries["src"].len()),
        2
    );
}

#[wasm_bindgen_test]
async fn tree_refresh_pauses_hidden_tabs_and_cancels_stale_requests() {
    let timers = Timers(refreshTimers());
    let auth_slot = std::rc::Rc::new(std::cell::Cell::new(None));
    let slot = auth_slot.clone();
    let mounted = mount_test(move |state| {
        slot.set(Some(expect_context::<AuthState>()));
        state.seed_project();
        editor_view(state)
    });
    settle().await;
    refreshHidden(&timers.0, true);
    refreshTick(&timers.0);
    settle().await;
    assert!(mounted.state.fake.calls.borrow().is_empty());
    refreshHidden(&timers.0, false);
    let (send, receive) = futures::channel::oneshot::channel();
    mounted
        .state
        .fake
        .file_list_results
        .borrow_mut()
        .push_back(receive);
    refreshTick(&timers.0);
    settle().await;
    refreshTick(&timers.0);
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
                Call::Request {
                    method: "list_files"
                }
            ))
            .count(),
        1
    );
    mounted.state.projects.active_project.set(None);
    settle().await;
    mounted.state.projects.active_project.set(Some(1));
    settle().await;
    let _ = send.send(Ok(vec![file("stale.rs")]));
    settle().await;
    assert!(
        mounted
            .state
            .workspace
            .entries
            .with_untracked(std::collections::HashMap::is_empty)
    );
    refreshTick(&timers.0);
    settle().await;
    assert_eq!(
        mounted
            .state
            .workspace
            .entries
            .with_untracked(|entries| entries[""].len()),
        0
    );
    let (send, receive) = futures::channel::oneshot::channel();
    mounted
        .state
        .fake
        .file_list_results
        .borrow_mut()
        .push_back(receive);
    refreshTick(&timers.0);
    settle().await;
    mounted.state.workspace.entries.update(|entries| {
        entries.insert("".into(), vec![file("newer.rs")]);
    });
    send.send(Ok(vec![file("older.rs")])).unwrap();
    settle().await;
    assert_eq!(
        mounted
            .state
            .workspace
            .entries
            .with_untracked(|entries| entries[""][0].path.clone()),
        "newer.rs"
    );
    let (send, receive) = futures::channel::oneshot::channel();
    mounted
        .state
        .fake
        .file_list_results
        .borrow_mut()
        .push_back(receive);
    refreshTick(&timers.0);
    settle().await;
    auth_slot.get().unwrap().logout();
    settle().await;
    let _ = send.send(Ok(vec![file("other-account.rs")]));
    settle().await;
    assert_eq!(
        mounted
            .state
            .workspace
            .entries
            .with_untracked(|entries| entries[""][0].path.clone()),
        "newer.rs"
    );
    drop(mounted);
    assert_eq!(refreshTimerCount(&timers.0), 0);
}
