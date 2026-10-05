use crate::state::{git::GitState, workspace::WorkspaceState};
use leptos::prelude::*;

/// Git status and diffs use the same shared actions and diff renderer as the editor.
#[component]
pub fn GitPane(
    on_open: Callback<String>,
    #[prop(default = Callback::new(|()| ()))] on_load_branches: Callback<()>,
    #[prop(default = Callback::new(|_: String| ()))] on_select_branch: Callback<String>,
    #[prop(default = Callback::new(|()| ()))] on_new_branch: Callback<()>,
    on_load_git_diff: Callback<()>,
    on_discard_git_diff: Callback<()>,
) -> impl IntoView {
    let git = expect_context::<GitState>();
    let workspace = expect_context::<WorkspaceState>();
    let layout = expect_context::<crate::state::layout::LayoutState>();
    Effect::new(move |_| {
        workspace.open_file.track();
        if layout.visible_panels.get().files
            && layout
                .preferences
                .with(|prefs| prefs.files_view == crate::state::responsive::FilesView::Changes)
            && git.status.get().is_some()
            && workspace.open_file.get_untracked().is_some()
        {
            on_load_git_diff.run(());
        }
    });
    view! { <div class="git-pane">
        <super::ui::PanelToolbar class="file-tree-header"><super::BranchPicker on_load=on_load_branches on_select=on_select_branch on_new=on_new_branch /></super::ui::PanelToolbar>
        <div class="git-files">
            <For each=move || {
                let mut files = git.status.get().map(|status| status.files.into_iter().collect::<Vec<_>>()).unwrap_or_default();
                files.sort_by(|a, b| a.0.cmp(&b.0)); files
            } key=|(path, _)| path.clone() children=move |(path, status)| {
                let click = path.clone();
                let selected = path.clone();
                view! { <button class="btn tree-item" class:selected=move || workspace.open_file.get().as_deref() == Some(selected.as_str()) on:click=move |_| { on_open.run(click.clone()); }><span class="tree-icon"><super::ui::Icon name=super::ui::IconName::File /></span><span class="tree-name">{path}</span><span class=format!("git-badge {}", status.css_class())>{status.badge()}</span></button> }
            } />
        </div>
        <div class="git-diff-actions">
            <span class="git-diff-path">{move || workspace.open_file.get().unwrap_or_else(|| "Select a changed file".into())}</span>
            <button class="btn" disabled=move || workspace.open_file.get().is_none() || git.status.get().is_none() on:click=move |_| on_load_git_diff.run(())>"Refresh diff"</button>
            <button class="btn" disabled=move || !git.can_revert(workspace.active_project.get(), workspace.open_file.get().as_deref()) on:click=move |_| on_discard_git_diff.run(())>"Revert file"</button>
        </div>
        {move || git.head_diff(workspace.active_project.get(), workspace.open_file.get(), workspace.content.get()).map(super::editor::render_inline_diff)}
    </div> }
}
