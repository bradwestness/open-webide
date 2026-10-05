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
    let ui = expect_context::<crate::state::ui::UiState>();
    let username = Signal::derive(move || auth.username.get());
    view! {
        <header class="topbar">
            <span class="logo">"Open WebIDE"</span>
            <span class="spacer" />
            <button class="btn ghost" title="Command palette (Ctrl/⌘+Shift+P)" on:click=move |_| ui.palette_open.set(true)>"Commands"</button>
            <Show when=move || username.get().is_some() fallback=|| ()>
                <span class="topbar-user">{move || username.get().unwrap_or_default()}</span>
            </Show>
            <button
                class="btn ghost topbar-action"
                title="Log out" aria-label="Log out"
                on:click=move |_| on_logout.run(())
            >
                <crate::components::ui::Icon name=crate::components::ui::IconName::LogOut /><span>"Log out"</span>
            </button>
            <button class="btn ghost topbar-action" title="Model setup" aria-label="Model setup" on:click=move |_| settings.begin_model_setup(settings.default_connection.get_untracked())><crate::components::ui::Icon name=crate::components::ui::IconName::SlidersHorizontal /><span>"Models"</span></button>
            <button
                class="btn ghost topbar-action"
                title="Settings" aria-label="Settings"
                on:click=move |_| on_open_settings.run(())
            >
                <crate::components::ui::Icon name=crate::components::ui::IconName::Settings /><span>"Settings"</span>
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
