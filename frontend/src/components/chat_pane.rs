use crate::state::{chat::ChatState, layout::LayoutState, projects::ProjectsState};
use leptos::prelude::*;
use openwebide_core::{
    FileDiff, ModelInfo, Role, diff_inline_lines,
    tui::{SessionTelemetry, SlashCommand, extract_editor_context_prelude, parse_thinking},
};
use web_sys::wasm_bindgen::JsCast;

pub use crate::conversation::{ConversationItem, ToolStepResult, item_key};

pub(crate) use crate::markdown::render as render_markdown;

/// Render the diff for a file edit: the changed path and the removed/added lines.
fn render_diff_view(diff: FileDiff) -> impl IntoView {
    let lines = diff_inline_lines(&diff);
    let path = diff.path.clone();
    view! {
        <div class="tui-diff-box">
            <div class="tui-diff-path">"diff: " {path}</div>
            <div class="tui-diff-lines">
                {lines.into_iter().map(|(mark, line)| {
                    let text = format!("{mark} {line}");
                    let line_class = if mark == '+' {
                        "tui-diff-line add"
                    } else {
                        "tui-diff-line del"
                    };
                    view! {
                        <div class=line_class>{text}</div>
                    }
                }).collect::<Vec<_>>()}
            </div>
        </div>
    }
}

/// Render a single assistant message with collapsible thinking stream.
fn render_assistant_message(content: String) -> AnyView {
    let parsed = parse_thinking(&content);
    let is_thinking_active = parsed.is_thinking;
    let thinking_text = parsed.thinking.unwrap_or_default();
    let answer_text = parsed.answer;
    let has_thinking = !thinking_text.is_empty();
    let thinking_expanded = RwSignal::new(is_thinking_active);

    let (thinking_sig, _) = signal(thinking_text);
    let (answer_sig, _) = signal(answer_text);

    view! {
        <div class="tui-stream-line tui-assistant">
            <div class="tui-glyph-header">
                <span class="tui-glyph assistant">"◇"</span>
                <span class="tui-role-label">"assistant"</span>
            </div>

            <Show when=move || has_thinking fallback=|| ()>
                <div class="tui-thinking-box">
                    <Show
                        when=move || is_thinking_active
                        fallback=move || {
                            let tok_approx = thinking_sig.with(|t| t.split_whitespace().count() * 4 / 3);
                            view! {
                                <div
                                    class="tui-thinking-summary"
                                    on:click=move |_| thinking_expanded.update(|e| *e = !*e)
                                    title="Click to toggle reasoning trace"
                                >
                                    <span class="tui-think-caret">
                                        {move || if thinking_expanded.get() { "▼" } else { "▶" }}
                                    </span>
                                    <span class="tui-think-badge">"💭 Thought"</span>
                                    <span class="tui-think-meta">
                                        {format!("(~{tok_approx} tokens)")}
                                    </span>
                                </div>
                            }
                        }
                    >
                        <div class="tui-thinking-summary active">
                            <span class="tui-spinner">"⠋"</span>
                            <span class="tui-think-badge">"💭 Thinking..."</span>
                        </div>
                    </Show>

                    <Show when=move || thinking_expanded.get() || is_thinking_active fallback=|| ()>
                        <div class="tui-thinking-trace">
                            <pre class="tui-thinking-pre">{move || thinking_sig.get()}</pre>
                        </div>
                    </Show>
                </div>
            </Show>

            <Show when=move || !answer_sig.with(|a| a.is_empty()) fallback=|| ()>
                <div
                    class="tui-assistant-body markdown"
                    inner_html=move || render_markdown(&answer_sig.get())
                />
            </Show>
        </div>
    }.into_any()
}

/// Render a single user message with extracted editor context pill if present.
fn render_user_message(content: String) -> AnyView {
    let (pill_opt, clean) = extract_editor_context_prelude(&content);
    let pill_label = pill_opt.map(|s| s.to_string());
    let clean_text = clean.to_string();

    let (pill_sig, _) = signal(pill_label);
    let (text_sig, _) = signal(clean_text);

    view! {
        <div class="tui-stream-line tui-user">
            <Show when=move || pill_sig.with(|p| p.is_some()) fallback=|| ()>
                <div class="tui-attached-pill">
                    <span class="tui-pill-icon">"📎"</span>
                    <span class="tui-pill-text">{move || pill_sig.get().unwrap_or_default()}</span>
                </div>
            </Show>
            <div class="tui-user-prompt">
                <span class="tui-glyph user">"❯"</span>
                <span class="tui-user-text">{move || text_sig.get()}</span>
            </div>
        </div>
    }
    .into_any()
}

