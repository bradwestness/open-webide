use super::support::{mount_test, settle};
use leptos::prelude::*;
use openwebide_core::{User, UserId, UserRole, WorkspaceMode};
use openwebide_frontend::{
    components::{PanelRail, PanelResizer, ToolPanel},
    state::{
        auth::AuthState,
        layout::{ActiveResizer, LayoutState, PANEL_VISIBILITY_KEY, Panel, PanelVisibility},
    },
    state_actions::{
        layout::LayoutActions,
        lifecycle::{ProjectEffectContext, install_project_effects},
    },
};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

fn user(id: i64) -> User {
    User {
        id: UserId::new(id),
        username: format!("user-{id}"),
        role: UserRole::User,
        created_at: 0,
    }
}

#[wasm_bindgen_test]
async fn panels_collapse_without_unmounting_and_persist_in_both_modes() {
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        let mounted = mount_test(move |state| {
            state.seed_project();
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            let auth = expect_context::<AuthState>();
            auth.set_user(user(1));
            let layout = expect_context::<LayoutState>();
            provide_context(LayoutActions::new(state.api, layout, auth, state.ui));
            view! {
                <PanelRail panels=vec![Panel::Sessions, Panel::Files, Panel::Editor, Panel::Chat] />
                <ToolPanel panel=Panel::Sessions><input value="server draft" /><PanelResizer kind=ActiveResizer::Sidebar /></ToolPanel>
                <ToolPanel panel=Panel::Files><input value="search draft" /></ToolPanel>
                <ToolPanel panel=Panel::Editor><textarea>"unsaved editor"</textarea></ToolPanel>
                <ToolPanel panel=Panel::Chat><textarea>"unsent prompt"</textarea></ToolPanel>
            }
        });
        settle().await;
        for panel in [Panel::Sessions, Panel::Files, Panel::Editor, Panel::Chat] {
            let selector = format!("#panel-{}", panel.id());
            let element = mounted.element(&selector);
            let child = element.first_element_child().unwrap();
            let button = format!("button[aria-controls='panel-{}']", panel.id());
            mounted.click(&button);
            settle().await;
            assert_eq!(
                mounted
                    .element(&button)
                    .get_attribute("aria-expanded")
                    .as_deref(),
                Some("false")
            );
            assert!(element.get_attribute("style").unwrap().contains("none"));
            assert!(
                child.is_same_node(
                    element
                        .first_element_child()
                        .as_ref()
                        .map(|node| node.as_ref())
                )
            );
            mounted.click(&button);
            settle().await;
            assert!(
                child.is_same_node(
                    element
                        .first_element_child()
                        .as_ref()
                        .map(|node| node.as_ref())
                )
            );
            assert_eq!(
                mounted
                    .element(&button)
                    .get_attribute("aria-expanded")
                    .as_deref(),
                Some("true")
            );
        }
        mounted.click("button[aria-controls='panel-editor']");
        mounted.click("button[aria-controls='panel-chat']");
        settle().await;
        let saved: PanelVisibility =
            serde_json::from_str(&mounted.state.fake.settings.borrow()[PANEL_VISIBILITY_KEY])
                .unwrap();
        assert!(!saved.editor && !saved.chat && saved.sessions && saved.files);
        let draft: web_sys::HtmlTextAreaElement =
            mounted.element("#panel-chat textarea").unchecked_into();
        assert_eq!(draft.value(), "unsent prompt");
    }
}

