use super::support::{Mounted, mount_test};
use leptos::prelude::*;
use openwebide_core::{BridgeClientMessage, BridgeServerMessage, RunEvent, ToolStreamChunk};
use openwebide_frontend::{
    bridge::{BridgeConfig, BridgeConn, BridgeStatus},
    components::{TerminalDock, TerminalPane},
    testing::fake_transport::FakeTransport,
    util::sleep_ms,
};
use std::{cell::Cell, rc::Rc};
use wasm_bindgen_test::*;

type TerminalFixture = (
    Mounted,
    Rc<FakeTransport>,
    Rc<Cell<usize>>,
    Rc<std::cell::RefCell<Option<BridgeConn>>>,
);

fn mount_terminal() -> TerminalFixture {
    let transport = Rc::new(FakeTransport::default());
    let credentials = Rc::new(Cell::new(0));
    let slot = Rc::new(std::cell::RefCell::new(None));
    let fake = transport.clone();
    let calls = credentials.clone();
    let connection_slot = slot.clone();
    let mounted = mount_test(move |state| {
        let bridge = BridgeConn::with_transport(
            BridgeConfig::new("ws://test"),
            fake,
            Rc::new(move || {
                let n = calls.get() + 1;
                calls.set(n);
                Box::pin(async move { Ok(format!("token-{n}")) })
            }),
        );
        *connection_slot.borrow_mut() = Some(bridge.clone());
        state.chat.show_terminal.set(true);
        view! { <TerminalDock bridge=bridge visible=state.chat.show_terminal /> }
    });
    (mounted, transport, credentials, slot)
}

fn ready(fake: &FakeTransport) {
    fake.reply(BridgeServerMessage::HelloOk {
        user_id: Some(1),
        protocol: 1,
        runs: false,
    });
}

#[wasm_bindgen_test]
async fn hello_precedes_list_and_unmount_kills_shells() {
    let (mounted, fake, _, slot) = mount_terminal();
    settle().await;
    assert!(matches!(
        fake.sent().as_slice(),
        [BridgeClientMessage::Hello { .. }]
    ));
    ready(&fake);
    settle().await;
    assert!(matches!(
        fake.sent().as_slice(),
        [BridgeClientMessage::Hello { .. }, BridgeClientMessage::List]
    ));
    mounted.click_text("+ Shell");
    settle().await;
    let id = fake
        .sent()
        .into_iter()
        .find_map(|message| match message {
            BridgeClientMessage::Spawn { id, .. } => Some(id),
            _ => None,
        })
        .unwrap();
    drop(mounted);
    assert!(
        fake.sent()
            .contains(&BridgeClientMessage::Kill { id, signal: None }),
        "sent on cleanup: {:?}",
        fake.sent()
    );
    slot.borrow_mut().take().unwrap().close();
}

#[wasm_bindgen_test]
async fn rejected_hello_badge_and_legacy_spawn() {
    let (mounted, fake, _, slot) = mount_terminal();
    settle().await;
    fake.reply(BridgeServerMessage::HelloError {
        message: "invalid token".into(),
    });
    settle().await;
    assert!(
        mounted
            .root
            .text_content()
            .unwrap()
            .contains("sign-in rejected")
    );
    mounted.click_text("+ Shell");
    assert!(
        !fake
            .sent()
            .iter()
            .any(|message| matches!(message, BridgeClientMessage::Spawn { .. }))
    );
    slot.borrow_mut().take().unwrap().close();
    drop(mounted);

    let (mounted, fake, _, slot) = mount_terminal();
    settle().await;
    fake.reply(BridgeServerMessage::Error {
        id: String::new(),
        message: "unknown hello".into(),
    });
    settle().await;
    assert_eq!(
        slot.borrow().as_ref().unwrap().status().get_untracked(),
        BridgeStatus::Legacy
    );
    mounted.click_text("+ Shell");
    assert!(
        fake.sent()
            .iter()
            .any(|message| matches!(message, BridgeClientMessage::Spawn { .. }))
    );
    drop(mounted);
    slot.borrow_mut().take().unwrap().close();
}

