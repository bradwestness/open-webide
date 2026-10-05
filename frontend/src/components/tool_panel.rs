use crate::{
    state::{
        layout::{ActiveResizer, LayoutState, Panel},
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
    let kind = match panel {
        Panel::Sessions => ActiveResizer::Sidebar,
        Panel::Files => ActiveResizer::Tree,
        Panel::Chat => ActiveResizer::Chat,
        Panel::Terminal => ActiveResizer::Terminal,
        _ => ActiveResizer::None,
    };
    view! { <section id=format!("panel-{}", panel.id()) class="tool-panel"
        class:tool-panel-center=panel == Panel::Editor
        style=move || {
            let order = layout.preferences.with(|prefs| prefs.order(panel.id()));
            format!("display: {}; order: {order}; --files-panel-width: {}px; --tool-window-width: {}px;", if layout.visible_panels.get().visible(panel) { "flex" } else { "none" }, layout.tree_width.get(), if panel == Panel::Terminal { layout.terminal_width.get() } else { 0.0 })
        }
        aria-label=panel.label()>
        <div class="tool-panel-heading">
            <span>{panel.label()}</span>
            <div class="tool-panel-actions">
                <button type="button" class="icon-btn ui-icon" aria-label=format!("Move {} left", panel.label()) title="Move panel left" on:click=move |_| actions.move_panel.run((panel, false))><crate::components::ui::Icon name=crate::components::ui::IconName::ArrowLeft /></button>
                <button type="button" class="icon-btn ui-icon" aria-label=format!("Move {} right", panel.label()) title="Move panel right" on:click=move |_| actions.move_panel.run((panel, true))><crate::components::ui::Icon name=crate::components::ui::IconName::ArrowRight /></button>
            </div>
            <button class="btn" title="Return to chat" on:click=move |_| actions.show.run(Panel::Chat)>"Back to chat"</button>
        </div>
        <div class="tool-panel-content" class:resizer-leading=move || layout.preferences.with(|prefs| prefs.side(panel.id())) == PanelSide::Right>
            {children()}
            {(kind != ActiveResizer::None).then(|| view! { <super::panel_resizer::PanelResizer kind=kind /> })}
        </div>
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

            </div>
        }).collect_view()}
    </nav> }
}

/// Files views stay mounted so switching tabs retains queries, selection and scroll.
#[component]
pub fn FilesPanel(children: Children) -> impl IntoView {
    use super::ui::{SegmentOption, SegmentedControl};
    use crate::state::responsive::FilesView;
    let layout = expect_context::<LayoutState>();
    let actions = expect_context::<LayoutActions>();
    view! { <div class="files-panel">
        <SegmentedControl options=vec![SegmentOption::new("Explorer", FilesView::Explorer), SegmentOption::new("Changes", FilesView::Changes)]
            value=Signal::derive(move || layout.preferences.with(|p| if p.files_view == FilesView::Search { FilesView::Explorer } else { p.files_view })) on_change=actions.select_files_view />
        {children()}
    </div> }
}
