use leptos::prelude::*;
use openwebide_frontend::{
    conversation::{ConversationItem, next_item_nonce},
    testing::fake_backend::Call,
};
use wasm_bindgen_test::*;

use super::support::{chat_view, mount_test, settle};

fn awaiting(id: &str) -> ConversationItem {
    ConversationItem::ToolStep {
        key: next_item_nonce(),
        id: id.into(),
        name: "write_file".into(),
        summary: "write file".into(),
        result: None,
        awaiting_permission: true,
    }
}

#[wasm_bindgen_test]
async fn permission_shortcuts_target_current_run_and_stop_cancels_prompts() {
    let mounted = mount_test(|state| {
        state.seed_project();
        state.seed_session();
        chat_view(state)
    });
    settle().await;
    let chat = mounted.state.chat;
    chat.streaming.set(true);
    chat.streaming_session.set(Some(1));
    chat.current_run_anchor.set(Some(7));
    chat.messages
        .set(vec![awaiting("a3t1c0"), awaiting("a7t1c0")]);
    settle().await;
    assert!(
        mounted
            .root
            .text_content()
            .unwrap()
            .contains("no longer pending")
    );
    mounted.input("y");
    mounted.key("Enter", "Enter", false);
    settle().await;
    assert!(
        !mounted
            .state
            .fake
            .calls
            .borrow()
            .iter()
            .any(|call| matches!(call, Call::SetPermission { .. }))
    );
    mounted.key("y", "KeyY", true);
    settle().await;
    assert!(
        mounted
            .state
            .fake
            .calls
            .borrow()
            .contains(&Call::SetPermission {
                session: 1,
                id: "a7t1c0".into(),
                approved: true
            })
    );
    chat.messages.update(|items| items.push(awaiting("a7t1c1")));
    settle().await;
    mounted.key("n", "KeyN", true);
    settle().await;
    assert!(
        mounted
            .state
            .fake
            .calls
            .borrow()
            .contains(&Call::SetPermission {
                session: 1,
                id: "a7t1c1".into(),
                approved: false
            })
    );
    mounted.key("y", "KeyY", true);
    settle().await;
    let permissions: Vec<_> = mounted
        .state
        .fake
        .calls
        .borrow()
        .iter()
        .filter(|call| matches!(call, Call::SetPermission { .. }))
        .cloned()
        .collect();
    assert_eq!(permissions.len(), 2);
    chat.messages.update(|items| items.push(awaiting("a7t1c2")));
    settle().await;
    mounted.click(".tui-btn-stop");
    settle().await;
    assert!(chat.messages.get_untracked().iter().any(|item| matches!(item, ConversationItem::ToolStep { id, awaiting_permission: false, result: Some(result), .. } if id == "a7t1c2" && result.summary == "cancelled")));
    assert!(
        mounted
            .state
            .fake
            .calls
            .borrow()
            .contains(&Call::CancelSession { session: 1 })
    );
}