#[wasm_bindgen_test]
async fn reconnect_attaches_last_seq_and_restart_opens_fresh_shell() {
    let (mounted, fake, credentials, slot) = mount_terminal();
    settle().await;
    ready(&fake);
    settle().await;
    mounted.click_text("+ Shell");
    let id = fake
        .sent()
        .into_iter()
        .find_map(|message| match message {
            BridgeClientMessage::Spawn { id, .. } => Some(id),
            _ => None,
        })
        .unwrap();
    fake.reply(BridgeServerMessage::Spawned {
        id: id.clone(),
        pid: 1,
        pty: true,
    });
    fake.reply(BridgeServerMessage::Output {
        id: id.clone(),
        seq: 7,
        stream: "pty".into(),
        data: "hi".into(),
    });
    fake.disconnect();
    settle().await;
    sleep_ms(1100).await;
    settle().await;
    assert_eq!(fake.connections(), 2);
    assert_eq!(credentials.get(), 2);
    assert!(
        matches!(fake.sent().last(), Some(BridgeClientMessage::Hello { token }) if token == "token-2")
    );
    ready(&fake);
    settle().await;
    assert!(fake.sent().contains(&BridgeClientMessage::Attach {
        id: id.clone(),
        last_seq: 7
    }));
    fake.reply(BridgeServerMessage::Error {
        id,
        message: "session not found".into(),
    });
    settle().await;
    assert!(
        mounted
            .root
            .text_content()
            .unwrap()
            .contains("Terminal ended (bridge restarted)")
    );
    assert_eq!(
        fake.sent()
            .iter()
            .filter(|message| matches!(message, BridgeClientMessage::Spawn { .. }))
            .count(),
        2
    );
    drop(mounted);
    slot.borrow_mut().take().unwrap().close();
}

#[wasm_bindgen_test]
async fn routes_runs_and_completions_without_terminal_delivery() {
    let (mounted, fake, _, slot) = mount_terminal();
    settle().await;
    ready(&fake);
    let bridge = slot.borrow().as_ref().unwrap().clone();
    let count = Rc::new(Cell::new(0));
    let received = count.clone();
    bridge.register_run(
        "r".into(),
        Rc::new(move |_| received.set(received.get() + 1)),
    );
    let mut chunks = bridge.register_completion("c".into());
    fake.reply(BridgeServerMessage::RunEvent {
        run_id: "r".into(),
        seq: 1,
        event: RunEvent::Cancelled,
    });
    fake.reply(BridgeServerMessage::CompletionChunk {
        id: "c".into(),
        chunk: ToolStreamChunk::Delta("delta".into()),
    });
    fake.reply(BridgeServerMessage::CompletionEnd {
        id: "c".into(),
        error: None,
    });
    assert_eq!(count.get(), 1);
    use futures::StreamExt;
    assert!(matches!(
        chunks.next().await,
        Some(BridgeServerMessage::CompletionChunk { .. })
    ));
    assert!(matches!(
        chunks.next().await,
        Some(BridgeServerMessage::CompletionEnd { .. })
    ));
    assert!(chunks.next().await.is_none());
    drop(mounted);
    bridge.close();
}

#[wasm_bindgen_test]
async fn replacement_after_effect_owner_cleanup_kills_old_shell() {
    let owner = Owner::new();
    let fake = Rc::new(FakeTransport::default());
    let bridge = owner.with(|| {
        BridgeConn::with_transport(
            BridgeConfig::new("ws://old"),
            fake.clone(),
            Rc::new(|| Box::pin(async { Ok("token".into()) })),
        )
    });
    let old = bridge.clone();
    let mounted = mount_test(move |_| view! { <TerminalPane bridge=bridge on_close=|| () /> });
    settle().await;
    ready(&fake);
    settle().await;
    mounted.click_text("+ Shell");
    let id = fake
        .sent()
        .into_iter()
        .find_map(|message| match message {
            BridgeClientMessage::Spawn { id, .. } => Some(id),
            _ => None,
        })
        .unwrap();
    owner.with_cleanup(|| old.close());
    assert_eq!(old.status().get_untracked(), BridgeStatus::Unavailable);
    assert!(old.send(BridgeClientMessage::List).is_err());
    drop(mounted);
    assert_eq!(
        fake.sent()
            .iter()
            .filter(|message| matches!(message,
        BridgeClientMessage::Kill { id: killed, .. } if killed == &id))
            .count(),
        1
    );
}

#[wasm_bindgen_test]
async fn logout_kills_shell_before_terminal_unmount() {
    use openwebide_frontend::state::auth::AuthState;
    let fake = Rc::new(FakeTransport::default());
    let transport = fake.clone();
    let auth_slot = Rc::new(std::cell::RefCell::new(None));
    let slot = auth_slot.clone();
    let mounted = mount_test(move |_| {
        let auth = expect_context::<AuthState>();
        *slot.borrow_mut() = Some(auth);
        let bridge = BridgeConn::with_transport(
            BridgeConfig::new("ws://test"),
            transport,
            Rc::new(|| Box::pin(async { Ok("token".into()) })),
        );
        auth.bridge.set_value(Some(bridge.clone()));
        view! { <TerminalPane bridge=bridge on_close=|| () /> }
    });
    settle().await;
    ready(&fake);
    settle().await;
    mounted.click_text("+ Shell");
    let state = &mounted.state;
    auth_slot.borrow().unwrap().reset_user_state(
        state.projects,
        state.workspace,
        state.git,
        state.chat,
        state.settings,
        state.ui,
    );
    assert!(
        fake.sent()
            .iter()
            .any(|message| matches!(message, BridgeClientMessage::Kill { signal: None, .. }))
    );
    drop(mounted);
    assert_eq!(
        fake.sent()
            .iter()
            .filter(|message| matches!(message, BridgeClientMessage::Kill { .. }))
            .count(),
        1
    );
}

