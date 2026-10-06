use leptos::prelude::*;
use wasm_bindgen::prelude::*;

#[wasm_bindgen(inline_js = r#"
export function install_state() {
    const fallback = {secure: window.isSecureContext === true, installed: matchMedia('(display-mode: standalone)').matches || navigator.standalone === true, available: false};
    return JSON.stringify(window.webideInstall?.state() ?? fallback);
}
export async function install_app() {
    return await window.webideInstall?.prompt() ?? false;
}
"#)]
extern "C" {
    fn install_state() -> String;
    #[wasm_bindgen(catch)]
    async fn install_app() -> Result<JsValue, JsValue>;
}

#[derive(Clone, Copy, Default, serde::Deserialize)]
struct InstallState {
    secure: bool,
    installed: bool,
    available: bool,
}
fn current_state() -> InstallState {
    serde_json::from_str(&install_state()).unwrap_or_default()
}

/// Installation is a browser capability, independent of the selected workspace.
#[component]
pub fn InstallApp() -> impl IntoView {
    let state = RwSignal::new(current_state());
    let pending = RwSignal::new(false);
    let error = RwSignal::new(None::<String>);
    let listener = window_event_listener(
        leptos::ev::Custom::new("webide-install-change"),
        move |_: web_sys::Event| {
            state.set(current_state());
        },
    );
    on_cleanup(move || listener.remove());
    view! {
        <div class="install-app-setting">
                <Show when=move || state.get().installed fallback=move || view! {
                    <Show when=move || state.get().secure fallback=|| view! {
                        <p>"To install Open WebIDE, open it over HTTPS. Tailscale Serve can provide a private HTTPS address without managing certificates."</p>
                        <a class="btn" href="https://github.com/openwebide/openwebide/blob/main/docs/tailscale.md" target="_blank" rel="noopener noreferrer">"Set up HTTPS"</a>
                    }>
                        <Show when=move || state.get().available fallback=|| view! {
                            <p>"Use your browser’s Install app command. On iPhone or iPad, open Safari, choose Share, then Add to Home Screen. If your browser does not support installation, you can keep using Open WebIDE in a tab."</p>
                        }>
                            <button class="btn" disabled=move || pending.get() on:click=move |_| {
                                pending.set(true);
                                error.set(None);
                                leptos::task::spawn_local(async move {
                                    if let Err(problem) = install_app().await {
                                        error.try_set(Some(format!("Could not open installation: {problem:?}")));
                                    }
                                    pending.try_set(false);
                                    state.try_set(current_state());
                                });
                            }>"Install Open WebIDE"</button>
                        </Show>
                    </Show>
                }> <p>"Open WebIDE is installed."</p> </Show>
                <Show when=move || error.get().is_some()><p role="alert">{move || error.get().unwrap_or_default()}</p></Show>
        </div>
    }
}
