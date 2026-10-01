use leptos::prelude::*;

use crate::api::HealthState;
use openwebide_frontend::state::{chat::ChatState, git::GitState};

#[component]
pub fn StatusBar(
    health: ReadSignal<Option<HealthState>>,
    on_toggle_terminal: impl Fn() + Copy + 'static,
    #[prop(default = Callback::new(|_| ()))] on_branch_click: Callback<()>,
    #[prop(default = Callback::new(|_| ()))] on_sync_click: Callback<()>,
) -> impl IntoView {
    let chat = expect_context::<ChatState>();
    let git = expect_context::<GitState>();
    let show_terminal = chat.show_terminal.read_only();
    let git_status: Signal<Option<openwebide_core::GitRepoStatus>> =
        Signal::derive(move || git.status.get());
    let approval_mode = Signal::derive(move || {
        chat.active_session
            .get()
            .and_then(|session_id| {
                chat.approval_mode
                    .with(|modes| modes.get(&session_id).copied())
            })
            .unwrap_or_default()
    });
    view! {
        <footer class="statusbar">
            <Show
                when=move || health.get().is_some()
                fallback=|| view! { <span class="status">"connecting…"</span> }
            >
                <Show
                    when=move || matches!(health.get(), Some(HealthState::Online { .. }))
                    fallback=|| view! {
                        <span class="status offline">"backend offline"</span>
                    }
                >
                    <span class="status online">
                        {move || match health.get() {
                            Some(HealthState::Online { version }) => {
                                format!("backend online · v{version}")
                            }
                            _ => "backend online".to_string(),
                        }}
                    </span>
                </Show>
            </Show>

            {move || {
                git_status.get().map(|status| {
                    let branch = status.branch.clone();
                    let ahead = status.ahead;
                    let behind = status.behind;
                    let insertions = status.line_stats.insertions;
                    let deletions = status.line_stats.deletions;
                    let is_clean = status.is_clean;

                    view! {
                        <span class="git-status-widget">
                            <button
                                class="git-branch-btn"
                                title="Active Git branch (click to switch)"
                                on:click=move |_| on_branch_click.run(())
                            >
                                " " {branch}
                            </button>
                            {if ahead > 0 || behind > 0 {
                                view! {
                                    <span class="git-divergence" title="Ahead/behind upstream commits">
                                        {format!("↑{ahead} ↓{behind}")}
                                    </span>
                                    <button
                                        class="git-sync-btn"
                                        title="Sync with upstream (pull & push)"
                                        on:click=move |_| on_sync_click.run(())
                                    >
                                        "Sync"
                                    </button>
                                }.into_any()
                            } else {
                                view! { <span /> }.into_any()
                            }}
                            {if insertions > 0 || deletions > 0 {
                                view! {
                                    <span class="git-line-stats" title="Uncommitted line changes">
                                        <span class="git-insertions">{format!("+{insertions}")}</span>
                                        <span class="git-deletions">{format!("-{deletions}")}</span>
                                    </span>
                                }.into_any()
                            } else if is_clean {
                                view! {
                                    <span class="git-clean" title="Working tree clean">"✓"</span>
                                }.into_any()
                            } else {
                                view! { <span /> }.into_any()
                            }}
                        </span>
                    }
                })
            }}

            <span class="spacer" />
            <Show when=move || approval_mode.get() == openwebide_agent::policy::ApprovalMode::AlwaysForSession>
                <span class="status-mode" title="Always approve tools (except run_command) for this session">
                    "[ALWAYS]"
                </span>
            </Show>
            <button
                class=move || if show_terminal.get() { "status-btn active" } else { "status-btn" }
                title="Toggle terminal dock (Ctrl+`)"
                on:click=move |_| on_toggle_terminal()
            >
                " Terminal"
            </button>
        </footer>
    }
}
