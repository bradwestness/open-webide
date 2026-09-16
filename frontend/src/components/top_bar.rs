use leptos::prelude::*;

use crate::api::HealthState;

#[component]
pub fn TopBar(health: ReadSignal<Option<HealthState>>) -> impl IntoView {
    view! {
        <header class="topbar">
            <span class="logo">"open-webide"</span>
            <span class="spacer" />
            <Show
                when=move || matches!(health.get(), Some(HealthState::Online { .. }))
                fallback=|| {
                    view! {
                        <span class="health-dot offline" title="backend offline" />
                    }
                }
            >
                <span class="health-dot online" title="backend online" />
            </Show>
        </header>
    }
}
