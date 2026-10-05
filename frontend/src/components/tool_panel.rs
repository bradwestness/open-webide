use crate::{
    state::{
        layout::{LayoutState, Panel},
        responsive::PanelSide,
    },
    state_actions::layout::LayoutActions,
};
use leptos::prelude::*;

/// Collapsing retains the mounted editor, drafts, run subscriptions and terminal.
#[component]
pub fn ToolPanel(panel: Panel, children: Children) -> impl IntoView {
    let layout = expect_context::<LayoutState>();
    let actions = expect_context::<LayoutActions>();
    view! { <section id=format!("panel-{}", panel.id()) class="tool-panel"
        class:tool-panel-center=panel == Panel::Editor
        style=move || {
            let order = layout.preferences.with(|prefs| prefs.order(panel.id()));
            format!("display: {}; order: {order}; --tool-window-width: {}px;", if layout.visible_panels.get().visible(panel) { "flex" } else { "none" }, panel.fixed_width())
        }
        aria-label=panel.label()>
        <div class="tool-panel-heading">
            <span>{panel.label()}</span>
            <button class="btn" title="Return to chat" on:click=move |_| actions.show.run(Panel::Chat)>"Back to chat"</button>
        </div>
        <div class="tool-panel-content">{children()}</div>
    </section> }
}

#[component]
pub fn PanelRail(panels: Vec<Panel>) -> impl IntoView {
    let layout = expect_context::<LayoutState>();
    let actions = expect_context::<LayoutActions>();
    view! { <nav class="panel-rail" aria-label="Workspace panels">
        {panels.into_iter().map(|panel| view! {
            <div class="panel-tab-group">
            <button class="btn panel-tab" class:is-open=move || layout.visible_panels.get().visible(panel)
                aria-controls=format!("panel-{}", panel.id()) aria-expanded=move || layout.visible_panels.get().visible(panel).to_string()
                disabled=move || !layout.available(panel)
                title=move || if layout.available(panel) { format!("Show or collapse {}", panel.label()) } else { format!("Open a project to use {}", panel.label()) } on:click=move |_| actions.toggle.run(panel)>{panel.label()}</button>
            <button class="btn panel-pin" disabled=move || !layout.available(panel)
                aria-label=move || format!("Pin {} to the {} side", panel.label(), if layout.preferences.with(|prefs| prefs.side(panel.id())) == PanelSide::Left { "right" } else { "left" })
                title="Pin to the other side"
                on:click=move |_| actions.pin.run((panel, if layout.preferences.with_untracked(|prefs| prefs.side(panel.id())) == PanelSide::Left { PanelSide::Right } else { PanelSide::Left }))>
                {move || if layout.preferences.with(|prefs| prefs.side(panel.id())) == PanelSide::Left { "→" } else { "←" }}
            </button>
            </div>
        }).collect_view()}
    </nav> }
}
