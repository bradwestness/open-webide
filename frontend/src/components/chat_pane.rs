use leptos::prelude::*;
use openwebide_core::{ChatMessage, FileDiff, ModelInfo, Role, diff_inline_lines};
use web_sys::wasm_bindgen::JsCast;

/// One item in the conversation: a chat message or an agent tool step.
#[derive(Debug, Clone)]
pub enum ConversationItem {
    /// A user or assistant chat message.
    Message(ChatMessage),
    /// An agent tool step: the call (always known) and, once it finishes, its
    /// result (which may carry a file diff for edits).
    ToolStep {
        id: String,
        name: String,
        summary: String,
        result: Option<ToolStepResult>,
    },
}

/// The outcome of a finished tool step.
#[derive(Debug, Clone)]
pub struct ToolStepResult {
    pub ok: bool,
    pub summary: String,
    pub diff: Option<FileDiff>,
}

/// A key for a conversation item that changes when the item's content changes
/// (a streamed delta, a tool result arriving) so Leptos' `For` re-renders it.
fn item_key(item: &ConversationItem) -> String {
    match item {
        ConversationItem::Message(m) => format!("m-{}-{}", m.id, m.content.len()),
        ConversationItem::ToolStep { id, result, .. } => {
            format!("t-{}-{}", id, if result.is_some() { 1 } else { 0 })
        }
    }
}

/// Render markdown to HTML for display in an assistant message or the editor
/// preview.
pub(crate) fn render_markdown(md: &str) -> String {
    let mut html = String::new();
    pulldown_cmark::html::push_html(&mut html, pulldown_cmark::Parser::new(md));
    html
}

