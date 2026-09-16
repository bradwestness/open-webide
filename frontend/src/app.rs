use leptos::prelude::*;
use openwebide_core::Connection;

use crate::api::{BackendApi, HealthState};
use crate::components::{ChatPane, Sidebar, StatusBar, TopBar};

#[component]
pub fn App() -> impl IntoView {
    let api = BackendApi::from_location();

    let (health, set_health) = signal(Option::<HealthState>::None);
    let (connections, set_connections) = signal(Vec::<Connection>::new());

    // One-shot initial load: check the backend, then fetch connections.
    Effect::new(move || {
        let api = api.clone();
        leptos::task::spawn_local(async move {
            let (state, backend_ok) = match api.health().await {
                Ok(h) => (HealthState::Online { version: h.version }, true),
                Err(_) => (HealthState::Offline, false),
            };
            set_health.set(Some(state));
            if backend_ok && let Ok(conns) = api.list_connections().await {
                set_connections.set(conns);
            }
        });
    });

    view! {
        <div class="app">
            <TopBar health=health />
            <div class="app-body">
                <Sidebar connections=connections />
                <ChatPane />
            </div>
            <StatusBar health=health />
        </div>
    }
}
