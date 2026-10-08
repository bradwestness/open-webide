use super::support::{Mounted, command_actions, mount_test, settle};
use leptos::prelude::*;
use openwebide_core::{Vfs, WorkspaceMode, vfs::VfsEntryKind};
use openwebide_frontend::{
    components::{CommandDialogs, Configuration, Sidebar, TabBar, TopBar},
    local_fs::BrowserFsaVfs,
    state::{layout::LayoutState, settings::ConfigurationSection},
    state_actions::settings::{SettingsActionContext, build_settings_actions},
    util::sleep_ms,
};
use wasm_bindgen::{JsCast, prelude::*};
use wasm_bindgen_test::*;

#[wasm_bindgen(inline_js = r#"
export async function omnibarFolder() {
  const root=await navigator.storage.getDirectory();
  const name='omnibar-'+crypto.randomUUID();
  const handle=await root.getDirectoryHandle(name,{create:true});
  return {handle,cleanup:()=>root.removeEntry(name,{recursive:true})};
}
export function omnibarHandle(folder) {return folder.handle;}
export async function omnibarCleanup(folder) {await folder.cleanup();}
"#)]
extern "C" {
    async fn omnibarFolder() -> JsValue;
    fn omnibarHandle(folder: &JsValue) -> JsValue;
    async fn omnibarCleanup(folder: &JsValue);
}

fn input(mounted: &Mounted, text: &str) {
    let field: web_sys::HtmlInputElement = mounted.element(".command-search").unchecked_into();
    field.set_value(text);
    field
        .dispatch_event(&web_sys::Event::new("input").unwrap())
        .unwrap();
}
async fn loaded(mounted: &Mounted) {
    for _ in 0..200 {
        if mounted
            .element(".command-search")
            .get_attribute("data-loading")
            .as_deref()
            == Some("false")
        {
            return;
        }
        sleep_ms(10).await;
    }
    panic!("Omnibar did not finish discovery");
}

#[wasm_bindgen_test]
async fn universal_search_discovers_nested_files_through_both_adapters_and_navigates_projects_and_sessions()
 {
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        let folder = omnibarFolder().await;
        let handle: web_sys::FileSystemDirectoryHandle = omnibarHandle(&folder).unchecked_into();
        let vfs = BrowserFsaVfs::new(handle.clone());
        vfs.create("src", VfsEntryKind::Directory).await.unwrap();
        vfs.create("target", VfsEntryKind::Directory).await.unwrap();
        vfs.write("src/main.rs", "fn main() {}\n").await.unwrap();
        vfs.write("target/ignored.rs", "ignored").await.unwrap();
        let mounted = mount_test(move |state| {
            state.seed_project();
            state.seed_session();
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            state.projects.local_handles.update(|handles| {
                handles.insert(1, handle.clone());
            });
            state.fake.files.borrow_mut().extend([
                ((1, "src/main.rs".into()), "fn main() {}\n".into()),
                ((1, "target/ignored.rs".into()), "ignored".into()),
            ]);
            let mut other = state.projects.project(1).unwrap();
            other.id = 2;
            other.name = "Other project".into();
            state
                .projects
                .projects
                .update(|projects| projects.push(other));
            let mut session = state.chat.sessions.get_untracked()[0].clone();
            session.id = 2;
            session.project_id = Some(2);
            session.name = "Other conversation".into();
            state.fake.sessions.borrow_mut().push(session);
            command_actions(state.clone());
            view! {<button class="opener" on:click=move |_|state.ui.palette_open.set(true)>"Search"</button><openwebide_frontend::components::Omnibar /><CommandDialogs />}
        });
        settle().await;
        mounted.click(".opener");
        settle().await;
        loaded(&mounted).await;
        input(&mounted, "/main");
        settle().await;
        assert_eq!(
            mounted
                .element("[data-result-kind=File] .omnibar-entry-detail")
                .text_content()
                .as_deref(),
            Some("src/main.rs")
        );
        mounted.click("[data-result-kind=File]");
        for _ in 0..100 {
            if mounted.state.workspace.content.get_untracked().as_str() == "fn main() {}\n" {
                break;
            }
            sleep_ms(10).await;
        }
        assert_eq!(
            mounted.state.workspace.content.get_untracked().as_str(),
            "fn main() {}\n"
        );
        assert!(!mounted.state.ui.palette_open.get_untracked());
        mounted.click(".opener");
        settle().await;
        loaded(&mounted).await;
        input(&mounted, "/ignored");
        settle().await;
        assert!(
            mounted
                .root
                .query_selector("[data-result-kind=File]")
                .unwrap()
                .is_none()
        );
        input(&mounted, "@Other conversation");
        settle().await;
        mounted.click("[data-result-kind=Session]");
        settle().await;
        assert_eq!(
            mounted.state.projects.active_project.get_untracked(),
            Some(2)
        );
        assert_eq!(mounted.state.chat.active_session.get_untracked(), Some(2));
        mounted.click(".opener");
        settle().await;
        loaded(&mounted).await;
        input(&mounted, "#test");
        settle().await;
        mounted.click("[data-result-kind=Project]");
        settle().await;
        assert_eq!(
            mounted.state.projects.active_project.get_untracked(),
            Some(1)
        );
        drop(mounted);
        omnibarCleanup(&folder).await;
    }
}

