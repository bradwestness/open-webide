use openwebide_core::{ChatMessage, Role, RunEvent};
use wasm_bindgen_test::*;

use super::support::{chat_view, mount_test, settle};

fn message(id: i64, role: Role, content: &str) -> ChatMessage {
    ChatMessage {
        id,
        session_id: 1,
        role,
        content: content.into(),
        created_at: 0,
        tool_calls: None,
        tool_call_id: None,
        usage: None,
    }
}

#[wasm_bindgen_test]
async fn interim_text_renders_above_tool_and_final_reply_below() {
    let mounted = mount_test(|state| {
        state.seed_project();
        state.seed_connection();
        state.seed_session();
        state.fake.scripted_events.borrow_mut().push_back(vec![
            RunEvent::Message {
                message: message(7, Role::User, "check"),
            },
            RunEvent::Delta {
                content: "Checking the file".into(),
            },
            RunEvent::Interim {
                message: message(8, Role::Assistant, "Checking the file"),
            },
            RunEvent::ToolCall {
                id: "a7t1c0".into(),
                name: "read_file".into(),
                summary: "read src/main.rs".into(),
            },
            RunEvent::ToolResult {
                id: "a7t1c0".into(),
                name: "read_file".into(),
                ok: true,
                summary: "read src/main.rs".into(),
                diff: None,
            },
            RunEvent::Delta {
                content: "Everything is ready".into(),
            },
            RunEvent::Done {
                message: message(9, Role::Assistant, "Everything is ready"),
            },
        ]);
        chat_view(state)
    });
    settle().await;
    mounted.input("check");
    mounted.key("Enter", "Enter", false);
    settle().await;
    let text = mounted.root.text_content().unwrap();
    let interim = text.find("Checking the file").unwrap();
    let tool = text.find("read src/main.rs").unwrap();
    let final_reply = text.find("Everything is ready").unwrap();
    assert!(interim < tool && tool < final_reply, "{text}");
    assert_eq!(text.matches("Checking the file").count(), 1);
    assert_eq!(text.matches("Everything is ready").count(), 1);
}
