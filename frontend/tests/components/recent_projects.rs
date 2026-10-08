use leptos::prelude::*;
use openwebide_core::{Project, WorkspaceMode};
use openwebide_frontend::components::{TabBar, TopBar};
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
            <TopBar on_open_settings=Callback::new(|()| ()) on_logout=Callback::new(|()| ()) on_open_project=Callback::new(move |id| { state.projects.open_tab(id); }) />
        }
    });
    mounted.click("[aria-label='App menu']");
    settle().await;
    assert_eq!(
        mounted
            .element(".recent-menu .recent-project-row")
            .child_element_count(),
        2
    );
    assert_eq!(
        mounted
            .element(".recent-project-row .recent-name")
            .text_content()
            .as_deref(),
        Some("Newest")
    );
    assert!(
        mounted
            .root
            .query_selector(".recent-project-row")
            .unwrap()
            .is_some()
    );
    mounted.click_text("Newest");
    settle().await;
    mounted.click("[aria-label='App menu']");
    settle().await;
    assert!(
        mounted
            .root
            .query_selector(".recent-project-row")
            .unwrap()
            .is_none()
    );
    assert_eq!(
        mounted
            .element(".recent-menu p.form-hint")
            .text_content()
            .as_deref(),
        Some("No other saved projects.")
    );
    mounted.state.projects.open_tab_ids.set(vec![1]);
    settle().await;
    assert_eq!(
        mounted
            .element(".recent-menu .recent-project-row")
            .child_element_count(),
        2
    );
    assert!(
        mounted
            .root
            .query_selector(".recent-project-row")
            .unwrap()
            .is_some()
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

#[wasm_bindgen_test]
async fn project_tab_context_actions_preserve_chat_and_the_selected_anchor_in_both_modes() {
    use openwebide_frontend::state_actions::projects::{
        ProjectsActionContext, build_projects_actions,
    };
    use wasm_bindgen::JsCast;
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        let mounted = mount_test(move |state| {
            state.seed_project();
            let mut projects = Vec::new();
            for id in 1..=3 {
                let mut project = state.projects.project(1).unwrap();
                project.id = id;
                project.name = format!("Project {id}");
                project.mode = mode;
                projects.push(project);
            }
            state.projects.projects.set(projects);
            state.projects.open_tab_ids.set(vec![1, 2, 3]);
            state.projects.active_project.set(Some(2));
            let actions = build_projects_actions(ProjectsActionContext {
                api: state.api,
                projects: state.projects,
                workspace: state.workspace,
                git: state.git,
                chat: state.chat,
                ui: state.ui,
                ensure_root: Callback::new(|_| ()),
                refresh_git: Callback::new(|()| ()),
            });
            view! { <TabBar on_select=actions.select_project on_select_chat=actions.select_chat on_close=actions.close_project on_tab_action=actions.tab_action /> }
        });
        settle().await;
        mounted
            .element("[data-project-tab='2']")
            .dispatch_event(&web_sys::MouseEvent::new("contextmenu").unwrap())
            .unwrap();
        settle().await;
        mounted.click_text("Move left");
        settle().await;
        assert_eq!(
            mounted.state.projects.open_tab_ids.get_untracked(),
            [2, 1, 3]
        );
        assert_eq!(
            mounted.state.projects.active_project.get_untracked(),
            Some(2)
        );
        mounted
            .element("[data-project-tab='2']")
            .dispatch_event(&web_sys::MouseEvent::new("contextmenu").unwrap())
            .unwrap();
        settle().await;
        let buttons = mounted
            .element(".ui-dropdown-menu")
            .query_selector_all("button")
            .unwrap();
        for index in 0..buttons.length() {
            let button = buttons
                .item(index)
                .unwrap()
                .unchecked_into::<web_sys::HtmlButtonElement>();
            if matches!(
                button.text_content().as_deref(),
                Some("Move left" | "Close all to left")
            ) {
                assert!(button.disabled());
            }
        }
        mounted.click_text("Close others");
        settle().await;
        assert_eq!(mounted.state.projects.open_tab_ids.get_untracked(), [2]);
        assert_eq!(
            mounted.state.projects.active_project.get_untracked(),
            Some(2)
        );
        assert!(mounted.root.query_selector(".chat-tab").unwrap().is_some());
        mounted.state.projects.open_tab_ids.set(vec![2, 1, 3]);
        settle().await;
        mounted
            .element("[data-project-tab='2']")
            .dispatch_event(&web_sys::MouseEvent::new("contextmenu").unwrap())
            .unwrap();
        settle().await;
        let stale_move = mounted.element(".ui-dropdown-menu button:nth-of-type(5)");
        mounted
            .state
            .auth
            .generation
            .update(|generation| *generation += 1);
        stale_move.click();
        assert_eq!(
            mounted.state.projects.open_tab_ids.get_untracked(),
            [2, 1, 3]
        );
    }
}

