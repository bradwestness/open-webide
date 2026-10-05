use super::support::{Mounted, mount_test, settle, wait_until};
use leptos::prelude::*;
use openwebide_core::{FileEntry, WorkspaceMode};
use openwebide_frontend::{
    components::{ConfirmDialog, FileTree, PromptDialog},
    state_actions::{file_tree::FileTreeActions, workspace::WorkspaceActions},
    workspace::Workspace,
};
use wasm_bindgen::{JsCast, prelude::*};
use wasm_bindgen_test::*;

#[wasm_bindgen(inline_js = r#"
export async function treeFolder() {
    const root = await navigator.storage.getDirectory();
    const name = 'tree-test-' + crypto.randomUUID();
    const handle = await root.getDirectoryHandle(name, {create:true});
    const file = await handle.getFileHandle('a.txt', {create:true});
    const writer = await file.createWritable(); await writer.write('Original'); await writer.close();
    return {root, name, handle};
}
export function treeHandle(folder) {return folder.handle;}
export async function treeCleanup(folder) {await folder.root.removeEntry(folder.name, {recursive:true});}
export function treeContext(row) {
    const box = row.getBoundingClientRect();
    row.dispatchEvent(new MouseEvent('contextmenu', {bubbles:true,cancelable:true,clientX:box.left+20,clientY:box.top+5}));
}
export function treeLongPress(row, move) {
    row.dispatchEvent(new PointerEvent('pointerdown', {bubbles:true,pointerType:'touch',clientX:40,clientY:50}));
    if (move) row.dispatchEvent(new PointerEvent('pointermove', {bubbles:true,pointerType:'touch',clientX:60,clientY:50}));
}
export function treeClipboardFallback() {
    const clipboard = Object.getOwnPropertyDescriptor(navigator, 'clipboard');
    const command = document.execCommand;
    const state = {text:null};
    Object.defineProperty(navigator, 'clipboard', {configurable:true,value:{writeText:()=>Promise.reject(new Error('denied'))}});
    document.execCommand = name => {state.text=document.activeElement.value;return name==='copy';};
    state.restore = () => {
        if (clipboard) Object.defineProperty(navigator, 'clipboard', clipboard);
        else delete navigator.clipboard;
        document.execCommand=command;
    };
    return state;
}
export function treeClipboardText(state) {return state.text;}
export function treeClipboardRestore(state) {state.restore();}
"#)]
extern "C" {
    #[wasm_bindgen(catch)]
    async fn treeFolder() -> Result<JsValue, JsValue>;
    fn treeHandle(folder: &JsValue) -> JsValue;
    #[wasm_bindgen(catch)]
    async fn treeCleanup(folder: &JsValue) -> Result<(), JsValue>;
    fn treeContext(row: &web_sys::HtmlElement);
    fn treeLongPress(row: &web_sys::HtmlElement, move_pointer: bool);
    fn treeClipboardFallback() -> JsValue;
    fn treeClipboardText(state: &JsValue) -> String;
    fn treeClipboardRestore(state: &JsValue);
}
fn file(path: &str) -> FileEntry {
    FileEntry {
        name: path.into(),
        path: path.into(),
        is_dir: false,
        size: 8,
    }
}
fn fixture(
    handle: Option<JsValue>,
) -> (
    Mounted,
    std::rc::Rc<std::cell::Cell<Option<FileTreeActions>>>,
) {
    let slot = std::rc::Rc::new(std::cell::Cell::new(None));
    let captured = slot.clone();
    let mounted = mount_test(move |state| {
        state.seed_project();
        state
            .fake
            .files
            .borrow_mut()
            .insert((1, "a.txt".into()), "Original".into());
        if let Some(handle) = handle {
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = WorkspaceMode::Local);
            state.projects.local_handles.update(|handles| {
                handles.insert(1, handle.unchecked_into());
            });
        }
        state.workspace.entries.update(|entries| {
            entries.insert("".into(), vec![file("a.txt")]);
        });
        let actions = WorkspaceActions::new(
            state.api,
            state.projects,
            state.workspace,
            state.ui,
            RwSignal::new(false),
            Callback::new(|()| ()),
        );
        captured.set(Some(expect_context::<FileTreeActions>()));
        view! {<FileTree on_toggle=actions.on_toggle on_open=actions.request_open /><PromptDialog /><ConfirmDialog />}
    });
    (mounted, slot)
}
fn choose(mounted: &Mounted, label: &str) {
    let buttons = mounted
        .root
        .query_selector_all(".ui-dropdown-menu button")
        .unwrap();
    let button = (0..buttons.length())
        .filter_map(|index| buttons.item(index))
        .filter_map(|node| node.dyn_into::<web_sys::HtmlElement>().ok())
        .find(|button| button.text_content().as_deref() == Some(label))
        .unwrap_or_else(|| panic!("Missing action {label}"));
    button.click();
}

