use leptos::prelude::*;
use openwebide_core::{Project, WorkspaceMode};
use openwebide_frontend::components::TabBar;
use wasm_bindgen_test::*;

use super::support::{mount_test, settle};

#[wasm_bindgen_test]
async fn recent_list_and_empty_message_use_the_same_projects() {
    let mounted = mount_test(|state| {
        state.projects.projects.set(
            [
                (1, "Open", "/open/"),
                (2, "Hidden", "open"),
                (3, "Old", "/café/"),
                (4, "Newest", "café"),
            ]
            .into_iter()
            .map(|(id, name, path)| Project {
                id,
                name: name.into(),
                path: Some(path.into()),
                mode: WorkspaceMode::Remote,
                user_id: None,
                created_at: id,
            })
            .collect(),
        );
        state.projects.open_tab_ids.set(vec![1]);
        view! {
            <TabBar on_select=Callback::new(|_| ()) on_close=Callback::new(|_| ())
                on_open_local=Callback::new(|()| ()) on_open_remote=Callback::new(|()| ())
                on_open_project=Callback::new(move |id| { state.projects.open_tab(id); })
                on_delete_project=Callback::new(|_| ()) />
        }
    });
    mounted.click_text("Recent");
    settle().await;
    assert_eq!(mounted.element(".recent-menu").child_element_count(), 1);
    assert_eq!(
        mounted.element(".recent-name").text_content().as_deref(),
        Some("Newest")
    );
    assert!(
        mounted
            .root
            .query_selector(".recent-menu .empty")
            .unwrap()
            .is_none()
    );
    mounted.click_text("Newest");
    settle().await;
    mounted.click_text("Recent");
    settle().await;
    assert!(
        mounted
            .root
            .query_selector(".recent-item")
            .unwrap()
            .is_none()
    );
    assert_eq!(
        mounted
            .element(".recent-menu .empty")
            .text_content()
            .as_deref(),
        Some("No other saved projects.")
    );
    mounted.state.projects.open_tab_ids.set(vec![1]);
    settle().await;
    assert_eq!(mounted.element(".recent-menu").child_element_count(), 1);
    assert!(
        mounted
            .root
            .query_selector(".recent-menu .empty")
            .unwrap()
            .is_none()
    );
}

#[wasm_bindgen_test]
async fn new_chat_creates_selects_and_persists_startup_context_without_running_model() {
    let mounted = mount_test(|state| {
        state.seed_project();
        state.seed_connection();
        let actions = super::support::chat_actions(state);
        view! { <button on:click=move |_| actions.on_new_session.run(())>"New chat"</button> }
    });
    settle().await;
    mounted.click_text("New chat");
    mounted.click_text("New chat");
    settle().await;
    let session = mounted.state.chat.active_session.get_untracked().unwrap();
    assert_eq!(mounted.state.chat.sessions.get_untracked().len(), 1);
    assert!(!mounted.state.chat.creating_session.get_untracked());
    let stored = mounted.state.fake.messages.borrow();
    assert!(
        matches!(&stored[&session][0], openwebide_core::ConversationEntry::Message(message)
        if message.role == openwebide_core::Role::System && message.content.starts_with(openwebide_core::RUN_CONTEXT_PREFIX))
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
}

#[wasm_bindgen::prelude::wasm_bindgen(inline_js = r#"
export async function recoveryPicker() {
    const root = await navigator.storage.getDirectory();
    const folder = await root.getDirectoryHandle('test', {create:true});
    const wrong = await root.getDirectoryHandle('other', {create:true});
    const original = window.showDirectoryPicker;
    const fixture = { folder, kind: 'cancel', calls: [] };
    window.showDirectoryPicker = async options => {
        fixture.calls.push(options);
        if (fixture.kind === 'cancel') throw new DOMException('cancelled', 'AbortError');
        return fixture.kind === 'wrong' ? wrong : folder;
    };
    fixture.restore = () => { window.showDirectoryPicker = original; };
    return fixture;
}
export function recoveryChoose(fixture, kind) { fixture.kind = kind; }
export function recoveryRestore(fixture) { fixture.restore(); }
export function recoveryStartedAt(fixture) { return fixture.calls.at(-1).startIn === fixture.folder; }
export function recoveryDeny(fixture) { Object.defineProperty(fixture.folder, 'requestPermission', {configurable:true,value:async () => 'denied'}); }
"#)]
extern "C" {
    async fn recoveryPicker() -> wasm_bindgen::JsValue;
    fn recoveryChoose(fixture: &wasm_bindgen::JsValue, kind: &str);
    fn recoveryRestore(fixture: &wasm_bindgen::JsValue);
    fn recoveryStartedAt(fixture: &wasm_bindgen::JsValue) -> bool;
    fn recoveryDeny(fixture: &wasm_bindgen::JsValue);
}

#[wasm_bindgen_test]
async fn missing_local_folder_can_be_reconnected_cancelled_and_validated() {
    let fixture = recoveryPicker().await;
    let mounted = mount_test(|state| {
        state.seed_project();
        state.projects.projects.update(|items| {
            items[0].id = 777;
            items[0].mode = WorkspaceMode::Local;
        });
        state.projects.active_project.set(Some(777));
        let actions = openwebide_frontend::state_actions::workspace::WorkspaceActions::new(
            state.api,
            state.projects,
            state.workspace,
            state.ui,
            RwSignal::new(false),
            Callback::new(|()| ()),
        );
        actions.ensure_root.run(777);
        view! { <button on:click=move |_| actions.on_grant_access.run(())>"Grant folder access"</button> }
    });
    settle().await;
    assert!(
        mounted
            .state
            .projects
            .needs_grant
            .get_untracked()
            .contains(&777)
    );
    mounted.click_text("Grant folder access");
    settle().await;
    assert!(
        mounted
            .state
            .projects
            .local_handles
            .get_untracked()
            .is_empty()
    );
    recoveryChoose(&fixture, "wrong");
    mounted.click_text("Grant folder access");
    settle().await;
    assert!(
        mounted
            .state
            .ui
            .toast
            .get_untracked()
            .unwrap()
            .contains("original folder")
    );
    recoveryChoose(&fixture, "correct");
    mounted.click_text("Grant folder access");
    for _ in 0..100 {
        openwebide_frontend::util::sleep_ms(5).await;
        settle().await;
        if !mounted
            .state
            .projects
            .needs_grant
            .get_untracked()
            .contains(&777)
        {
            break;
        }
    }
    assert!(
        mounted
            .state
            .projects
            .local_handles
            .get_untracked()
            .contains_key(&777)
    );
    assert!(
        !mounted
            .state
            .projects
            .needs_grant
            .get_untracked()
            .contains(&777)
    );
    assert!(
        openwebide_frontend::idb::load_handle(777)
            .await
            .unwrap()
            .is_some()
    );
    recoveryDeny(&fixture);
    mounted.state.projects.needs_grant.update(|ids| {
        ids.insert(777);
    });
    mounted.click_text("Grant folder access");
    for _ in 0..100 {
        openwebide_frontend::util::sleep_ms(5).await;
        settle().await;
        if !mounted
            .state
            .projects
            .needs_grant
            .get_untracked()
            .contains(&777)
        {
            break;
        }
    }
    assert!(recoveryStartedAt(&fixture));
    assert!(
        !mounted
            .state
            .projects
            .needs_grant
            .get_untracked()
            .contains(&777)
    );
    drop(mounted);
    recoveryRestore(&fixture);
    openwebide_frontend::idb::delete_handle(777).await.unwrap();
}
