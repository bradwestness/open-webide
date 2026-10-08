use crate::state::layout::LayoutState;
use leptos::prelude::*;

/// Conversation navigation; app configuration lives in the app menu.
#[component]
pub fn Sidebar(
    on_select_session: Callback<i64>,
    on_new_session: Callback<()>,
    on_rename_session: Callback<i64>,
    on_delete_session: Callback<i64>,
) -> impl IntoView {
    let layout = expect_context::<LayoutState>();
    view! { <aside class="sidebar" style=move || format!("width: {}px; flex: none;", layout.sidebar_width.get())>
        <super::session_list::SessionList on_select=on_select_session on_new=on_new_session on_rename=on_rename_session on_delete=on_delete_session />
        <super::memories::Memories />
    </aside> }
}
