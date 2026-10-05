use leptos::prelude::*;
use openwebide_core::{User, UserId, UserRole};
use openwebide_frontend::{
    components::ui::{Button, ButtonSize, ButtonVariant},
    state::{
        auth::AuthState,
        layout::LayoutState,
        settings::{SettingsState, Theme},
    },
    state_actions::{
        lifecycle::{ProjectEffectContext, install_project_effects},
        settings::{SettingsActionContext, build_settings_actions},
    },
};
use wasm_bindgen_test::*;

use super::support::{mount_test, settle};

#[wasm_bindgen_test]
async fn button_variants_sizes_and_disabled_callbacks_share_the_base() {
    let clicks = RwSignal::new(0);
    let disabled = RwSignal::new(true);
    let mounted = mount_test(move |_| {
        view! {
            <div>
                <Button class="default" on_click=Callback::new(move |_| clicks.update(|n| *n += 1))>"Default"</Button>
                <Button variant=ButtonVariant::Primary size=ButtonSize::Sm class="primary">"Primary"</Button>
                <Button variant=ButtonVariant::Success class="success">"Success"</Button>
                <Button variant=ButtonVariant::Danger class="dangerous">"Danger"</Button>
                <Button variant=ButtonVariant::Ghost class="ghostly">"Ghost"</Button>
                <Button class="disabled" disabled=Signal::from(disabled) on_click=Callback::new(move |_| clicks.update(|n| *n += 1))>"Disabled"</Button>
            </div>
        }
    });
    settle().await;
    for (selector, modifier) in [
        (".default", "md"),
        (".primary", "send"),
        (".success", "approve"),
        (".dangerous", "danger"),
        (".ghostly", "ghost"),
    ] {
        let classes = mounted.element(selector).class_name();
        assert!(classes.split_whitespace().any(|class| class == "btn"));
        assert!(classes.split_whitespace().any(|class| class == modifier));
        assert!(!classes.contains("ui-btn"));
    }
    assert!(mounted.element(".primary").class_name().contains("sm"));
    mounted.click(".disabled");
    assert_eq!(clicks.get_untracked(), 0);
    mounted.click(".default");
    assert_eq!(clicks.get_untracked(), 1);
    disabled.set(false);
    settle().await;
    mounted.click(".disabled");
    assert_eq!(clicks.get_untracked(), 2);
}

