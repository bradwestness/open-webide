//! Bottom dock terminal pane powered by WebSocket bridge daemon.
//!
//! Provides an interactive PTY terminal and command execution dock in the IDE.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::bridge::{BridgeConn, BridgeStatus};
use leptos::html::Div;
use leptos::prelude::*;
use openwebide_core::{BridgeClientMessage, BridgeServerMessage};
use web_sys::wasm_bindgen::JsCast;

struct ProjectSpawnFallback {
    message: BridgeClientMessage,
    current: std::rc::Rc<dyn Fn() -> bool>,
}

static COUNTER: AtomicU64 = AtomicU64::new(1);

fn next_session_id() -> String {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    // JS milliseconds are rounded down and saturate at the u64 bounds.
    let now = js_sys::Date::now().max(0.0) as u64;
    let count = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("term-{now}-{count}")
}

#[component]
pub fn TerminalDock(bridge: BridgeConn, visible: RwSignal<bool>) -> impl IntoView {
    let opened = Memo::new(move |previous: Option<&bool>| {
        visible.get() || previous.copied().unwrap_or(false)
    });
    let bridge = StoredValue::new_local(bridge);
    view! {
        <Show when=move || opened.get()>
            <div class="terminal-visibility" class:hidden=move || !visible.get()>
                <TerminalPane bridge=bridge.get_value() on_close=move || visible.set(false) />
            </div>
        </Show>
    }
}