#[wasm_bindgen_test]
async fn discovery_failures_keep_commands_available_and_late_results_cannot_cross_accounts_or_projects()
 {
    let mounted = mount_test(|state| {
        state.seed_project();
        command_actions(state.clone());
        view! {<button class="opener" on:click=move |_|state.ui.palette_open.set(true)>"Search"</button><openwebide_frontend::components::Omnibar /><CommandDialogs />}
    });
    settle().await;
    let (send, receive) = futures::channel::oneshot::channel();
    mounted
        .state
        .fake
        .file_list_results
        .borrow_mut()
        .push_back(receive);
    mounted.click(".opener");
    settle().await;
    input(&mounted, ">settings");
    settle().await;
    assert!(
        mounted
            .root
            .query_selector("#command-settings")
            .unwrap()
            .is_some()
    );
    mounted.state.projects.active_project.set(None);
    settle().await;
    assert!(!mounted.state.ui.palette_open.get_untracked());
    let _ = send.send(Ok(vec![openwebide_core::FileEntry {
        name: "stale".into(),
        path: "stale".into(),
        is_dir: false,
        size: 0,
    }]));
    settle().await;
    assert!(mounted.state.ui.toast.get_untracked().is_none());
    mounted.state.projects.active_project.set(Some(1));
    settle().await;
    let (send, receive) = futures::channel::oneshot::channel();
    mounted
        .state
        .fake
        .file_list_results
        .borrow_mut()
        .push_back(receive);
    mounted.click(".opener");
    settle().await;
    mounted
        .state
        .auth
        .generation
        .update(|generation| *generation += 1);
    settle().await;
    assert!(!mounted.state.ui.palette_open.get_untracked());
    let _ = send.send(Err("stale error".into()));
    settle().await;
    assert!(mounted.state.ui.toast.get_untracked().is_none());
    let (send, receive) = futures::channel::oneshot::channel();
    mounted
        .state
        .fake
        .file_list_results
        .borrow_mut()
        .push_back(receive);
    mounted.click(".opener");
    settle().await;
    send.send(Err("listing failed".into())).unwrap();
    settle().await;
    loaded(&mounted).await;
    input(&mounted, ">settings");
    settle().await;
    assert!(
        mounted
            .element(".omnibar")
            .text_content()
            .unwrap()
            .contains("listing failed")
    );
    mounted.click("#command-settings");
    assert!(mounted.state.settings.show_settings.get_untracked());
}