#[wasm_bindgen_test]
async fn unmount_during_reconnect_delivers_pending_kill_after_hello() {
    let (mounted, fake, _, slot) = mount_terminal();
    settle().await;
    ready(&fake);
    settle().await;
    mounted.click_text("+ Shell");
    let id = fake
        .sent()
        .into_iter()
        .find_map(|message| match message {
            BridgeClientMessage::Spawn { id, .. } => Some(id),
            _ => None,
        })
        .unwrap();
    fake.disconnect();
    settle().await;
    drop(mounted);
    let kill = BridgeClientMessage::Kill { id, signal: None };
    assert!(!fake.sent().contains(&kill));
    sleep_ms(1100).await;
    settle().await;
    assert!(matches!(
        fake.sent().last(),
        Some(BridgeClientMessage::Hello { .. })
    ));
    assert!(!fake.sent().contains(&kill));
    ready(&fake);
    settle().await;
    assert_eq!(fake.sent().last(), Some(&kill));
    slot.borrow_mut().take().unwrap().close();
}

#[wasm_bindgen_test]
async fn close_during_reconnect_retains_cleanup_without_consumers() {
    let (mounted, fake, _, slot) = mount_terminal();
    settle().await;
    ready(&fake);
    settle().await;
    mounted.click_text("+ Shell");
    fake.disconnect();
    settle().await;
    slot.borrow_mut().take().unwrap().close();
    drop(mounted);
    sleep_ms(1100).await;
    settle().await;
    assert_eq!(fake.connections(), 2);
    assert!(matches!(
        fake.sent().last(),
        Some(BridgeClientMessage::Hello { .. })
    ));
    ready(&fake);
    settle().await;
    assert!(matches!(
        fake.sent().last(),
        Some(BridgeClientMessage::Kill { signal: None, .. })
    ));
    sleep_ms(1100).await;
    settle().await;
    assert_eq!(fake.connections(), 2);
}

fn mount_test_command() -> TerminalFixture {
    let fake = Rc::new(FakeTransport::default());
    let transport = fake.clone();
    let slot = Rc::new(std::cell::RefCell::new(None));
    let connection_slot = slot.clone();
    let mounted = mount_test(move |state| {
        state.seed_project();
        let bridge = BridgeConn::with_transport(
            BridgeConfig::new("ws://test"),
            transport,
            Rc::new(|| Box::pin(async { Ok("token".into()) })),
        );
        *connection_slot.borrow_mut() = Some(bridge.clone());
        state.bridge.set(Some(bridge.clone()));
        let bridge = StoredValue::new_local(bridge);
        view! {
            {super::support::chat_view(state.clone())}
            <TerminalDock bridge=bridge.get_value() visible=state.chat.show_terminal />
        }
    });
    (mounted, fake, Rc::new(Cell::new(0)), slot)
}

#[wasm_bindgen_test]
async fn test_slash_command_opens_terminal_and_spawns_in_project() {
    let (mounted, fake, _, slot) = mount_test_command();
    settle().await;
    ready(&fake);
    settle().await;
    mounted.input("/test parser");
    mounted.key("Enter", "Enter", false);
    settle().await;
    assert!(mounted.state.chat.show_terminal.get_untracked());
    assert!(fake.sent().iter().any(|message| matches!(message,
        BridgeClientMessage::Spawn { command, args, cwd, .. }
        if command == "sh" && args == &["-lc", "cargo test -- 'parser'"]
            && cwd.as_deref() == Some("test")
    )));
    drop(mounted);
    slot.borrow_mut().take().unwrap().close();
}

#[wasm_bindgen_test]
async fn test_slash_command_with_unavailable_bridge_renders_notice() {
    let (mounted, fake, _, slot) = mount_test_command();
    settle().await;
    slot.borrow().as_ref().unwrap().close();
    settle().await;
    mounted.input("/test parser");
    mounted.key("Enter", "Enter", false);
    settle().await;
    assert!(!mounted.state.chat.show_terminal.get_untracked());
    assert!(
        mounted.root.text_content().unwrap().contains(
            "The terminal bridge isn't connected; start openwebide-bridge and try again."
        )
    );
    assert!(
        !fake
            .sent()
            .iter()
            .any(|message| matches!(message, BridgeClientMessage::Spawn { .. }))
    );
    drop(mounted);
    slot.borrow_mut().take().unwrap().close();
}

