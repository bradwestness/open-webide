use std::time::Duration;

use js_sys::Date;

use crate::state::{
    chat::ChatState, layout::LayoutState, projects::ProjectsState, settings::SettingsState,
};
use leptos::prelude::*;
use openwebide_core::{
    Connection, FileDiff, ModelInfo, Role, diff_inline_detailed, diff_inline_lines,
    tui::{SessionTelemetry, SlashCommand, extract_editor_context_prelude, parse_thinking},
};
use web_sys::wasm_bindgen::JsCast;

pub use crate::conversation::{ConversationItem, ToolStepResult};

pub(crate) use crate::markdown::render as render_markdown;

/// Format elapsed seconds as `Xs`, `YmXs`, or `ZhYmXs` depending on magnitude.
fn format_elapsed(secs: u32) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m{}s", secs / 60, secs % 60)
    } else {
        format!("{}h{}m{}s", secs / 3600, (secs % 3600) / 60, secs % 60)
    }
}

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

fn render_approval_diff(diff: Memo<Option<FileDiff>>) -> impl IntoView {
    let expanded = RwSignal::new(false);
    let lines = Memo::new(move |_| {
        diff.with(|diff| diff.as_ref().map(diff_inline_detailed).unwrap_or_default())
    });
    let has_more = move || lines.with(|lines| lines.len() > 40);
    view! {
        <div class="tui-diff-box">
            <div class="tui-diff-path">"diff: " {move || diff.with(|diff| diff.as_ref().map(|diff| diff.path.clone()).unwrap_or_default())}</div>
            <div class="tui-diff-lines">
                {move || lines.with(|lines| {
                    lines.iter().take(if expanded.get() { lines.len() } else { 40 }).cloned().map(|line| {
                        let class = if line.marker == '+' { "tui-diff-line add" } else { "tui-diff-line del" };
                        view! {
                            <div class=class>
                                <span>{line.marker} " "</span>
                                {super::editor::render_diff_chunks(line.chunks)}
                                {line.ending_note.map(|note| view! { <span class="form-hint">{note}</span> })}
                            </div>
                        }
                    }).collect::<Vec<_>>()
                })}
            </div>
            <Show when=move || has_more() && !expanded.get()>
                <button class="btn" on:click=move |_| expanded.set(true)>"Show full diff"</button>
            </Show>
        </div>
    }
}