#[wasm_bindgen_test]
async fn tree_menu_rename_dirty_guard_delete_and_stale_dialog_work_in_both_modes() {
    for local in [false, true] {
        let folder = if local {
            Some(treeFolder().await.unwrap())
        } else {
            None
        };
        let (mounted, slot) = fixture(folder.as_ref().map(treeHandle));
        settle().await;
        let actions = slot.get().unwrap();
        let files = Workspace::for_project(mounted.state.api, mounted.state.projects, 1).unwrap();
        mounted.state.workspace.open_file.set(Some("a.txt".into()));
        mounted.state.workspace.content.set("Unsaved".into());
        mounted.state.workspace.dirty.set(true);
        actions.move_entry(&file("a.txt"), true);
        settle().await;
        assert!(mounted.state.ui.prompt.get_untracked().is_none());
        assert!(
            mounted
                .state
                .ui
                .toast
                .get_untracked()
                .unwrap()
                .contains("unsaved")
        );
        assert_eq!(files.read("a.txt").await.unwrap(), "Original");
        mounted.state.workspace.dirty.set(false);
        let row = mounted.element(".tree-item");
        assert_eq!(
            row.get_attribute("data-tree-path").as_deref(),
            Some("a.txt"),
            "{}",
            row.outer_html()
        );
        treeContext(&row);
        settle().await;
        assert!(
            mounted
                .root
                .query_selector(".ui-dropdown-menu")
                .unwrap()
                .is_some()
        );
        choose(&mounted, "Rename");
        settle().await;
        assert!(
            mounted
                .root
                .query_selector(".ui-dropdown-menu")
                .unwrap()
                .is_none()
        );
        mounted
            .state
            .ui
            .prompt
            .get_untracked()
            .unwrap()
            .on_submit
            .run("renamed.txt".into());
        wait_until("rename completion", || !actions.busy.get_untracked()).await;
        let renamed = files.read("renamed.txt").await;
        assert!(
            renamed.is_ok(),
            "Rename failed: toast={:?}, entries={:?}, local={local}",
            mounted.state.ui.toast.get_untracked(),
            files.list("").await.unwrap()
        );
        assert_eq!(renamed.unwrap(), "Original");
        assert!(
            !files
                .list("")
                .await
                .unwrap()
                .iter()
                .any(|entry| entry.path == "a.txt")
        );
        assert_eq!(
            mounted.state.workspace.open_file.get_untracked().as_deref(),
            Some("renamed.txt")
        );
        // A dialog opened in the old scope must not mutate any project after a switch.
        actions.move_entry(&file("renamed.txt"), true);
        let pending = mounted.state.ui.prompt.get_untracked().unwrap();
        mounted.state.projects.active_project.set(None);
        settle().await;
        pending.on_submit.run("stale.txt".into());
        settle().await;
        assert!(
            !files
                .list("")
                .await
                .unwrap()
                .iter()
                .any(|entry| entry.path == "stale.txt")
        );
        mounted.state.projects.active_project.set(Some(1));
        settle().await;
        mounted
            .state
            .workspace
            .open_file
            .set(Some("renamed.txt".into()));
        mounted.state.workspace.dirty.set(true);
        actions.delete(&file("renamed.txt"));
        settle().await;
        let confirm = mounted.state.ui.confirm.get_untracked().unwrap();
        assert!(confirm.message.contains("Unsaved"));
        assert_eq!(files.read("renamed.txt").await.unwrap(), "Original");
        confirm.action.run(());
        wait_until("delete completion", || !actions.busy.get_untracked()).await;
        assert!(files.list("").await.unwrap().is_empty());
        assert!(mounted.state.workspace.open_file.get_untracked().is_none());
        assert!(!mounted.state.workspace.dirty.get_untracked());
        if let Some(folder) = folder {
            treeCleanup(&folder).await.unwrap();
        }
        drop(mounted);
    }
}