#[wasm_bindgen_test]
async fn repeated_test_preserves_output_and_interrupt_until_exit() {
    let (mounted, fake, _, slot) = mount_test_command();
    settle().await;
    ready(&fake);
    settle().await;
    mounted.input("/test slow");
    mounted.key("Enter", "Enter", false);
    settle().await;
    let first_id = fake
        .sent()
        .into_iter()
        .find_map(|message| match message {
            BridgeClientMessage::Spawn { id, .. } => Some(id),
            _ => None,
        })
        .unwrap();

    fake.reply(BridgeServerMessage::Sessions {
        sessions: Vec::new(),
    });
    mounted.input("/test before-ack");
    mounted.key("Enter", "Enter", false);
    settle().await;
    fake.reply(BridgeServerMessage::Spawned {
        id: first_id.clone(),
        pid: 52,
        pty: true,
    });
    mounted.input("/test parser");
    mounted.key("Enter", "Enter", false);
    settle().await;
    assert_eq!(
        fake.sent()
            .iter()
            .filter(|message| matches!(message, BridgeClientMessage::Spawn { .. }))
            .count(),
        1
    );
    assert!(
        mounted
            .root
            .text_content()
            .unwrap()
            .contains("Terminal is busy")
    );
    fake.reply(BridgeServerMessage::Output {
        id: first_id.clone(),
        seq: 1,
        data: "slow test failed".into(),
        stream: "stdout".into(),
    });
    settle().await;
    assert!(
        mounted
            .root
            .text_content()
            .unwrap()
            .contains("slow test failed")
    );
    mounted.click_text("Kill");
    assert!(fake.sent().contains(&BridgeClientMessage::Kill {
        id: first_id.clone(),
        signal: Some("SIGINT".into()),
    }));
    fake.reply(BridgeServerMessage::Exited {
        id: first_id,
        exit_code: Some(1),
        signal: None,
    });
    settle().await;
    assert!(mounted.root.text_content().unwrap().contains("exit code 1"));
    mounted.input("/test parser");
    mounted.key("Enter", "Enter", false);
    settle().await;
    assert_eq!(
        fake.sent()
            .iter()
            .filter(|message| matches!(message, BridgeClientMessage::Spawn { .. }))
            .count(),
        2
    );
    let second_id = fake
        .sent()
        .into_iter()
        .rev()
        .find_map(|message| match message {
            BridgeClientMessage::Spawn { id, .. } => Some(id),
            _ => None,
        })
        .unwrap();
    fake.reply(BridgeServerMessage::Error {
        id: second_id,
        message: "failed to spawn".into(),
    });
    mounted.input("/test retry");
    mounted.key("Enter", "Enter", false);
    settle().await;
    assert_eq!(
        fake.sent()
            .iter()
            .filter(|message| matches!(message, BridgeClientMessage::Spawn { .. }))
            .count(),
        3
    );
    drop(mounted);
    slot.borrow_mut().take().unwrap().close();
}

#[wasm_bindgen_test]
async fn reconnect_reconciles_test_spawn_without_acknowledgement() {
    for survives in [false, true] {
        let (mounted, fake, _, slot) = mount_test_command();
        settle().await;
        ready(&fake);
        settle().await;
        mounted.input("/test parser");
        mounted.key("Enter", "Enter", false);
        settle().await;
        let id = fake
            .sent()
            .into_iter()
            .find_map(|message| match message {
                BridgeClientMessage::Spawn { id, .. } => Some(id),
                _ => None,
            })
            .unwrap();
        fake.disconnect();
        settle().await;
        sleep_ms(1100).await;
        settle().await;
        assert_eq!(fake.connections(), 2);
        ready(&fake);
        settle().await;
        assert_eq!(fake.sent().last(), Some(&BridgeClientMessage::List));
        fake.reply(BridgeServerMessage::Sessions {
            sessions: if survives {
                vec![openwebide_core::BridgeSessionInfo {
                    id: id.clone(),
                    command: "sh".into(),
                    running: true,
                    pty: true,
                    started_at: 0,
                }]
            } else {
                Vec::new()
            },
        });
        settle().await;
        if survives {
            assert_eq!(
                fake.sent().last(),
                Some(&BridgeClientMessage::Attach {
                    id: id.clone(),
                    last_seq: 0,
                })
            );
            fake.reply(BridgeServerMessage::Output {
                id: id.clone(),
                seq: 1,
                stream: "pty".into(),
                data: "recovered test output".into(),
            });
            mounted.input("/test busy");
            mounted.key("Enter", "Enter", false);
            settle().await;
            assert_eq!(
                fake.sent()
                    .iter()
                    .filter(|message| matches!(message, BridgeClientMessage::Spawn { .. }))
                    .count(),
                1
            );
            assert!(
                mounted
                    .root
                    .text_content()
                    .unwrap()
                    .contains("Terminal is busy")
            );
            assert!(
                mounted
                    .root
                    .text_content()
                    .unwrap()
                    .contains("recovered test output")
            );
            mounted.click_text("Kill");
            assert_eq!(
                fake.sent().last(),
                Some(&BridgeClientMessage::Kill {
                    id: id.clone(),
                    signal: Some("SIGINT".into()),
                })
            );
            fake.reply(BridgeServerMessage::Exited {
                id,
                exit_code: Some(0),
                signal: None,
            });
            settle().await;
        }
        mounted.input("/test retry");
        mounted.key("Enter", "Enter", false);
        settle().await;
        assert_eq!(
            fake.sent()
                .iter()
                .filter(|message| matches!(message, BridgeClientMessage::Spawn { .. }))
                .count(),
            2
        );
        drop(mounted);
        slot.borrow_mut().take().unwrap().close();
    }
}

