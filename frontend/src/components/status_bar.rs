use leptos::prelude::*;

use crate::api::HealthState;
use crate::state::{chat::ChatState, git::GitState};

#[component]
pub fn StatusBar(
    health: ReadSignal<Option<HealthState>>,
    on_toggle_terminal: impl Fn() + Copy + 'static,
    #[prop(default = Callback::new(|_| ()))] on_branch_click: Callback<()>,
    #[prop(default = Callback::new(|()| ()))] on_load_branches: Callback<()>,
    #[prop(default = Callback::new(|_: String| ()))] on_select_branch: Callback<String>,
    #[prop(default = Callback::new(|_| ()))] on_sync_click: Callback<()>,
) -> impl IntoView {
    let editor_actions = crate::state_actions::editor::EditorActions::new(expect_context::<
        crate::state::workspace::WorkspaceState,
    >());
    let preparing = Signal::derive(move || editor_actions.syntax_preparation_pending());
    let chrome = use_context::<super::editor_chrome::EditorFooterMount>();
    let chat = expect_context::<ChatState>();
    let git = expect_context::<GitState>();
    let show_terminal = chat.show_terminal.read_only();
    let ui = expect_context::<crate::state::ui::UiState>();
    let layout = expect_context::<crate::state::layout::LayoutState>();
    let git_status: Signal<Option<openwebide_core::GitRepoStatus>> =
        Signal::derive(move || git.status.get());
    view! {
        <footer class="statusbar">
            {chrome.map(|chrome| view! { <div class="status-editor-slot" node_ref=chrome.0 /> })}
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

            <Show when=move || git_status.with(Option::is_some)>
                <span class="git-status-widget">
                    <super::BranchPicker above=true on_load=on_load_branches on_select=on_select_branch on_new=on_branch_click />
                    {move || git_status.get().map(|status| {
                        let ahead = status.ahead;
                        let behind = status.behind;
                        let insertions = status.line_stats.insertions;
                        let deletions = status.line_stats.deletions;
                        view! {
                            <Show when={move || ahead > 0 || behind > 0}>
                                <span class="git-divergence" title="Ahead/behind upstream commits">{format!("↑{ahead} ↓{behind}")}</span>
                                <button class="git-sync-btn" title="Sync with upstream (pull & push)" on:click=move |_| on_sync_click.run(())>"Sync"</button>
                            </Show>
                            <Show when={move || insertions > 0 || deletions > 0} fallback=move || status.is_clean.then(|| view! { <span class="git-clean" title="Working tree clean">"✓"</span> })>
                                <span class="git-line-stats" title="Uncommitted line changes">
                                    <span class="git-insertions">{format!("+{insertions}")}</span>
                                    <span class="git-deletions">{format!("-{deletions}")}</span>
                                </span>
                            </Show>
                        }
                    })}
                </span>
            </Show>

            <span class="spacer" />
            <span class="status-preparation" title=move || if preparing.get() { "Preparing syntax…" } else { "" }>
                <Show when=move || preparing.get()>
                    <super::ui::LoadingStatus label="Preparing syntax…" compact=true />
                </Show>
            </span>
            <button type="button" class=move || if ui.plugins_open.get() { "status-btn active" } else { "status-btn" }
                title="Browse and manage plugins" aria-haspopup="dialog" aria-expanded=move || ui.plugins_open.get().to_string()
                on:click=move |_| ui.plugins_open.update(|open| *open = !*open)>
                <super::ui::Icon name=super::ui::IconName::Puzzle />"Plugins"
            </button>
            <button
                class=move || if show_terminal.get() { "status-btn active" } else { "status-btn" }
                disabled=move || !layout.available(crate::state::layout::Panel::Terminal)
                title=move || if layout.available(crate::state::layout::Panel::Terminal) { "Toggle Output and terminal dock (Ctrl+`)" } else { "Open a project to use Terminal" }
                on:click=move |_| on_toggle_terminal()
            >
                <super::ui::Icon name=super::ui::IconName::Terminal />"Output"
            </button>
        </footer>
    }
}