#[wasm_bindgen_test]
async fn startup_retains_prepaint_theme_until_database_settings_arrive() {
    let document = web_sys::window().unwrap().document().unwrap();
    let root = document.document_element().unwrap();
    let original = root.get_attribute("data-theme");
    let storage = web_sys::window().unwrap().local_storage().unwrap().unwrap();
    storage.set_item("owide-theme", "dark").unwrap();
    // The blocking backend script has already painted the saved light preference.
    root.set_attribute("data-theme", "light").unwrap();
    let html = include_str!("../../index.html");
    assert!(html.contains("<script src=\"/api/theme.js\"></script>"));
    assert!(!html.contains("localStorage"));
    let (release, pending) = futures::channel::oneshot::channel();
    let notifications = RwSignal::new(false);
    let mounted = mount_test(move |mut state| {
        state.settings = SettingsState::new(Theme::from_root(), "ws://localhost:3001".into());
        state.settings.browser_notifications = notifications;
        build_settings_actions(SettingsActionContext {
            api: state.api,
            settings: state.settings,
            ui: state.ui,
        });
        state
            .fake
            .settings_load_results
            .borrow_mut()
            .push_back(pending);
        let auth = expect_context::<AuthState>();
        auth.set_user(User {
            id: UserId::new(1),
            username: "test".into(),
            role: UserRole::User,
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
    settle().await;
    assert_eq!(root.get_attribute("data-theme").as_deref(), Some("light"));
    release
        .send(Ok(std::collections::BTreeMap::from([
            ("theme".into(), "dark".into()),
            ("browser_notifications".into(), "true".into()),
        ])))
        .unwrap();
    // Startup includes an IndexedDB await before the independent settings reads.
    for _ in 0..100 {
        openwebide_frontend::util::sleep_ms(10).await;
        if root.get_attribute("data-theme").as_deref() == Some("dark") {
            break;
        }
    }
    assert_eq!(root.get_attribute("data-theme").as_deref(), Some("dark"));
    assert!(notifications.get_untracked());
    drop(mounted);
    storage.remove_item("owide-theme").unwrap();
    if let Some(original) = original {
        root.set_attribute("data-theme", &original).unwrap();
    } else {
        root.remove_attribute("data-theme").unwrap();
    }
}

#[wasm_bindgen_test]
async fn typed_theme_controls_update_and_save_the_same_database_values() {
    use openwebide_frontend::components::Settings;
    let root = document().document_element().unwrap();
    let original = root.get_attribute("data-theme");
    let mounted = mount_test(|state| {
        let actions = build_settings_actions(SettingsActionContext {
            api: state.api,
            settings: state.settings,
            ui: state.ui,
        });
        view! {
            <Settings on_set_notifications=actions.on_set_notifications on_set_theme=actions.on_set_theme

                on_set_default_prompt=actions.on_set_default_prompt
                on_set_bridge_url=actions.on_set_bridge_url />
        }
    });
    for (index, theme) in [Theme::System, Theme::Dark, Theme::Light]
        .into_iter()
        .enumerate()
    {
        mounted.click(&format!(
            ".ui-field-group:nth-of-type(2) .ui-seg-btn:nth-child({})",
            index + 1
        ));
        settle().await;
        assert_eq!(mounted.state.settings.theme.get_untracked(), theme);
        assert_eq!(
            mounted.state.fake.settings.borrow()["theme"],
            theme.as_str()
        );
        let effective = if theme == Theme::System {
            if web_sys::window()
                .unwrap()
                .match_media("(prefers-color-scheme: dark)")
                .unwrap()
                .unwrap()
                .matches()
            {
                "dark"
            } else {
                "light"
            }
        } else {
            theme.as_str()
        };
        assert_eq!(root.get_attribute("data-theme").as_deref(), Some(effective));
    }
    drop(mounted);
    if let Some(original) = original {
        root.set_attribute("data-theme", &original).unwrap();
    } else {
        root.remove_attribute("data-theme").unwrap();
    }
}

#[wasm_bindgen_test]
async fn statusline_preserves_compact_telemetry_text() {
    use openwebide_core::SessionTelemetry;
    let mounted = mount_test(super::support::chat_view);
    settle().await;
    mounted.state.chat.session_telemetry.set(SessionTelemetry {
        context: None,
        model: "café".into(),
        context_tokens: 12345,
        context_limit: 65536,
        current_speed_tps: Some(12.345),
        context_estimated: true,
        context_limit_estimated: true,
        speed_estimated: true,
        tool_calls_count: 7,
        ..SessionTelemetry::default()
    });
    settle().await;
    assert_eq!(
        mounted.element(".tui-model-name").text_content().as_deref(),
        Some("café")
    );
    assert_eq!(
        mounted.element(".tui-ctx-gauge").text_content().as_deref(),
        Some("Ctx: ~12.3k/~66k (19%)")
    );
    assert_eq!(
        mounted.element(".tui-speed").text_content().as_deref(),
        Some("~12.3 t/s")
    );
    assert_eq!(
        mounted
            .element(".tui-tools-count")
            .text_content()
            .as_deref(),
        Some("7 tools")
    );
    mounted.state.chat.session_telemetry.set(SessionTelemetry {
        context_limit: 0,
        context_limit_estimated: false,
        ..SessionTelemetry::default()
    });
    settle().await;
    assert_eq!(
        mounted.element(".tui-ctx-gauge").text_content().as_deref(),
        Some("Ctx: 0.0k/0k (0%)")
    );
    assert_eq!(
        mounted.element(".tui-speed").text_content().as_deref(),
        Some("-- t/s")
    );
}

#[wasm_bindgen_test]
async fn shared_icons_and_tooltips_are_labeled_themeable_and_cleaned_up() {
    use super::support::wait_until;
    use openwebide_frontend::components::ui::{Icon, IconButton, IconName};
    let mounted = mount_test(|_| {
        openwebide_frontend::viewport::install_action_tooltips();
        view! {
            <style>{include_str!("../../styles.css")}</style>
            <IconButton label="Settings" on_click=Callback::new(|_| ())><Icon name=IconName::Settings /></IconButton>
        }
    });
    settle().await;
    let button = mounted.element("button");
    assert_eq!(
        button.get_attribute("aria-label").as_deref(),
        Some("Settings")
    );
    let icon = mounted.element("svg");
    assert_eq!(icon.get_attribute("width").as_deref(), Some("20"));
    assert_eq!(
        icon.get_attribute("stroke").as_deref(),
        Some("currentColor")
    );
    assert_eq!(
        mounted
            .element(".ui-icon-glyph")
            .get_attribute("aria-hidden")
            .as_deref(),
        Some("true")
    );
    button.focus().unwrap();
    button
        .dispatch_event(&web_sys::Event::new("focus").unwrap())
        .unwrap();
    wait_until("keyboard action tooltip", || {
        web_sys::window()
            .unwrap()
            .document()
            .unwrap()
            .get_element_by_id("ui-action-tooltip")
            .is_some_and(|tip| !tip.has_attribute("hidden"))
    })
    .await;
    let document = web_sys::window().unwrap().document().unwrap();
    let tooltip = document.get_element_by_id("ui-action-tooltip").unwrap();
    assert_eq!(tooltip.text_content().as_deref(), Some("Settings"));
    assert_eq!(
        button.get_attribute("aria-describedby").as_deref(),
        Some("ui-action-tooltip")
    );
    drop(mounted);
    assert!(document.get_element_by_id("ui-action-tooltip").is_none());
    assert_eq!(button.get_attribute("title").as_deref(), Some("Settings"));
}

#[wasm_bindgen_test]
async fn shared_dropdowns_fit_the_viewport_skip_disabled_options_and_dismiss_before_modals() {
    use openwebide_frontend::components::{
        Modal,
        dropdown::{DropdownSelect, SelectOption},
        ui::{DialogBody, PanelSearchRow},
    };
    use wasm_bindgen::JsCast;
    for mode in [
        openwebide_core::WorkspaceMode::Local,
        openwebide_core::WorkspaceMode::Remote,
    ] {
        let shown = RwSignal::new(true);
        let value = RwSignal::new("b".to_string());
        let options = RwSignal::new(vec![
            SelectOption::new("a", "Alpha"),
            SelectOption::new("b", "Beta"),
            SelectOption {
                value: "disabled".into(),
                label: "Disabled".into(),
                disabled: true,
            },
            SelectOption::new("g", "Gamma"),
        ]);
        let mounted = mount_test(move |state| {
            state.seed_project();
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            view! { <style>{include_str!("../../styles.css")}</style>
                <Show when=move || shown.get()>
                    <Modal title=Signal::derive(|| "Choices".into()) on_close=Callback::new(move |()| shown.set(false))>
                        <DialogBody>
                            <DropdownSelect label="Choice" value=Signal::derive(move || value.get()) options=Signal::derive(move || options.get()) on_change=Callback::new(move |selection| value.set(selection)) />
                            <PanelSearchRow><input class="form-input panel-search-input" aria-label="Files search" /></PanelSearchRow>
                            <PanelSearchRow><input class="form-input panel-search-input" aria-label="Sessions search" /></PanelSearchRow>
                        </DialogBody>
                    </Modal>
                </Show>
            }
        });
        settle().await;
        mounted.click(".ui-dropdown-trigger");
        settle().await;
        let keyboard = |key: &str| {
            let init = web_sys::KeyboardEventInit::new();
            init.set_key(key);
            init.set_bubbles(true);
            init.set_cancelable(true);
            web_sys::window()
                .unwrap()
                .document()
                .unwrap()
                .active_element()
                .unwrap()
                .dispatch_event(
                    &web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init)
                        .unwrap(),
                )
                .unwrap();
        };
        assert_eq!(
            web_sys::window()
                .unwrap()
                .document()
                .unwrap()
                .active_element()
                .unwrap()
                .get_attribute("data-value")
                .as_deref(),
            Some("b")
        );
        keyboard("ArrowDown");
        assert_eq!(
            web_sys::window()
                .unwrap()
                .document()
                .unwrap()
                .active_element()
                .unwrap()
                .get_attribute("data-value")
                .as_deref(),
            Some("g")
        );
        keyboard("Home");
        assert_eq!(
            web_sys::window()
                .unwrap()
                .document()
                .unwrap()
                .active_element()
                .unwrap()
                .get_attribute("data-value")
                .as_deref(),
            Some("a")
        );
        keyboard("b");
        assert_eq!(
            web_sys::window()
                .unwrap()
                .document()
                .unwrap()
                .active_element()
                .unwrap()
                .get_attribute("data-value")
                .as_deref(),
            Some("b")
        );
        options.update(|options| {
            options.extend(
                (0..60)
                    .map(|index| SelectOption::new(index.to_string(), format!("Choice {index}"))),
            );
        });
        settle().await;
        let menu = mounted
            .element(".ui-dropdown-menu")
            .get_bounding_client_rect();
        let viewport = web_sys::window()
            .unwrap()
            .inner_height()
            .unwrap()
            .as_f64()
            .unwrap();
        assert!(menu.top() >= 0.0 && menu.bottom() <= viewport);
        assert!(menu.height() <= 320.0);
        for theme in ["", "light"] {
            let html = web_sys::window()
                .unwrap()
                .document()
                .unwrap()
                .document_element()
                .unwrap();
            html.set_attribute("data-theme", theme).unwrap();
            let inputs = mounted
                .root
                .query_selector_all(".panel-search-input")
                .unwrap();
            let first: web_sys::Element = inputs.item(0).unwrap().unchecked_into();
            let second: web_sys::Element = inputs.item(1).unwrap().unchecked_into();
            let style = |element| {
                web_sys::window()
                    .unwrap()
                    .get_computed_style(element)
                    .unwrap()
                    .unwrap()
            };
            for property in ["padding", "height", "font-size", "background-color"] {
                assert_eq!(
                    style(&first).get_property_value(property).unwrap(),
                    style(&second).get_property_value(property).unwrap()
                );
            }
        }
        web_sys::window()
            .unwrap()
            .document()
            .unwrap()
            .document_element()
            .unwrap()
            .remove_attribute("data-theme")
            .unwrap();
        keyboard("Escape");
        settle().await;
        assert!(shown.get_untracked());
        assert!(
            mounted
                .root
                .query_selector(".ui-dropdown-menu")
                .unwrap()
                .is_none()
        );
        assert!(
            web_sys::window()
                .unwrap()
                .document()
                .unwrap()
                .active_element()
                .unwrap()
                .is_same_node(Some(mounted.element(".ui-dropdown-trigger").as_ref()))
        );
        super::support::choose_dropdown(&mounted, ".ui-dropdown-trigger", "g").await;
        assert_eq!(value.get_untracked(), "g");
        mounted.click(".ui-dropdown-trigger");
        settle().await;
        mounted.click(".ui-dropdown-backdrop");
        settle().await;
        assert!(
            mounted
                .root
                .query_selector(".ui-dropdown-menu")
                .unwrap()
                .is_none()
        );
        keyboard("Escape");
        settle().await;
        assert!(!shown.get_untracked());
    }
}

#[wasm_bindgen_test]
async fn overflow_actions_close_before_dialogs_and_keep_dialog_callbacks_alive() {
    use openwebide_frontend::{
        components::{PromptDialog, dropdown::ActionMenu},
        state::ui::PromptRequest,
    };
    let submitted = RwSignal::new(String::new());
    let mounted = mount_test(move |state| {
        let ui = state.ui;
        view! {
            <style>{include_str!("../../styles.css")}</style>
            <ActionMenu aria_label="Example actions">
                <button role="menuitem" disabled=true>"Unavailable"</button>
                <button role="menuitem" class="rename" on:click=move |_| ui.set_prompt(PromptRequest {
                    title: "Rename".into(), value: "Original".into(), placeholder: String::new(), submit_label: "Save".into(),
                    on_submit: Callback::new(move |value| submitted.set(value)),
                })>"Rename"</button>
            </ActionMenu>
            <PromptDialog />
        }
    });
    settle().await;
    assert!(
        mounted
            .root
            .query_selector(".ui-dropdown-menu")
            .unwrap()
            .is_none()
    );
    mounted.click(".ui-dropdown-trigger");
    settle().await;
    assert_eq!(
        web_sys::window()
            .unwrap()
            .document()
            .unwrap()
            .active_element()
            .unwrap(),
        mounted.element(".rename").into()
    );
    mounted.click(".rename");
    settle().await;
    assert!(
        mounted
            .root
            .query_selector(".ui-dropdown-menu")
            .unwrap()
            .is_none()
    );
    assert!(
        mounted.element(".modal").contains(
            web_sys::window()
                .unwrap()
                .document()
                .unwrap()
                .active_element()
                .as_ref()
                .map(AsRef::as_ref)
        )
    );
    use wasm_bindgen::JsCast;
    mounted
        .element(".modal input")
        .unchecked_into::<web_sys::HtmlInputElement>()
        .set_value("Changed");
    mounted.click(".modal-footer .send");
    settle().await;
    assert_eq!(submitted.get_untracked(), "Changed");
    assert!(mounted.root.query_selector(".modal").unwrap().is_none());
    assert_eq!(
        web_sys::window()
            .unwrap()
            .document()
            .unwrap()
            .active_element()
            .unwrap(),
        mounted.element(".ui-dropdown-trigger").into()
    );
}

#[wasm_bindgen_test]
async fn action_menus_open_from_rows_headers_and_touch_without_activating_them() {
    use openwebide_frontend::components::dropdown::ActionMenu;
    use wasm_bindgen::JsCast;
    let activations = RwSignal::new(0_u32);
    let mounted = mount_test(move |_| {
        view! {
            { ["session", "connection", "tui-user", "tool-panel-heading", "panel-toolbar", "git-diff-actions"].into_iter().map(|class| view! {
                <div class=class data-context-menu="">
                    <button class="primary" on:click=move |_| activations.update(|count| *count += 1)>"Primary action"</button>
                    <div class="nested-actions"><ActionMenu aria_label="Context actions"><button role="menuitem">"Secondary action"</button></ActionMenu></div>
                </div>
            }).collect_view() }
        }
    });
    settle().await;
    for class in [
        "session",
        "connection",
        "tui-user",
        "tool-panel-heading",
        "panel-toolbar",
        "git-diff-actions",
    ] {
        let row = mounted.element(&format!(".{class}"));
        let event = web_sys::MouseEventInit::new();
        event.set_bubbles(true);
        event.set_cancelable(true);
        event.set_client_x(100);
        event.set_client_y(80);
        let event =
            web_sys::MouseEvent::new_with_mouse_event_init_dict("contextmenu", &event).unwrap();
        row.dispatch_event(&event).unwrap();
        settle().await;
        assert!(event.default_prevented());
        assert!(row.query_selector(".ui-dropdown-menu").unwrap().is_some());
        mounted.click(".ui-dropdown-backdrop");
        settle().await;
        assert!(row.query_selector(".ui-dropdown-menu").unwrap().is_none());
        let primary: web_sys::HtmlElement = row
            .query_selector(".primary")
            .unwrap()
            .unwrap()
            .unchecked_into();
        let pointer = web_sys::PointerEventInit::new();
        pointer.set_bubbles(true);
        pointer.set_pointer_type("touch");
        pointer.set_client_x(100);
        pointer.set_client_y(80);
        primary
            .dispatch_event(
                &web_sys::PointerEvent::new_with_event_init_dict("pointerdown", &pointer).unwrap(),
            )
            .unwrap();
        openwebide_frontend::util::sleep_ms(550).await;
        settle().await;
        assert!(row.query_selector(".ui-dropdown-menu").unwrap().is_some());
        primary.click(); // Synthetic click following the completed long press.
        assert_eq!(activations.get_untracked(), 0);
        mounted.click(".ui-dropdown-backdrop");
        settle().await;
        assert!(row.query_selector(".ui-dropdown-menu").unwrap().is_none());
        assert!(mounted.root.contains(Some(row.as_ref())));
    }
}