#[wasm_bindgen_test]
async fn incremental_lines_batch_clear_scroll_and_cleanup() {
    use wasm_bindgen::JsCast;
    let (mounted, fake, _, slot) = mount_terminal();
    settle().await;
    ready(&fake);
    fake.reply(BridgeServerMessage::Spawned {
        id: "lines".into(),
        pid: 1,
        pty: true,
    });
    settle().await;
    mounted.click_text("Clear");
    settle().await;
    let output = mounted
        .root
        .query_selector(".terminal-output")
        .unwrap()
        .unwrap()
        .unchecked_into::<web_sys::HtmlElement>();
    output
        .set_attribute(
            "style",
            "height:80px;overflow:auto;white-space:pre-wrap;line-height:18px",
        )
        .unwrap();
    let style = web_sys::window()
        .unwrap()
        .document()
        .unwrap()
        .create_element("style")
        .unwrap();
    style.set_text_content(Some(".terminal-line{min-height:18px}"));
    mounted.root.append_child(&style).unwrap();
    let send = |seq, data: &str| {
        fake.reply(BridgeServerMessage::Output {
            id: "lines".into(),
            seq,
            stream: "pty".into(),
            data: data.into(),
        });
    };
    send(1, "\x1b[1;31mold\n\x1b[0mcurrent");
    settle().await;
    let old = output.query_selector(".terminal-line").unwrap().unwrap();
    let current = output.query_selector(".terminal-current").unwrap().unwrap();
    assert_eq!(old.text_content().unwrap(), "old");
    assert!(old.inner_html().contains("term-bold term-red"));
    send(2, "-one");
    send(3, "-two");
    super::support::settle().await;
    assert_eq!(current.text_content().unwrap(), "current");
    settle().await;
    assert_eq!(current.text_content().unwrap(), "current-one-two");
    send(4, "\nnext\n");
    settle().await;
    assert!(
        old.is_same_node(Some(
            output
                .query_selector(".terminal-line")
                .unwrap()
                .unwrap()
                .as_ref()
        ))
    );
    send(5, &"long output\n".repeat(100));
    settle().await;
    assert!(
        f64::from(output.scroll_height() - output.client_height()) - output.scroll_top() <= 1.0
    );
    output.set_scroll_top(0.0);
    output
        .dispatch_event(&web_sys::Event::new("scroll").unwrap())
        .unwrap();
    send(6, "while reading\n");
    settle().await;
    assert!(output.scroll_top().abs() < 1.0);
    output.set_scroll_top(f64::from(output.scroll_height()));
    output
        .dispatch_event(&web_sys::Event::new("scroll").unwrap())
        .unwrap();
    send(7, "following again\n");
    settle().await;
    assert!(
        f64::from(output.scroll_height() - output.client_height()) - output.scroll_top() <= 1.0
    );
    send(8, "\x1b[31mred\x1b[");
    mounted.click_text("Clear");
    send(9, "safe<>&");
    settle().await;
    assert_eq!(output.text_content().unwrap(), "safe<>&");
    assert!(!output.inner_html().contains("term-red"));
    send(10, &"retained\n".repeat(10_050));
    settle().await;
    assert_eq!(output.child_element_count(), 10_000);
    send(11, "pending\n");
    let html = output.inner_html();
    drop(mounted);
    settle().await;
    assert_eq!(output.inner_html(), html);
    slot.borrow_mut().take().unwrap().close();
}

