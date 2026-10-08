use leptos::prelude::*;
use openwebide_core::AssistanceKind;
#[component]
pub fn GitDrafts() -> impl IntoView {
    let actions = crate::state_actions::git_assistance::GitAssistance::new(
        expect_context(),
        expect_context(),
        expect_context(),
        expect_context(),
        expect_context(),
        expect_context(),
    );
    let chat = expect_context::<crate::state::chat::ChatState>();
    view! {
        <div class="git-draft-actions">
            <super::dropdown::ActionMenu aria_label="Draft Git descriptions">
                <button class="ui-dropdown-item" role="menuitem" disabled=move || actions.busy.get() || chat.streaming.get() on:click=move |_| actions.generate.run(AssistanceKind::Commit)><super::ui::Icon name=super::ui::IconName::GitCommitHorizontal/><span>"Draft commit message"</span></button>
                <button class="ui-dropdown-item" role="menuitem" disabled=move || actions.busy.get() || chat.streaming.get() on:click=move |_| actions.generate.run(AssistanceKind::Branch)><super::ui::Icon name=super::ui::IconName::GitBranch/><span>"Draft branch name"</span></button>
                <button class="ui-dropdown-item" role="menuitem" disabled=move || actions.busy.get() || chat.streaming.get() on:click=move |_| actions.generate.run(AssistanceKind::PullRequest)><super::ui::Icon name=super::ui::IconName::GitPullRequest/><span>"Draft PR description"</span></button>
            </super::dropdown::ActionMenu>
            <Show when=move || actions.busy.get()><p class="form-hint" role="status">"Drafting…"</p></Show>
            <Show when=move || actions.error.get().is_some()><p class="form-hint" role="alert">{move || actions.error.get().unwrap_or_default()}</p></Show>
            <Show when=move || !actions.draft.get().is_empty()>
                <super::ui::FormField label="Git draft"><textarea class="form-input" aria-label="Git draft" rows="5" prop:value=move || actions.draft.get() on:input=move |event| actions.draft.set(event_target_value(&event))></textarea></super::ui::FormField>
                <button class="btn ghost" on:click=move |_| {
                    if let Some(window) = web_sys::window() { let _ = window.navigator().clipboard().write_text(&actions.draft.get_untracked()); }
                }>"Copy draft"</button>
            </Show>
        </div>
    }
}