#[wasm_bindgen_test]
async fn phone_drawer_and_desktop_menu_share_configuration_and_project_actions() {
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        let opened = RwSignal::new(0);
        let layout_slot = std::rc::Rc::new(std::cell::Cell::new(None));
        let slot = layout_slot.clone();
        let mounted = mount_test(move |state| {
            state.seed_project();
            state.seed_session();
            state.seed_connection();
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            state.projects.open_tab_ids.set(vec![1]);
            command_actions(state.clone());
            let settings = build_settings_actions(SettingsActionContext {
                api: state.api,
                settings: state.settings,
                ui: state.ui,
            });
            let layout = expect_context::<LayoutState>();
            slot.set(Some(layout));
            view! {<style>{include_str!("../../styles.css")}</style><div class="app" style="width:1000px;height:700px" class:phone-layout=move ||layout.phone.get()>
                <TopBar on_open_settings=settings.on_open_settings on_logout=Callback::new(|()|()) on_open_local=Callback::new(move |()|opened.update(|count|*count+=1)) on_open_remote=Callback::new(move |()|opened.update(|count|*count+=10))><TabBar show_chat=false on_select=Callback::new(|_|()) on_select_chat=Callback::new(|()|()) on_close=Callback::new(|_|()) /></TopBar>
                <Sidebar on_select_session=Callback::new(|_|()) on_new_session=Callback::new(|()|()) on_rename_session=Callback::new(|_|()) on_delete_session=Callback::new(|_|()) />
                <Configuration on_new_connection=settings.on_new_connection on_edit_connection=settings.on_edit_connection on_cancel_connection=settings.on_cancel_connection on_delete_connection=settings.on_delete_connection on_new_prompt=settings.on_new_prompt on_edit_prompt=settings.on_edit_prompt on_save_prompt=settings.on_save_prompt on_cancel_prompt=settings.on_cancel_prompt on_delete_prompt=settings.on_delete_prompt />
                <CommandDialogs />
            </div>}
        });
        settle().await;
        assert_logo_fits_app_bar(&mounted);
        assert!(
            !mounted
                .element(".sidebar")
                .text_content()
                .unwrap()
                .contains("Servers")
        );
        assert!(
            !mounted
                .element(".sidebar")
                .text_content()
                .unwrap()
                .contains("System prompts")
        );
        let top = mounted.element(".topbar").get_bounding_client_rect();
        let tabs = mounted.element(".tabbar-wrap").get_bounding_client_rect();
        assert!(tabs.top() >= top.top() && tabs.bottom() <= top.bottom() + 1.0);
        mounted.click("[aria-label='App menu']");
        settle().await;
        mounted.click_text("Open local project");
        settle().await;
        assert_eq!(opened.get_untracked(), 1);
        mounted.click("[aria-label='App menu']");
        settle().await;
        mounted.click_text("Open remote project");
        settle().await;
        assert_eq!(opened.get_untracked(), 11);
        mounted.click("[aria-label='App menu']");
        settle().await;
        mounted.click_text("Servers");
        settle().await;
        assert_eq!(
            mounted.state.settings.configuration.get_untracked(),
            Some(ConfigurationSection::Servers)
        );
        assert!(
            mounted
                .element(".app-configuration")
                .text_content()
                .unwrap()
                .contains("Ollama")
        );
        mounted.state.settings.configuration.set(None);
        settle().await;
        let layout = layout_slot.get().unwrap();
        layout
            .preferences
            .update(|prefs| prefs.mode = openwebide_frontend::state::responsive::LayoutMode::Phone);
        mounted
            .element(".app")
            .set_attribute("style", "width:320px;height:700px")
            .unwrap();
        settle().await;
        assert_logo_fits_app_bar(&mounted);
        mounted.click(".app-drawer-trigger");
        settle().await;
        let drawer = mounted.element(".app-drawer").get_bounding_client_rect();
        assert!(drawer.width() <= 340.0);
        for item in ["Open local project", "Open remote project"] {
            let buttons = mounted
                .element(".app-drawer")
                .query_selector_all("button")
                .unwrap();
            let target = (0..buttons.length())
                .filter_map(|i| buttons.item(i))
                .filter_map(|node| node.dyn_into::<web_sys::Element>().ok())
                .find(|button| button.text_content().unwrap_or_default().contains(item))
                .unwrap();
            assert!(target.get_bounding_client_rect().height() >= 44.0);
        }
        mounted.click_text("System prompts");
        settle().await;
        assert_eq!(
            mounted.state.settings.configuration.get_untracked(),
            Some(ConfigurationSection::SystemPrompts)
        );
        mounted.state.settings.configuration.set(None);
        settle().await;
    }
}