/// Render a single assistant message with collapsible thinking stream.
fn render_assistant_message(content: Memo<String>) -> AnyView {
    let parsed = Memo::new(move |_| content.with(|content| parse_thinking(content)));
    let thinking_sig =
        Memo::new(move |_| parsed.with(|parsed| parsed.thinking.clone().unwrap_or_default()));
    let answer_sig = Memo::new(move |_| parsed.with(|parsed| parsed.answer.clone()));
    let is_thinking_active = Memo::new(move |_| parsed.with(|parsed| parsed.is_thinking));
    let thinking_expanded = RwSignal::new(false);

    // Elapsed time of the live thinking phase: the timer runs only while the
    // model streams reasoning, then the last value is frozen for the summary.
    // `std::time::Instant` is unimplemented on wasm32, so track the start as
    // `Date::now()` milliseconds instead.
    let started: StoredValue<Option<f64>> = StoredValue::new(None);
    let elapsed_secs = RwSignal::new(0u32);
    let timer: StoredValue<Option<IntervalHandle>> = StoredValue::new(None);
    let update_elapsed = move || {
        if let Some(started_at) = started.get_value() {
            // Elapsed time is non-negative and far below u32::MAX seconds,
            // so the float-to-int cast is safe.
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let secs = ((Date::now() - started_at) / 1000.0) as u32;
            elapsed_secs.set(secs);
        }
    };
    Effect::new(move |_| {
        if is_thinking_active.get() {
            started.set_value(Some(Date::now()));
            elapsed_secs.set(0);
            timer.set_value(
                set_interval_with_handle(update_elapsed, Duration::from_millis(500)).ok(),
            );
        } else {
            update_elapsed();
            started.set_value(None);
            timer.update_value(|timer| {
                if let Some(handle) = timer.take() {
                    handle.clear();
                }
            });
        }
    });
    on_cleanup(move || {
        if let Some(handle) = timer.get_value() {
            handle.clear();
        }
    });

    view! {
        <div class="tui-stream-line tui-assistant">
            <Show when=move || !thinking_sig.with(String::is_empty) fallback=|| ()>
                <div class="tui-thinking-box">
                    <Show
                        when=move || is_thinking_active.get()
                        fallback=move || {
                            let tok_approx = move || thinking_sig.with(|t| t.split_whitespace().count() * 4 / 3);
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
                                        {move || {
                                            let tokens = tok_approx();
                                            let secs = elapsed_secs.get();
                                            if secs > 0 {
                                                format!("(~{tokens} tokens · {})", format_elapsed(secs))
                                            } else {
                                                format!("(~{tokens} tokens)")
                                            }
                                        }}
                                    </span>
                                </div>
                            }
                        }
                    >
                        <div
                            class="tui-thinking-summary active"
                            on:click=move |_| thinking_expanded.update(|e| *e = !*e)
                            title="Click to toggle reasoning trace"
                        >
                            <span class="tui-think-caret">
                                {move || if thinking_expanded.get() { "▼" } else { "▶" }}
                            </span>
                            <span class="tui-think-badge">"💭 Thinking..."</span>
                            <span class="tui-spinner"/>
                            <span class="tui-think-meta">
                                {move || format_elapsed(elapsed_secs.get())}
                            </span>
                        </div>
                    </Show>

                    <Show when=move || thinking_expanded.get() fallback=|| ()>
                        <div class="tui-thinking-trace">
                            <pre class="tui-thinking-pre">{move || thinking_sig.get()}</pre>
                        </div>
                    </Show>
                </div>
            </Show>

            <Show when=move || !answer_sig.with(String::is_empty) fallback=|| ()>
                <div
                    class="tui-assistant-body markdown"
                    inner_html=move || render_markdown(&answer_sig.get())
                />
            </Show>
        </div>
    }.into_any()
}