#[wasm_bindgen_test]
async fn unfinished_controls_do_not_hide_notices_or_next_process_output() {
    let (mounted, fake, _, slot) = mount_terminal();
    settle().await;
    ready(&fake);
    fake.reply(BridgeServerMessage::Spawned {
        id: "old".into(),
        pid: 1,
        pty: true,
    });
    fake.reply(BridgeServerMessage::Output {
        id: "old".into(),
        seq: 1,
        stream: "pty".into(),
        data: "visible\n\x1b]0;title".into(),
    });
    fake.reply(BridgeServerMessage::Exited {
        id: "old".into(),
        exit_code: Some(0),
        signal: None,
    });
    settle().await;
    let output = mounted
        .root
        .query_selector(".terminal-output")
        .unwrap()
        .unwrap();
    assert!(output.text_content().unwrap().contains("visible"));
    assert!(
        output
            .text_content()
            .unwrap()
            .contains("Process finished with exit code 0")
    );
    fake.reply(BridgeServerMessage::Spawned {
        id: "next".into(),
        pid: 2,
        pty: true,
    });
    fake.reply(BridgeServerMessage::Output {
        id: "next".into(),
        seq: 1,
        stream: "pty".into(),
        data: "next shell output\n\x1b]split title".into(),
    });
    fake.reply(BridgeServerMessage::Output {
        id: "next".into(),
        seq: 2,
        stream: "pty".into(),
        data: " hidden\x07after split control\n\x1b]unfinished".into(),
    });
    settle().await;
    let text = output.text_content().unwrap();
    assert!(text.contains("Process spawned (PID: 2)"));
    assert!(text.contains("next shell output"));
    assert!(text.contains("after split control"));
    assert!(!text.contains("hidden"));
    fake.reply(BridgeServerMessage::Spawned {
        id: "replacement".into(),
        pid: 3,
        pty: true,
    });
    fake.reply(BridgeServerMessage::Output {
        id: "replacement".into(),
        seq: 1,
        stream: "pty".into(),
        data: "replacement output\n\x1b[".into(),
    });
    fake.reply(BridgeServerMessage::Error {
        id: "replacement".into(),
        message: "local notice".into(),
    });
    settle().await;
    let text = output.text_content().unwrap();
    assert!(text.contains("Process spawned (PID: 3)"));
    assert!(text.contains("replacement output"));
    assert!(text.contains("Bridge error: local notice"));
    assert!(text.contains("visible"));
    drop(mounted);
    slot.borrow_mut().take().unwrap().close();
}

#[wasm_bindgen_test]
async fn busy_and_recoverable_error_notices_preserve_running_process_controls() {
    let (mounted, fake, _, slot) = mount_test_command();
    settle().await;
    ready(&fake);
    mounted.state.chat.show_terminal.set(true);
    settle().await;
    fake.reply(BridgeServerMessage::Spawned {
        id: "running".into(),
        pid: 1,
        pty: true,
    });
    fake.reply(BridgeServerMessage::Output {
        id: "running".into(),
        seq: 1,
        stream: "pty".into(),
        data: "visible\n\x1b]0;first half".into(),
    });
    mounted.input("/test parser");
    mounted.key("Enter", "Enter", false);
    settle().await;
    let output = mounted
        .root
        .query_selector(".terminal-output")
        .unwrap()
        .unwrap();
    assert!(output.text_content().unwrap().contains("Terminal is busy"));
    fake.reply(BridgeServerMessage::Error {
        id: "running".into(),
        message: "recoverable error".into(),
    });
    settle().await;
    assert!(
        output
            .text_content()
            .unwrap()
            .contains("Bridge error: recoverable error")
    );
    fake.reply(BridgeServerMessage::Output {
        id: "running".into(),
        seq: 2,
        stream: "pty".into(),
        data: " second half\x07after title\n".into(),
    });
    settle().await;
    let text = output.text_content().unwrap();
    assert!(text.contains("visible"));
    assert!(text.contains("after title"));
    assert!(!text.contains("first half"));
    assert!(!text.contains("second half"));
    drop(mounted);
    slot.borrow_mut().take().unwrap().close();
}

