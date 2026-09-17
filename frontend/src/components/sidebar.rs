use leptos::prelude::*;
use openwebide_core::{ChatSession, Connection};

#[component]
pub fn Sidebar(
    connections: ReadSignal<Vec<Connection>>,
    sessions: ReadSignal<Vec<ChatSession>>,
    active_id: ReadSignal<Option<i64>>,
    on_select: Callback<i64>,
    on_new: Callback<()>,
    on_rename: Callback<i64>,
    on_delete: Callback<i64>,
) -> impl IntoView {
    view! {
        <aside class="sidebar">
            <div class="sidebar-section">
                <div class="section-header">
                    <h2>"Sessions"</h2>
                    <button
                        class="icon-btn"
                        title="New session"
                        on:click=move |_| on_new.run(())
                    >
                        "+"
                    </button>
                </div>
                <For
                    each=move || sessions.get()
                    key=|s| s.id
                    children=move |s| {
                        view! {
                            <div
                                class=move || {
                                    if active_id.get() == Some(s.id) {
                                        "session active".to_string()
                                    } else {
                                        "session".to_string()
                                    }
                                }
                            >
                                <span
                                    class="session-name"
                                    on:click=move |_| on_select.run(s.id)
                                >
                                    {s.name}
                                </span>
                                <span class="session-actions">
                                    <button
                                        class="icon-btn"
                                        title="Rename"
                                        on:click=move |_| on_rename.run(s.id)
                                    >
                                        "✎"
                                    </button>
                                    <button
                                        class="icon-btn"
                                        title="Delete"
                                        on:click=move |_| on_delete.run(s.id)
                                    >
                                        "✕"
                                    </button>
                                </span>
                            </div>
                        }
                    }
                />
                <Show when=move || sessions.get().is_empty() fallback=|| ()>
                    <p class="empty">"No sessions yet — create one."</p>
                </Show>
            </div>
            <div class="sidebar-section">
                <h2>"Connections"</h2>
                <For
                    each=move || connections.get()
                    key=|c| c.id
                    children=move |c| {
                        view! {
                            <div class="connection">
                                <span class="conn-name">{move || c.name.clone()}</span>
                                <span class="conn-kind">{c.kind.as_str()}</span>
                            </div>
                        }
                    }
                />
                <Show when=move || connections.get().is_empty() fallback=|| ()>
                    <p class="empty">"No connections yet — add one via the API."</p>
                </Show>
            </div>
            <div class="sidebar-section">
                <h2>"System prompts"</h2>
                <p class="empty">"Coming soon"</p>
            </div>
        </aside>
    }
}
