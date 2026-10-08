use leptos::prelude::*;
use openwebide_frontend::{
    components::SearchPane, state_actions::workspace::WorkspaceActions, util::sleep_ms,
};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

use super::support::{Mounted, mount_test, settle};

fn mount_search() -> Mounted {
    mount_test(|state| {
        state.seed_project();
        let actions = WorkspaceActions::new(
            state.api,
            state.projects,
            state.workspace,
            state.ui,
            RwSignal::new(false),
            Callback::new(|()| ()),
        );
        let layout = expect_context::<openwebide_frontend::state::layout::LayoutState>();
        let layout_actions = openwebide_frontend::state_actions::layout::LayoutActions::new(
            state.api, layout, state.auth, state.ui,
        );
        provide_context(layout_actions);
        let ignored = RwSignal::new(false);
        view! {
            <openwebide_frontend::components::FilesPanel on_new_file=Callback::new(|()| ()) on_new_dir=Callback::new(|()| ())>
            <SearchPane on_open=actions.request_open
                on_search_input=actions.on_search_input on_cancel_search=actions.on_cancel_search
                on_search=actions.on_search on_clear_search=actions.on_clear_search
                include_ignored=ignored.read_only()
                on_toggle_include_ignored=Callback::new(move |()| ignored.update(|v| *v = !*v))><input class="remembered-file-view" value="retained selection" /></SearchPane>
            </openwebide_frontend::components::FilesPanel>
        }
    })
}

fn input(mounted: &Mounted, query: &str) {
    let input: web_sys::HtmlInputElement = mounted.element(".search-input").unchecked_into();
    input.set_value(query);
    input
        .dispatch_event(&web_sys::Event::new("input").unwrap())
        .unwrap();
}

fn now() -> f64 {
    js_sys::Function::new_no_args("return performance.now()")
        .call0(&wasm_bindgen::JsValue::NULL)
        .unwrap()
        .as_f64()
        .unwrap()
}

#[wasm_bindgen_test]
async fn measure_search_typing() {
    for run in 0..3 {
        let mounted = mount_search();
        settle().await;
        let (send, receive) = futures::channel::oneshot::channel();
        mounted
            .state
            .fake
            .search_results
            .borrow_mut()
            .push_back(receive);
        input(&mounted, "old");
        sleep_ms(350).await;
        let before = mounted.state.fake.search_requests.borrow().len();
        let mut final_input = 0.0;
        for event in 0..10 {
            final_input = now();
            input(&mounted, &format!("query{event}"));
            settle().await;
            if event < 9 {
                sleep_ms(50).await;
            }
        }
        send.send(Ok(Vec::new())).unwrap();
        settle().await;
        while mounted
            .state
            .fake
            .search_requests
            .borrow()
            .last()
            .unwrap()
            .1
            != "query9"
        {
            sleep_ms(5).await;
        }
        console_log!(
            "search run {run}: requests/10 inputs={}, latest-query latency={:.1}ms (slow old response included)",
            mounted.state.fake.search_requests.borrow().len() - before,
            now() - final_input
        );
    }
}

#[wasm_bindgen_test]
async fn typing_waits_and_toggle_dispatches_latest_options_immediately() {
    let mounted = mount_search();
    settle().await;
    input(&mounted, "first");
    sleep_ms(100).await;
    input(&mounted, "   ");
    sleep_ms(100).await;
    assert!(mounted.state.fake.search_requests.borrow().is_empty());
    mounted.element("button[title^='Include ignored']").click();
    settle().await;
    assert_eq!(mounted.state.fake.search_requests.borrow().len(), 1);
    let request = mounted.state.fake.search_requests.borrow()[0].clone();
    assert_eq!(request.1, "   ");
    assert!(request.2.include_ignored);
    sleep_ms(350).await;
    assert_eq!(mounted.state.fake.search_requests.borrow().len(), 1);
    input(&mounted, "final");
    sleep_ms(100).await;
    assert_eq!(mounted.state.fake.search_requests.borrow().len(), 1);
    sleep_ms(250).await;
    let requests = mounted.state.fake.search_requests.borrow();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[1].1, "final");
    assert!(requests[1].2.include_ignored);
}

#[wasm_bindgen_test]
async fn old_responses_cannot_publish_during_or_after_debounce() {
    for response_delay in [100, 350] {
        let mounted = mount_search();
        settle().await;
        let (send, receive) = futures::channel::oneshot::channel();
        mounted
            .state
            .fake
            .search_results
            .borrow_mut()
            .push_back(receive);
        input(&mounted, "old");
        sleep_ms(350).await;
        input(&mounted, "new");
        sleep_ms(response_delay).await;
        send.send(Ok(vec![openwebide_core::SearchHit {
            path: "stale.txt".into(),
            line: 1,
            text: "stale".into(),
        }]))
        .unwrap();
        settle().await;
        assert!(!mounted.root.text_content().unwrap().contains("stale"));
        sleep_ms(350).await;
        assert_eq!(
            mounted.state.workspace.search.get_untracked(),
            Some(Vec::new())
        );
    }
}