/// Render a single user message with extracted editor context pill if present.
fn render_user_message(content: Memo<String>) -> AnyView {
    let pill_sig = Memo::new(move |_| {
        content.with(|content| {
            extract_editor_context_prelude(content)
                .0
                .map(ToString::to_string)
        })
    });
    let text_sig = Memo::new(move |_| {
        content.with(|content| extract_editor_context_prelude(content).1.to_string())
    });

    view! {
        <div class="tui-stream-line tui-user">
            <Show when=move || pill_sig.with(Option::is_some) fallback=|| ()>
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
fn render_tool_step(
    item: RwSignal<ConversationItem>,
    awaiting_step: Memo<Option<(String, String)>>,
    on_permission: Callback<(String, bool)>,
    on_permission_always: Callback<String>,
) -> impl IntoView {
    let result_sig = Memo::new(move |_| {
        item.with(|item| match item {
            ConversationItem::ToolStep { result, .. } => result.clone(),
            _ => None,
        })
    });
    let id_sig = Memo::new(move |_| {
        item.with(|item| match item {
            ConversationItem::ToolStep { id, .. } => id.clone(),
            _ => String::new(),
        })
    });
    let name_sig = Memo::new(move |_| {
        item.with(|item| match item {
            ConversationItem::ToolStep { name, .. } => name.clone(),
            _ => String::new(),
        })
    });
    let summary = Memo::new(move |_| {
        item.with(|item| match item {
            ConversationItem::ToolStep { summary, .. } => summary.clone(),
            _ => String::new(),
        })
    });
    let awaiting_permission = Memo::new(move |_| {
        item.with(|item| {
            matches!(
                item,
                ConversationItem::ToolStep {
                    awaiting_permission: true,
                    ..
                }
            )
        })
    });
    let is_current_awaiting = Memo::new(move |_| {
        awaiting_step.with(|step| {
            step.as_ref()
                .is_some_and(|(id, _)| id_sig.with(|row_id| id == row_id))
        })
    });
    let preview = Memo::new(move |_| {
        item.with(|item| match item {
            ConversationItem::ToolStep { diff, .. } => diff.clone(),
            _ => None,
        })
    });
    let note = Memo::new(move |_| {
        item.with(|item| match item {
            ConversationItem::ToolStep { note, .. } => note.clone(),
            _ => None,
        })
    });
    let status_class = move || {
        result_sig.with(|result| match result {
            Some(result) if result.ok => "ok",
            Some(_) => "err",
            None if awaiting_permission.get() => "awaiting",
            None => "running",
        })
    };
    let status_badge = move || match status_class() {
        "ok" => "[✔ ok]",
        "err" => "[✖ err]",
        "awaiting" => "[? permission required]",
        _ => "[⠋ running]",
    };
    let show_diff = RwSignal::new(true);

    view! {
        <div class=move || format!("tui-box tui-tool-box {}", status_class())>
            <div class="tui-tool-topbar">
                <span class="tui-box-corner">"┌─"</span>
                <span class="tui-tool-tag">"[tool]"</span>
                <span class="tui-tool-title">{move || format!(" {}(\"{}\") ", name_sig.get(), summary.get())}</span>
                <span class="tui-tool-spacer"></span>
                <span class=move || format!("tui-tool-status-badge {}", status_class())>{status_badge}</span>
                <span class="tui-box-corner">"─┐"</span>
            </div>

            <div class="tui-tool-inner">
                <Show when=move || result_sig.with(Option::is_none)>
                    {move || note.get().map(|note| view! { <div class="tui-perm-text">{note}</div> })}
                    <Show when=move || show_diff.get() && preview.with(|diff| diff.as_ref().is_some_and(|diff| !diff.old_unavailable))>
                        {render_approval_diff(preview)}
                    </Show>
                    <Show when=move || preview.with(|diff| diff.as_ref().is_some_and(|diff| diff.old_unavailable))>
                        <div class="tui-perm-text">"diff unavailable"</div>
                    </Show>
                </Show>
                <Show
                    when=move || awaiting_permission.get() && result_sig.get().is_none() && is_current_awaiting.get()
                    fallback=|| ()
                >
                    <div class="tui-permission-prompt">
                        <div class="tui-perm-text">
                            {move || format!("? Allow {} \"{}\"?", name_sig.get(), summary.get())}
                        </div>
                        <div class="tui-perm-buttons">
                            <button
                                class="btn approve tui-perm-btn btn-y"
                                title="Approve this call [Alt+Y]"
                                on:click=move |_| on_permission.run((id_sig.get(), true))
                            >
                                "[Alt+Y]es"
                            </button>
                            <button
                                class="btn deny tui-perm-btn btn-n"
                                title="Deny this call [Alt+N]"
                                on:click=move |_| on_permission.run((id_sig.get(), false))
                            >
                                "[Alt+N]o"
                            </button>
                            <Show
                                when=move || name_sig.get() == "write_file"
                                fallback=|| ()
                            >
                                <button
                                    class="btn send tui-perm-btn btn-a"
                                    title="Auto-accept edits for this session [Alt+A]"
                                    on:click=move |_| on_permission_always.run(id_sig.get())
                                >
                                    "[Alt+A]uto-accept edits"
                                </button>
                            </Show>
                            <button
                                class="btn ghost tui-perm-btn btn-d"
                                title="Toggle diff inspection [Alt+D]"
                                on:click=move |_| show_diff.update(|v| *v = !*v)
                            >
                                "[Alt+D]iff"
                            </button>
                        </div>
                    </div>
                </Show>

                <Show
                    when=move || awaiting_permission.get() && result_sig.get().is_none() && !is_current_awaiting.get()
                    fallback=|| ()
                >
                    <div class="tui-tool-pending muted">
                        "[canceled: no longer pending]"
                    </div>
                </Show>

                <Show
                    when=move || result_sig.get().is_none() && !awaiting_permission.get()
                    fallback=|| ()
                >
                    <div class="tui-tool-pending">
                        <span class="tui-spinner"/>
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

/// Discover models only when a connection is expanded in the picker.
#[component]
fn ConnectionModels(
    connection: Connection,
    active_connection: Signal<Option<i64>>,
    models: ReadSignal<Vec<ModelInfo>>,
    on_select: Callback<(i64, String)>,
) -> impl IntoView {
    let api = expect_context::<crate::backend::Api>();
    let chat = expect_context::<ChatState>();
    let id = connection.id;
    let enabled = connection.enabled;
    let expanded = RwSignal::new(active_connection.get_untracked() == Some(id));
    let discovered = RwSignal::new(Vec::<ModelInfo>::new());
    let loading = RwSignal::new(false);
    let error = RwSignal::new(Option::<String>::None);
    let request = StoredValue::new(0_u64);
    Effect::new(move |_| {
        request.update_value(|generation| *generation += 1);
        let generation = request.get_value();
        if !expanded.get() || active_connection.get() == Some(id) || !enabled {
            return;
        }
        loading.set(true);
        error.set(None);
        leptos::task::spawn_local(async move {
            let result = api.with_value(Clone::clone).list_models(id).await;
            if request.try_get_value() != Some(generation) {
                return;
            }
            loading.set(false);
            match result {
                Ok(models) => discovered.set(models),
                Err(message) => error.set(Some(message)),
            }
        });
    });
    let available = Signal::derive(move || {
        if active_connection.get() == Some(id) {
            models.get()
        } else {
            discovered.get()
        }
    });
    view! {
        <div class="tui-connection-group" data-connection-id=id.to_string()>
            <button class="btn recent-item tui-connection-heading"
                aria-expanded=move || expanded.get().to_string()
                disabled=!enabled
                on:click=move |_| expanded.update(|value| *value = !*value)>
                {move || if expanded.get() { "▾" } else { "▸" }} " " {connection.name}
            </button>
            <Show when=move || expanded.get()>
                <Show when=move || loading.get()><span class="form-hint">"Loading models…"</span></Show>
                <Show when=move || error.get().is_some()><span class="form-hint">{move || error.get()}</span></Show>
                <Show when=move || !loading.get() && error.get().is_none() && available.get().is_empty()>
                    <span class="form-hint">"No models available"</span>
                </Show>
                <For each=move || available.get() key=|model| model.name.clone() children=move |model| {
                    let name = model.name.clone();
                    let active_name = name.clone();
                    view! {
                        <button class="btn recent-item tui-connection-model"
                            aria-current=move || (active_connection.get() == Some(id) && chat.session_telemetry.with(|telemetry| telemetry.model == active_name)).to_string()
                            disabled=move || chat.streaming.get() || chat.connection_changing.get()
                            on:click=move |_| on_select.run((id, name.clone()))>
                            {model.name}
                        </button>
                    }
                } />
            </Show>
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
    on_select_connection_model: Callback<(i64, String)>,
) -> impl IntoView {
    let gauge_color_class = move || {
        let pct = session_telemetry.with(SessionTelemetry::context_percent);
        if pct >= 85.0 {
            "ctx-danger"
        } else if pct >= 70.0 {
            "ctx-warning"
        } else {
            "ctx-normal"
        }
    };

    let show_model_menu = RwSignal::new(false);
    let chat = expect_context::<ChatState>();
    let settings = expect_context::<SettingsState>();
    let active_connection = Signal::derive(move || {
        chat.active_session
            .get()
            .and_then(|id| {
                chat.sessions.with(|sessions| {
                    sessions
                        .iter()
                        .find(|session| session.id == id)
                        .and_then(|session| session.connection_id)
                })
            })
            .or_else(|| {
                chat.draft_connection
                    .get()
                    .filter(|_| chat.active_session.get().is_none())
            })
            .or_else(|| {
                settings
                    .model_setup
                    .get()
                    .defaults
                    .primary
                    .map(|selection| selection.server_id)
            })
            .or_else(|| settings.default_connection.get())
            .or_else(|| {
                settings.connections.with(|connections| {
                    connections
                        .iter()
                        .find(|connection| connection.enabled)
                        .map(|connection| connection.id)
                })
            })
    });
    let on_select = Callback::new(move |selection: (i64, String)| {
        on_select_connection_model.run(selection);
        show_model_menu.set(false);
    });

    view! {
        <div class="tui-statusline">
            <super::approval_mode::ApprovalModePicker />
            <Show when=move || streaming.get() || has_awaiting.get()><span class="tui-run-state">{move || if has_awaiting.get() { "Awaiting" } else { "Running" }}</span></Show>
            <span class="tui-sep">"│"</span>
            <span class="tui-model-name" title="Choose connection and model" on:click=move |_| show_model_menu.set(!show_model_menu.get())>
                {move || session_telemetry.with(|telemetry| telemetry.model.clone())}
                <Show when=move || show_model_menu.get() fallback=|| ()>
                    <div class="recent-backdrop" on:click=move |e| { e.stop_propagation(); show_model_menu.set(false); } />
                    <div class="recent-menu tui-model-menu" on:click=move |e| e.stop_propagation()>
                        <For each=move || settings.connections.get()
                            key=|connection| (connection.id, connection.name.clone(), connection.base_url.clone(), connection.enabled)
                            children=move |connection| view! {
                                <ConnectionModels connection=connection active_connection=active_connection models=models on_select=on_select />
                            }
                        />
                    </div>
                </Show>
            </span>
            <span class="tui-sep">"│"</span>
            <span class=move || format!("tui-ctx-gauge {}", gauge_color_class()) title="Context Window Utilization">
                "Ctx: "
                {move || session_telemetry.with(SessionTelemetry::compact_context_tokens)}
                "/"
                {move || session_telemetry.with(SessionTelemetry::compact_context_limit)}
                " ("
                {move || format!("{:.0}%", session_telemetry.with(SessionTelemetry::context_percent))}
                ") "
                {move || session_telemetry.with(SessionTelemetry::gauge_bar)}
            </span>
            <span class="tui-sep">"│"</span>
            <span class="tui-speed" title="Generation Speed">
                {move || session_telemetry.with(SessionTelemetry::speed_text)}
            </span>
            <span class="tui-sep">"│"</span>
            <span class="tui-workspace-mode">
                {move || if local_mode.get() { "Local" } else { "Remote" }}
            </span>
            <span class="tui-sep">"│"</span>
            <span class="tui-tools-count">
                {move || format!("{} tools", session_telemetry.with(|telemetry| telemetry.tool_calls_count))}
            </span>
        </div>
    }
}

#[component]
pub fn ChatPane(
    on_select_connection_model: Callback<(i64, String)>,
    on_send: Callback<()>,
    on_open_local: Callback<()>,
    on_open_remote: Callback<()>,
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
    let messages = chat.messages;
    let streaming = chat.streaming.read_only();
    let draft = chat.draft.read_only();
    let set_draft = chat.draft.write_only();
    let models = chat.models.read_only();
    let active_context = chat.active_editor_context.read_only();
    let set_active_context = chat.active_editor_context.write_only();
    let session_telemetry = chat.session_telemetry.read_only();
    let has_session = chat.has_session;
    let local_mode = Signal::from(projects.local_mode);
    let has_project = move || projects.active_project.get().is_some();
    let scroll_ref = NodeRef::<leptos::html::Div>::new();
    let input_ref = NodeRef::<leptos::html::Textarea>::new();

    // Readline prompt history state
    let prompt_history = chat.prompt_history;
    let history_index = RwSignal::new(Option::<usize>::None);
    let draft_backup = RwSignal::new(String::new());

    // Check if any tool step is currently awaiting permission
    let awaiting_step_id = chat.awaiting_step_id;
    let awaiting_step = Memo::new(move |_| {
        let awaiting_id = awaiting_step_id.get()?;
        messages.handles.with(|handles| {
            handles
                .iter()
                .rev()
                .find_map(|handle| handle.permission.get().filter(|(id, _)| *id == awaiting_id))
        })
    });
    let has_awaiting = Signal::derive(move || awaiting_step.get().is_some());

    // Keep the newest message in view as tokens arrive.
    Effect::new(move || {
        messages.changed.get();
        if let Some(el) = scroll_ref.get() {
            el.set_scroll_top(f64::from(el.scroll_height()));
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
            if !has_project() {
                return;
            }
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
                when=move || !messages.handles.with(Vec::is_empty)
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
                        each=move || messages.handles.with(|handles| handles.iter().copied().filter(|handle| handle.visible.get()).collect::<Vec<_>>())
                        key=|handle| handle.key
                        children=move |handle| {
                            let item = handle.item;
                            match item.get_untracked() {
                                ConversationItem::Stopped { .. } => view! { <div class="stopped-marker tui-stopped-marker">"⏹ execution aborted"</div> }.into_any(),
                                ConversationItem::ToolStep { .. } => render_tool_step(item, awaiting_step, on_permission, on_permission_always).into_any(),
                                ConversationItem::Message(_) | ConversationItem::Notice { .. } => {
                                    let content = Memo::new(move |_| item.with(|item| match item {
                                        ConversationItem::Message(message) => message.content.clone(),
                                        ConversationItem::Notice { text, .. } => text.clone(),
                                        _ => String::new(),
                                    }));
                                    let system = Memo::new(move |_| item.with(|item| matches!(item, ConversationItem::Message(message) if message.role == Role::System)));
                                    let assistant = Memo::new(move |_| item.with(|item| !matches!(item, ConversationItem::Message(message) if message.role != Role::Assistant)));
                                    view! {
                                        <Show when=move || system.get() fallback=move || view! {
                                            <Show when=move || assistant.get() fallback=move || render_user_message(content)>
                                                {render_assistant_message(content)}
                                            </Show>
                                        }>
                                            <details class="tui-thinking-box">
                                                <summary class="tui-thinking-summary">"Run context"</summary>
                                                <div class="tui-thinking-trace">
                                                    <pre class="tui-thinking-pre">{move || {
                                                        let text = content.get();
                                                        text.strip_prefix(openwebide_core::RUN_CONTEXT_PREFIX).unwrap_or(&text).to_string()
                                                    }}</pre>
                                                </div>
                                            </details>
                                        </Show>
                                    }.into_any()
                                }
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
                on_select_connection_model=on_select_connection_model
            />

            <Show when=move || active_context.get().is_some() fallback=|| ()>
                <div class="tui-active-context-pill">
                    <span class="pill-icon">"📎"</span>
                    <span class="pill-text">
                        {move || active_context.get().as_ref().map(openwebide_core::EditorContext::pill_label).unwrap_or_default()}
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

            <Show when=move || !has_project()>
                <div class="chat-hint">
                    <p>"Open a project to start a session"</p>
                    <button class="btn" on:click=move |_| on_open_local.run(())>"Open local"</button>
                    " "
                    <button class="btn" on:click=move |_| on_open_remote.run(())>"Open remote"</button>
                </div>
            </Show>

            <div class="composer tui-composer">
                <span class="tui-prompt-glyph">"❯"</span>
                <textarea
                    class="composer-input tui-input"
                    node_ref=input_ref
                    disabled=move || !has_project() && !streaming.get()
                    placeholder=move || {
                        if let Some((_, name)) = awaiting_step.get() {
                            if name == "write_file" {
                                "? Tool awaiting approval: press [Alt+Y]es, [Alt+N]o, or [Alt+A]uto-accept edits..."
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
                        if !has_project() {
                            return;
                        }
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
                            if !has_project()
                                && !(streaming.get()
                                    && (key == "Escape" || ((e.ctrl_key() || e.meta_key()) && key == "c")))
                            {
                                e.prevent_default();
                                return;
                            }

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
                                if code == "KeyA" && name == "write_file" {
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
                                disabled=move || !has_project() || chat.creating_session.get() || draft.with(|d| d.trim().is_empty())
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
