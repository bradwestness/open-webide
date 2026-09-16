use leptos::prelude::*;

use crate::api::HealthState;

#[component]
pub fn StatusBar(health: ReadSignal<Option<HealthState>>) -> impl IntoView {
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
            <span class="spacer" />
            <span class="hint">"scaffold — chat and model list are stubs"</span>
        </footer>
    }
}
