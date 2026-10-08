use leptos::prelude::*;
use openwebide_core::FileEntry;
use openwebide_frontend::{
    components::{ConfirmDialog, FileBrowser, PromptDialog, Settings},
    state::ui::{ConfirmRequest, PromptRequest},
    state_actions::lifecycle::install_keyboard_shortcuts,
};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

use super::support::{mount_test, settle, wait_until};

#[wasm_bindgen::prelude::wasm_bindgen(
    inline_js = "export function noticeFetch(html) { const original=window.fetch; let requests=0; let fail=false; window.fetch=async request => { if (!request.url.includes('/about-software-')) return original(request); requests++; return new Response(html, {status:fail?503:200}); }; return { count:()=>requests, fail:value=>{fail=value;}, restore:()=>{window.fetch=original;} }; }"
)]
extern "C" {
    #[wasm_bindgen::prelude::wasm_bindgen(js_name = noticeFetch)]
    fn notice_fetch(html: &str) -> js_sys::Object;
}

fn notice_call(mock: &js_sys::Object, name: &str) -> wasm_bindgen::JsValue {
    let function: js_sys::Function = js_sys::Reflect::get(mock, &name.into())
        .unwrap()
        .unchecked_into();
    function.call0(mock).unwrap()
}

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
    let field = mounted.element(".ui-seg-btn");
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

#[wasm_bindgen_test]
async fn about_shows_build_and_local_notices_in_both_modes_and_restores_focus() {
    use openwebide_core::{User, UserId, UserRole, WorkspaceMode};
    use openwebide_frontend::{
        commands::Command,
        components::{CommandDialogs, TopBar},
    };
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        let mock = notice_fetch(include_str!(concat!(
            env!("OUT_DIR"),
            "/about-software.html"
        )));
        let mounted = mount_test(move |state| {
            state.seed_project();
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            state.auth.set_user(User {
                id: UserId::new(1),
                username: "test".into(),
                role: UserRole::User,
                created_at: 0,
            });
            super::support::command_actions(state);
            view! {
                <style>{include_str!("../../styles.css")}</style>
                <TopBar on_open_settings=Callback::new(|()| ()) on_logout=Callback::new(|()| ()) />
                <CommandDialogs />
            }
        });
        settle().await;
        mounted.click("[aria-label='App menu']");
        settle().await;
        mounted.element("[aria-label='About']").focus().unwrap();
        mounted.click("[aria-label='About']");
        settle().await;
        let panel = mounted.element("[role='dialog']");
        let text = panel.text_content().unwrap();
        assert!(text.contains(env!("CARGO_PKG_VERSION")));
        assert!(text.contains("Commit"));
        assert!(!text.contains("SIL OPEN FONT LICENSE"));
        assert_eq!(notice_call(&mock, "count").as_f64(), Some(0.0));
        let overview = mounted
            .element(".about-overview")
            .get_bounding_client_rect();
        let details = mounted
            .element(".about-overview > div")
            .get_bounding_client_rect();
        assert!(
            (overview.x() + overview.width() / 2.0 - details.x() - details.width() / 2.0).abs()
                < 2.0
        );
        assert!(
            (overview.y() + overview.height() / 2.0 - details.y() - details.height() / 2.0).abs()
                < 2.0
        );
        assert_eq!(
            mounted
                .element("#about-tab-overview")
                .get_attribute("aria-selected")
                .as_deref(),
            Some("true")
        );
        key(
            &mounted.element("#about-tab-overview"),
            "ArrowRight",
            false,
            false,
        );
        settle().await;
        assert!(active().is_same_node(Some(&mounted.element("#about-tab-software"))));
        assert_eq!(
            mounted
                .element("#about-tab-software")
                .get_attribute("aria-selected")
                .as_deref(),
            Some("true")
        );
        wait_until("license notices", || {
            panel
                .text_content()
                .unwrap()
                .contains("SIL OPEN FONT LICENSE")
        })
        .await;
        let text = panel.text_content().unwrap();
        assert!(!text.contains("Commit"));
        assert!(text.contains("SIL OPEN FONT LICENSE"));
        assert!(text.contains("Lucide icons"));
        assert!(text.contains("leptos"));
        assert!(text.contains("Copyright (c) 2022 Greg Johnston"));
        assert_eq!(notice_call(&mock, "count").as_f64(), Some(1.0));
        let body = mounted.element(".about-body");
        let notices_panel = mounted.element(".about-software");
        assert!(notices_panel.scroll_height() > notices_panel.client_height());
        assert!(body.scroll_height() <= body.client_height());
        let contents = mounted.element(".about-content").inner_html();
        assert!(
            mounted
                .root
                .query_selector(".ui-dropdown-menu")
                .unwrap()
                .is_none()
        );
        key(&panel, "Escape", false, false);
        settle().await;
        assert!(
            mounted
                .root
                .query_selector("[role='dialog']")
                .unwrap()
                .is_none()
        );
        assert!(active().is_same_node(Some(&mounted.element("[aria-label='App menu']"))));
        mounted.state.ui.palette_open.set(true);
        settle().await;
        let search: web_sys::HtmlInputElement = mounted.element(".command-search").unchecked_into();
        search.set_value("licenses");
        let input = web_sys::EventInit::new();
        input.set_bubbles(true);
        search
            .dispatch_event(&web_sys::Event::new_with_event_init_dict("input", &input).unwrap())
            .unwrap();
        settle().await;
        mounted.click("#command-about");
        settle().await;
        assert!(!mounted.state.ui.palette_open.get_untracked());
        assert!(
            mounted
                .element(".about-content")
                .text_content()
                .unwrap()
                .contains("Commit")
        );
        let fail: js_sys::Function = js_sys::Reflect::get(&mock, &"fail".into())
            .unwrap()
            .unchecked_into();
        fail.call1(&mock, &true.into()).unwrap();
        mounted.click("#about-tab-software");
        settle().await;
        assert!(
            mounted
                .element("[role='alert']")
                .text_content()
                .unwrap()
                .contains("Could not load")
        );
        fail.call1(&mock, &false.into()).unwrap();
        mounted.click(".about-content .btn");
        wait_until("retried license notices", || {
            mounted
                .element(".about-content")
                .text_content()
                .unwrap()
                .contains("SIL OPEN FONT LICENSE")
        })
        .await;
        assert_eq!(mounted.element(".about-content").inner_html(), contents);
        key(
            &mounted.element("#about-tab-software"),
            "Home",
            false,
            false,
        );
        settle().await;
        assert!(
            mounted
                .element(".about-content")
                .text_content()
                .unwrap()
                .contains("Commit")
        );
        assert_eq!(notice_call(&mock, "count").as_f64(), Some(3.0));
        mounted.click("#about-tab-software");
        settle().await;
        assert_eq!(notice_call(&mock, "count").as_f64(), Some(3.0));
        mounted.state.ui.about_open.set(false);
        settle().await;
        notice_call(&mock, "restore");
        assert!(Command::About.unavailable(Default::default()).is_none());
    }
}
