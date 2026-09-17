use leptos::prelude::*;
use openwebide_core::{ChatMessage, ChatSession, Connection, Role};
use web_sys::AbortController;

use crate::api::{BackendApi, HealthState, SseEvent};
use crate::components::{ChatPane, Sidebar, StatusBar, TopBar};

#[component]
pub fn App() -> impl IntoView {
    let api = BackendApi::from_location();

    let (health, set_health) = signal(Option::<HealthState>::None);
    let (connections, set_connections) = signal(Vec::<Connection>::new());
    let (sessions, set_sessions) = signal(Vec::<ChatSession>::new());
    let (active_id, set_active_id) = signal(Option::<i64>::None);
    let (messages, set_messages) = signal(Vec::<ChatMessage>::new());
    let (streaming, set_streaming) = signal(false);
    let (error, set_error) = signal(Option::<String>::None);
    let (draft, set_draft) = signal(String::new());
    let (has_session, set_has_session) = signal(false);
    let abort = RwSignal::new(None);

    // Derive a plain `bool` from the active-session id for the chat pane.
    Effect::new(move || {
        set_has_session.set(active_id.get().is_some());
    });

    // One-shot initial load: check the backend, then fetch connections and sessions.
    Effect::new({
        let api = api.clone();
        move || {
            let api = api.clone();
            leptos::task::spawn_local(async move {
                let (state, backend_ok) = match api.health().await {
                    Ok(h) => (HealthState::Online { version: h.version }, true),
                    Err(_) => (HealthState::Offline, false),
                };
                set_health.set(Some(state));
                if !backend_ok {
                    return;
                }
                if let Ok(conns) = api.list_connections().await {
                    set_connections.set(conns);
                }
                if let Ok(list) = api.list_sessions().await {
                    set_sessions.set(list);
                }
            });
        }
    });

    // Load the message history whenever the active session changes.
    Effect::new({
        let api = api.clone();
        move || {
            let id = active_id.get();
            let api = api.clone();
            leptos::task::spawn_local(async move {
                let msgs = match id {
                    Some(id) => api.list_messages(id).await.unwrap_or_default(),
                    None => Vec::new(),
                };
                set_messages.set(msgs);
            });
        }
    });

    let on_select = Callback::new(move |id: i64| {
        set_error.set(None);
        set_active_id.set(Some(id));
    });

    let on_new = {
        let api = api.clone();
        Callback::new(move |_| {
            let api = api.clone();
            let name = format!("Session {}", sessions.get_untracked().len() + 1);
            leptos::task::spawn_local(async move {
                match api.create_session(&name, None, None).await {
                    Ok(session) => {
                        set_sessions.update(|list| list.push(session.clone()));
                        set_error.set(None);
                        set_active_id.set(Some(session.id));
                    }
                    Err(e) => set_error.set(Some(e)),
                }
            });
        })
    };

    let on_rename = {
        let api = api.clone();
        Callback::new(move |id: i64| {
            let Some(window) = web_sys::window() else {
                return;
            };
            let current = sessions
                .with(|list| list.iter().find(|s| s.id == id).map(|s| s.name.clone()))
                .unwrap_or_default();
            let Ok(Some(name)) = window.prompt_with_message_and_default("Rename session", &current)
            else {
                return;
            };
            let name = name.trim().to_string();
            if name.is_empty() {
                return;
            }
            let api = api.clone();
            leptos::task::spawn_local(async move {
                match api.rename_session(id, &name).await {
                    Ok(updated) => set_sessions.update(|list| {
                        if let Some(s) = list.iter_mut().find(|s| s.id == id) {
                            *s = updated;
                        }
                    }),
                    Err(e) => set_error.set(Some(e)),
                }
            });
        })
    };

    let on_delete = {
        let api = api.clone();
        Callback::new(move |id: i64| {
            let Some(window) = web_sys::window() else {
                return;
            };
            let Ok(confirmed) =
                window.confirm_with_message("Delete this session and its messages?")
            else {
                return;
            };
            if !confirmed {
                return;
            }
            let api = api.clone();
            leptos::task::spawn_local(async move {
                if let Err(e) = api.delete_session(id).await {
                    set_error.set(Some(e));
                    return;
                }
                set_sessions.update(|list| list.retain(|s| s.id != id));
                if active_id.get_untracked() == Some(id) {
                    set_active_id.set(None);
                }
            });
        })
    };

    let on_send = {
        let api = api.clone();
        Callback::new(move |_| {
            let content = draft.with(|d| d.trim().to_string());
            if content.is_empty() || streaming.get_untracked() {
                return;
            }
            let Some(session_id) = active_id.get_untracked() else {
                return;
            };
            let Ok(controller) = AbortController::new() else {
                return;
            };
            set_draft.set(String::new());
            set_streaming.set(true);
            set_error.set(None);
            abort.set(Some(controller.clone()));
            let api = api.clone();
            leptos::task::spawn_local(async move {
                let result = api
                    .send_message(
                        session_id,
                        &content,
                        None,
                        Some(&controller.signal()),
                        move |event| {
                            // Only touch the view if this session is still on screen;
                            // the stream keeps going in the background and the
                            // server persists the reply either way.
                            if active_id.get_untracked() != Some(session_id) {
                                return;
                            }
                            match event {
                                SseEvent::Message(msg) => {
                                    set_messages.update(|m| m.push(msg));
                                }
                                SseEvent::Delta(delta) => {
                                    set_messages.update(|m| {
                                        let extend_last = m
                                            .last()
                                            .is_some_and(|last| last.role == Role::Assistant);
                                        if extend_last {
                                            if let Some(last) = m.last_mut() {
                                                last.content.push_str(&delta);
                                            }
                                        } else {
                                            m.push(ChatMessage {
                                                id: 0,
                                                session_id,
                                                role: Role::Assistant,
                                                content: delta,
                                                created_at: 0,
                                            });
                                        }
                                    });
                                }
                                SseEvent::Done(msg) => {
                                    set_messages.update(|m| {
                                        if let Some(last) = m.last_mut()
                                            && last.role == Role::Assistant
                                        {
                                            *last = msg;
                                        }
                                    });
                                }
                                SseEvent::Error(e) => set_error.set(Some(e)),
                            }
                        },
                    )
                    .await;
                if let Err(e) = result {
                    // An abort surfaces as a transport error; don't show it.
                    if !controller.signal().aborted() {
                        set_error.set(Some(e));
                    }
                }
                set_streaming.set(false);
                abort.set(None);
            });
        })
    };

    let on_stop = Callback::new(move |_| {
        abort.with(|a| {
            if let Some(c) = a.as_ref() {
                c.abort();
            }
        });
    });

    view! {
        <div class="app">
            <TopBar health=health />
            <div class="app-body">
                <Sidebar
                    connections=connections
                    sessions=sessions
                    active_id=active_id
                    on_select=on_select
                    on_new=on_new
                    on_rename=on_rename
                    on_delete=on_delete
                />
                <ChatPane
                    messages=messages
                    streaming=streaming
                    draft=draft
                    set_draft=set_draft
                    error=error
                    has_session=has_session
                    on_send=on_send
                    on_stop=on_stop
                />
            </div>
            <StatusBar health=health />
        </div>
    }
}
