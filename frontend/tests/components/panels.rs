use super::support::{mount_test, settle};
use leptos::prelude::*;
use openwebide_core::{User, UserId, UserRole, WorkspaceMode};
use openwebide_frontend::{
    components::{PanelRail, ToolPanel},
    state::{
        auth::AuthState,
        layout::{LayoutState, PANEL_VISIBILITY_KEY, Panel, PanelVisibility},
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
                <ToolPanel panel=Panel::Sessions><input value="server draft" /></ToolPanel>
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
            mounted.click(&format!(
                "#panel-{} button[aria-label='Minimize {}']",
                panel.id(),
                panel.label()
            ));
            settle().await;
            assert_eq!(
                mounted
                    .element(&button)
                    .get_attribute("aria-expanded")
                    .as_deref(),
                Some("false")
            );
            assert!(element.get_attribute("style").unwrap().contains("none"));
            assert!(child.is_same_node(element.first_element_child().as_ref().map(AsRef::as_ref)));
            mounted.click(&button);
            settle().await;
            assert!(child.is_same_node(element.first_element_child().as_ref().map(AsRef::as_ref)));
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
            Some(if matches!(panel, "files" | "editor") {
                "false"
            } else {
                "true"
            })
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
        state.seed_project();
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

#[wasm_bindgen_test]
async fn projectless_chat_hides_and_disables_files_and_editor_without_saving_over_preferences() {
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        let actions_slot = RwSignal::new(None);
        let layout_slot = RwSignal::new(None);
        let mounted = mount_test(move |state| {
            state.seed_project();
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            let auth = expect_context::<AuthState>();
            auth.set_user(user(1));
            let layout = expect_context::<LayoutState>();
            let actions = LayoutActions::new(state.api, layout, auth, state.ui);
            actions_slot.set(Some(actions));
            layout_slot.set(Some(layout));
            provide_context(actions);
            view! {
                <PanelRail panels=vec![Panel::Sessions, Panel::Files, Panel::Editor, Panel::Chat] />
                <ToolPanel panel=Panel::Files><input value="file draft" /></ToolPanel>
                <ToolPanel panel=Panel::Editor><textarea>"editor draft"</textarea></ToolPanel>
                <ToolPanel panel=Panel::Chat><textarea>"chat draft"</textarea></ToolPanel>
            }
        });
        settle().await;
        let layout = layout_slot.get_untracked().unwrap();
        actions_slot
            .get_untracked()
            .unwrap()
            .toggle
            .run(Panel::Files);
        settle().await;
        let saved = mounted.state.fake.settings.borrow()[PANEL_VISIBILITY_KEY].clone();
        let preferences = layout.panels.get_untracked();
        assert!(!preferences.files && preferences.editor);
        let editor = mounted.element("#panel-editor textarea");
        mounted.state.projects.active_project.set(None);
        settle().await;
        for panel in [Panel::Files, Panel::Editor] {
            let button = mounted.element(&format!("button[aria-controls='panel-{}']", panel.id()));
            assert!(button.has_attribute("disabled"));
            assert_eq!(
                button.get_attribute("aria-expanded").as_deref(),
                Some("false")
            );
            assert!(
                mounted
                    .element(&format!("#panel-{}", panel.id()))
                    .get_attribute("style")
                    .unwrap()
                    .contains("none")
            );
            actions_slot.get_untracked().unwrap().toggle.run(panel);
        }
        settle().await;
        assert_eq!(layout.panels.get_untracked(), preferences);
        assert_eq!(
            mounted.state.fake.settings.borrow()[PANEL_VISIBILITY_KEY],
            saved
        );
        mounted.state.projects.active_project.set(Some(1));
        settle().await;
        assert_eq!(layout.visible_panels.get_untracked(), preferences);
        assert!(
            !mounted
                .element("button[aria-controls='panel-editor']")
                .has_attribute("disabled")
        );
        assert!(
            mounted
                .element("#panel-editor")
                .get_attribute("style")
                .unwrap()
                .contains("flex")
        );
        assert!(editor.is_same_node(Some(mounted.element("#panel-editor textarea").as_ref())));
    }
}

#[wasm_bindgen_test]
async fn phone_sheets_keep_drafts_and_desktop_preferences_in_both_modes() {
    use super::support::wait_until;
    use openwebide_frontend::state::responsive::{LAYOUT_PREFERENCES_KEY, LayoutMode, PanelSide};
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        let slot = std::rc::Rc::new(std::cell::Cell::new(None));
        let read = slot.clone();
        let mounted = mount_test(move |state| {
            state.seed_project();
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            let auth = expect_context::<AuthState>();
            auth.set_user(user(1));
            let layout = expect_context::<LayoutState>();
            let actions = LayoutActions::new(state.api, layout, auth, state.ui);
            provide_context(actions);
            read.set(Some((layout, actions)));
            view! {
                <PanelRail panels=vec![Panel::Files, Panel::Chat, Panel::Terminal, Panel::Git, Panel::Search] />
                <ToolPanel panel=Panel::Files><input value="file draft" /></ToolPanel>
                <ToolPanel panel=Panel::Chat><textarea>"chat draft"</textarea></ToolPanel>
                <ToolPanel panel=Panel::Search><input value="query" /></ToolPanel>
                <ToolPanel panel=Panel::Git><span>"Git"</span></ToolPanel>
                <ToolPanel panel=Panel::Terminal><input value="command" /></ToolPanel>
            }
        });
        settle().await;
        let (layout, actions) = slot.get().unwrap();
        let remembered = layout.panels.get_untracked();
        let draft = mounted.element("#panel-chat textarea");
        layout.fit(390.0);
        settle().await;
        assert!(layout.phone.get_untracked());
        assert!(layout.visible_panels.get_untracked().chat);
        for panel in [Panel::Files, Panel::Search, Panel::Git, Panel::Terminal] {
            actions.show.run(panel);
            settle().await;
            assert!(layout.visible_panels.get_untracked().visible(
                if matches!(panel, Panel::Git | Panel::Search) {
                    Panel::Files
                } else {
                    panel
                }
            ));
            assert!(!layout.visible_panels.get_untracked().chat);
            mounted.click(&format!(
                "#panel-{} .tool-panel-heading > button",
                if matches!(panel, Panel::Git | Panel::Search) {
                    Panel::Files
                } else {
                    panel
                }
                .id()
            ));
            settle().await;
            assert!(layout.visible_panels.get_untracked().chat);
        }
        assert_eq!(layout.panels.get_untracked(), remembered);
        assert!(draft.is_same_node(Some(mounted.element("#panel-chat textarea").as_ref())));
        actions.set_mode.run(LayoutMode::Desktop);
        settle().await;
        assert!(!layout.phone.get_untracked());
        assert_eq!(layout.panels.get_untracked(), remembered);
        actions.pin.run((Panel::Chat, PanelSide::Left));
        wait_until("saved workspace layout", || {
            mounted.state.fake.settings.borrow().get(LAYOUT_PREFERENCES_KEY)
                .and_then(|value| serde_json::from_str::<openwebide_frontend::state::responsive::LayoutPreferences>(value).ok())
                .is_some_and(|prefs| prefs.mode == LayoutMode::Desktop && prefs.side("chat") == PanelSide::Left)
        }).await;
        let saved: openwebide_frontend::state::responsive::LayoutPreferences =
            serde_json::from_str(&mounted.state.fake.settings.borrow()[LAYOUT_PREFERENCES_KEY])
                .unwrap();
        assert_eq!(saved.mode, LayoutMode::Desktop);
        assert_eq!(saved.side("chat"), PanelSide::Left);
        layout.active_project.set(None);
        actions.set_mode.run(LayoutMode::Phone);
        settle().await;
        for panel in [Panel::Files, Panel::Terminal, Panel::Git, Panel::Search] {
            assert!(!layout.available(panel));
        }
        assert!(layout.visible_panels.get_untracked().chat);
    }
}

