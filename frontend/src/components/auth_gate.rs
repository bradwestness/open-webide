use leptos::prelude::*;
use leptos::task::spawn_local;
use openwebide_core::User;
use web_sys::wasm_bindgen::JsCast;

use crate::api::BackendApi;

/// The sign-in / account-creation gate shown before the app is usable.
/// Registration is only open while no accounts exist; once the first account
/// is created the backend rejects further registrations with a 403.
#[component]
pub fn AuthGate(api: BackendApi, on_authed: Callback<User>) -> impl IntoView {
    // false = sign in, true = create account.
    let register = RwSignal::new(false);
    let username = RwSignal::new(String::new());
    let password = RwSignal::new(String::new());
    let error = RwSignal::new(Option::<String>::None);
    let busy = RwSignal::new(false);

    let submit = {
        let api = api.clone();
        Callback::new(move |_| {
            let user = username.with(|u| u.trim().to_string());
            let pass = password.get();
            if user.is_empty() || pass.is_empty() {
                error.set(Some("Enter a username and password.".to_string()));
                return;
            }
            let is_register = register.get();
            error.set(None);
            busy.set(true);
            let api = api.clone();
            spawn_local(async move {
                let result = if is_register {
                    api.register(&user, &pass).await
                } else {
                    api.login(&user, &pass).await
                };
                busy.set(false);
                match result {
                    Ok(u) => on_authed.run(u),
                    Err(e) => error.set(Some(e)),
                }
            });
        })
    };

    view! {
        <div class="auth-gate">
            <div class="auth-card">
                <h1 class="auth-logo">"open-webide"</h1>
                <p class="auth-subtitle">
                    {move || {
                        if register.get() {
                            "Create your local account"
                        } else {
                            "Sign in to continue"
                        }
                    }}
                </p>
                <div class="auth-field">
                    <label class="auth-label" for="auth-username">"Username"</label>
                    <input
                        id="auth-username"
                        type="text"
                        class="form-input"
                        autocomplete="username"
                        value=move || username.get()
                        on:input=move |e: web_sys::Event| {
                            if let Some(target) = e.target()
                                && let Some(input) = target.dyn_ref::<web_sys::HtmlInputElement>()
                            {
                                username.set(input.value());
                            }
                        }
                        on:keydown=move |e: leptos::ev::KeyboardEvent| {
                            if e.key() == "Enter" {
                                submit.run(());
                            }
                        }
                    />
                </div>
                <div class="auth-field">
                    <label class="auth-label" for="auth-password">"Password"</label>
                    <input
                        id="auth-password"
                        r#type="password"
                        class="form-input"
                        autocomplete=move || {
                            if register.get() {
                                "new-password".to_string()
                            } else {
                                "current-password".to_string()
                            }
                        }
                        value=move || password.get()
                        on:input=move |e: web_sys::Event| {
                            if let Some(target) = e.target()
                                && let Some(input) = target.dyn_ref::<web_sys::HtmlInputElement>()
                            {
                                password.set(input.value());
                            }
                        }
                        on:keydown=move |e: leptos::ev::KeyboardEvent| {
                            if e.key() == "Enter" {
                                submit.run(());
                            }
                        }
                    />
                </div>
                <Show when=move || error.get().is_some() fallback=|| ()>
                    <p class="auth-error">{move || error.get().unwrap_or_default()}</p>
                </Show>
                <button
                    class="btn"
                    disabled=move || busy.get()
                    on:click=move |_| submit.run(())
                >
                    {move || {
                        if busy.get() {
                            "Please wait…"
                        } else if register.get() {
                            "Create account"
                        } else {
                            "Sign in"
                        }
                    }}
                </button>
                <button
                    class="auth-toggle"
                    on:click=move |_| register.update(|r| *r = !*r)
                >
                    {move || {
                        if register.get() {
                            "Already have an account? Sign in"
                        } else {
                            "No account? Create one"
                        }
                    }}
                </button>
            </div>
        </div>
    }
}
