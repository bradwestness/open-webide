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
    view! { <div class="git-pane" class:compact-tree=move || !layout.phone.get()>
            <super::ui::PanelToolbar class="file-tree-header"><super::BranchPicker on_load=on_load_branches on_select=on_select_branch on_new=on_new_branch /></super::ui::PanelToolbar>
            <div class="git-files" role="tree" aria-label="Changed files">
                <For each=move || {
                    let mut files = git.status.get().map(|status| status.files.into_iter().collect::<Vec<_>>()).unwrap_or_default();
                    files.sort_by(|a, b| a.0.cmp(&b.0)); files
                } key=move |(path, status)| (workspace.active_project.get_untracked(), path.clone(), status.css_class()) children=move |(path, status)| {
                    let _ = status;
                    view! { <super::file_tree::FileTreeEntry entry=openwebide_core::FileEntry {name:path.clone(),path,is_dir:false,size:0} depth=0 on_toggle=Callback::new(|_: String| ()) on_open=on_open changes_only=true /> }
                } />
            </div>
            <div class="git-diff-actions" data-context-menu="">
                <span class="git-diff-path">{move || workspace.open_file.get().unwrap_or_else(|| "Select a changed file".into())}</span>
    <super::dropdown::ActionMenu aria_label="Diff actions">            <button role="menuitem" class="ui-dropdown-item recent-item btn" disabled=move || workspace.open_file.get().is_none() || git.status.get().is_none() on:click=move |_| on_load_git_diff.run(())>"Refresh diff"</button>
                <button role="menuitem" class="ui-dropdown-item recent-item btn" disabled=move || !git.can_revert(workspace.active_project.get(), workspace.open_file.get().as_deref()) on:click=move |_| on_discard_git_diff.run(())>"Revert file"</button></super::dropdown::ActionMenu>
            </div>
            {move || git.head_diff(workspace.active_project.get(), workspace.open_file.get(), workspace.content.get().into()).map(super::editor::render_inline_diff)}
        </div> }
}