#[wasm_bindgen_test]
async fn git_window_loads_diffs_only_when_open_in_a_repository_in_both_modes() {
    use openwebide_frontend::components::GitPane;
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        let loads = RwSignal::new(0);
        let slot = std::rc::Rc::new(std::cell::Cell::new(None));
        let read = slot.clone();
        let mounted = mount_test(move |state| {
            state.seed_project();
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            state.workspace.open_file.set(Some("demo.rs".into()));
            let layout = expect_context::<LayoutState>();
            read.set(Some(layout));
            view! { <GitPane on_open=Callback::new(|_| ()) on_load_git_diff=Callback::new(move |()| loads.update(|value| *value += 1)) on_discard_git_diff=Callback::new(|()| ()) /> }
        });
        settle().await;
        assert_eq!(loads.get_untracked(), 0);
        let layout = slot.get().unwrap();
        layout.preferences.update(|prefs| {
            prefs.files_view = openwebide_frontend::state::responsive::FilesView::Changes;
        });
        layout.panels.update(|panels| panels.files = true);
        settle().await;
        assert_eq!(loads.get_untracked(), 0);
        mounted
            .state
            .git
            .status
            .set(Some(openwebide_core::GitRepoStatus::default()));
        settle().await;
        assert_eq!(loads.get_untracked(), 1);
        layout.panels.update(|panels| panels.files = false);
        settle().await;
        mounted
            .state
            .workspace
            .open_file
            .set(Some("other.rs".into()));
        settle().await;
        assert_eq!(loads.get_untracked(), 1);
        layout.preferences.update(|prefs| {
            prefs.files_view = openwebide_frontend::state::responsive::FilesView::Changes;
        });
        layout.panels.update(|panels| panels.files = true);
        settle().await;
        assert_eq!(loads.get_untracked(), 2);
        mounted.state.projects.active_project.set(None);
        settle().await;
        assert_eq!(loads.get_untracked(), 2);
    }
}

