use leptos::prelude::*;
use openwebide_core::FileEntry;
use openwebide_frontend::{
    components::{ConfirmDialog, FileBrowser, PromptDialog, Settings},
    state::ui::{ConfirmRequest, PromptRequest},
    state_actions::lifecycle::install_keyboard_shortcuts,
};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

use super::support::{mount_test, settle};

fn active() -> web_sys::Element {
    document().active_element().unwrap()
}

fn key(target: &web_sys::HtmlElement, name: &str, shift: bool, ctrl: bool) -> bool {
    let init = web_sys::KeyboardEventInit::new();
    init.set_key(name);
    init.set_shift_key(shift);
    init.set_ctrl_key(ctrl);
    init.set_bubbles(true);
    init.set_cancelable(true);
    target
        .dispatch_event(
            &web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap(),
        )
        .unwrap()
}

#[wasm_bindgen_test]
async fn confirm_focus_traps_live_controls_and_restores_opener() {
    let mounted = mount_test(|state| {
        state.seed_project();
        super::support::command_actions(state.clone());
        install_keyboard_shortcuts(state.chat);
        view! { <button class="opener">"Open"</button> <ConfirmDialog /> }
    });
    let opener = mounted.element(".opener");
    opener.focus().unwrap();
    mounted.state.ui.set_confirm(ConfirmRequest {
        title: "Delete?".into(),
        message: "Permanent deletion".into(),
        confirm_label: "Delete".into(),
        action: Callback::new(|()| ()),
    });
    settle().await;
    let panel = mounted.element(".modal");
    let primary = mounted.element(".danger");
    assert!(active().is_same_node(Some(&primary)));
    assert_eq!(panel.get_attribute("role").as_deref(), Some("dialog"));
    assert_eq!(panel.get_attribute("aria-modal").as_deref(), Some("true"));
    assert_eq!(
        document()
            .get_element_by_id(&panel.get_attribute("aria-labelledby").unwrap())
            .unwrap()
            .text_content()
            .as_deref(),
        Some("Delete?")
    );
    assert_eq!(
        document()
            .get_element_by_id(&panel.get_attribute("aria-describedby").unwrap())
            .unwrap()
            .text_content()
            .as_deref(),
        Some("Permanent deletion")
    );
    key(&primary, "Tab", false, false);
    assert!(active().is_same_node(Some(&mounted.element(".icon-btn"))));
    key(&mounted.element(".icon-btn"), "Tab", true, false);
    assert!(active().is_same_node(Some(&primary)));
    mounted
        .element(".modal-footer .btn")
        .set_attribute("disabled", "")
        .unwrap();
    primary.set_attribute("hidden", "").unwrap();
    key(&mounted.element(".icon-btn"), "Tab", false, false);
    assert!(active().is_same_node(Some(&mounted.element(".icon-btn"))));
    mounted
        .element(".icon-btn")
        .set_attribute("disabled", "")
        .unwrap();
    key(&panel, "Tab", false, false);
    assert!(active().is_same_node(Some(&panel)));
    opener.focus().unwrap();
    // An unfocused headless document can change activeElement without emitting focusin.
    opener
        .dispatch_event(&web_sys::Event::new("focusin").unwrap())
        .unwrap();
    assert!(
        panel.contains(Some(&active())),
        "active={}, panel={}",
        active().outer_html(),
        panel.outer_html()
    );
    key(&panel, "`", false, true);
    assert!(!mounted.state.chat.show_terminal.get_untracked());
    assert!(!key(&panel, "Escape", false, false));
    settle().await;
    assert!(mounted.state.ui.confirm.get_untracked().is_none());
    assert!(active().is_same_node(Some(&opener)));
    assert!(key(&opener, "Tab", false, false));
    key(&opener, "`", false, true);
    assert!(mounted.state.chat.show_terminal.get_untracked());
}