/// Render an agent tool step as a terminal TUI box with box-drawing glyphs.
#[allow(clippy::too_many_arguments)]
fn render_tool_step(
    id: String,
    name: String,
    summary: String,
    result: Option<ToolStepResult>,
    awaiting_permission: bool,
    is_current_awaiting: bool,
    on_permission: Callback<(String, bool)>,
    on_permission_always: Callback<String>,
) -> impl IntoView {
    let status_class = match result.as_ref() {
        Some(r) if r.ok => "ok",
        Some(_) => "err",
        None if awaiting_permission => "awaiting",
        None => "running",
    };

    let status_badge = match result.as_ref() {
        Some(r) if r.ok => "[✔ ok]",
        Some(_) => "[✖ err]",
        None if awaiting_permission => "[? permission required]",
        None => "[⠋ running]",
    };

    let (result_sig, _set_result) = signal(result);
    let (id_sig, _set_id) = signal(id);
    let (name_sig, _set_name) = signal(name.clone());
    let show_diff = RwSignal::new(true);

    view! {
        <div class=format!("tui-box tui-tool-box {status_class}")>
            <div class="tui-tool-topbar">
                <span class="tui-box-corner">"┌─"</span>
                <span class="tui-tool-tag">"[tool]"</span>
                <span class="tui-tool-title">{format!(" {name}(\"{summary}\") ")}</span>
                <span class="tui-tool-spacer"></span>
                <span class=format!("tui-tool-status-badge {status_class}")>{status_badge}</span>
                <span class="tui-box-corner">"─┐"</span>
            </div>

            <div class="tui-tool-inner">
                <Show
                    when=move || awaiting_permission && result_sig.get().is_none() && is_current_awaiting
                    fallback=|| ()
                >
                    <div class="tui-permission-prompt">
                        <div class="tui-perm-text">
                            {format!("? Allow {name} \"{summary}\"?")}
                        </div>
                        <div class="tui-perm-buttons">
                            <button
                                class="tui-perm-btn btn-y"
                                title="Approve this call [Alt+Y]"
                                on:click=move |_| on_permission.run((id_sig.get(), true))
                            >
                                "[Alt+Y]es"
                            </button>
                            <button
                                class="tui-perm-btn btn-n"
                                title="Deny this call [Alt+N]"
                                on:click=move |_| on_permission.run((id_sig.get(), false))
                            >
                                "[Alt+N]o"
                            </button>
                            <Show
                                when=move || openwebide_agent::policy::always_approvable(&name_sig.get())
                                fallback=|| ()
                            >
                                <button
                                    class="tui-perm-btn btn-a"
                                    title="Always approve for this session [Alt+A]"
                                    on:click=move |_| on_permission_always.run(id_sig.get())
                                >
                                    "[Alt+A]lways"
                                </button>
                            </Show>
                            <button
                                class="tui-perm-btn btn-d"
                                title="Toggle diff inspection [Alt+D]"
                                on:click=move |_| show_diff.update(|v| *v = !*v)
                            >
                                "[Alt+D]iff"
                            </button>
                        </div>
                    </div>
                </Show>

                <Show
                    when=move || awaiting_permission && result_sig.get().is_none() && !is_current_awaiting
                    fallback=|| ()
                >
                    <div class="tui-tool-pending muted">
                        "[canceled: no longer pending]"
                    </div>
                </Show>

                <Show
                    when=move || result_sig.get().is_none() && !awaiting_permission
                    fallback=|| ()
                >
                    <div class="tui-tool-pending">
                        <span class="tui-spinner">"⠋"</span>
                        " executing tool call on host..."
                    </div>
                </Show>

                <Show
                    when=move || result_sig.get().is_some_and(|r| r.diff.is_some()) && show_diff.get()
                    fallback=|| ()
                >
                    {move || {
                        let r = result_sig.get().unwrap();
                        render_diff_view(r.diff.unwrap())
                    }}
                </Show>

                <Show
                    when=move || result_sig.get().is_some_and(|r| r.diff.is_none())
                    fallback=|| ()
                >
                    {move || {
                        let r = result_sig.get().unwrap();
                        view! {
                            <div class="tui-tool-summary-out">{r.summary.clone()}</div>
                        }
                    }}
                </Show>
            </div>

            <div class="tui-tool-bottombar">
                <span class="tui-box-corner">"└"</span>
                <span class="tui-tool-barline"></span>
                <span class="tui-box-corner">"┘"</span>
            </div>
        </div>
    }
}

