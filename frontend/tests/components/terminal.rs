use super::support::{Mounted, mount_test, settle};
use leptos::prelude::*;
use openwebide_core::{BridgeClientMessage, BridgeServerMessage, RunEvent, ToolStreamChunk};
use openwebide_frontend::{
    bridge::{BridgeConfig, BridgeConn, BridgeStatus},
    components::TerminalPane,
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
    let mounted = mount_test(move |_| {
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
        view! { <TerminalPane bridge=bridge on_close=|| () /> }
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
            .contains(&BridgeClientMessage::Kill { id, signal: None })
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