#[component]
pub fn TerminalPane(bridge: BridgeConn, on_close: impl Fn() + Copy + 'static) -> impl IntoView {
    let status = RwSignal::from(bridge.status());
    let bridge = StoredValue::new_local(bridge);
    let output = StoredValue::new(crate::terminal_output::TerminalOutput::default());
    let lines = RwSignal::new(Vec::<Arc<crate::terminal_output::Line>>::new());
    let current_html = RwSignal::new(String::new());
    let request = StoredValue::new(None::<i32>);
    let following = RwSignal::new(true);
    let active_session = RwSignal::new(Option::<String>::None);
    let input_text = RwSignal::new(String::new());
    let history = RwSignal::new(Vec::<String>::new());
    let history_index = RwSignal::new(Option::<usize>::None);
    let last_seq = RwSignal::new(0u64);

    let output_ref = NodeRef::<Div>::new();
    let callback = StoredValue::new_local(wasm_bindgen::closure::Closure::<dyn FnMut()>::new(
        move || {
            request.set_value(None);
            output.update_value(|output| {
                if output.take_structure_changed() {
                    lines.set(output.completed().cloned().collect());
                }
                current_html.set(output.current_html());
            });
            // Reactive subscribers publish the new DOM before this scroll measurement.
            leptos::leptos_dom::helpers::queue_microtask(move || {
                if following.try_get_untracked() == Some(true)
                    && let Some(Some(element)) = output_ref.try_get_untracked()
                {
                    element.set_scroll_top(f64::from(element.scroll_height()));
                }
            });
        },
    ));
    let schedule = move || {
        if request.get_value().is_none() {
            let id = callback.with_value(|callback| {
                window().request_animation_frame(callback.as_ref().unchecked_ref())
            });
            if let Ok(id) = id {
                request.set_value(Some(id));
            }
        }
    };
    let append = move |text: &str| {
        if let Some(Some(element)) = output_ref.try_get_untracked() {
            following.set(
                f64::from(element.scroll_height() - element.client_height()) - element.scroll_top()
                    <= 32.0,
            );
        }
        output.update_value(|output| output.push(text));
        schedule();
    };
    let append_notice = move |text: &str| {
        if let Some(Some(element)) = output_ref.try_get_untracked() {
            following.set(
                f64::from(element.scroll_height() - element.client_height()) - element.scroll_top()
                    <= 32.0,
            );
        }
        output.update_value(|output| output.push_notice(text));
        schedule();
    };
    let append_process_notice = move |text: &str| {
        output.update_value(crate::terminal_output::TerminalOutput::discard_pending_escape);
        append_notice(text);
    };
    on_cleanup(move || {
        if let Some(id) = request.get_value() {
            let _ = window().cancel_animation_frame(id);
        }
    });

    let tx_outbound = bridge;
    let spawned_ids: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let reconnecting = RwSignal::new(false);
    let pending_spawn_ids = RwSignal::new(Vec::<String>::new());
    let projects = expect_context::<crate::state::projects::ProjectsState>();
    let auth = expect_context::<crate::state::auth::AuthState>();
    let project_host = expect_context::<crate::project_host::ProjectHost>();
    let settings = expect_context::<crate::state::settings::SettingsState>();
    let pending_spawns = RwSignal::new(0usize);
    let alive = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let alive_for_cleanup = alive.clone();
    on_cleanup(move || alive_for_cleanup.store(false, Ordering::Relaxed));
    let remote_fallbacks =
        StoredValue::new_local(std::collections::HashMap::<String, ProjectSpawnFallback>::new());
    let ids_for_spawn = spawned_ids.clone();
    let spawn = StoredValue::new_local(
        move |args: Vec<String>, notice: Option<String>, exclusive: bool| {
            if !status.get_untracked().terminal_ready() {
                return;
            }
            let project = projects
                .active_project
                .get_untracked()
                .and_then(|id| projects.project(id));
            let generation = auth.generation.get_untracked();
            let url = settings.bridge_url.get_untracked();
            let sender = bridge.get_value();
            let ids = ids_for_spawn.clone();
            let alive = alive.clone();
            let project_id = project.as_ref().map(|project| project.id);
            let current = std::rc::Rc::new(move || {
                alive.load(Ordering::Relaxed)
                    && auth.generation.try_get_untracked() == Some(generation)
                    && settings.bridge_url.try_get_untracked().as_ref() == Some(&url)
            });
            let current_for_finish = current.clone();
            let finish = move |cwd: Option<String>| {
                if !current_for_finish() {
                    return;
                }
                pending_spawns.update(|pending| *pending = pending.saturating_sub(1));
                if exclusive
                    && (active_session.get_untracked().is_some() || !ids.lock().unwrap().is_empty())
                {
                    append_notice(
                        "Terminal is busy; wait for the current process to finish before running /test again.\n",
                    );
                    return;
                }
                let cwd = if project_id.is_some_and(|id| projects.project(id).is_none()) {
                    None
                } else {
                    cwd
                };
                let fallback = cwd.is_none();
                let id = next_session_id();
                let message = BridgeClientMessage::Spawn {
                    id: id.clone(),
                    command: "sh".into(),
                    args,
                    cwd,
                    env: std::collections::HashMap::new(),
                    pty: true,
                    cols: 120,
                    rows: 30,
                };
                if sender.send(message.clone()).is_ok() {
                    if project_id.is_some() && !fallback {
                        remote_fallbacks.update_value(|fallbacks| {
                            fallbacks.insert(
                                id.clone(),
                                ProjectSpawnFallback {
                                    message,
                                    current: current_for_finish.clone(),
                                },
                            );
                        });
                    }
                    ids.lock().unwrap().push(id);
                    if fallback {
                        append_notice(
                            "Project folder unavailable; starting in the bridge workspace root.\n",
                        );
                    }
                    if let Some(notice) = notice {
                        append_process_notice(&notice);
                    }
                }
            };
            pending_spawns.update(|pending| *pending += 1);
            if let Some(host) = project_host.resolve_immediate(project_id) {
                finish(host.ok().and_then(|host| host.cwd()));
                return;
            }
            leptos::task::spawn_local(async move {
                let cwd = project_host
                    .resolve_guarded(project_id, true, move || current())
                    .await
                    .ok()
                    .and_then(|host| host.cwd());
                finish(cwd);
            });
        },
    );
    let spawn_fresh = StoredValue::new_local(move || {
        spawn.with_value(|spawn| spawn(vec!["-i".into()], None, false));
    });
    let ids_for_messages = spawned_ids.clone();
    bridge.with_value(|bridge| {
        bridge.register_terminal(std::rc::Rc::new(move |message| match message {
            BridgeServerMessage::Spawned { id, pid, .. } => {
                remote_fallbacks.update_value(|fallbacks| {
                    fallbacks.remove(&id);
                });
                active_session.set(Some(id));
                last_seq.set(0);
                append_process_notice(&format!("\x1b[90m[Process spawned (PID: {pid})]\x1b[0m\n"));
            }
            BridgeServerMessage::Output { id, seq, data, .. } => {
                if active_session.get_untracked().as_ref() != Some(&id) {
                    return;
                }
                last_seq.set(seq);
                append(&data);
            }
            BridgeServerMessage::Exited {
                id,
                exit_code,
                signal,
            } => {
                ids_for_messages
                    .lock()
                    .unwrap()
                    .retain(|tracked| tracked != &id);
                if active_session.get_untracked().as_ref() == Some(&id) {
                    active_session.set(None);
                }
                let code = match (exit_code, signal) {
                    (Some(code), _) => format!("exit code {code}"),
                    (_, Some(signal)) => format!("signal {signal}"),
                    _ => "unknown status".into(),
                };
                append_process_notice(&format!("\x1b[90m[Process finished with {code}]\x1b[0m\n"));
            }
            BridgeServerMessage::Error { id, message } => {
                let mut fallback = None;
                remote_fallbacks.update_value(|fallbacks| fallback = fallbacks.remove(&id));
                if let Some(mut fallback) = fallback
                    && let BridgeClientMessage::Spawn { cwd, .. } = &mut fallback.message
                    && let Some(path) = cwd.as_ref()
                    // These are the bridge resolver's exact errors, not process-launch failures.
                    && (message == format!("cwd does not exist: {path}")
                        || message == format!("cwd escapes workspace root: {path}"))
                    && (fallback.current)()
                {
                    *cwd = None;
                    if tx_outbound
                        .with_value(|bridge| bridge.send(fallback.message))
                        .is_ok()
                    {
                        append_notice(
                            "Project folder unavailable; starting in the bridge workspace root.\n",
                        );
                        return;
                    }
                }

                if reconnecting.get_untracked()
                    && active_session.get_untracked().as_ref() == Some(&id)
                    && message.contains("session not found")
                {
                    append_process_notice("Terminal ended (bridge restarted)\n");
                    ids_for_messages
                        .lock()
                        .unwrap()
                        .retain(|tracked| tracked != &id);
                    active_session.set(None);
                    last_seq.set(0);
                    reconnecting.set(false);
                    spawn_fresh.with_value(|spawn| spawn());
                } else {
                    if active_session.get_untracked().as_ref() != Some(&id) {
                        ids_for_messages
                            .lock()
                            .unwrap()
                            .retain(|tracked| tracked != &id);
                    }
                    append_notice(&format!("\x1b[31m[Bridge error: {message}]\x1b[0m\n"));
                }
            }
            BridgeServerMessage::Sessions { sessions: active } => {
                let pending = pending_spawn_ids.get_untracked();
                pending_spawn_ids.set(Vec::new());
                ids_for_messages.lock().unwrap().retain(|id| {
                    !pending.contains(id) || active.iter().any(|session| &session.id == id)
                });
                if active_session.get_untracked().is_none()
                    && let Some(session) =
                        active.iter().find(|session| pending.contains(&session.id))
                {
                    output.update_value(
                        crate::terminal_output::TerminalOutput::discard_pending_escape,
                    );
                    active_session.set(Some(session.id.clone()));
                    last_seq.set(0);
                    reconnecting.set(true);
                    tx_outbound.with_value(|bridge| {
                        let _ = bridge.send(BridgeClientMessage::Attach {
                            id: session.id.clone(),
                            last_seq: 0,
                        });
                    });
                }
            }
            _ => {}
        }));
    });
    let ids_for_list = spawned_ids.clone();
    Effect::new(move |previous: Option<bool>| {
        let ready = status.get().terminal_ready();
        if ready && !previous.unwrap_or(false) {
            // A List response cannot confirm absence of spawns sent after that List.
            pending_spawn_ids.set(ids_for_list.lock().unwrap().clone());
            bridge.with_value(|bridge| {
                let _ = bridge.send(BridgeClientMessage::List);
                if let Some(id) = active_session.get_untracked() {
                    reconnecting.set(true);
                    let _ = bridge.send(BridgeClientMessage::Attach {
                        id,
                        last_seq: last_seq.get_untracked(),
                    });
                }
            });
        }
        ready
    });
    let spawned_ids_for_cleanup = spawned_ids.clone();
    on_cleanup(move || {
        bridge.with_value(|bridge| {
            for id in spawned_ids_for_cleanup.lock().unwrap().iter() {
                bridge.kill_shell(id.clone());
            }
            bridge.unregister_terminal();
        });
    });

    let terminal_cmd = expect_context::<crate::state::layout::LayoutState>().terminal_cmd;
    let command_ids = spawned_ids.clone();
    Effect::new(move |_| {
        if terminal_cmd.get().is_none() {
            return;
        }
        let mut command = None;
        terminal_cmd.update(|pending| command = pending.take());
        let Some(cmd) = command else { return };
        if !status.get_untracked().terminal_ready() {
            return;
        }
        if pending_spawns.get_untracked() != 0
            || active_session.get_untracked().is_some()
            || !command_ids.lock().unwrap().is_empty()
        {
            append_notice(
                "Terminal is busy; wait for the current process to finish before running /test again.\n",
            );
            return;
        }
        spawn.with_value(|spawn| {
            spawn(
                vec!["-lc".into(), cmd.clone()],
                Some(format!("\x1b[36m❯ {cmd}\x1b[0m\n")),
                true,
            );
        });
    });
    let spawn_shell = move || spawn_fresh.with_value(|spawn| spawn());

    let tx_for_kill = tx_outbound;
    let kill_current = move || {
        if let Some(id) = active_session.get() {
            let sender = tx_for_kill.get_value();
            let _ = sender.send(BridgeClientMessage::Kill {
                id,
                signal: Some("SIGINT".into()),
            });
        }
    };

    let clear_output = move || {
        output.update_value(crate::terminal_output::TerminalOutput::clear);
        schedule();
    };

    let tx_for_submit = tx_outbound;
    let on_submit_command = move || {
        if !status.get_untracked().terminal_ready() {
            return;
        }
        let cmd = input_text.get().trim().to_string();
        if cmd.is_empty() {
            if let Some(id) = active_session.get() {
                {
                    let sender = tx_for_submit.get_value();
                    let _ = sender.send(BridgeClientMessage::Input {
                        id,
                        data: "\n".into(),
                    });
                }
            } else {
                spawn_shell();
            }
            return;
        }

        // Add to history
        history.update(|h| h.push(cmd.clone()));
        history_index.set(None);
        input_text.set(String::new());

        {
            let sender = tx_for_submit.get_value();
            match active_session.get() {
                Some(id) => {
                    let _ = sender.send(BridgeClientMessage::Input {
                        id,
                        data: format!("{cmd}\n"),
                    });
                }
                None => {
                    spawn.with_value(|spawn| {
                        spawn(
                            vec!["-lc".into(), "--".into(), cmd.clone()],
                            Some(format!("\x1b[36m❯ {cmd}\x1b[0m\n")),
                            false,
                        );
                    });
                }
            }
        }
    };

    let spawn_shell_btn = spawn_shell;
    let kill_current_btn = kill_current;
    let on_submit_btn = on_submit_command;

    let on_keydown = move |ev: web_sys::KeyboardEvent| match ev.key().as_str() {
        "Enter" => {
            ev.prevent_default();
            on_submit_command();
        }
        "c" if ev.ctrl_key() => {
            ev.prevent_default();
            kill_current();
        }
        "l" if ev.ctrl_key() => {
            ev.prevent_default();
            clear_output();
        }
        "ArrowUp" => {
            ev.prevent_default();
            let hist = history.get();
            if hist.is_empty() {
                return;
            }
            let next_idx = match history_index.get() {
                None => hist.len().saturating_sub(1),
                Some(i) => i.saturating_sub(1),
            };
            history_index.set(Some(next_idx));
            if let Some(item) = hist.get(next_idx) {
                input_text.set(item.clone());
            }
        }
        "ArrowDown" => {
            ev.prevent_default();
            let hist = history.get();
            if hist.is_empty() {
                return;
            }
            if let Some(i) = history_index.get() {
                if i + 1 < hist.len() {
                    let next_idx = i + 1;
                    history_index.set(Some(next_idx));
                    input_text.set(hist[next_idx].clone());
                } else {
                    history_index.set(None);
                    input_text.set(String::new());
                }
            }
        }
        _ => {}
    };

    // The keyed list captures its owner; keep that owner separate from shell cleanup.
    let line_owner = Owner::new();
    let completed_lines = line_owner.with(|| {
        view! {
            <For each=move || lines.get() key=|line| line.id children=move |line| {
                view! { <div class="terminal-line" inner_html=line.html.clone() /> }
            } />
        }
    });
    let completed_lines =
        leptos::tachys::reactive_graph::OwnedView::new_with_owner(completed_lines, line_owner);

    view! {
        <div class="terminal-dock">
            <div class="terminal-header">
                <div class="terminal-title">
                    <span class="terminal-glyph">""</span>
                    <span class="terminal-label">"Terminal"</span>
                    <span class=move || match status.get() {
                        BridgeStatus::Ready { .. } | BridgeStatus::Legacy => "term-status online",
                        BridgeStatus::Connecting => "term-status connecting",
                        BridgeStatus::Unavailable | BridgeStatus::Rejected => "term-status offline",
                    }>
                        {move || match status.get() {
                            BridgeStatus::Ready { .. } | BridgeStatus::Legacy => "connected · ws:3001",
                            BridgeStatus::Connecting => "connecting...",
                            BridgeStatus::Unavailable => "bridge offline",
                            BridgeStatus::Rejected => "sign-in rejected",
                        }}
                    </span>

                </div>
                <div class="terminal-actions">
                    <button
                        class="btn ghost sm term-btn"
                        title="New interactive shell"
                        on:click=move |_| spawn_shell_btn()
                    >
                        "+ Shell"
                    </button>
                    <button
                        class="btn ghost sm term-btn"
                        title="Interrupt active process (Ctrl+C)"
                        on:click=move |_| kill_current_btn()
                    >
                        "■ Kill"
                    </button>
                    <button
                        class="btn ghost sm term-btn"
                        title="Clear output (Ctrl+L)"
                        on:click=move |_| clear_output()
                    >
                        "⌫ Clear"
                    </button>
                    <button
                        class="icon-btn term-close-btn"
                        title="Close terminal (Ctrl+`)"
                        on:click=move |_| on_close()
                    >
                        "✕"
                    </button>
                </div>
            </div>

            <div
                class="terminal-output"
                node_ref=output_ref
                on:scroll=move |_| {
                    if let Some(element) = output_ref.get_untracked() {
                        following.set(f64::from(element.scroll_height() - element.client_height()) - element.scroll_top() <= 32.0);
                    }
                }
            >
                {completed_lines}
                <div class="terminal-line terminal-current" inner_html=move || current_html.get() />
            </div>

            <div class="terminal-input-bar">
                <span class="terminal-prompt">"❯"</span>
                <input
                    type="text"
                    class="terminal-input"
                    placeholder="Type command or shell input... (Enter to send, Ctrl+C to interrupt)"
                    prop:value=move || input_text.get()
                    on:input=move |ev| {
                        let target = ev.target().unwrap().unchecked_into::<web_sys::HtmlInputElement>();
                        input_text.set(target.value());
                    }
                    on:keydown=on_keydown
                />
                <button
                    class="btn send sm term-send-btn"
                    title="Send command"
                    on:click=move |_| on_submit_btn()
                >
                    "Send"
                </button>
            </div>
        </div>
    }
}