#[wasm_bindgen_test]
async fn visibility_retains_shell_output_and_hidden_reconnect_sequence() {
    let (mounted, fake, _, slot) = mount_test_command();
    settle().await;
    ready(&fake);
    settle().await;
    assert!(
        mounted
            .root
            .query_selector(".terminal-dock")
            .unwrap()
            .is_none()
    );
    mounted.state.chat.show_terminal.set(true);
    settle().await;
    assert!(
        !fake
            .sent()
            .iter()
            .any(|message| matches!(message, BridgeClientMessage::Spawn { .. }))
    );
    mounted.click_text("+ Shell");
    let id = fake
        .sent()
        .into_iter()
        .find_map(|message| match message {
            BridgeClientMessage::Spawn { id, cwd, .. } => {
                assert_eq!(cwd.as_deref(), Some("test"));
                Some(id)
            }
            _ => None,
        })
        .unwrap();
    fake.reply(BridgeServerMessage::Spawned {
        id: id.clone(),
        pid: 1,
        pty: true,
    });
    let dock = mounted
        .root
        .query_selector(".terminal-dock")
        .unwrap()
        .unwrap();
    mounted.click_text("✕");
    settle().await;
    assert!(
        mounted
            .root
            .query_selector(".terminal-visibility.hidden")
            .unwrap()
            .is_some()
    );
    fake.reply(BridgeServerMessage::Output {
        id: id.clone(),
        seq: 7,
        stream: "pty".into(),
        data: "hidden output".into(),
    });
    settle().await;
    assert!(dock.text_content().unwrap().contains("hidden output"));
    fake.disconnect();
    sleep_ms(1100).await;
    ready(&fake);
    settle().await;
    assert!(fake.sent().contains(&BridgeClientMessage::Attach {
        id: id.clone(),
        last_seq: 7
    }));
    mounted.input("/test busy");
    mounted.key("Enter", "Enter", false);
    settle().await;
    assert!(dock.text_content().unwrap().contains("Terminal is busy"));
    assert!(
        dock.is_same_node(Some(
            mounted
                .root
                .query_selector(".terminal-dock")
                .unwrap()
                .unwrap()
                .as_ref()
        ))
    );
    assert_eq!(
        fake.sent()
            .iter()
            .filter(|message| matches!(message, BridgeClientMessage::Spawn { .. }))
            .count(),
        1
    );
    assert!(
        !fake
            .sent()
            .iter()
            .any(|message| matches!(message, BridgeClientMessage::Kill { .. }))
    );
    drop(mounted);
    assert!(
        fake.sent()
            .contains(&BridgeClientMessage::Kill { id, signal: None })
    );
    slot.borrow_mut().take().unwrap().close();
}

#[wasm_bindgen_test]
async fn every_remote_spawn_uses_request_project_and_existing_shell_stays_put() {
    use wasm_bindgen::JsCast;
    let (mounted, fake, _, slot) = mount_terminal();
    mounted.state.seed_project();
    settle().await;
    ready(&fake);
    settle().await;
    let submit = |text: &str| {
        let input = mounted
            .root
            .query_selector(".terminal-input")
            .unwrap()
            .unwrap()
            .unchecked_into::<web_sys::HtmlInputElement>();
        input.set_value(text);
        input
            .dispatch_event(&web_sys::Event::new("input").unwrap())
            .unwrap();
        mounted.click_text("Send");
    };
    submit("");
    let id = fake
        .sent()
        .into_iter()
        .find_map(|message| match message {
            BridgeClientMessage::Spawn { id, cwd, .. } => {
                assert_eq!(cwd.as_deref(), Some("test"));
                Some(id)
            }
            _ => None,
        })
        .unwrap();
    fake.reply(BridgeServerMessage::Spawned {
        id: id.clone(),
        pid: 1,
        pty: true,
    });
    mounted
        .state
        .projects
        .projects
        .update(|projects| projects[0].path = Some("other".into()));
    submit("pwd");
    assert!(fake.sent().contains(&BridgeClientMessage::Input {
        id: id.clone(),
        data: "pwd\n".into()
    }));
    fake.disconnect();
    sleep_ms(1100).await;
    ready(&fake);
    settle().await;
    fake.reply(BridgeServerMessage::Error {
        id,
        message: "session not found".into(),
    });
    assert!(
        matches!(fake.sent().last(), Some(BridgeClientMessage::Spawn { cwd, args, .. }) if cwd.as_deref() == Some("other") && args == &["-i"])
    );
    submit("echo typed");
    assert!(
        matches!(fake.sent().last(), Some(BridgeClientMessage::Spawn { cwd, args, .. }) if cwd.as_deref() == Some("other") && args == &["-lc", "--", "echo typed"])
    );
    mounted
        .state
        .projects
        .projects
        .update(|projects| projects[0].path = Some(String::new()));
    mounted.click_text("+ Shell");
    assert!(
        matches!(fake.sent().last(), Some(BridgeClientMessage::Spawn { cwd, .. }) if cwd.as_deref() == Some("."))
    );
    mounted.state.projects.active_project.set(None);
    mounted.click_text("+ Shell");
    assert!(matches!(
        fake.sent().last(),
        Some(BridgeClientMessage::Spawn { cwd: None, .. })
    ));
    settle().await;
    assert!(
        mounted
            .root
            .text_content()
            .unwrap()
            .contains("starting in the bridge workspace root")
    );
    drop(mounted);
    slot.borrow_mut().take().unwrap().close();
}