/// Powerline-style statusline segment above composer.
#[component]
fn TuiStatusLine(
    streaming: ReadSignal<bool>,
    has_awaiting: Signal<bool>,
    session_telemetry: ReadSignal<SessionTelemetry>,
    local_mode: Signal<bool>,
    models: ReadSignal<Vec<ModelInfo>>,
    on_select_model: Callback<Option<String>>,
) -> impl IntoView {
    let mode_state = move || {
        if has_awaiting.get() {
            ("AWAITING", "mode-awaiting")
        } else if streaming.get() {
            ("RUNNING", "mode-running")
        } else {
            ("NORMAL", "mode-normal")
        }
    };

    let gauge_color_class = move || {
        let pct = session_telemetry.get().context_percent();
        if pct >= 85.0 {
            "ctx-danger"
        } else if pct >= 70.0 {
            "ctx-warning"
        } else {
            "ctx-normal"
        }
    };

    let show_model_menu = RwSignal::new(false);

    view! {
        <div class="tui-statusline">
            <span class=move || format!("tui-mode-badge {}", mode_state().1)>
                "[" {move || mode_state().0} "]"
            </span>
            <span class="tui-sep">"│"</span>
            <span class="tui-model-name" title="Active Model" style="cursor: pointer; position: relative;" on:click=move |_| show_model_menu.set(!show_model_menu.get())>
                {move || session_telemetry.get().model}
                <Show when=move || show_model_menu.get() fallback=|| ()>
                    <div class="recent-backdrop" on:click=move |e| { e.stop_propagation(); show_model_menu.set(false); } />
                    <div class="recent-menu" style="bottom: 100%; top: auto; min-width: 200px;" on:click=move |e| e.stop_propagation()>
                        <div
                            class="recent-item"
                            on:click=move |_| {
                                on_select_model.run(None);
                                show_model_menu.set(false);
                            }
                        >
                            "Default model"
                        </div>
                        <For
                            each=move || models.get()
                            key=|m| m.name.clone()
                            children=move |m| {
                                let name = m.name.clone();
                                let sel_name = name.clone();
                                view! {
                                    <div
                                        class="recent-item"
                                        on:click=move |_| {
                                            on_select_model.run(Some(sel_name.clone()));
                                            show_model_menu.set(false);
                                        }
                                    >
                                        {name}
                                    </div>
                                }
                            }
                        />
                    </div>
                </Show>
            </span>
            <span class="tui-sep">"│"</span>
            <span class=move || format!("tui-ctx-gauge {}", gauge_color_class()) title="Context Window Utilization">
                "Ctx: "
                {move || {
                    let t = session_telemetry.get();
                    format!(
                        "{}{:.1}k",
                        SessionTelemetry::approx(t.context_estimated),
                        t.context_tokens as f64 / 1000.0
                    )
                }}
                "/"
                {move || {
                    let t = session_telemetry.get();
                    format!(
                        "{}{:.0}k",
                        SessionTelemetry::approx(t.context_limit_estimated),
                        t.context_limit as f64 / 1000.0
                    )
                }}
                " ("
                {move || format!("{:.0}%", session_telemetry.get().context_percent())}
                ") "
                {move || session_telemetry.get().gauge_bar()}
            </span>
            <span class="tui-sep">"│"</span>
            <span class="tui-speed" title="Generation Speed">
                {move || {
                    let t = session_telemetry.get();
                    match t.current_speed_tps {
                        Some(speed) => format!(
                            "{}{speed:.1} t/s",
                            SessionTelemetry::approx(t.speed_estimated)
                        ),
                        None => "-- t/s".to_string(),
                    }
                }}
            </span>
            <span class="tui-sep">"│"</span>
            <span class="tui-workspace-mode">
                {move || if local_mode.get() { "Local" } else { "Remote" }}
            </span>
            <span class="tui-sep">"│"</span>
            <span class="tui-tools-count">
                {move || format!("{} tools", session_telemetry.get().tool_calls_count)}
            </span>
        </div>
    }
}