#[wasm_bindgen_test]
async fn shared_panel_resizers_follow_the_panel_side_and_save_widths_in_both_modes() {
    use openwebide_frontend::state::{
        layout::ActiveResizer,
        responsive::{LayoutMode, PanelSide},
    };
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        let slot = std::rc::Rc::new(std::cell::Cell::new(None));
        let read = slot.clone();
        let mounted = mount_test(move |state| {
            state.seed_project();
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            let auth = expect_context::<AuthState>();
            auth.set_user(user(1));
            let layout = expect_context::<LayoutState>();
            let actions = LayoutActions::new(state.api, layout, auth, state.ui);
            provide_context(actions);
            read.set(Some((layout, actions)));
            view! {
                <ToolPanel panel=Panel::Sessions><span>"Sessions"</span></ToolPanel>
                <ToolPanel panel=Panel::Files><span>"Files"</span></ToolPanel>
                <ToolPanel panel=Panel::Chat><span>"Chat"</span></ToolPanel>
                <ToolPanel panel=Panel::Terminal><span>"Terminal"</span></ToolPanel>
            }
        });
        settle().await;
        let (layout, actions) = slot.get().unwrap();
        actions.set_mode.run(LayoutMode::Desktop);
        layout.fit(2400.0);
        actions.show.run(Panel::Terminal);
        settle().await;
        for (panel, kind) in [
            (Panel::Sessions, ActiveResizer::Sidebar),
            (Panel::Files, ActiveResizer::Tree),
            (Panel::Chat, ActiveResizer::Chat),
            (Panel::Terminal, ActiveResizer::Terminal),
        ] {
            let separator = mounted.element(&format!("#panel-{} [role=separator]", panel.id()));
            for side in [PanelSide::Left, PanelSide::Right] {
                actions.pin.run((panel, side));
                settle().await;
                layout.fit(2400.0);
                let before = layout.width(kind).get_untracked();
                let init = web_sys::KeyboardEventInit::new();
                init.set_key("ArrowRight");
                init.set_cancelable(true);
                separator
                    .dispatch_event(
                        &web_sys::KeyboardEvent::new_with_keyboard_event_init_dict(
                            "keydown", &init,
                        )
                        .unwrap(),
                    )
                    .unwrap();
                settle().await;
                let expected = before + if side == PanelSide::Left { 20.0 } else { -20.0 };
                assert!((layout.width(kind).get_untracked() - expected).abs() < f64::EPSILON);
                assert_eq!(
                    mounted.state.fake.settings.borrow()[kind.setting_key()],
                    expected.to_string()
                );
            }
        }
        assert_eq!(
            mounted
                .root
                .query_selector_all(".panel-resizer")
                .unwrap()
                .length(),
            4
        );
        actions.move_panel.run((Panel::Terminal, false));
        settle().await;
        assert!(
            mounted
                .state
                .fake
                .settings
                .borrow()
                .contains_key("workspace_layout")
        );
        assert!(mounted.root.query_selector(".panel-pin").unwrap().is_none());
    }
}