#[wasm_bindgen_test]
async fn tree_menu_keyboard_touch_dismissal_and_editable_chat_prompts_in_both_modes() {
    for local in [false, true] {
        let folder = if local {
            Some(treeFolder().await.unwrap())
        } else {
            None
        };
        let (mounted, _) = fixture(folder.as_ref().map(treeHandle));
        settle().await;
        let row = mounted.element(".tree-item");
        row.focus().unwrap();
        let init = web_sys::KeyboardEventInit::new();
        init.set_key("F10");
        init.set_shift_key(true);
        init.set_bubbles(true);
        init.set_cancelable(true);
        row.dispatch_event(
            &web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap(),
        )
        .unwrap();
        settle().await;
        let menu = mounted.element(".ui-dropdown-menu");
        assert!(
            menu.contains(
                web_sys::window()
                    .unwrap()
                    .document()
                    .unwrap()
                    .active_element()
                    .as_ref()
                    .map(AsRef::as_ref)
            )
        );
        mounted.state.chat.draft.set("Existing question".into());
        choose(&mounted, "Explain in chat");
        settle().await;
        assert_eq!(
            mounted.state.chat.draft.get_untracked(),
            "Existing question\n\nExplain @file:\"a.txt\""
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
                    openwebide_frontend::testing::fake_backend::Call::SendMessage { .. }
                ))
        );
        treeLongPress(&row, true);
        openwebide_frontend::util::sleep_ms(550).await;
        settle().await;
        assert!(
            mounted
                .root
                .query_selector(".ui-dropdown-menu")
                .unwrap()
                .is_none()
        );
        treeLongPress(&row, false);
        openwebide_frontend::util::sleep_ms(550).await;
        settle().await;
        assert!(
            mounted
                .root
                .query_selector(".ui-dropdown-menu")
                .unwrap()
                .is_some()
        );
        mounted.click(".ui-dropdown-backdrop");
        settle().await;
        assert!(
            mounted
                .root
                .query_selector(".ui-dropdown-menu")
                .unwrap()
                .is_none()
        );
        assert!(mounted.root.query_selector(".tree-item").unwrap().is_some());
        if let Some(folder) = folder {
            treeCleanup(&folder).await.unwrap();
        }
        drop(mounted);
    }
}

#[wasm_bindgen_test]
async fn tree_folder_create_move_ignore_and_collision_contract_in_both_modes() {
    use openwebide_core::vfs::VfsEntryKind;
    for local in [false, true] {
        let folder = if local {
            Some(treeFolder().await.unwrap())
        } else {
            None
        };
        let (mounted, slot) = fixture(folder.as_ref().map(treeHandle));
        settle().await;
        let actions = slot.get().unwrap();
        let files = Workspace::for_project(mounted.state.api, mounted.state.projects, 1).unwrap();
        actions.create("", VfsEntryKind::Directory);
        mounted
            .state
            .ui
            .prompt
            .get_untracked()
            .unwrap()
            .on_submit
            .run("src/empty".into());
        wait_until("folder creation", || !actions.busy.get_untracked()).await;
        assert!(
            files
                .list("src")
                .await
                .unwrap()
                .iter()
                .any(|entry| entry.path == "src/empty" && entry.is_dir)
        );
        actions.create("src", VfsEntryKind::File);
        mounted
            .state
            .ui
            .prompt
            .get_untracked()
            .unwrap()
            .on_submit
            .run("src/new.txt".into());
        wait_until("file creation", || !actions.busy.get_untracked()).await;
        files.write("src/new.txt", "Nested data").await.unwrap();
        files
            .write_bytes("src/bytes.bin", &[0, 255, 128, 10])
            .await
            .unwrap();
        // Move a whole folder, preserving empty children and the open file path.
        mounted
            .state
            .workspace
            .open_file
            .set(Some("src/new.txt".into()));
        mounted.state.workspace.dirty.set(false);
        let directory = FileEntry {
            name: "src".into(),
            path: "src".into(),
            is_dir: true,
            size: 0,
        };
        actions.move_entry(&directory, false);
        mounted
            .state
            .ui
            .prompt
            .get_untracked()
            .unwrap()
            .on_submit
            .run("lib".into());
        wait_until("folder move", || !actions.busy.get_untracked()).await;
        assert_eq!(files.read("lib/new.txt").await.unwrap(), "Nested data");
        assert!(files.list("lib/empty").await.unwrap().is_empty());
        assert_eq!(
            files.read_bytes("lib/bytes.bin").await.unwrap(),
            [0, 255, 128, 10]
        );
        assert_eq!(
            mounted.state.workspace.open_file.get_untracked().as_deref(),
            Some("lib/new.txt")
        );
        // Existing destinations are never overwritten, including folders.
        actions.move_entry(&file("a.txt"), false);
        mounted
            .state
            .ui
            .prompt
            .get_untracked()
            .unwrap()
            .on_submit
            .run("lib/new.txt".into());
        wait_until("collision result", || !actions.busy.get_untracked()).await;
        assert!(
            mounted
                .state
                .ui
                .toast
                .get_untracked()
                .unwrap()
                .contains("already exists")
        );
        assert_eq!(files.read("a.txt").await.unwrap(), "Original");
        assert_eq!(files.read("lib/new.txt").await.unwrap(), "Nested data");
        files.write(".gitignore", "# Keep\r\n/cache").await.unwrap();
        mounted.state.workspace.pending_edits.update(|edits| {
            edits.insert(
                ".gitignore".into(),
                openwebide_core::FileDiff {
                    path: ".gitignore".into(),
                    old: Some("# Keep\r\n/cache".into()),
                    new: "pending review".into(),
                    old_unavailable: false,
                    backup_path: None,
                },
            );
        });
        actions.ignore(&file("lib/new.txt"));
        settle().await;
        assert_eq!(files.read(".gitignore").await.unwrap(), "# Keep\r\n/cache");
        assert!(
            mounted
                .state
                .ui
                .toast
                .get_untracked()
                .unwrap()
                .contains("pending edits")
        );
        mounted
            .state
            .workspace
            .pending_edits
            .set(Default::default());
        actions.ignore(&FileEntry {
            name: "lib".into(),
            path: "lib".into(),
            is_dir: true,
            size: 0,
        });
        wait_until("ignore rule", || !actions.busy.get_untracked()).await;
        assert_eq!(
            files.read(".gitignore").await.unwrap(),
            "# Keep\r\n/cache\r\n/lib/\r\n"
        );
        // Local operations must leave the fake remote filesystem alone.
        if local {
            assert_eq!(
                mounted
                    .state
                    .fake
                    .files
                    .borrow()
                    .get(&(1, "a.txt".into()))
                    .unwrap(),
                "Original"
            );
        }
        if let Some(folder) = folder {
            treeCleanup(&folder).await.unwrap();
        }
        drop(mounted);
    }
}