#[wasm_bindgen_test]
async fn compact_editor_uses_one_footer_and_shared_tree_preferences_in_both_modes() {
    use super::support::{editor_view, wait_until};
    use openwebide_frontend::{
        components::{
            FileTree, FilesPanel, StatusBar, ToolPanel, editor_chrome::EditorFooterMount,
        },
        state::layout::Panel,
        state_actions::layout::LayoutActions,
    };
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        let slot = std::rc::Rc::new(std::cell::Cell::new(None));
        let chrome_slot = slot.clone();
        let mounted = mount_test(move |state| {
            state.seed_project();
            state.seed_session();
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            state.workspace.open_file.set(Some("fixture.rs".into()));
            state.workspace.content.set("fn main() {}\n".into());
            state.workspace.entries.update(|entries| {
                entries.insert(
                    String::new(),
                    vec![openwebide_core::FileEntry {
                        name: ".hidden".into(),
                        path: ".hidden".into(),
                        is_dir: true,
                        size: 0,
                    }],
                );
            });
            provide_context(EditorFooterMount::new());
            command_actions(state.clone());
            chrome_slot.set(Some(expect_context::<LayoutActions>()));
            let health = RwSignal::new(None);
            let layout = expect_context::<LayoutState>();
            let editor = editor_view(state);
            view! { <style>{include_str!("../../styles.css")}</style><div class="app" class:phone-layout=move || layout.phone.get() style="width:1000px;height:700px"><ToolPanel panel=Panel::Editor>{editor}</ToolPanel><FilesPanel on_new_file=Callback::new(|()|()) on_new_dir=Callback::new(|()|())><FileTree on_toggle=Callback::new(|_|()) on_open=Callback::new(|_|()) /></FilesPanel><StatusBar health=health.read_only() on_toggle_terminal=||() /><openwebide_frontend::components::Omnibar /><CommandDialogs /></div> }
        });
        settle().await;
        wait_until("editor status in app footer", || {
            mounted
                .root
                .query_selector(".statusbar .editor-cursor-status")
                .unwrap()
                .is_some()
        })
        .await;
        assert!(
            mounted
                .root
                .query_selector(".editor .editor-footer")
                .unwrap()
                .is_none()
        );
        assert_eq!(
            mounted
                .root
                .query_selector_all(".editor-footer")
                .unwrap()
                .length(),
            1
        );
        assert!(
            mounted
                .element("#panel-editor .tool-panel-heading")
                .has_attribute("hidden")
        );
        assert!(
            mounted
                .root
                .query_selector(".editor-path")
                .unwrap()
                .is_none()
        );
        assert!(
            mounted
                .root
                .query_selector(".editor-find")
                .unwrap()
                .is_none()
        );
        mounted.click("button[aria-label='Find in file (Ctrl/⌘F)']");
        settle().await;
        assert!(
            mounted
                .root
                .query_selector(".editor-find")
                .unwrap()
                .is_some()
        );
        mounted.click("button[aria-label='Close find']");
        settle().await;
        mounted.click(".statusbar button[aria-label='Indentation settings']");
        settle().await;
        assert!(
            mounted
                .element(".statusbar .ui-dropdown-menu")
                .text_content()
                .unwrap()
                .contains("Detected / defaults")
        );
        mounted.click(".ui-dropdown-backdrop");
        assert!(
            mounted
                .root
                .query_selector("[data-tree-path='.hidden']")
                .unwrap()
                .is_none()
        );
        slot.get().unwrap().set_tree_preferences.run(true);
        settle().await;
        assert!(
            mounted
                .root
                .query_selector("[data-tree-path='.hidden']")
                .unwrap()
                .is_some()
        );
        assert!(
            mounted
                .element(".file-tree")
                .class_list()
                .contains("compact-tree")
        );
        let tab = mounted.element("[data-editor-tab='fixture.rs']");
        assert_eq!(tab.get_attribute("title").as_deref(), Some("fixture.rs"));
        let header = mounted.element(".editor-header").get_bounding_client_rect();
        let tabs = mounted
            .element(".editor-file-tabs")
            .get_bounding_client_rect();
        assert!(tabs.top() >= header.top() && tabs.bottom() <= header.bottom());
        let layout_actions = slot.get().unwrap();
        layout_actions
            .set_mode
            .run(openwebide_frontend::state::responsive::LayoutMode::Phone);
        layout_actions.show.run(Panel::Editor);
        mounted
            .element(".app")
            .set_attribute("style", "width:320px;height:700px")
            .unwrap();
        settle().await;
        assert!(
            !mounted
                .element(".file-tree")
                .class_list()
                .contains("compact-tree")
        );
        assert!(
            mounted
                .element(".tree-item")
                .get_bounding_client_rect()
                .height()
                >= 44.0
        );
        let selector = mounted.element(".editor-view-narrow button[aria-label='Editor view']");
        assert!(selector.get_bounding_client_rect().height() >= 44.0);
        assert!(
            mounted
                .element("button[aria-label='Find in file (Ctrl/⌘F)']")
                .get_bounding_client_rect()
                .height()
                >= 44.0
        );
        mounted.state.ui.palette_open.set(true);
        settle().await;
        input(&mounted, ">convert indentation");
        settle().await;
        assert!(
            !mounted
                .element("#command-convert-indentation")
                .has_attribute("disabled")
        );
        mounted.state.workspace.merge_pending(
            1,
            openwebide_core::FileDiff {
                path: "fixture.rs".into(),
                old: Some("before".into()),
                new: "after".into(),
                old_unavailable: false,
                backup_path: None,
            },
        );
        settle().await;
        assert!(
            mounted
                .element("#command-convert-indentation")
                .has_attribute("disabled")
        );
        mounted.state.ui.palette_open.set(false);
        mounted
            .state
            .workspace
            .pending_edits
            .set(Default::default());
        settle().await;
    }
}

fn assert_logo_fits_app_bar(mounted: &super::support::Mounted) {
    let bar = mounted
        .element(".app-navigation")
        .get_bounding_client_rect();
    let mark = mounted
        .element(".app-navigation .logo-mark")
        .get_bounding_client_rect();
    let svg = mounted
        .element(".app-navigation .logo-mark svg")
        .get_bounding_client_rect();
    assert!((mark.width() - 28.0).abs() < 1.0);
    assert!((mark.height() - 28.0).abs() < 1.0);
    assert!((svg.width() - mark.width()).abs() < 1.0);
    assert!((svg.height() - mark.height()).abs() < 1.0);
    assert!(bar.height() <= 48.0);
    assert!(mark.top() >= bar.top() && mark.bottom() <= bar.bottom());
}
