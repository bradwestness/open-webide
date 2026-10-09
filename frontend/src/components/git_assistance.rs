use leptos::prelude::*;
use openwebide_core::AssistanceKind;
#[component]
pub fn GitDrafts(actions: crate::state_actions::git_assistance::GitAssistance) -> impl IntoView {
    let chat = expect_context::<crate::state::chat::ChatState>();
    let git = expect_context::<crate::state::git::GitState>();
    let show_note = RwSignal::new(false);
    view! {
            <div class="git-draft-actions">
                <super::dropdown::ActionMenu aria_label="Draft Git descriptions">
                    <button type="button" class="ui-dropdown-item recent-item btn" role="menuitem" disabled=move || actions.busy.get() || chat.streaming.get() || git.commit_busy.get() || git.branch_busy.get() || git.sync_busy.get().is_some() || git.path_changes.with(|changes|changes.as_ref().is_none_or(|changes|changes.staged.is_empty())) on:click=move |_| actions.generate_staged.run(())><super::ui::Icon name=super::ui::IconName::GitCommitHorizontal/><span>"Draft commit message"</span></button>
                    <button type="button" class="ui-dropdown-item recent-item btn" role="menuitem" disabled=move || actions.busy.get() || chat.streaming.get() on:click=move |_| actions.generate.run(AssistanceKind::Branch)><super::ui::Icon name=super::ui::IconName::GitBranch/><span>"Draft branch name"</span></button>
                    <button type="button" class="ui-dropdown-item recent-item btn" role="menuitem" disabled=move || actions.busy.get() || chat.streaming.get() on:click=move |_| actions.generate.run(AssistanceKind::PullRequest)><super::ui::Icon name=super::ui::IconName::GitPullRequest/><span>"Draft PR description"</span></button>
                    <button type="button" class="ui-dropdown-item recent-item btn" role="menuitem" disabled=move ||actions.note.get().is_none() on:click=move |_|show_note.set(true)><super::ui::Icon name=super::ui::IconName::Info /><span>"Draft model and limits"</span></button>
                </super::dropdown::ActionMenu>
                <Show when=move ||show_note.get() && actions.note.get().is_some()><div class="ui-feedback-overlay"><super::ui::FormNotice>{move ||actions.note.get().unwrap_or_default()}</super::ui::FormNotice><super::ui::IconButton label="Dismiss draft limits" on_click=Callback::new(move |_|show_note.set(false))><super::ui::Icon name=super::ui::IconName::X /></super::ui::IconButton></div></Show>
    <super::ui::FeedbackOverlay message=actions.error.read_only() on_dismiss=Callback::new(|()|()) />
                <Show when=move || !actions.draft.get().is_empty() && actions.kind.get()!=Some(AssistanceKind::Commit)>
                    <div class="ui-feedback-overlay"><super::ui::FormField label="Git draft"><textarea class="form-input" aria-label="Git draft" rows="5" prop:value=move || actions.draft.get() on:input=move |event| actions.draft.set(event_target_value(&event))></textarea></super::ui::FormField>
                    <button type="button" class="btn ghost" on:click=move |_| {
                        if let Some(window) = web_sys::window() { let _ = window.navigator().clipboard().write_text(&actions.draft.get_untracked()); }
                    }>"Copy draft"</button></div>
                </Show>
            </div>
        }
}