#[wasm_bindgen_test]
async fn copy_path_falls_back_after_clipboard_denial_and_restores_focus() {
    let (mounted, slot) = fixture(None);
    settle().await;
    let row = mounted.element(".tree-item");
    row.focus().unwrap();
    let clipboard = treeClipboardFallback();
    slot.get().unwrap().copy_path("nested/café file.txt");
    settle().await;
    let copied = treeClipboardText(&clipboard);
    treeClipboardRestore(&clipboard);
    assert_eq!(copied, "nested/café file.txt");
    assert!(
        document()
            .active_element()
            .unwrap()
            .is_same_node(Some(row.as_ref()))
    );
    assert!(mounted.state.ui.toast.get_untracked().is_none());
}

#[wasm_bindgen_test]
async fn tree_dialogs_abort_after_account_folder_and_bridge_changes() {
    use openwebide_core::vfs::VfsEntryKind;
    let folder = treeFolder().await.unwrap();
    let (mounted, slot) = fixture(Some(treeHandle(&folder)));
    settle().await;
    let actions = slot.get().unwrap();
    let files = Workspace::for_project(mounted.state.api, mounted.state.projects, 1).unwrap();
    actions.create("", VfsEntryKind::File);
    let pending = mounted.state.ui.prompt.get_untracked().unwrap();
    mounted
        .state
        .projects
        .projects
        .update(|projects| projects[0].path = Some("verified/bridge/path".into()));
    pending.on_submit.run("metadata-change.txt".into());
    wait_until("creation after local discovery metadata changes", || {
        !actions.busy.get_untracked()
    })
    .await;
    assert_eq!(files.read("metadata-change.txt").await.unwrap(), "");
    for change in 0..3 {
        actions.create("", VfsEntryKind::File);
        let pending = mounted.state.ui.prompt.get_untracked().unwrap();
        match change {
            0 => mounted
                .state
                .auth
                .generation
                .update(|generation| *generation += 1),
            1 => mounted.state.projects.local_handles.update(|handles| {
                handles.remove(&1);
            }),
            _ => mounted
                .state
                .settings
                .bridge_url
                .set("ws://different.test:3001".into()),
        }
        // Do not settle: guards must detect changes before any Effect runs.
        pending.on_submit.run(format!("stale-{change}"));
        settle().await;
        assert!(
            !files
                .list("")
                .await
                .unwrap()
                .iter()
                .any(|entry| entry.path == format!("stale-{change}"))
        );
        mounted.state.projects.local_handles.update(|handles| {
            handles.insert(1, treeHandle(&folder).unchecked_into());
        });
        settle().await;
    }
    treeCleanup(&folder).await.unwrap();
    drop(mounted);
}

