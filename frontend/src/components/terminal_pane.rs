//! Bottom dock terminal pane powered by WebSocket bridge daemon.
//!
//! Provides an interactive PTY terminal and command execution dock in the IDE.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use futures::{SinkExt, StreamExt};
use gloo_net::websocket::Message;
use gloo_net::websocket::futures::WebSocket;
use leptos::html::Div;
use leptos::prelude::*;
use leptos::task::spawn_local;
use openwebide_core::{BridgeClientMessage, BridgeServerMessage, BridgeSessionInfo};
use web_sys::wasm_bindgen::JsCast;

static COUNTER: AtomicU64 = AtomicU64::new(1);

fn next_session_id() -> String {
    let now = js_sys::Date::now() as u64;
    let count = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("term-{now}-{count}")
}

fn default_bridge_ws_url() -> String {
    if let Some(window) = web_sys::window()
        && let Ok(loc) = window.location().hostname()
        && !loc.is_empty()
    {
        return format!("ws://{loc}:3001");
    }
    "ws://127.0.0.1:3001".to_string()
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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ConnectionStatus {
    Connecting,
    Connected,
    Disconnected,
}

#[component]
pub fn TerminalPane(on_close: impl Fn() + Copy + 'static) -> impl IntoView {
    let status = RwSignal::new(ConnectionStatus::Connecting);
    let raw_output = RwSignal::new(String::new());
    let active_session = RwSignal::new(Option::<String>::None);
    let sessions = RwSignal::new(Vec::<BridgeSessionInfo>::new());
    let input_text = RwSignal::new(String::new());
    let history = RwSignal::new(Vec::<String>::new());
    let history_index = RwSignal::new(Option::<usize>::None);
    let last_seq = RwSignal::new(0u64);

    let output_ref = NodeRef::<Div>::new();

    // Outbound channel from UI to the WebSocket writer task
    let (tx_outbound, mut rx_outbound) = futures::channel::mpsc::channel::<BridgeClientMessage>(32);
    let tx_outbound = Arc::new(std::sync::Mutex::new(tx_outbound));

    // Connect to bridge daemon over WebSocket
    let ws_url = default_bridge_ws_url();
    let tx_clone = tx_outbound.clone();

    let url = ws_url.clone();
    let tx = tx_clone.clone();

    spawn_local(async move {
        status.set(ConnectionStatus::Connecting);
        raw_output.update(|s| s.push_str("\x1b[90mConnecting to bridge daemon...\x1b[0m\n"));

        let ws = match WebSocket::open(&url) {
            Ok(w) => w,
            Err(e) => {
                status.set(ConnectionStatus::Disconnected);
                raw_output.update(|s| {
                        s.push_str(&format!(
                            "\x1b[31mFailed to connect to bridge at {url}: {e}\n\
                             Start the daemon with `openwebide-bridge` to enable the terminal.\x1b[0m\n"
                        ));
                    });
                return;
            }
        };

        status.set(ConnectionStatus::Connected);
        raw_output.update(|s| {
            s.push_str(&format!(
                "\x1b[32m✔ Connected to bridge daemon ({url})\x1b[0m\n\
                     \x1b[90mType a shell command or press Enter to spawn a shell.\x1b[0m\n"
            ));
        });

        let (mut ws_write, mut ws_read) = ws.split();

        // Task 1: Outbound message sender
        spawn_local(async move {
            while let Some(client_msg) = rx_outbound.next().await {
                if let Ok(json_str) = serde_json::to_string(&client_msg)
                    && ws_write.send(Message::Text(json_str)).await.is_err()
                {
                    break;
                }
            }
        });

        // Automatically request active sessions
        if let Ok(mut sender) = tx.lock() {
            let _ = sender.try_send(BridgeClientMessage::List);
        }

        // Task 2: Inbound message reader
        while let Some(msg_result) = ws_read.next().await {
            let text = match msg_result {
                Ok(Message::Text(t)) => t,
                Ok(Message::Bytes(b)) => String::from_utf8_lossy(&b).to_string(),
                _ => continue,
            };

            let server_msg: BridgeServerMessage = match serde_json::from_str(&text) {
                Ok(m) => m,
                Err(_) => continue,
            };

            match server_msg {
                BridgeServerMessage::Spawned { id, pid, .. } => {
                    active_session.set(Some(id.clone()));
                    raw_output.update(|s| {
                        s.push_str(&format!("\x1b[90m[Process spawned (PID: {pid})]\x1b[0m\n"));
                    });
                }
                BridgeServerMessage::Output { seq, data, .. } => {
                    last_seq.set(seq);
                    raw_output.update(|s| s.push_str(&data));

                    // Auto-scroll output container
                    if let Some(el) = output_ref.get() {
                        let div: &web_sys::HtmlElement = el.as_ref();
                        div.set_scroll_top(div.scroll_height() as f64);
                    }
                }
                BridgeServerMessage::Exited {
                    exit_code, signal, ..
                } => {
                    let code_str = match (exit_code, signal) {
                        (Some(c), _) => format!("exit code {c}"),
                        (None, Some(sig)) => format!("signal {sig}"),
                        (None, None) => "unknown status".to_string(),
                    };
                    raw_output.update(|s| {
                        s.push_str(&format!(
                            "\x1b[90m[Process finished with {code_str}]\x1b[0m\n"
                        ));
                    });
                }
                BridgeServerMessage::Error { message, .. } => {
                    raw_output.update(|s| {
                        s.push_str(&format!("\x1b[31m[Bridge error: {message}]\x1b[0m\n"));
                    });
                }
                BridgeServerMessage::Sessions { sessions: active } => {
                    sessions.set(active);
                }
            }
        }

        status.set(ConnectionStatus::Disconnected);
        raw_output.update(|s| {
            s.push_str("\x1b[33m[Bridge daemon disconnected]\x1b[0m\n");
        });
    });

    let tx_for_shell = tx_outbound.clone();
    let spawn_shell = move || {
        let id = next_session_id();
        if let Ok(mut sender) = tx_for_shell.lock() {
            let _ = sender.try_send(BridgeClientMessage::Spawn {
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

    let tx_for_kill = tx_outbound.clone();
    let kill_current = move || {
        if let Some(id) = active_session.get()
            && let Ok(mut sender) = tx_for_kill.lock()
        {
            let _ = sender.try_send(BridgeClientMessage::Kill {
                id,
                signal: Some("SIGINT".into()),
            });
        }
    };

    let clear_output = move || {
        raw_output.set(String::new());
    };

    let tx_for_submit = tx_outbound.clone();
    let on_submit_command = move || {
        let cmd = input_text.get().trim().to_string();
        if cmd.is_empty() {
            if let Some(id) = active_session.get() {
                if let Ok(mut sender) = tx_for_submit.lock() {
                    let _ = sender.try_send(BridgeClientMessage::Input {
                        id,
                        data: "\n".into(),
                    });
                }
            } else {
                let id = next_session_id();
                if let Ok(mut sender) = tx_for_submit.lock() {
                    let _ = sender.try_send(BridgeClientMessage::Spawn {
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

        if let Ok(mut sender) = tx_for_submit.lock() {
            match active_session.get() {
                Some(id) => {
                    let _ = sender.try_send(BridgeClientMessage::Input {
                        id,
                        data: format!("{cmd}\n"),
                    });
                }
                None => {
                    let id = next_session_id();
                    raw_output.update(|s| {
                        s.push_str(&format!("\x1b[36m❯ {cmd}\x1b[0m\n"));
                    });
                    let _ = sender.try_send(BridgeClientMessage::Spawn {
                        id,
                        command: cmd,
                        args: Vec::new(),
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
    let kill_current_btn = kill_current.clone();
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
                        ConnectionStatus::Connected => "term-status online",
                        ConnectionStatus::Connecting => "term-status connecting",
                        ConnectionStatus::Disconnected => "term-status offline",
                    }>
                        {move || match status.get() {
                            ConnectionStatus::Connected => "connected · ws:3001",
                            ConnectionStatus::Connecting => "connecting...",
                            ConnectionStatus::Disconnected => "bridge offline",
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
