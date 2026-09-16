use leptos::prelude::*;

#[component]
pub fn ChatPane() -> impl IntoView {
    view! {
        <main class="chat-pane">
            <div class="empty-state">
                <h1>"Open WebIDE"</h1>
                <p>"A WebAssembly IDE for local-LLM coding agents."</p>
                <p class="muted">"Sessions and chat arrive in the next milestone."</p>
            </div>
        </main>
    }
}