#[wasm_bindgen_test]
async fn project_close_menu_and_middle_click_close_only_the_target_in_both_modes() {
    use openwebide_frontend::state_actions::projects::{
        ProjectsActionContext, build_projects_actions,
    };
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        let mounted = mount_test(move |state| {
            state.seed_project();
            let projects = (1..=3)
                .map(|id| {
                    let mut project = state.projects.project(1).unwrap();
                    project.id = id;
                    project.mode = mode;
                    project
                })
                .collect();
            state.projects.projects.set(projects);
            state.projects.open_tab_ids.set(vec![1, 2, 3]);
            let actions = build_projects_actions(ProjectsActionContext {
                api: state.api,
                projects: state.projects,
                workspace: state.workspace,
                git: state.git,
                chat: state.chat,
                ui: state.ui,
                ensure_root: Callback::new(|_| ()),
                refresh_git: Callback::new(|()| ()),
            });
            view! { <TabBar on_select=actions.select_project on_select_chat=actions.select_chat on_close=actions.close_project on_tab_action=actions.tab_action /> }
        });
        settle().await;
        let event = web_sys::MouseEventInit::new();
        event.set_button(1);
        event.set_bubbles(true);
        event.set_cancelable(true);
        mounted
            .element("[data-project-tab='2']")
            .dispatch_event(
                &web_sys::MouseEvent::new_with_mouse_event_init_dict("auxclick", &event).unwrap(),
            )
            .unwrap();
        settle().await;
        assert_eq!(mounted.state.projects.open_tab_ids.get_untracked(), [1, 3]);
        assert_eq!(
            mounted.state.projects.active_project.get_untracked(),
            Some(1)
        );
        mounted
            .element("[data-project-tab='1']")
            .dispatch_event(&web_sys::MouseEvent::new("contextmenu").unwrap())
            .unwrap();
        settle().await;
        mounted.click_text("Close");
        settle().await;
        assert_eq!(mounted.state.projects.open_tab_ids.get_untracked(), [3]);
        assert_eq!(
            mounted.state.projects.active_project.get_untracked(),
            Some(3)
        );
    }
}

#[wasm_bindgen::prelude::wasm_bindgen(inline_js = r#"
export function projectTabObserver(root) {
    const records = []; const observer = new MutationObserver(changes => records.push(...changes));
    observer.observe(root, {subtree:true, childList:true, attributes:true});
    return {observer,records};
}
export function projectTabChanges(state) { const count = state.records.length + state.observer.takeRecords().length; state.records.length=0; return count; }
export function projectTabStop(state) { state.observer.disconnect(); }
"#)]
extern "C" {
    fn projectTabObserver(root: &web_sys::HtmlElement) -> wasm_bindgen::JsValue;
    fn projectTabChanges(state: &wasm_bindgen::JsValue) -> u32;
    fn projectTabStop(state: &wasm_bindgen::JsValue);
}

#[wasm_bindgen_test]
async fn project_tab_nodes_and_scroll_stay_stable_during_file_and_project_switches() {
    use openwebide_frontend::state_actions::projects::{
        ProjectsActionContext, build_projects_actions,
    };
    let capture = std::rc::Rc::new(std::cell::Cell::new(None));
    let slot = capture.clone();
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        let capture = capture.clone();
        let mounted = mount_test(move |state| {
            state.seed_project();
            let projects = (1..=6)
                .map(|id| {
                    let mut project = state.projects.project(1).unwrap();
                    project.id = id;
                    project.mode = mode;
                    project.name = format!("Long project name number {id}");
                    project
                })
                .collect();
            state.projects.projects.set(projects);
            state.projects.open_tab_ids.set(vec![1, 2, 3, 4, 5, 6]);
            let actions = build_projects_actions(ProjectsActionContext {
                api: state.api,
                projects: state.projects,
                workspace: state.workspace,
                git: state.git,
                chat: state.chat,
                ui: state.ui,
                ensure_root: Callback::new(|_| ()),
                refresh_git: Callback::new(|()| ()),
            });
            capture.set(Some(actions.select_project));
            view! { <style>{include_str!("../../styles.css")}</style><div class="app" style="width:1000px;height:600px">
                <TopBar on_open_settings=Callback::new(|()| ()) on_logout=Callback::new(|()| ())>
                    <TabBar show_chat=false on_select=actions.select_project on_select_chat=actions.select_chat on_close=actions.close_project on_tab_action=actions.tab_action />
                </TopBar>
            </div> }
        });
        settle().await;
        let strip = mounted.element(".app-navigation .tabbar");
        strip.set_scroll_left(80.0);
        let scroll = strip.scroll_left();
        let tab = mounted.element("[data-project-tab='3']");
        let bounds = tab.get_bounding_client_rect();
        let observer = projectTabObserver(&strip);
        for path in ["a.rs", "b.md", "a.rs"] {
            mounted.state.workspace.open_file.set(Some(path.into()));
            mounted.state.workspace.content.set(path.into());
            mounted.state.projects.open_tab(1);
            mounted.state.projects.select_project(1);
            settle().await;
            assert_eq!(
                projectTabChanges(&observer),
                0,
                "Unrelated updates repainted project tabs"
            );
        }
        projectTabStop(&observer);
        for id in [2, 1, 3, 1] {
            slot.get().unwrap().run(id);
            settle().await;
            assert!(tab.is_same_node(Some(&mounted.element("[data-project-tab='3']"))));
            assert_eq!(strip.scroll_left().to_bits(), scroll.to_bits());
            let current = tab.get_bounding_client_rect();
            assert!((current.x() - bounds.x()).abs() < 0.1);
            assert!((current.width() - bounds.width()).abs() < 0.1);
        }
    }
}