#[wasm_bindgen_test]
async fn stacked_prompt_submits_once_and_returns_focus_to_settings() {
    let submits = RwSignal::new(0);
    let count = submits;
    let mounted = mount_test(|state| {
        view! {
            <Show when=move || state.settings.show_settings.get()>
                <Settings on_set_notifications=Callback::new(|_| ()) on_set_theme=Callback::new(|_| ()) on_set_default_prompt=Callback::new(|_| ()) on_set_bridge_url=Callback::new(|_| ()) />
            </Show>
            <PromptDialog /> <ConfirmDialog />
        }
    });
    mounted.state.settings.show_settings.set(true);
    settle().await;
    let field = mounted.element("input[name=workspace-layout]");
    assert!(active().is_same_node(Some(&field)));
    mounted.state.ui.set_prompt(PromptRequest {
        title: "Name".into(),
        value: "Example".into(),
        placeholder: String::new(),
        submit_label: "Create".into(),
        on_submit: Callback::new(move |value: String| {
            assert_eq!(value, "Example");
            count.update(|count| *count += 1);
        }),
    });
    settle().await;
    let input = mounted.element(".modal-sm input");
    assert!(active().is_same_node(Some(&input)));
    mounted.state.ui.set_confirm(ConfirmRequest {
        title: "Confirm".into(),
        message: "Sure?".into(),
        confirm_label: "Yes".into(),
        action: Callback::new(|()| ()),
    });
    settle().await;
    key(&mounted.element(".danger"), "Escape", false, false);
    settle().await;
    assert!(mounted.state.ui.confirm.get_untracked().is_none());
    assert!(mounted.state.ui.prompt.get_untracked().is_some());
    assert!(active().is_same_node(Some(&input)));
    key(&input, "Enter", false, false);
    settle().await;
    assert_eq!(submits.get_untracked(), 1);
    assert!(mounted.state.ui.prompt.get_untracked().is_none());
    assert!(active().is_same_node(Some(&field)));
    key(&field, "Escape", false, false);
    settle().await;
    assert!(!mounted.state.settings.show_settings.get_untracked());
}

#[wasm_bindgen_test]
async fn folder_navigation_has_live_keyboard_controls_and_safe_missing_opener() {
    let selected = RwSignal::new(None::<String>);
    let shown = RwSignal::new(false);
    let mounted = mount_test(move |state| {
        state.fake.browse_entries.borrow_mut().insert(
            String::new(),
            vec![FileEntry {
                name: "folder".into(),
                path: "folder".into(),
                is_dir: true,
                size: 0,
            }],
        );
        view! {
            <button class="opener">"Open"</button>
            <Show when=move || shown.get()>
                <FileBrowser on_close=Callback::new(move |()| shown.set(false)) on_select=Callback::new(move |path| selected.set(Some(path))) />
            </Show>
        }
    });
    mounted.element(".opener").focus().unwrap();
    shown.set(true);
    settle().await;
    assert!(active().is_same_node(Some(&mounted.element(".send"))));
    let folder = mounted.element(".browser-item.dir");
    assert_eq!(folder.tag_name(), "BUTTON");
    assert!(!folder.has_attribute("disabled"));
    key(&mounted.element(".send"), "Tab", false, false);
    key(&mounted.element(".icon-btn"), "Tab", false, false);
    assert!(active().is_same_node(Some(&folder)));
    // Native buttons activate with Enter/Space; click is the resulting activation event.
    folder.click();
    settle().await;
    assert!(!folder.is_connected());
    assert!(
        mounted.element(".modal").contains(Some(&active())),
        "folder navigation lost dialog focus: {}",
        active().tag_name()
    );
    assert!(
        mounted
            .element(".browser-path")
            .text_content()
            .unwrap()
            .contains("folder")
    );
    let up = mounted.element(".browser-item");
    assert_eq!(up.tag_name(), "BUTTON");
    up.focus().unwrap();
    up.click();
    settle().await;
    assert!(!up.is_connected());
    assert!(
        mounted.element(".modal").contains(Some(&active())),
        "parent navigation lost dialog focus: {}",
        active().tag_name()
    );
    assert!(
        mounted
            .element(".browser-path")
            .text_content()
            .unwrap()
            .contains("~/source")
    );
    mounted.element(".opener").remove();
    mounted.element(".send").click();
    settle().await;
    assert_eq!(selected.get_untracked().as_deref(), Some(""));
    assert!(mounted.root.query_selector(".modal").unwrap().is_none());
    assert!(key(&mounted.root, "Tab", false, false));
    shown.set(true);
    settle().await;
    drop(mounted);
    settle().await;
    let body: web_sys::HtmlElement = document().body().unwrap().unchecked_into();
    assert!(key(&body, "Escape", false, false));
}
