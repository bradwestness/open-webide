use crate::state::git::GitState;
use leptos::prelude::*;

/// The same native branch selector in Changes and the footer.
#[component]
pub fn BranchPicker(
    #[prop(default = Callback::new(|()| ()))] on_load: Callback<()>,
    #[prop(default = Callback::new(|_: String| ()))] on_select: Callback<String>,
    #[prop(default = Callback::new(|()| ()))] on_new: Callback<()>,
) -> impl IntoView {
    let git = expect_context::<GitState>();
    let current = move || {
        git.status.with(|status| {
            status
                .as_ref()
                .map(|status| status.branch.clone())
                .unwrap_or_default()
        })
    };
    view! {
        <label class="branch-picker">
            <super::ui::Icon name=super::ui::IconName::GitBranch />
            <select class="form-input git-branch-select" aria-label="Git branch" title="Switch Git branch or create a new branch"
                disabled=move || git.branch_busy.get() || git.status.get().is_none()
                prop:value=move || format!("branch:{}", current()) on:focus=move |_| on_load.run(()) on:pointerdown=move |_| on_load.run(())
                on:change=move |event| {
                    let value = event_target_value(&event);
                    // Restore immediately: branch labels change only after checkout succeeds.
                    let select: web_sys::HtmlSelectElement = event_target(&event);
                    select.set_value(&format!("branch:{}", current()));
                    if value == "new" { on_new.run(()); }
                    else if let Some(branch) = value.strip_prefix("branch:") && branch != current() { on_select.run(branch.to_string()); }
                }>
                <option value=move || format!("branch:{}", current())>{current}</option>
                <For each=move || { git.branches.get().into_iter().filter(|branch| branch.name != current()).collect::<Vec<_>>() } key=|branch| branch.name.clone() children=move |branch| { let value = format!("branch:{}", branch.name); view! { <option value=value>{branch.name}</option> } } />
                <Show when=move || git.branches_loading.get()><option disabled>"Loading branches…"</option></Show>
                <Show when=move || git.branches_error.get().is_some()><option disabled>"Could not load branches — reopen to retry"</option></Show>
                <option value="new">"New branch…"</option>
            </select>
        </label>
    }
}
