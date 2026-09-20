use leptos::prelude::*;

use crate::api::HealthState;

#[component]
pub fn TopBar(
    health: ReadSignal<Option<HealthState>>,
    username: ReadSignal<Option<String>>,
    on_open_settings: Callback<()>,
    on_logout: Callback<()>,
) -> impl IntoView {
    view! {
        <header class="topbar">
            <span class="logo">"open-webide"</span>
            <span class="spacer" />
            <Show when=move || username.get().is_some() fallback=|| ()>
                <span class="topbar-user">{move || username.get().unwrap_or_default()}</span>
            </Show>
            <button
                class="icon-btn"
                title="Log out"
                on:click=move |_| on_logout.run(())
            >
                "⎋"
            </button>
            <button
                class="icon-btn"
                title="Settings"
                on:click=move |_| on_open_settings.run(())
            >
                "⚙"
            </button>
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
