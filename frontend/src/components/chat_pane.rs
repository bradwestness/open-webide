use leptos::prelude::*;
use openwebide_core::{ChatMessage, Role};
use web_sys::wasm_bindgen::JsCast;

/// Render markdown to HTML for display in an assistant message.
fn render_markdown(md: &str) -> String {
    let mut html = String::new();
    pulldown_cmark::html::push_html(&mut html, pulldown_cmark::Parser::new(md));
    html
}

#[component]
pub fn ChatPane(
    messages: ReadSignal<Vec<ChatMessage>>,
    streaming: ReadSignal<bool>,
    draft: ReadSignal<String>,
    set_draft: WriteSignal<String>,
    error: ReadSignal<Option<String>>,
    has_session: ReadSignal<bool>,
    on_send: Callback<()>,
    on_stop: Callback<()>,
) -> impl IntoView {
    let scroll_ref = NodeRef::<leptos::html::Div>::new();
    let input_ref = NodeRef::<leptos::html::Textarea>::new();

    // Keep the newest message in view as tokens arrive.
    Effect::new(move || {
        let _ = messages.get();
        if let Some(el) = scroll_ref.get() {
            el.set_scroll_top(el.scroll_height());
        }
    });

    // Mirror the draft signal into the textarea so it clears after a send
    // without fighting the user's cursor while they type.
    Effect::new(move || {
        let value = draft.get();
        if let Some(ta) = input_ref.get()
            && ta.value() != value
        {
            ta.set_value(&value);
        }
    });

    view! {
        <main class="chat-pane">
            <Show
                when=move || !messages.get().is_empty()
                fallback=move || {
                    view! {
                        <Show
                            when=move || has_session.get()
                            fallback=move || {
                                view! {
                                    <div class="empty-state">
                                        <h1>"Open WebIDE"</h1>
                                        <p>"A WebAssembly IDE for local-LLM coding agents."</p>
                                        <p class="muted">"Create a session in the sidebar to start chatting."</p>
                                    </div>
                                }
                            }
                        >
                            <div class="messages">
                                <p class="empty">"No messages yet — say hello."</p>
                            </div>
                        </Show>
                    }
                }
            >
                <div class="messages" node_ref=scroll_ref>
                    <For
                        each=move || messages.get()
                        key=|m| (m.id, m.role as u8, m.content.len())
                        children=move |m| {
                            let is_assistant = m.role == Role::Assistant;
                            let role = m.role.as_str().to_string();
                            // Hold the (fixed, per-key) content in a signal so the
                            // view closures below stay re-callable (`Fn`).
                            let (content, _set_content) = signal(m.content.clone());
                            view! {
                                <div
                                    class=move || {
                                        if is_assistant {
                                            "message assistant".to_string()
                                        } else {
                                            "message user".to_string()
                                        }
                                    }
                                >
                                    <div class="msg-role">{role}</div>
                                    <Show
                                        when=move || is_assistant
                                        fallback=move || {
                                            view! {
                                                <div class="msg-body plain">{content.get()}</div>
                                            }
                                        }
                                    >
                                        {view! {
                                            <div
                                                class="msg-body markdown"
                                                inner_html=move || render_markdown(&content.get())
                                            />
                                        }}
                                    </Show>
                                </div>
                            }
                        }
                    />
                </div>
            </Show>
            <Show when=move || error.get().is_some() fallback=|| ()>
                <div class="chat-error">{move || error.get().unwrap_or_default()}</div>
            </Show>
            <div class="composer">
                <textarea
                    class="composer-input"
                    node_ref=input_ref
                    placeholder=move || {
                        if has_session.get() {
                            "Send a message… (Enter to send, Shift+Enter for a newline)"
                        } else {
                            "Create a session to start chatting"
                        }
                    }
                    on:input=move |e: web_sys::Event| {
                        if let Some(target) = e.target()
                            && let Some(textarea) = target.dyn_ref::<web_sys::HtmlTextAreaElement>()
                        {
                            set_draft.set(textarea.value());
                        }
                    }
                    on:keydown=move |e: leptos::ev::KeyboardEvent| {
                        if e.key() == "Enter" && !e.shift_key() {
                            e.prevent_default();
                            on_send.run(());
                        }
                    }
                />
                <Show
                    when=move || streaming.get()
                    fallback=move || {
                        view! {
                            <button
                                class="btn send"
                                disabled=move || draft.with(|d| d.trim().is_empty()) || !has_session.get()
                                on:click=move |_| on_send.run(())
                            >
                                "Send"
                            </button>
                        }
                    }
                >
                    <button class="btn stop" on:click=move |_| on_stop.run(())>"Stop"</button>
                </Show>
            </div>
        </main>
    }
}