#[wasm_bindgen_test]
async fn remote_cwd_failure_retries_original_spawn_at_root_once() {
    use wasm_bindgen::JsCast;
    for action in ["shell", "empty", "typed", "test", "escape"] {
        let (mounted, fake, _, slot) = if action == "test" {
            mount_test_command()
        } else {
            mount_terminal()
        };
        mounted.state.seed_project();
        settle().await;
        ready(&fake);
        settle().await;
        match action {
            "shell" | "escape" => mounted.click_text("+ Shell"),
            "test" => {
                mounted.input("/test");
                mounted.key("Enter", "Enter", false);
            }
            _ => {
                let input = mounted
                    .root
                    .query_selector(".terminal-input")
                    .unwrap()
                    .unwrap()
                    .unchecked_into::<web_sys::HtmlInputElement>();
                input.set_value(if action == "typed" { "echo typed" } else { "" });
                input
                    .dispatch_event(&web_sys::Event::new("input").unwrap())
                    .unwrap();
                mounted.click_text("Send");
            }
        }
        settle().await;
        let mut expected = fake
            .sent()
            .into_iter()
            .find(|message| matches!(message, BridgeClientMessage::Spawn { .. }))
            .unwrap();
        let BridgeClientMessage::Spawn { id, cwd, .. } = &mut expected else {
            unreachable!()
        };
        let id = id.clone();
        assert_eq!(cwd.as_deref(), Some("test"));
        *cwd = None;
        mounted
            .state
            .projects
            .projects
            .update(|projects| projects[0].path = Some("other".into()));
        fake.reply(BridgeServerMessage::Error {
            id: id.clone(),
            message: if action == "escape" {
                "cwd escapes workspace root: test"
            } else {
                "cwd does not exist: test"
            }
            .into(),
        });
        assert_eq!(fake.sent().last(), Some(&expected));
        settle().await;
        assert!(
            mounted
                .root
                .text_content()
                .unwrap()
                .contains("Project folder unavailable; starting in the bridge workspace root.")
        );
        let count = fake.sent().len();
        fake.reply(BridgeServerMessage::Error {
            id,
            message: "cwd does not exist: test".into(),
        });
        assert_eq!(fake.sent().len(), count, "root failure must not retry");
        drop(mounted);
        slot.borrow_mut().take().unwrap().close();
    }
}

#[wasm_bindgen_test]
async fn remote_cwd_fallback_ignores_unrelated_errors_and_stale_requests() {
    for case in ["launch", "wrong-path", "account", "connection", "spawned"] {
        let fake = Rc::new(FakeTransport::default());
        let transport = fake.clone();
        let slot = Rc::new(std::cell::RefCell::new(None::<BridgeConn>));
        let connection_slot = slot.clone();
        let auth_slot = Rc::new(Cell::new(None));
        let auth_capture = auth_slot.clone();
        let mounted = mount_test(move |state| {
            auth_capture.set(Some(expect_context::<
                openwebide_frontend::state::auth::AuthState,
            >()));
            let bridge = BridgeConn::with_transport(
                BridgeConfig::new("ws://test"),
                transport,
                Rc::new(|| Box::pin(async { Ok("token".into()) })),
            );
            *connection_slot.borrow_mut() = Some(bridge.clone());
            state.chat.show_terminal.set(true);
            view! { <TerminalDock bridge=bridge visible=state.chat.show_terminal /> }
        });
        mounted.state.seed_project();
        settle().await;
        ready(&fake);
        settle().await;
        mounted.click_text("+ Shell");
        let id = fake
            .sent()
            .into_iter()
            .find_map(|message| match message {
                BridgeClientMessage::Spawn { id, .. } => Some(id),
                _ => None,
            })
            .unwrap();
        match case {
            "account" => auth_slot
                .get()
                .unwrap()
                .generation
                .update(|generation| *generation += 1),
            "connection" => mounted.state.settings.bridge_url.set("ws://other".into()),
            "spawned" => fake.reply(BridgeServerMessage::Spawned {
                id: id.clone(),
                pid: 1,
                pty: true,
            }),
            _ => {}
        }
        let count = fake.sent().len();
        fake.reply(BridgeServerMessage::Error {
            id,
            message: match case {
                "launch" => "failed to spawn process: cwd does not exist: test",
                "wrong-path" => "cwd does not exist: unrelated",
                _ => "cwd does not exist: test",
            }
            .into(),
        });
        assert_eq!(fake.sent().len(), count, "{case}");
        settle().await;
        assert!(
            !mounted
                .root
                .text_content()
                .unwrap()
                .contains("starting in the bridge workspace root")
        );
        drop(mounted);
        slot.borrow_mut().take().unwrap().close();
    }
}

async fn settle() {
    super::support::settle().await;
    let frame = js_sys::Promise::new(&mut |resolve, _| {
        web_sys::window()
            .unwrap()
            .request_animation_frame(&resolve)
            .unwrap();
    });
    wasm_bindgen_futures::JsFuture::from(frame).await.unwrap();
    super::support::settle().await;
}