#[wasm_bindgen_test]
async fn saved_panel_layout_restores_and_account_changes_clear_it() {
    let slot = std::rc::Rc::new(std::cell::Cell::new(None));
    let auth_slot = slot.clone();
    let mounted = mount_test(move |state| {
        let auth = expect_context::<AuthState>();
        auth_slot.set(Some(auth));
        auth.set_user(user(1));
        let layout = expect_context::<LayoutState>();
        state.fake.settings.borrow_mut().insert(
            PANEL_VISIBILITY_KEY.into(),
            r#"{"sessions":false,"files":true,"editor":false,"chat":true}"#.into(),
        );
        provide_context(LayoutActions::new(state.api, layout, auth, state.ui));
        install_project_effects(ProjectEffectContext {
            api: state.api,
            health: RwSignal::new(None),
            auth,
            settings: state.settings,
            projects: state.projects,
            chat: state.chat,
            layout,
            select_project: Callback::new(|_| ()),
        });
        view! { <PanelRail panels=vec![Panel::Sessions, Panel::Files, Panel::Editor, Panel::Chat] /> }
    });
    for _ in 0..100 {
        openwebide_frontend::util::sleep_ms(10).await;
        if mounted
            .element("button[aria-controls='panel-sessions']")
            .get_attribute("aria-expanded")
            .as_deref()
            == Some("false")
        {
            break;
        }
    }
    assert_eq!(
        mounted
            .element("button[aria-controls='panel-sessions']")
            .get_attribute("aria-expanded")
            .as_deref(),
        Some("false")
    );
    assert_eq!(
        mounted
            .element("button[aria-controls='panel-editor']")
            .get_attribute("aria-expanded")
            .as_deref(),
        Some("false")
    );
    slot.get().unwrap().logout();
    settle().await;
    for panel in ["sessions", "files", "editor", "chat"] {
        assert_eq!(
            mounted
                .element(&format!("button[aria-controls='panel-{panel}']"))
                .get_attribute("aria-expanded")
                .as_deref(),
            Some("true")
        );
    }
}

#[wasm_bindgen_test]
async fn scroll_regions_follow_dark_and_light_theme_tokens() {
    let document = web_sys::window().unwrap().document().unwrap();
    let root = document.document_element().unwrap();
    let original = root.get_attribute("data-theme");
    let mounted = mount_test(
        |_| view! { <style>{include_str!("../../styles.css")}</style><div class="scroll-check" style="overflow:auto;height:40px"><div style="height:200px">"scroll"</div></div> },
    );
    let mut colors = Vec::new();
    for theme in ["dark", "light"] {
        root.set_attribute("data-theme", theme).unwrap();
        let style = web_sys::window()
            .unwrap()
            .get_computed_style(&mounted.element(".scroll-check"))
            .unwrap()
            .unwrap();
        let colors_for_theme = style.get_property_value("scrollbar-color").unwrap();
        assert!(!colors_for_theme.is_empty() && colors_for_theme != "auto");
        colors.push(colors_for_theme);
    }
    assert_ne!(colors[0], colors[1]);
    if let Some(original) = original {
        root.set_attribute("data-theme", &original).unwrap();
    } else {
        root.remove_attribute("data-theme").unwrap();
    }
}

#[wasm_bindgen_test]
async fn rapid_panel_toggles_save_in_order_and_late_loading_preserves_changes() {
    let slot = std::rc::Rc::new(std::cell::Cell::new(None));
    let state_slot = slot.clone();
    let mounted = mount_test(move |state| {
        let auth = expect_context::<AuthState>();
        auth.set_user(user(1));
        let layout = expect_context::<LayoutState>();
        state_slot.set(Some(layout));
        provide_context(LayoutActions::new(state.api, layout, auth, state.ui));
        view! { <PanelRail panels=vec![Panel::Sessions, Panel::Files, Panel::Editor, Panel::Chat] /> }
    });
    settle().await;
    let (release, pending) = futures::channel::oneshot::channel();
    mounted
        .state
        .fake
        .panel_save_results
        .borrow_mut()
        .push_back(pending);
    mounted.click("button[aria-controls='panel-chat']");
    settle().await;
    mounted.click("button[aria-controls='panel-editor']");
    mounted.click("button[aria-controls='panel-chat']");
    settle().await;
    // Settings arriving after the user's first interaction must not undo it.
    slot.get().unwrap().restore_panels(Some(
        &serde_json::to_string(&PanelVisibility::default()).unwrap(),
    ));
    assert_eq!(
        mounted
            .element("button[aria-controls='panel-editor']")
            .get_attribute("aria-expanded")
            .as_deref(),
        Some("false")
    );
    release.send(Ok(())).unwrap();
    settle().await;
    let saved: PanelVisibility =
        serde_json::from_str(&mounted.state.fake.settings.borrow()[PANEL_VISIBILITY_KEY]).unwrap();
    assert!(!saved.editor && saved.chat);
    let writes = mounted.state.fake.calls.borrow().iter().filter(|call| matches!(call, openwebide_frontend::testing::fake_backend::Call::SetSetting { key, .. } if key == PANEL_VISIBILITY_KEY)).count();
    assert_eq!(writes, 2);
}