/// Render a single chat message (assistant = markdown, user = plain text).
fn render_message(m: ChatMessage) -> impl IntoView {
    let is_assistant = m.role == Role::Assistant;
    let role = m.role.as_str().to_string();
    // Hold the (fixed, per-key) content in a signal so the view closures below
    // stay re-callable (`Fn`).
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

/// Render the diff for a file edit: the changed path and the removed/added
/// middle lines.
fn render_diff_view(diff: FileDiff) -> impl IntoView {
    let lines = diff_inline_lines(&diff);
    let path = diff.path.clone();
    view! {
        <div class="tool-diff">
            <div class="tool-diff-path">{path}</div>
            {lines.into_iter().map(|(mark, line)| {
                let text = format!("{mark} {line}");
                let line_class = if mark == '+' {
                    "diff-line add".to_string()
                } else {
                    "diff-line del".to_string()
                };
                view! {
                    <div class=move || line_class.clone()>
                        {text}
                    </div>
                }
            }).collect::<Vec<_>>()}
        </div>
    }
}

/// Render an agent tool step as a card: which file, what action, and the
/// result (a diff for edits, or a one-line summary otherwise). The three
/// possible bodies (pending / diff / summary) are mutually exclusive, so each
/// is a flat `Show` — `view!` blocks have content-dependent types and can't be
/// branched with Rust `if`/`match`.
fn render_tool_step(
    name: String,
    summary: String,
    result: Option<ToolStepResult>,
) -> impl IntoView {
    let status_class = match result.as_ref() {
        Some(r) if r.ok => "ok",
        Some(_) => "err",
        None => "pending",
    };
    let header_class = format!("tool-step-header {status_class}");
    let (result_sig, _set_result) = signal(result);
    view! {
        <div class="tool-step">
            <div class=move || header_class.clone()>
                <span class="tool-step-name">{name}</span>
                <span class="tool-step-summary">{summary}</span>
            </div>
            <Show when=move || result_sig.get().is_none() fallback=|| ()>
                <div class="tool-pending">"running…"</div>
            </Show>
            <Show
                when=move || result_sig.get().is_some_and(|r| r.diff.is_some())
                fallback=|| ()
            >
                {move || {
                    let r = result_sig.get().unwrap();
                    render_diff_view(r.diff.clone().unwrap())
                }}
            </Show>
            <Show
                when=move || result_sig.get().is_some_and(|r| r.diff.is_none())
                fallback=|| ()
            >
                {move || {
                    let r = result_sig.get().unwrap();
                    view! {
                        <div class="tool-result">{r.summary.clone()}</div>
                    }
                }}
            </Show>
        </div>
    }
}

#[component]
pub fn ChatPane(
    messages: ReadSignal<Vec<ConversationItem>>,
    streaming: ReadSignal<bool>,
    draft: ReadSignal<String>,
    set_draft: WriteSignal<String>,
    error: ReadSignal<Option<String>>,
    has_session: ReadSignal<bool>,
    local_mode: ReadSignal<bool>,
    models: ReadSignal<Vec<ModelInfo>>,
    selected_model: ReadSignal<Option<String>>,
    on_select_model: Callback<Option<String>>,
    on_send: Callback<()>,
    on_stop: Callback<()>,
) -> impl IntoView {
    let scroll_ref = NodeRef::<leptos::html::Div>::new();
    let input_ref = NodeRef::<leptos::html::Textarea>::new();
    let model_ref = NodeRef::<leptos::html::Select>::new();

    // Keep the model picker's value in sync with the chosen model. Leptos 0.8
    // has no reactive `value` attribute for `<select>`, so set the DOM value
    // directly whenever the selection or the option list changes.
    Effect::new(move || {
        let value = selected_model.get().unwrap_or_default();
        let _ = models.get();
        if let Some(el) = model_ref.get() {
            el.set_value(&value);
        }
    });

    // Keep the newest message in view as tokens arrive.
    Effect::new(move || {
        let _ = messages.get();
        if let Some(el) = scroll_ref.get() {
            el.set_scroll_top(el.scroll_height() as f64);
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
                        key=|item| item_key(item)
                        children=move |item| {
                            // `view!` blocks have content-dependent types, so the
                            // message/tool-step dispatch can't be a Rust `match`;
                            // use `Show` to branch between the two renderers.
                            let (item_sig, _set_item) = signal(item);
                            let is_message = matches!(item_sig.get(), ConversationItem::Message(_));
                            let (is_msg, _set_is_msg) = signal(is_message);
                            view! {
                                <Show
                                    when=move || is_msg.get()
                                    fallback=move || {
                                        let (name, summary, result) = match item_sig.get() {
                                            ConversationItem::ToolStep {
                                                id: _,
                                                name,
                                                summary,
                                                result,
                                            } => (name, summary, result),
                                            ConversationItem::Message(_) => {
                                                unreachable!("not a tool step")
                                            }
                                        };
                                        render_tool_step(name, summary, result)
                                    }
                                >
                                    {move || {
                                        let m = match item_sig.get() {
                                            ConversationItem::Message(m) => m,
                                            ConversationItem::ToolStep { .. } => {
                                                unreachable!("not a message")
                                            }
                                        };
                                        render_message(m)
                                    }}
                                </Show>
                            }
                        }
                    />
                </div>
            </Show>
            <Show when=move || error.get().is_some() fallback=|| ()>
                <div class="chat-error">{move || error.get().unwrap_or_default()}</div>
            </Show>
            <Show when=move || local_mode.get() fallback=|| ()>
                <div class="chat-hint">
                    "Local-mode projects use plain chat; agentic file tools need a remote (Spin-hosted) project."
                </div>
            </Show>
            <div class="composer">
                <Show when=move || has_session.get() && !models.get().is_empty() fallback=|| ()>
                    <select
                        class="model-select"
                        node_ref=model_ref
                        on:change=move |e: web_sys::Event| {
                            if let Some(target) = e.target()
                                && let Some(sel) = target.dyn_ref::<web_sys::HtmlSelectElement>()
                            {
                                let value = sel.value();
                                on_select_model.run(if value.is_empty() {
                                    None
                                } else {
                                    Some(value)
                                });
                            }
                        }
                    >
                        <option value="">Default model</option>
                        {models
                            .get()
                            .into_iter()
                            .map(|m| {
                                let name = m.name.clone();
                                view! { <option value=name>{name.clone()}</option> }
                            })
                            .collect::<Vec<_>>()}
                    </select>
                </Show>
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
