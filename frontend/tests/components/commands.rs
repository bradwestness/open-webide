use super::support::{command_actions, mount_test, settle};
use leptos::prelude::*;
use openwebide_core::WorkspaceMode;
use openwebide_frontend::{
    commands::Command,
    components::CommandDialogs,
    state::layout::{LayoutState, Panel},
    state_actions::lifecycle::install_keyboard_shortcuts,
};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;
fn key(
    target: &web_sys::HtmlElement,
    name: &str,
    code: &str,
    ctrl: bool,
    meta: bool,
    shift: bool,
) -> bool {
    let init = web_sys::KeyboardEventInit::new();
    init.set_key(name);
    init.set_code(code);
    init.set_ctrl_key(ctrl);
    init.set_meta_key(meta);
    init.set_shift_key(shift);
    init.set_bubbles(true);
    init.set_cancelable(true);
    target
        .dispatch_event(
            &web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap(),
        )
        .unwrap()
}
fn query(mounted: &super::support::Mounted, text: &str) {
    let input: web_sys::HtmlInputElement = mounted.element(".command-search").unchecked_into();
    input.set_value(text);
    input
        .dispatch_event(&web_sys::Event::new("input").unwrap())
        .unwrap();
}
#[wasm_bindgen_test]
async fn palette_search_keyboard_capabilities_and_actions_share_every_mode() {
    for mode in [
        Some(WorkspaceMode::Local),
        Some(WorkspaceMode::Remote),
        None,
    ] {
        let mounted = mount_test(move |state| {
            if let Some(mode) = mode {
                state.seed_project();
                state
                    .projects
                    .projects
                    .update(|projects| projects[0].mode = mode);
            }
            state.seed_session();
            if mode.is_none() {
                state
                    .chat
                    .sessions
                    .update(|sessions| sessions[0].project_id = None);
            }
            command_actions(state.clone());
            install_keyboard_shortcuts(state.chat);
            view! { <button class="opener" on:click=move |_| state.ui.palette_open.set(true)>"Commands"</button><textarea class="composer-input"/><CommandDialogs/> }
        });
        settle().await;
        let opener = mounted.element(".opener");
        opener.focus().unwrap();
        assert!(!key(&opener, "P", "KeyP", true, false, true));
        settle().await;
        let search = mounted.element(".command-search");
        assert!(
            document()
                .active_element()
                .unwrap()
                .is_same_node(Some(&search))
        );
        assert_eq!(search.get_attribute("role").as_deref(), Some("combobox"));
        assert_eq!(
            mounted.element("#command-files").has_attribute("disabled"),
            mode.is_none()
        );
        assert!(mounted.element("#command-stop").has_attribute("disabled"));
        query(&mounted, "no matching action xyz");
        settle().await;
        assert_eq!(
            mounted
                .element(".command-results .empty")
                .text_content()
                .as_deref(),
            Some("No matching commands")
        );
        key(&search, "Enter", "Enter", false, false, false);
        settle().await;
        assert!(mounted.state.ui.palette_open.get_untracked());
        query(&mounted, "theme");
        settle().await;
        assert!(
            mounted
                .root
                .query_selector(".command-option:nth-child(2)")
                .unwrap()
                .is_none()
        );
        assert!(
            mounted
                .root
                .query_selector("#command-settings")
                .unwrap()
                .is_some()
        );
        key(&search, "Enter", "Enter", false, false, false);
        settle().await;
        assert!(mounted.state.settings.show_settings.get_untracked());
        assert!(!mounted.state.ui.palette_open.get_untracked());
        mounted.state.settings.show_settings.set(false);
        opener.click();
        settle().await;
        query(&mounted, "toggle");
        settle().await;
        key(
            &mounted.element(".command-search"),
            "ArrowDown",
            "ArrowDown",
            false,
            false,
            false,
        );
        settle().await;
        let chosen = mounted
            .element(".command-search")
            .get_attribute("aria-activedescendant")
            .unwrap();
        assert!(
            !mounted
                .element(&format!("#{chosen}"))
                .has_attribute("disabled")
        );
        assert_eq!(
            chosen,
            if mode.is_none() {
                "command-chat"
            } else {
                "command-files"
            }
        );
        key(
            &mounted.element(".command-search"),
            "Escape",
            "Escape",
            false,
            false,
            false,
        );
        settle().await;
        assert!(!mounted.state.ui.palette_open.get_untracked());
        assert!(
            document()
                .active_element()
                .unwrap()
                .is_same_node(Some(&opener))
        );
        // macOS modifier and shortcut overlay use the same global dispatcher.
        assert!(!key(&opener, "/", "Slash", false, true, false));
        settle().await;
        assert!(mounted.state.ui.shortcuts_open.get_untracked());
        assert!(
            mounted
                .element(".keyboard-shortcuts")
                .text_content()
                .unwrap()
                .contains("Ctrl/⌘+Shift+P")
        );
        key(&mounted.element(".modal"), "P", "KeyP", true, false, true);
        settle().await;
        assert!(!mounted.state.ui.palette_open.get_untracked());
        key(
            &mounted.element(".modal"),
            "Escape",
            "Escape",
            false,
            false,
            false,
        );
        settle().await;
        // Opening the palette does not alter the draft.
        assert!(mounted.state.chat.draft.get_untracked().is_empty());
    }
}
#[wasm_bindgen_test]
async fn commands_close_on_scope_change_and_reject_stale_project_actions() {
    let mounted = mount_test(|state| {
        state.seed_project();
        state.seed_session();
        let actions = command_actions(state.clone());
        install_keyboard_shortcuts(state.chat);
        view! { <button class="opener" on:click=move |_| actions.run.run(Command::Palette)>"Commands"</button><CommandDialogs/> }
    });
    settle().await;
    mounted.click(".opener");
    settle().await;
    let stale_settings = mounted.element("#command-settings");
    mounted.state.projects.active_project.set(None);
    stale_settings.click();
    assert!(!mounted.state.settings.show_settings.get_untracked());
    settle().await;
    assert!(!mounted.state.ui.palette_open.get_untracked());
    mounted.click(".opener");
    settle().await;
    assert!(mounted.element("#command-files").has_attribute("disabled"));
    mounted.state.chat.active_session.set(Some(99));
    settle().await;
    assert!(!mounted.state.ui.palette_open.get_untracked());
    mounted.click(".opener");
    settle().await;
    mounted
        .state
        .auth
        .generation
        .update(|generation| *generation += 1);
    settle().await;
    assert!(!mounted.state.ui.palette_open.get_untracked());
}
#[wasm_bindgen_test]
async fn palette_panel_actions_and_direct_shortcuts_use_one_layout_facade() {
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        let mounted = mount_test(move |state| {
            state.seed_project();
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            let actions = command_actions(state.clone());
            install_keyboard_shortcuts(state.chat);
            let layout = expect_context::<LayoutState>();
            view! { <button class="opener" on:click=move |_| actions.run.run(Command::Palette)>"Commands"</button>
            <span class="terminal-visible">{move || layout.visible_panels.get().terminal.to_string()}</span>
            <span class="files-visible">{move || layout.visible_panels.get().visible(Panel::Files).to_string()}</span>
            <CommandDialogs/> }
        });
        settle().await;
        mounted.click(".opener");
        settle().await;
        mounted.click("#command-files");
        settle().await;
        assert_eq!(
            mounted.element(".files-visible").text_content().as_deref(),
            Some("false")
        );
        assert!(!mounted.state.ui.palette_open.get_untracked());
        let opener = mounted.element(".opener");
        assert!(!key(&opener, "`", "Backquote", true, false, false));
        settle().await;
        assert_eq!(
            mounted
                .element(".terminal-visible")
                .text_content()
                .as_deref(),
            Some("true")
        );
        opener.click();
        settle().await;
        mounted.click("#command-terminal");
        settle().await;
        assert_eq!(
            mounted
                .element(".terminal-visible")
                .text_content()
                .as_deref(),
            Some("false")
        );
    }
}

