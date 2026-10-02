use leptos::prelude::*;
use openwebide_core::{Project, WorkspaceMode};
use openwebide_frontend::components::TabBar;
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
            <TabBar on_select=Callback::new(|_| ()) on_close=Callback::new(|_| ())
                on_open_local=Callback::new(|()| ()) on_open_remote=Callback::new(|()| ())
                on_open_project=Callback::new(move |id| { state.projects.open_tab(id); })
                on_delete_project=Callback::new(|_| ()) />
        }
    });
    mounted.click_text("Recent");
    settle().await;
    assert_eq!(mounted.element(".recent-menu").child_element_count(), 1);
    assert_eq!(
        mounted.element(".recent-name").text_content().as_deref(),
        Some("Newest")
    );
    assert!(
        mounted
            .root
            .query_selector(".recent-menu .empty")
            .unwrap()
            .is_none()
    );
    mounted.click_text("Newest");
    settle().await;
    mounted.click_text("Recent");
    settle().await;
    assert!(
        mounted
            .root
            .query_selector(".recent-item")
            .unwrap()
            .is_none()
    );
    assert_eq!(
        mounted
            .element(".recent-menu .empty")
            .text_content()
            .as_deref(),
        Some("No other saved projects.")
    );
    mounted.state.projects.open_tab_ids.set(vec![1]);
    settle().await;
    assert_eq!(mounted.element(".recent-menu").child_element_count(), 1);
    assert!(
        mounted
            .root
            .query_selector(".recent-menu .empty")
            .unwrap()
            .is_none()
    );
}
