use crate::{
    state::layout::{LayoutState, Panel},
    state_actions::layout::LayoutActions,
};
use leptos::prelude::*;

/// Collapsing retains the mounted editor, drafts, run subscriptions and terminal.
#[component]
pub fn ToolPanel(panel: Panel, children: Children) -> impl IntoView {
    let layout = expect_context::<LayoutState>();
    view! { <div id=format!("panel-{}", panel.id()) class="tool-panel" style=move || if layout.visible_panels.get().visible(panel) { "display: contents" } else { "display: none" }>{children()}</div> }
}

#[component]
pub fn PanelRail(panels: Vec<Panel>) -> impl IntoView {
    let layout = expect_context::<LayoutState>();
    let actions = expect_context::<LayoutActions>();
    view! { <nav class="panel-rail" aria-label="Workspace panels">
        {panels.into_iter().map(|panel| view! {
            <button class="btn panel-tab" class:is-open=move || layout.visible_panels.get().visible(panel)
                aria-controls=format!("panel-{}", panel.id()) aria-expanded=move || layout.visible_panels.get().visible(panel).to_string()
                disabled=move || !layout.available(panel)
                title=move || if layout.available(panel) { format!("Show or collapse {}", panel.label()) } else { format!("Open a project to use {}", panel.label()) } on:click=move |_| actions.toggle.run(panel)>{panel.label()}</button>
        }).collect_view()}
    </nav> }
}