#[wasm_bindgen_test]
async fn clear_cancels_before_and_after_dispatch() {
    let mounted = mount_search();
    settle().await;
    input(&mounted, "pending");
    sleep_ms(150).await;
    input(&mounted, "");
    assert!(mounted.state.workspace.search.get_untracked().is_none());
    sleep_ms(200).await;
    assert!(mounted.state.fake.search_requests.borrow().is_empty());
    let (send, receive) = futures::channel::oneshot::channel();
    mounted
        .state
        .fake
        .search_results
        .borrow_mut()
        .push_back(receive);
    input(&mounted, "dispatched");
    sleep_ms(350).await;
    input(&mounted, "");
    send.send(Ok(Vec::new())).unwrap();
    settle().await;
    assert!(mounted.state.workspace.search.get_untracked().is_none());
}

#[wasm_bindgen_test]
async fn project_switch_back_and_unmount_cancel_pending_search() {
    let mounted = mount_search();
    settle().await;
    input(&mounted, "pending");
    mounted.state.projects.active_project.set(None);
    mounted.state.projects.active_project.set(Some(1));
    sleep_ms(350).await;
    assert!(mounted.state.fake.search_requests.borrow().is_empty());
    let (send, receive) = futures::channel::oneshot::channel();
    mounted
        .state
        .fake
        .search_results
        .borrow_mut()
        .push_back(receive);
    input(&mounted, "old project");
    sleep_ms(350).await;
    mounted.state.projects.active_project.set(None);
    mounted.state.projects.active_project.set(Some(1));
    send.send(Err("stale error".into())).unwrap();
    settle().await;
    assert!(mounted.state.ui.toast.get_untracked().is_none());
    input(&mounted, "unmounted");
    let fake = mounted.state.fake.clone();
    let count = fake.search_requests.borrow().len();
    drop(mounted);
    sleep_ms(350).await;
    assert_eq!(fake.search_requests.borrow().len(), count);
}

#[wasm_bindgen_test]
async fn persistent_search_replaces_file_view_and_clearing_restores_it_in_both_modes() {
    use leptos::prelude::*;
    use openwebide_core::WorkspaceMode;
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        let mounted = mount_search();
        mounted
            .state
            .projects
            .projects
            .update(|projects| projects[0].mode = mode);
        settle().await;
        let file_view = mounted.element(".remembered-file-view");
        let views = mounted.element(".search-file-views");
        assert!(!views.has_attribute("hidden"));
        mounted.click("[data-file-search-toggle]");
        settle().await;
        input(&mounted, "needle");
        settle().await;
        assert!(views.has_attribute("hidden"));
        assert!(!mounted.element(".search-results").has_attribute("hidden"));
        input(&mounted, "");
        settle().await;
        assert!(!views.has_attribute("hidden"));
        assert!(mounted.element(".search-results").has_attribute("hidden"));
        assert!(file_view.is_same_node(Some(mounted.element(".remembered-file-view").as_ref())));
        openwebide_frontend::util::sleep_ms(300).await;
        assert!(mounted.state.fake.search_requests.borrow().is_empty());
    }
}

#[wasm_bindgen_test]
async fn files_search_is_on_demand_and_escape_cancels_and_restores_focus_in_both_modes() {
    use openwebide_core::WorkspaceMode;
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        let mounted = mount_search();
        mounted
            .state
            .projects
            .projects
            .update(|projects| projects[0].mode = mode);
        settle().await;
        assert!(mounted.element("#project-search").has_attribute("hidden"));
        mounted.click("[data-file-search-toggle]");
        settle().await;
        let field = mounted.element(".search-input");
        assert!(
            web_sys::window()
                .unwrap()
                .document()
                .unwrap()
                .active_element()
                .unwrap()
                .is_same_node(Some(&field))
        );
        input(&mounted, "pending");
        settle().await;
        assert!(
            !mounted
                .element(".files-panel-toolbar")
                .has_attribute("hidden")
        );
        let init = web_sys::KeyboardEventInit::new();
        init.set_key("Escape");
        init.set_bubbles(true);
        field
            .dispatch_event(
                &web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init)
                    .unwrap(),
            )
            .unwrap();
        settle().await;
        assert!(mounted.element("#project-search").has_attribute("hidden"));
        assert!(
            !mounted
                .element(".search-file-views")
                .has_attribute("hidden")
        );
        assert!(
            web_sys::window()
                .unwrap()
                .document()
                .unwrap()
                .active_element()
                .unwrap()
                .is_same_node(Some(&mounted.element("[data-file-search-toggle]")))
        );
        sleep_ms(350).await;
        assert!(mounted.state.fake.search_requests.borrow().is_empty());
    }
}
