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
    let mounted = mount_test(move |mut state| {
        state.settings = SettingsState::new(Theme::from_root(), "ws://localhost:3001".into());
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
        .send(Ok(std::collections::BTreeMap::from([(
            "theme".into(),
            "dark".into(),
        )])))
        .unwrap();
    // Startup includes an IndexedDB await before the independent settings reads.
    for _ in 0..100 {
        openwebide_frontend::util::sleep_ms(10).await;
        if root.get_attribute("data-theme").as_deref() == Some("dark") {
            break;
        }
    }
    assert_eq!(root.get_attribute("data-theme").as_deref(), Some("dark"));
    drop(mounted);
    storage.remove_item("owide-theme").unwrap();
    if let Some(original) = original {
        root.set_attribute("data-theme", &original).unwrap();
    } else {
        root.remove_attribute("data-theme").unwrap();
    }
}
