use crate::state::auth::AuthState;
use leptos::prelude::*;

use crate::api::HealthState;

#[component]
pub fn TopBar(
    health: ReadSignal<Option<HealthState>>,
    on_open_settings: Callback<()>,
    on_logout: Callback<()>,
) -> impl IntoView {
    let auth = expect_context::<AuthState>();
    let settings = expect_context::<crate::state::settings::SettingsState>();
    let username = Signal::derive(move || auth.username.get());
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
            <button class="icon-btn" title="Model setup" on:click=move |_| settings.begin_model_setup(settings.default_connection.get_untracked())>"☷"</button>
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