#[wasm_bindgen_test]
#[allow(
    clippy::cast_possible_truncation,
    reason = "Browser hit testing uses f32 coordinates and pointer events use integer CSS pixels."
)]
async fn dock_resize_targets_are_visible_and_resize_actual_geometry_in_both_modes() {
    use openwebide_frontend::state::{
        layout::ActiveResizer,
        responsive::{LayoutMode, PanelSide},
    };
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        let slot = std::rc::Rc::new(std::cell::Cell::new(None));
        let read = slot.clone();
        let mounted = mount_test(move |state| {
            state.seed_project();
            state.auth.set_user(user(1));
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            let layout = expect_context::<LayoutState>();
            let actions = LayoutActions::new(state.api, layout, state.auth, state.ui);
            provide_context(actions);
            read.set(Some((layout, actions)));
            view! {
                <style>{include_str!("../../styles.css")}</style>
                <div class="app" style="height:400px;width:1000px"><div class="app-body">
                    <ToolPanel panel=Panel::Sessions><div class="sidebar" style="width:240px;flex:none">"Sessions"</div></ToolPanel>
                    <ToolPanel panel=Panel::Files><div class="search-pane">"Files"</div></ToolPanel>
                    <ToolPanel panel=Panel::Chat><div class="chat-pane" style="width:420px;flex:none">"Chat"</div></ToolPanel>
                    <ToolPanel panel=Panel::Terminal><div class="terminal-visibility"><div class="terminal-dock">"Terminal"</div></div></ToolPanel>
                </div></div>
            }
        });
        let (layout, actions) = slot.get().unwrap();
        actions.set_mode.run(LayoutMode::Desktop);
        for (panel, kind) in [
            (Panel::Sessions, ActiveResizer::Sidebar),
            (Panel::Files, ActiveResizer::Tree),
            (Panel::Chat, ActiveResizer::Chat),
            (Panel::Terminal, ActiveResizer::Terminal),
        ] {
            layout.panels.set(PanelVisibility {
                sessions: panel == Panel::Sessions,
                files: panel == Panel::Files,
                chat: panel == Panel::Chat,
                terminal: panel == Panel::Terminal,
                editor: false,
                ..PanelVisibility::default()
            });
            for side in [PanelSide::Left, PanelSide::Right] {
                actions.pin.run((panel, side));
                settle().await;
                let dock = mounted.element(&format!("#panel-{}", panel.id()));
                let separator =
                    mounted.element(&format!("#panel-{} > [role=separator]", panel.id()));
                let rect = separator.get_bounding_client_rect();
                assert!(rect.height() > 300.0);
                let x = rect.x() + rect.width() / 2.0;
                let y = rect.y() + rect.height() / 2.0;
                let hit = web_sys::window()
                    .unwrap()
                    .document()
                    .unwrap()
                    .element_from_point(x as f32, y as f32)
                    .unwrap();
                assert!(
                    hit.is_same_node(Some(separator.as_ref())),
                    "{} handle must receive pointer input",
                    panel.id()
                );
                let before = dock.get_bounding_client_rect().width();
                let event = |name, x: f64| {
                    let init = web_sys::PointerEventInit::new();
                    init.set_client_x(x as i32);
                    init.set_bubbles(true);
                    web_sys::PointerEvent::new_with_event_init_dict(name, &init).unwrap()
                };
                hit.dispatch_event(&event("pointerdown", x)).unwrap();
                web_sys::window()
                    .unwrap()
                    .dispatch_event(&event(
                        "pointermove",
                        x + if side == PanelSide::Left { 30.0 } else { -30.0 },
                    ))
                    .unwrap();
                web_sys::window()
                    .unwrap()
                    .dispatch_event(&event("pointerup", x))
                    .unwrap();
                settle().await;
                assert!(
                    (dock.get_bounding_client_rect().width() - before - 30.0).abs() < 1.0,
                    "{} geometry follows drag",
                    panel.id()
                );
                assert_eq!(
                    mounted.state.fake.settings.borrow()[kind.setting_key()],
                    layout.width(kind).get_untracked().to_string()
                );
            }
        }
    }
}