#[component]
pub fn ChatPane(
    on_select_model: Callback<Option<String>>,
    on_send: Callback<()>,
    on_resume_local_run: Callback<()>,
    on_stop: Callback<()>,
    /// Approve or deny a gated tool call: `(tool_call_id, approved)`.
    on_permission: Callback<(String, bool)>,
    on_permission_always: Callback<String>,
    on_slash_command: Callback<SlashCommand>,
) -> impl IntoView {
    let chat = expect_context::<ChatState>();
    let layout = expect_context::<LayoutState>();
    let projects = expect_context::<ProjectsState>();
    let messages = chat.messages.read_only();
    let streaming = chat.streaming.read_only();
    let draft = chat.draft.read_only();
    let set_draft = chat.draft.write_only();
    let models = chat.models.read_only();
    let selected_model = chat.selected_model.read_only();
    let active_context = chat.active_editor_context.read_only();
    let set_active_context = chat.active_editor_context.write_only();
    let session_telemetry = chat.session_telemetry.read_only();
    let has_session = chat.has_session;
    let local_mode = Signal::from(projects.local_mode);
    let scroll_ref = NodeRef::<leptos::html::Div>::new();
    let input_ref = NodeRef::<leptos::html::Textarea>::new();
    let model_ref = NodeRef::<leptos::html::Select>::new();

    // Readline prompt history state
    let prompt_history = chat.prompt_history;
    let history_index = RwSignal::new(Option::<usize>::None);
    let draft_backup = RwSignal::new(String::new());

    // Check if any tool step is currently awaiting permission
    let awaiting_step_id = chat.awaiting_step_id;
    let awaiting_step = Memo::new(move |_| {
        let awaiting_id = awaiting_step_id.get()?;
        messages.get().into_iter().rev().find_map(|item| {
            if let ConversationItem::ToolStep {
                id,
                name,
                awaiting_permission: true,
                result: None,
                ..
            } = item
                && id == awaiting_id
            {
                return Some((id, name));
            }
            None
        })
    });
    let has_awaiting = Signal::derive(move || awaiting_step.get().is_some());

    // Keep the model picker's value in sync with the chosen model.
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

    // Mirror the draft signal into the textarea so it clears after a send.
    Effect::new(move || {
        let value = draft.get();
        if let Some(ta) = input_ref.get()
            && ta.value() != value
        {
            ta.set_value(&value);
        }
    });

    let submit_or_command = {
        move || {
            let current = draft.get().trim().to_string();
            if current.is_empty() {
                return;
            }

            prompt_history.update(|history| crate::history::push_history(history, current.clone()));
            history_index.set(None);

            if current.starts_with('/')
                && let Some(cmd) = SlashCommand::parse(&current)
            {
                set_draft.set(String::new());
                on_slash_command.run(cmd);
                return;
            }

            on_send.run(());
        }
    };

    view! {
        <main
            class="chat-pane tui-pane"
            style=move || format!("width: {}px; flex: none;", layout.chat_width.get())
        >
            <Show
                when=move || !messages.get().is_empty()
                fallback=move || {
                    view! {
                        <Show
                            when=move || has_session.get()
                            fallback=move || {
                                view! {
                                    <div class="empty-state tui-empty-state">
                                        <div class="tui-banner-ascii">
                                            "┌────────────────────────────────────────────────────────┐\n\
                                             │ Open WebIDE Terminal Execution Surface                 │\n\
                                             │ A WebAssembly IDE for local-LLM coding agents.         │\n\
                                             └────────────────────────────────────────────────────────┘"
                                        </div>
                                        <p class="muted">"Type a prompt or /help to inspect available commands."</p>
                                    </div>
                                }
                            }
                        >
                            <div class="messages tui-stream">
                                <div class="tui-stream-spacer"></div>
                                <p class="empty tui-empty">"Stream initialized. Ready for execution."</p>
                            </div>
                        </Show>
                    }
                }
            >
                <div class="messages tui-stream" node_ref=scroll_ref>
                    <div class="tui-stream-spacer"></div>
                    <For
                        each=move || { messages.get().into_iter().filter(|item| {
                            !matches!(item, ConversationItem::Message(message) if message.role == Role::Assistant && message.content.is_empty() && message.tool_calls.is_some())
                        }).collect::<Vec<_>>() }
                        key=|item| item_key(item)
                        children=move |item| {
                            let (item_sig, _set_item) = signal(item);
                            let is_stopped = matches!(item_sig.get(), ConversationItem::Stopped { .. });
                            let (stopped, _set_stopped) = signal(is_stopped);
                            let is_message = matches!(
                                item_sig.get(),
                                ConversationItem::Message(_) | ConversationItem::Notice { .. }
                            );
                            let (is_msg, _set_is_msg) = signal(is_message);

                            view! {
                                <Show
                                    when=move || stopped.get()
                                    fallback=move || {
                                        view! {
                                            <Show
                                                when=move || is_msg.get()
                                                fallback=move || {
                                                    let (id, name, summary, result, awaiting) =
                                                        match item_sig.get() {
                                                            ConversationItem::ToolStep {
                                                                id,
                                                                name,
                                                                summary,
                                                                result,
                                                                awaiting_permission,
                                                                key: _,
                                                            } => {
                                                                (
                                                                    id,
                                                                    name,
                                                                    summary,
                                                                    result,
                                                                    awaiting_permission,
                                                                )
                                                            }
                                                            _ => unreachable!("not a tool step"),
                                                        };
                                                    let is_current_awaiting = awaiting_step.get().is_some_and(|(cur_id, _)| cur_id == id);
                                                    render_tool_step(
                                                        id,
                                                        name,
                                                        summary,
                                                        result,
                                                        awaiting,
                                                        is_current_awaiting,
                                                        on_permission,
                                                        on_permission_always,
                                                    )
                                                }
                                            >
                                                {move || {
                                                    let (role, content) = match item_sig.get() {
                                                        ConversationItem::Message(message) => (message.role, message.content),
                                                        ConversationItem::Notice { text, .. } => (Role::Assistant, text),
                                                        _ => unreachable!("not a message or notice"),
                                                    };
                                                    if role == Role::Assistant {
                                                        render_assistant_message(content)
                                                    } else {
                                                        render_user_message(content)
                                                    }
                                                }}
                                            </Show>
                                        }
                                    }
                                >
                                    <div class="stopped-marker tui-stopped-marker">"⏹ execution aborted"</div>
                                </Show>
                            }
                        }
                    />
                    <Show when=move || chat.interrupted_run.get().is_some() && !streaming.get()>
                        <div class="tui-stopped-marker">
                            "This run was interrupted. "
                            <button class="btn send" on:click=move |_| on_resume_local_run.run(())>"Resume"</button>
                            <button class="btn" on:click=move |_| chat.dismiss_interrupted_run()>"Dismiss"</button>
                        </div>
                    </Show>
                </div>
            </Show>

            <TuiStatusLine
                streaming=streaming
                has_awaiting=has_awaiting
                session_telemetry=session_telemetry
                local_mode=local_mode
                models=models
                on_select_model=on_select_model
            />

            <Show when=move || active_context.get().is_some() fallback=|| ()>
                <div class="tui-active-context-pill">
                    <span class="pill-icon">"📎"</span>
                    <span class="pill-text">
                        {move || active_context.get().as_ref().map(|c| c.pill_label()).unwrap_or_default()}
                    </span>
                    <button
                        class="pill-dismiss"
                        title="Detach editor context (Esc)"
                        on:click=move |_| set_active_context.set(None)
                    >
                        "×"
                    </button>
                </div>
            </Show>

            <div class="composer tui-composer">
                <span class="tui-prompt-glyph">"❯"</span>
                <textarea
                    class="composer-input tui-input"
                    node_ref=input_ref
                    placeholder=move || {
                        if let Some((_, name)) = awaiting_step.get() {
                            if openwebide_agent::policy::always_approvable(&name) {
                                "? Tool awaiting approval: press [Alt+Y]es, [Alt+N]o, or [Alt+A]lways..."
                            } else {
                                "? Tool awaiting approval: press [Alt+Y]es, or [Alt+N]o..."
                            }
                        } else if has_session.get() {
                            "Ask a question or /command (Enter to send, Shift+Enter for newline, Up/Down for history)"
                        } else {
                            "Start a session or /help (Enter to send, Shift+Enter for newline)"
                        }
                    }
                    on:input=move |e: web_sys::Event| {
                        if let Some(target) = e.target()
                            && let Some(textarea) = target.dyn_ref::<web_sys::HtmlTextAreaElement>()
                        {
                            set_draft.set(textarea.value());
                        }
                    }
                    on:keydown={
                        let submit = submit_or_command;
                        move |e: leptos::ev::KeyboardEvent| {
                            let key = e.key();

                            // Intercept permission handshake if waiting for approval
                            if let Some((id, name)) = awaiting_step.get()
                                && e.alt_key() && !e.ctrl_key() && !e.meta_key()
                            {
                                let code = e.code();
                                if code == "KeyY" {
                                    e.prevent_default();
                                    on_permission.run((id, true));
                                    return;
                                }
                                if code == "KeyN" {
                                    e.prevent_default();
                                    on_permission.run((id, false));
                                    return;
                                }
                                if code == "KeyA" && openwebide_agent::policy::always_approvable(&name) {
                                    e.prevent_default();
                                    on_permission_always.run(id);
                                    return;
                                }
                            }

                            // Enter submits
                            if key == "Enter" && !e.shift_key() {
                                e.prevent_default();
                                submit();
                                return;
                            }

                            // Readline history navigation with Up/Down
                            if key == "ArrowUp" {
                                let hist = prompt_history.get();
                                if !hist.is_empty() {
                                    let is_at_start = input_ref.get()
                                        .and_then(|el| el.selection_start().ok().flatten())
                                        .unwrap_or(0) == 0;
                                    if is_at_start || draft.get().is_empty() {
                                        e.prevent_default();
                                        match history_index.get() {
                                            None => {
                                                draft_backup.set(draft.get());
                                                let last_idx = hist.len() - 1;
                                                history_index.set(Some(last_idx));
                                                set_draft.set(hist[last_idx].clone());
                                            }
                                            Some(idx) if idx > 0 => {
                                                let prev = idx - 1;
                                                history_index.set(Some(prev));
                                                set_draft.set(hist[prev].clone());
                                            }
                                            _ => {}
                                        }
                                    }
                                }
                                return;
                            }

                            if key == "ArrowDown" {
                                let hist = prompt_history.get();
                                if let Some(idx) = history_index.get() {
                                    e.prevent_default();
                                    if idx + 1 < hist.len() {
                                        let next = idx + 1;
                                        history_index.set(Some(next));
                                        set_draft.set(hist[next].clone());
                                    } else {
                                        history_index.set(None);
                                        set_draft.set(draft_backup.get());
                                    }
                                }
                                return;
                            }

                            // Esc detaches context pill if draft is empty, or cancels run if streaming
                            if key == "Escape" {
                                if active_context.get().is_some() && draft.get().trim().is_empty() {
                                    e.prevent_default();
                                    set_active_context.set(None);
                                    return;
                                }
                                if streaming.get() {
                                    e.prevent_default();
                                    on_stop.run(());
                                    return;
                                }
                            }

                            // Ctrl+C cancels streaming if no text selected
                            if (e.ctrl_key() || e.meta_key()) && key == "c" && streaming.get() {
                                let has_selection = input_ref.get()
                                    .map(|el| {
                                        let s = el.selection_start().ok().flatten().unwrap_or(0);
                                        let end = el.selection_end().ok().flatten().unwrap_or(0);
                                        end > s
                                    })
                                    .unwrap_or(false);
                                if !has_selection {
                                    e.prevent_default();
                                    on_stop.run(());
                                }
                            }
                        }
                    }
                />

                <Show
                    when=move || streaming.get()
                    fallback=move || {
                        let submit = submit_or_command;
                        view! {
                            <button
                                class="btn send tui-btn-send"
                                disabled=move || draft.with(|d| d.trim().is_empty())
                                on:click=move |_| submit()
                            >
                                "Send"
                            </button>
                        }
                    }
                >
                    <button class="btn stop tui-btn-stop" on:click=move |_| on_stop.run(())>"Stop"</button>
                </Show>
            </div>
        </main>
    }
}
