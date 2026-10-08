//! Background notification facade. Browser code supplies PushManager and worker messaging.
use crate::{
    backend::Api,
    notifications::RunNotifications,
    state::{auth::AuthState, chat::ChatState, layout::LayoutState, settings::SettingsState},
};
use leptos::prelude::*;

#[allow(
    clippy::too_many_arguments,
    reason = "the facade binds the existing app stores once"
)]
pub fn install(
    api: Api,
    auth: AuthState,
    settings: SettingsState,
    chat: ChatState,
    layout: LayoutState,
    notifications: RunNotifications,
    open: Callback<i64>,
) {
    #[cfg(not(target_arch = "wasm32"))]
    let _ = (api, auth, settings, chat, layout, notifications, open);
    #[cfg(target_arch = "wasm32")]
    browser::install(api, auth, settings, chat, layout, notifications, open);
}

#[cfg(target_arch = "wasm32")]
mod browser {
    use super::*;
    use wasm_bindgen::{JsCast, prelude::*};
    #[wasm_bindgen(
        inline_js = "export function pushContext(value) { window.webidePush?.context(value); } export async function pushSubscribe(key, revision) { if (!window.webidePush) throw new Error('Background notifications unavailable'); return window.webidePush.subscribe(key, revision); } export function pushOpen(callback) { return window.webidePush?.listen(callback); }"
    )]
    extern "C" {
        #[wasm_bindgen(js_name = pushContext)]
        fn context(value: JsValue);
        #[wasm_bindgen(catch, js_name = pushSubscribe)]
        async fn subscribe(key: &str, revision: u32) -> Result<JsValue, JsValue>;
        #[wasm_bindgen(js_name = pushOpen)]
        fn listen(callback: &js_sys::Function) -> JsValue;
    }
    #[derive(serde::Serialize)]
    struct Context {
        user_id: Option<i64>,
        session_id: Option<i64>,
        chat_visible: bool,
        enabled: bool,
        revision: u32,
    }
    #[derive(Clone, Copy, serde::Deserialize)]
    struct Target {
        user_id: i64,
        session_id: i64,
    }
    pub fn install(
        api: Api,
        auth: AuthState,
        settings: SettingsState,
        chat: ChatState,
        layout: LayoutState,
        notifications: RunNotifications,
        open: Callback<i64>,
    ) {
        let revision = RwSignal::new(0u32);
        let pending = RwSignal::new(None::<Target>);
        let callback = Closure::<dyn FnMut(JsValue)>::new(move |value| {
            if let Ok(target) = js_sys::JSON::stringify(&value)
                .ok()
                .and_then(|value| value.as_string())
                .and_then(|value| serde_json::from_str::<Target>(&value).ok())
                .ok_or(())
            {
                pending.set(Some(target));
            }
        });
        let cleanup = listen(callback.as_ref().unchecked_ref());
        let listener = StoredValue::new_local((cleanup, callback));
        on_cleanup(move || {
            listener.with_value(|(cleanup, _callback)| {
                if let Some(cleanup) = cleanup.dyn_ref::<js_sys::Function>() {
                    let _ = cleanup.call0(&JsValue::NULL);
                }
            });
        });
        Effect::new(move |_| {
            let user = auth.user.get().map(|user| user.id.get());
            let loaded = chat.sessions.get();
            {
                if let Some(target) = pending.get()
                    && user == Some(target.user_id)
                    && loaded.iter().any(|session| session.id == target.session_id)
                {
                    open.run(target.session_id);
                    pending.set(None);
                } else if pending.get().is_some()
                    && auth.checked.get()
                    && user != pending.get().map(|target| target.user_id)
                {
                    pending.set(None);
                }
            }
        });
        Effect::new(move |_| {
            let user_id = auth.user.get().map(|user| user.id.get());
            let enabled = settings.browser_notifications.get();
            context(
                js_sys::JSON::parse(
                    &serde_json::to_string(&Context {
                        user_id,
                        session_id: chat.active_session.get(),
                        chat_visible: layout.visible_panels.get().chat,
                        enabled,
                        revision: revision.get(),
                    })
                    .expect("notification context serializes"),
                )
                .expect("notification context parses"),
            );
        });
        Effect::new(move |_| {
            let generation = auth.generation.get();
            let enabled = settings.browser_notifications.get();
            let user_id = auth.user.get().map(|user| user.id.get());
            let granted = notifications.permission.get()
                == crate::notifications::NotificationPermission::Granted;
            revision.update(|revision| *revision = revision.wrapping_add(1));
            let current = revision.get_untracked();
            notifications.push_ready.set(false);
            context(
                js_sys::JSON::parse(
                    &serde_json::to_string(&Context {
                        user_id,
                        session_id: chat.active_session.get_untracked(),
                        chat_visible: layout.visible_panels.get_untracked().chat,
                        enabled,
                        revision: current,
                    })
                    .expect("notification context serializes"),
                )
                .expect("notification context parses"),
            );
            if user_id.is_none() || !enabled || !granted {
                return;
            }
            let backend = api.get_value();
            leptos::task::spawn_local(async move {
                let result = async {
                    let key = backend.push_config().await?;
                    let value = subscribe(&key.public_key, current)
                        .await
                        .map_err(|error| format!("{error:?}"))?;
                    let subscription = serde_json::from_str(
                        &js_sys::JSON::stringify(&value)
                            .map_err(|_| "Invalid subscription")?
                            .as_string()
                            .ok_or("Invalid subscription")?,
                    )
                    .map_err(|error| error.to_string())?;
                    if revision.try_get_untracked() != Some(current)
                        || auth.generation.try_get_untracked() != Some(generation)
                    {
                        return Err("Account changed".into());
                    }
                    backend.save_push_subscription(&subscription).await
                }
                .await;
                if revision.try_get_untracked() == Some(current)
                    && auth.generation.try_get_untracked() == Some(generation)
                {
                    notifications.push_ready.set(result.is_ok());
                }
            });
        });
    }
}
