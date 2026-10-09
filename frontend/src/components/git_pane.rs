use crate::state::{git::GitState, workspace::WorkspaceState};
use leptos::prelude::*;

/// Git status and diffs use the same shared actions and diff renderer as the editor.
#[component]
pub fn GitPane(
    on_open: Callback<String>,
    #[prop(default = Callback::new(|()| ()))] on_show_history: Callback<()>,
    #[prop(default = Callback::new(|()| ()))] on_load_branches: Callback<()>,
    #[prop(default = Callback::new(|_: String| ()))] on_select_branch: Callback<String>,
    #[prop(default = Callback::new(|()| ()))] on_new_branch: Callback<()>,
    #[prop(default = Callback::new(|_: String| ()))] on_sync: Callback<String>,
    #[prop(default = Callback::new(|_: openwebide_core::GitCommitRequest| ()))] on_commit: Callback<
        openwebide_core::GitCommitRequest,
    >,
    on_load_git_diff: Callback<()>,
    on_discard_git_diff: Callback<()>,
) -> impl IntoView {
    let git = expect_context::<GitState>();
    let workspace = expect_context::<WorkspaceState>();
    let layout = expect_context::<crate::state::layout::LayoutState>();
    let assistance = crate::state_actions::git_assistance::GitAssistance::new(
        expect_context(),
        expect_context(),
        expect_context(),
        expect_context(),
        expect_context(),
        expect_context(),
    );
    assistance.auto_commit(Signal::derive(move || {
        layout.visible_panels.get().files
            && layout
                .preferences
                .with(|prefs| prefs.files_view == crate::state::responsive::FilesView::Changes)
    }));
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
            <super::ui::PanelToolbar class="file-tree-header">
                <super::BranchPicker on_load=on_load_branches on_select=on_select_branch on_new=on_new_branch />
                <super::ui::IconButton label="Show History" on_click=Callback::new(move |_| on_show_history.run(()))><super::ui::Icon name=super::ui::IconName::FileClock /></super::ui::IconButton>
                <super::dropdown::ActionMenu aria_label="Git actions">
                    { ["fetch", "pull", "push"].into_iter().map(|action| view! {
                        <button role="menuitem" class="ui-dropdown-item btn" disabled=move || git.sync_busy.get().is_some() || git.branch_busy.get() || git.commit_busy.get() on:click=move |_| on_sync.run(action.into())><super::ui::Icon name=match action {"fetch"=>super::ui::IconName::RefreshCw,"pull"=>super::ui::IconName::ArrowDown,_=>super::ui::IconName::ArrowUp} /><span>{match action {"fetch"=>"Fetch", "pull"=>"Pull", _=>"Push"}}</span></button>
                    }).collect_view() }
                </super::dropdown::ActionMenu>
                <Show when=move || git.sync_busy.get().is_some()><super::ui::LoadingStatus label="Running Git operation…" compact=true /></Show>
            </super::ui::PanelToolbar>
            {move || git.sync_error.get().map(|error| view!{<p class="git-history-error" role="alert">{error}</p>})}
            {move || git.sync_notice.get().map(|notice| view!{<details class="git-sync-result"><summary>"Git operation complete"</summary><pre>{notice}</pre></details>})}
            <div class="git-working-tree">
            <form class="git-commit-form" on:submit=move |event| {event.prevent_default(); on_commit.run(openwebide_core::GitCommitRequest {message:git.commit_message.get_untracked(),paths:None,include_untracked:false,staged_only:true});}>
                <div class="git-commit-heading"><span class="form-label">"Commit message"</span><super::ui::IconButton label="Draft staged commit message" disabled=assistance.busy.read_only() on_click=Callback::new(move |_|assistance.generate_staged.run(()))><super::ui::Icon name=super::ui::IconName::Sparkles /></super::ui::IconButton><button class="icon-btn ui-icon" aria-label="Commit staged changes" title="Commit staged changes" type="submit" disabled=move ||git.commit_busy.get() ||git.branch_busy.get() ||git.sync_busy.get().is_some() ||git.commit_message.with(|message|message.trim().is_empty()) ||git.path_changes.with(|changes|changes.as_ref().is_none_or(|changes|changes.staged.is_empty()))><super::ui::Icon name=super::ui::IconName::GitCommitHorizontal /></button></div>
                <super::ui::TextArea label="Commit message" value=git.commit_message.read_only() on_change=Callback::new(move |message|git.commit_message.set(message)) rows=3 maxlength=65536 disabled=Signal::derive(move ||git.commit_busy.get()) />
                <Show when=move ||git.commit_busy.get()><super::ui::LoadingStatus label="Committing…" /></Show>
                {move ||git.commit_error.get().map(|error|view!{<p class="git-history-error" role="alert">{error}</p>})}
                <p class="form-hint">"Commits only the staged changes. Save editor changes before staging."</p>
            </form>
            <super::git_assistance::GitDrafts actions=assistance />
            <super::git_changes::GitChanges on_open=on_open />
            <div class="git-diff-actions" data-context-menu="">
                <span class="git-diff-path">{move || workspace.open_file.get().unwrap_or_else(|| "Select a changed file".into())}</span>
    <super::dropdown::ActionMenu aria_label="Diff actions">            <button role="menuitem" class="ui-dropdown-item recent-item btn" disabled=move || workspace.open_file.get().is_none() || git.status.get().is_none() on:click=move |_| on_load_git_diff.run(())><super::ui::Icon name=super::ui::IconName::RefreshCw /><span>"Refresh diff"</span></button>
                <button role="menuitem" class="ui-dropdown-item recent-item btn" disabled=move || !git.can_revert(workspace.active_project.get(), workspace.open_file.get().as_deref()) on:click=move |_| on_discard_git_diff.run(())><super::ui::Icon name=super::ui::IconName::Undo2 /><span>"Revert file"</span></button></super::dropdown::ActionMenu>
            </div>
            {move || git.head_diff(workspace.active_project.get(), workspace.open_file.get(), workspace.content.get().into()).map(super::editor::render_inline_diff)}
            </div>
        </div> }
}