#[wasm_bindgen_test]
async fn palette_captures_editor_selection_and_focuses_chat_without_overwriting_drafts() {
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        let mounted = mount_test(move |state| {
            state.seed_project();
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            state.workspace.open_file.set(Some("sample.rs".into()));
            state.workspace.content.set("first\nselected\nlast".into());
            state.chat.draft.set("unsent draft".into());
            let actions = command_actions(state.clone());
            install_keyboard_shortcuts(state.chat);
            view! { <button class="opener" on:click=move |_| actions.run.run(Command::Palette)>"Commands"</button>
            {super::support::editor_view(state.clone())}<textarea class="composer-input"/><CommandDialogs/> }
        });
        settle().await;
        let editor: web_sys::HtmlTextAreaElement =
            mounted.element(".editor-textarea").unchecked_into();
        editor.set_selection_range(6, 14).unwrap();
        mounted.click(".opener");
        settle().await;
        mounted.click("#command-capture-editor");
        settle().await;
        super::support::wait_until("composer focus after closing the palette", || {
            document().active_element().is_some_and(|element| {
                element.is_same_node(Some(&mounted.element(".composer-input")))
            })
        })
        .await;
        let context = mounted
            .state
            .chat
            .active_editor_context
            .get_untracked()
            .unwrap();
        assert_eq!(context.file_path, "sample.rs");
        assert_eq!(context.selection.unwrap().text, "selected");
        assert_eq!(mounted.state.chat.draft.get_untracked(), "unsent draft");
        assert!(
            document()
                .active_element()
                .unwrap()
                .is_same_node(Some(&mounted.element(".composer-input")))
        );
        mounted.click(".opener");
        settle().await;
        mounted.click("#command-focus-chat");
        // A deferred focus action must not steal focus after switching sessions.
        mounted.state.chat.active_session.set(Some(99));
        settle().await;
        editor.focus().unwrap();
        openwebide_frontend::util::sleep_ms(100).await;
        assert!(
            document()
                .active_element()
                .unwrap()
                .is_same_node(Some(&editor))
        );
    }
}
