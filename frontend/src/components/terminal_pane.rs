//! Bottom dock terminal pane powered by WebSocket bridge daemon.
//!
//! Provides an interactive PTY terminal and command execution dock in the IDE.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::bridge::{BridgeConn, BridgeStatus};
use leptos::html::Div;
use leptos::prelude::*;
use openwebide_core::{BridgeClientMessage, BridgeServerMessage, BridgeSessionInfo};
use web_sys::wasm_bindgen::JsCast;

static COUNTER: AtomicU64 = AtomicU64::new(1);

fn next_session_id() -> String {
    let now = js_sys::Date::now() as u64;
    let count = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("term-{now}-{count}")
}

/// Convert basic ANSI escape codes into HTML spans for colorized terminal rendering.
pub fn ansi_to_html(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    let mut in_span = false;

    while let Some(ch) = chars.next() {
        if ch == '\x1b' && chars.peek() == Some(&'[') {
            chars.next(); // consume '['
            let mut code = String::new();
            while let Some(&c) = chars.peek() {
                if c.is_ascii_digit() || c == ';' {
                    code.push(c);
                    chars.next();
                } else {
                    chars.next(); // consume ending char (usually 'm')
                    break;
                }
            }

            if in_span {
                out.push_str("</span>");
                in_span = false;
            }

            match code.as_str() {
                "0" | "" => {}
                "1" => {
                    out.push_str("<span class=\"term-bold\">");
                    in_span = true;
                }
                "30" => {
                    out.push_str("<span class=\"term-black\">");
                    in_span = true;
                }
                "31" => {
                    out.push_str("<span class=\"term-red\">");
                    in_span = true;
                }
                "32" => {
                    out.push_str("<span class=\"term-green\">");
                    in_span = true;
                }
                "33" => {
                    out.push_str("<span class=\"term-yellow\">");
                    in_span = true;
                }
                "34" => {
                    out.push_str("<span class=\"term-blue\">");
                    in_span = true;
                }
                "35" => {
                    out.push_str("<span class=\"term-magenta\">");
                    in_span = true;
                }
                "36" => {
                    out.push_str("<span class=\"term-cyan\">");
                    in_span = true;
                }
                "37" => {
                    out.push_str("<span class=\"term-white\">");
                    in_span = true;
                }
                "90" => {
                    out.push_str("<span class=\"term-bright-black\">");
                    in_span = true;
                }
                "91" => {
                    out.push_str("<span class=\"term-bright-red\">");
                    in_span = true;
                }
                "92" => {
                    out.push_str("<span class=\"term-bright-green\">");
                    in_span = true;
                }
                "93" => {
                    out.push_str("<span class=\"term-bright-yellow\">");
                    in_span = true;
                }
                "94" => {
                    out.push_str("<span class=\"term-bright-blue\">");
                    in_span = true;
                }
                "95" => {
                    out.push_str("<span class=\"term-bright-magenta\">");
                    in_span = true;
                }
                "96" => {
                    out.push_str("<span class=\"term-bright-cyan\">");
                    in_span = true;
                }
                "97" => {
                    out.push_str("<span class=\"term-bright-white\">");
                    in_span = true;
                }
                _ => {}
            }
        } else {
            match ch {
                '&' => out.push_str("&amp;"),
                '<' => out.push_str("&lt;"),
                '>' => out.push_str("&gt;"),
                '"' => out.push_str("&quot;"),
                '\r' => {}
                _ => out.push(ch),
            }
        }
    }

    if in_span {
        out.push_str("</span>");
    }

    out
}

