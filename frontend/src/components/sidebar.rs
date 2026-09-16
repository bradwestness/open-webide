use leptos::prelude::*;
use openwebide_core::Connection;

#[component]
pub fn Sidebar(connections: ReadSignal<Vec<Connection>>) -> impl IntoView {
    view! {
        <aside class="sidebar">
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