#[wasm_bindgen_test]
async fn changes_and_explorer_share_file_layout_and_single_view_headings_in_both_modes() {
    use openwebide_core::{FileEntry, GitFileStatus, GitRepoStatus};
    use openwebide_frontend::components::{FileTree, GitPane};
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        let mounted = mount_test(move |state| {
            state.seed_project();
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            state.workspace.entries.update(|entries| {
                entries.insert(
                    String::new(),
                    vec![FileEntry {
                        name: "demo.rs".into(),
                        path: "demo.rs".into(),
                        is_dir: false,
                        size: 20,
                    }],
                );
            });
            state.workspace.open_file.set(Some("demo.rs".into()));
            state.git.status.set(Some(GitRepoStatus {
                branch: "main".into(),
                files: [("demo.rs".into(), GitFileStatus::Modified)].into(),
                ..GitRepoStatus::default()
            }));
            view! {
                <style>{include_str!("../../styles.css")}</style>
                <div style="display:flex;width:700px;height:500px">
                    <FileTree on_toggle=Callback::new(|_| ()) on_open=Callback::new(|_| ()) on_new_file=Callback::new(|()| ()) on_new_dir=Callback::new(|()| ()) />
                    <GitPane on_open=Callback::new(move |path| state.workspace.open_file.set(Some(path))) on_load_git_diff=Callback::new(|()| ()) on_discard_git_diff=Callback::new(|()| ()) />
                </div>
            }
        });
        settle().await;
        let style = |selector| {
            web_sys::window()
                .unwrap()
                .get_computed_style(&mounted.element(selector))
                .unwrap()
                .unwrap()
        };
        assert!(mounted.root.query_selector("h2").unwrap().is_none());
        for property in ["padding-left", "padding-right", "padding-top"] {
            assert_eq!(
                style(".file-tree").get_property_value(property).unwrap(),
                style(".git-pane").get_property_value(property).unwrap()
            );
        }
        for property in [
            "padding-left",
            "padding-top",
            "font-size",
            "gap",
            "background-color",
        ] {
            assert_eq!(
                style(".file-tree .tree-item")
                    .get_property_value(property)
                    .unwrap(),
                style(".git-files .tree-item")
                    .get_property_value(property)
                    .unwrap()
            );
        }
        assert_eq!(
            style(".git-files .tree-item")
                .get_property_value("justify-content")
                .unwrap(),
            "flex-start"
        );
        assert!(
            mounted
                .element(".git-files .tree-item")
                .class_list()
                .contains("selected")
        );
        assert!(
            mounted
                .element(".git-files .tree-item .tree-icon")
                .query_selector("svg")
                .unwrap()
                .is_some()
        );
        mounted.state.workspace.open_file.set(None);
        settle().await;
        assert!(
            !mounted
                .element(".git-files .tree-item")
                .class_list()
                .contains("selected")
        );
        mounted.click(".git-files .tree-item");
        settle().await;
        assert_eq!(
            mounted.state.workspace.open_file.get_untracked().as_deref(),
            Some("demo.rs")
        );
    }
}