#[component]
pub fn TerminalPane(bridge: BridgeConn, on_close: impl Fn() + Copy + 'static) -> impl IntoView {
    let status = RwSignal::from(bridge.status());
    let bridge = StoredValue::new_local(bridge);
    let raw_output = RwSignal::new(String::new());
    let active_session = RwSignal::new(Option::<String>::None);
    let sessions = RwSignal::new(Vec::<BridgeSessionInfo>::new());
    let input_text = RwSignal::new(String::new());
    let history = RwSignal::new(Vec::<String>::new());
    let history_index = RwSignal::new(Option::<usize>::None);
    let last_seq = RwSignal::new(0u64);

    let output_ref = NodeRef::<Div>::new();

    let tx_outbound = bridge;
    let spawned_ids: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let spawned_ids_for_conn = spawned_ids.clone();
    let reconnecting = RwSignal::new(false);
    let pending_spawn_ids = RwSignal::new(Vec::<String>::new());
    let spawn_fresh = move || {
        if !status.get_untracked().terminal_ready() {
            return;
        }
        let id = next_session_id();
        if bridge
            .with_value(|bridge| {
                bridge.send(BridgeClientMessage::Spawn {
                    id: id.clone(),
                    command: "sh".into(),
                    args: vec!["-i".into()],
                    cwd: None,
                    env: std::collections::HashMap::new(),
                    pty: true,
                    cols: 120,
                    rows: 30,
                })
            })
            .is_ok()
        {
            spawned_ids_for_conn.lock().unwrap().push(id);
        }
    };
    let spawn_fresh = StoredValue::new_local(spawn_fresh);
    let ids_for_messages = spawned_ids.clone();
    bridge.with_value(|bridge| {
        bridge.register_terminal(std::rc::Rc::new(move |message| match message {
            BridgeServerMessage::Spawned { id, pid, .. } => {
                active_session.set(Some(id));
                last_seq.set(0);
                raw_output.update(|text| {
                    text.push_str(&format!("\x1b[90m[Process spawned (PID: {pid})]\x1b[0m\n"))
                });
            }
            BridgeServerMessage::Output { id, seq, data, .. } => {
                if active_session.get_untracked().as_ref() != Some(&id) {
                    return;
                }
                last_seq.set(seq);
                raw_output.update(|text| text.push_str(&data));
                if let Some(Some(el)) = output_ref.try_get_untracked() {
                    let div: &web_sys::HtmlElement = el.as_ref();
                    div.set_scroll_top(div.scroll_height() as f64);
                }
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
                raw_output.update(|text| {
                    text.push_str(&format!("\x1b[90m[Process finished with {code}]\x1b[0m\n"))
                });
            }
            BridgeServerMessage::Error { id, message } => {
                if reconnecting.get_untracked()
                    && active_session.get_untracked().as_ref() == Some(&id)
                    && message.contains("session not found")
                {
                    raw_output.update(|text| text.push_str("Terminal ended (bridge restarted)\n"));
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
                    raw_output.update(|text| {
                        text.push_str(&format!("\x1b[31m[Bridge error: {message}]\x1b[0m\n"))
                    });
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
                sessions.set(active);
            }
            _ => {}
        }))
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
    let projects = expect_context::<crate::state::projects::ProjectsState>();
    let command_ids = spawned_ids.clone();
    let command_pending = RwSignal::new(false);
    let reject_busy_command = move || {
        raw_output.update(|text| {
            text.push_str("Terminal is busy; wait for the current process to finish before running /test again.\n");
        });
    };
    let command_alive = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let alive_for_cleanup = command_alive.clone();
    on_cleanup(move || alive_for_cleanup.store(false, Ordering::Relaxed));
    #[cfg(target_arch = "wasm32")]
    let api = expect_context::<crate::backend::Api>();
    #[cfg(target_arch = "wasm32")]
    let settings = expect_context::<crate::state::settings::SettingsState>();
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
        if command_pending.get_untracked()
            || active_session.get_untracked().is_some()
            || !command_ids.lock().unwrap().is_empty()
        {
            reject_busy_command();
            return;
        }
        command_pending.set(true);
        let project = projects
            .active_project
            .get_untracked()
            .and_then(|id| projects.project(id));
        let ids = command_ids.clone();
        let alive = command_alive.clone();
        let sender = bridge.get_value();
        #[cfg(target_arch = "wasm32")]
        let handle = project.as_ref().and_then(|project| {
            projects
                .local_handles
                .with_untracked(|handles| handles.get(&project.id).cloned())
        });
        #[cfg(target_arch = "wasm32")]
        let config = crate::bridge::BridgeConfig::new(&settings.bridge_url.get_untracked());
        leptos::task::spawn_local(async move {
            let cwd = match project {
                Some(project) if project.mode == openwebide_core::WorkspaceMode::Remote => {
                    project.path
                }
                Some(project) => {
                    #[cfg(target_arch = "wasm32")]
                    {
                        match handle {
                            Some(handle) => {
                                crate::local_agent::resolve_bridge_cwd(
                                    api,
                                    handle,
                                    project.id,
                                    &config,
                                    &crate::bridge::BridgeCredentials::new(api),
                                )
                                .await
                            }
                            None => None,
                        }
                    }
                    #[cfg(not(target_arch = "wasm32"))]
                    {
                        let _ = project;
                        None
                    }
                }
                None => None,
            };
            if !alive.load(Ordering::Relaxed) {
                return;
            }
            command_pending.set(false);
            if active_session.get_untracked().is_some() || !ids.lock().unwrap().is_empty() {
                reject_busy_command();
                return;
            }
            let id = next_session_id();
            if sender
                .send(BridgeClientMessage::Spawn {
                    id: id.clone(),
                    command: "sh".into(),
                    args: vec!["-lc".into(), cmd.clone()],
                    cwd,
                    env: std::collections::HashMap::new(),
                    pty: true,
                    cols: 120,
                    rows: 30,
                })
                .is_ok()
            {
                ids.lock().unwrap().push(id);
                raw_output.update(|text| text.push_str(&format!("\x1b[36m❯ {cmd}\x1b[0m\n")));
            }
        });
    });

    let tx_for_shell = tx_outbound;
    let spawned_ids_for_shell = spawned_ids.clone();
    let spawn_shell = move || {
        if !status.get_untracked().terminal_ready() {
            return;
        }
        let id = next_session_id();
        {
            let sender = tx_for_shell.get_value();
            if let Ok(mut ids) = spawned_ids_for_shell.lock() {
                ids.push(id.clone());
            }
            let _ = sender.send(BridgeClientMessage::Spawn {
                id,
                command: "sh".into(),
                args: vec!["-i".into()],
                cwd: None,
                env: std::collections::HashMap::new(),
                pty: true,
                cols: 120,
                rows: 30,
            });
        }
    };

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
        raw_output.set(String::new());
    };

    let tx_for_submit = tx_outbound;
    let spawned_ids_for_submit = spawned_ids.clone();
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
                let id = next_session_id();
                {
                    let sender = tx_for_submit.get_value();
                    if let Ok(mut ids) = spawned_ids_for_submit.lock() {
                        ids.push(id.clone());
                    }
                    let _ = sender.send(BridgeClientMessage::Spawn {
                        id,
                        command: "sh".into(),
                        args: vec!["-i".into()],
                        cwd: None,
                        env: std::collections::HashMap::new(),
                        pty: true,
                        cols: 120,
                        rows: 30,
                    });
                }
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
                    let id = next_session_id();
                    raw_output.update(|s| {
                        s.push_str(&format!("\x1b[36m❯ {cmd}\x1b[0m\n"));
                    });
                    if let Ok(mut ids) = spawned_ids_for_submit.lock() {
                        ids.push(id.clone());
                    }
                    let _ = sender.send(BridgeClientMessage::Spawn {
                        id,
                        command: "sh".into(),
                        args: vec!["-lc".into(), "--".into(), cmd],
                        cwd: None,
                        env: std::collections::HashMap::new(),
                        pty: true,
                        cols: 120,
                        rows: 30,
                    });
                }
            }
        }
    };

    let spawn_shell_btn = spawn_shell.clone();
    let kill_current_btn = kill_current;
    let on_submit_btn = on_submit_command.clone();

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
                        class="term-btn"
                        title="New interactive shell"
                        on:click=move |_| spawn_shell_btn()
                    >
                        "+ Shell"
                    </button>
                    <button
                        class="term-btn"
                        title="Interrupt active process (Ctrl+C)"
                        on:click=move |_| kill_current_btn()
                    >
                        "■ Kill"
                    </button>
                    <button
                        class="term-btn"
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
                inner_html=move || ansi_to_html(&raw_output.get())
            />

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
                    class="term-send-btn"
                    title="Send command"
                    on:click=move |_| on_submit_btn()
                >
                    "Send"
                </button>
            </div>
        </div>
    }
}