fn git_response(mounted: &Mounted, response: Result<openwebide_core::git::GitPathChanges, String>) {
    let (sender, receiver) = futures::channel::oneshot::channel();
    mounted
        .state
        .fake
        .git_path_results
        .borrow_mut()
        .push_back(receiver);
    sender.send(response).unwrap();
}
#[wasm_bindgen_test]
async fn tree_git_actions_share_both_adapters_and_report_failures() {
    use super::project_git::{Http, gitCalls, gitChange, gitHttp, restore_token, token};
    use openwebide_core::git::{GitPathAction, GitPathChanges};
    let old = token().await;
    let http = Http(gitHttp());
    for local in [false, true] {
        let folder = if local {
            Some(treeFolder().await.unwrap())
        } else {
            None
        };
        let (mounted, slot) = fixture(folder.as_ref().map(treeHandle));
        if local {
            mounted
                .state
                .settings
                .bridge_url
                .set("ws://git.test:3001".into());
        }
        settle().await;
        let actions = slot.get().unwrap();
        let changed = GitPathChanges {
            has_head: true,
            staged: ["a.txt".into()].into(),
            unstaged: ["a.txt".into()].into(),
            ..Default::default()
        };
        if !local {
            git_response(&mounted, Ok(changed.clone()));
        }
        treeContext(&mounted.element(".tree-item"));
        wait_until("Git menu discovery", || {
            mounted
                .root
                .query_selector_all(".ui-dropdown-menu button:not(:disabled)")
                .unwrap()
                .length()
                >= 12
        })
        .await;
        if !local {
            git_response(&mounted, Ok(changed.clone()));
            git_response(&mounted, Ok(GitPathChanges::default()));
        }
        choose(&mounted, "Stage");
        wait_until("Git stage", || !actions.busy.get_untracked()).await;
        for action in [GitPathAction::Unstage, GitPathAction::Revert] {
            if !local {
                git_response(&mounted, Ok(changed.clone()));
                git_response(&mounted, Ok(GitPathChanges::default()));
            }
            actions.git_action("a.txt", action);
            if action == GitPathAction::Revert {
                let confirm = mounted.state.ui.confirm.get_untracked().unwrap();
                assert!(confirm.message.contains("Untracked files are preserved"));
                confirm.action.run(());
            }
            wait_until("Git action", || !actions.busy.get_untracked()).await;
        }
        if local {
            gitChange(&http.0, "pathError", true);
        } else {
            git_response(&mounted, Ok(changed.clone()));
            git_response(&mounted, Err("path action failed".into()));
        }
        actions.git_action("a.txt", GitPathAction::Stage);
        wait_until("Git action error", || !actions.busy.get_untracked()).await;
        assert!(
            mounted
                .state
                .ui
                .toast
                .get_untracked()
                .unwrap()
                .contains("path action failed")
        );
        if local {
            assert!(mounted.state.fake.git_path_requests.borrow().is_empty());
            let calls: Vec<serde_json::Value> = serde_json::from_str(&gitCalls(&http.0)).unwrap();
            let requests = calls
                .iter()
                .filter(|call| call["path"] == "/git/path")
                .collect::<Vec<_>>();
            assert_eq!(requests.len(), 4);
            for request in requests {
                assert_eq!(request["authorization"], "Bearer git-test-token");
                assert_eq!(request["body"]["cwd"], "repos/local");
                assert_eq!(request["body"]["path"], "a.txt");
            }
            gitChange(&http.0, "pathError", false);
        } else {
            let requests = mounted.state.fake.git_path_requests.borrow();
            assert_eq!(requests.len(), 4);
            assert!(
                requests
                    .iter()
                    .all(|(project, request)| *project == Some(1) && request.path == "a.txt")
            );
        }
        if let Some(folder) = folder {
            treeCleanup(&folder).await.unwrap();
        }
        drop(mounted);
    }
    restore_token(old).await;
}
