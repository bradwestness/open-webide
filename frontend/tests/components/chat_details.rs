use super::support::{chat_view, mount_test, settle, wait_until};
use leptos::prelude::*;
use openwebide_core::{ChatMessage, ConversationEntry, Role, TurnTelemetry, WorkspaceMode};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}
fn dialog() -> Option<web_sys::Element> {
    document().query_selector("[role='dialog']").unwrap()
}
fn close_dialog() {
    dialog()
        .unwrap()
        .query_selector(".modal-footer .btn:not(.ghost)")
        .unwrap()
        .unwrap()
        .unchecked_into::<web_sys::HtmlElement>()
        .click();
}

#[wasm_bindgen_test]
async fn generation_statistics_use_recorded_calls_and_close_on_scope_changes() {
    for mode in [
        Some(WorkspaceMode::Local),
        Some(WorkspaceMode::Remote),
        None,
    ] {
        let mounted = mount_test(move |state| {
            state.seed_connection();
            state.seed_session();
            if let Some(mode) = mode {
                state.seed_project();
                state
                    .projects
                    .projects
                    .update(|projects| projects[0].mode = mode);
            } else {
                state
                    .chat
                    .sessions
                    .update(|sessions| sessions[0].project_id = None);
            }
            let message = |id, completion_tokens, estimated| {
                ConversationEntry::Message(ChatMessage {
                    id,
                    session_id: 1,
                    role: Role::Assistant,
                    content: "Saved reply".into(),
                    created_at: 0,
                    tool_calls: None,
                    tool_call_id: None,
                    usage: Some(TurnTelemetry {
                        prompt_tokens: 700,
                        completion_tokens,
                        eval_duration_ms: 4000,
                        estimated,
                        ..Default::default()
                    }),
                })
            };
            state
                .fake
                .messages
                .borrow_mut()
                .insert(1, vec![message(2, 400, false), message(3, 200, true)]);
            view! { <style>{include_str!("../../styles.css")}</style>{chat_view(state)} }
        });
        wait_until("restored generation speed", || {
            mounted
                .state
                .chat
                .session_telemetry
                .get_untracked()
                .current_speed_tps
                == Some(50.0)
        })
        .await;
        mounted.click("button[aria-label='View generation statistics']");
        wait_until("generation statistics dialog", || {
            document()
                .query_selector(".generation-statistics")
                .unwrap()
                .is_some()
        })
        .await;
        let panel = dialog().unwrap();
        let text = panel.text_content().unwrap();
        for detail in [
            "~50.0 t/s",
            "~700",
            "~200",
            "4.00s",
            "~1400",
            "~600",
            "Call 1",
            "Call 2",
        ] {
            assert!(text.contains(detail), "Missing {detail}: {text}");
        }
        let rows = panel
            .query_selector_all(".generation-chart-track > span")
            .unwrap();
        assert_eq!(rows.length(), 2);
        for (index, width) in [(0, "100%"), (1, "50%")] {
            assert_eq!(
                rows.item(index)
                    .unwrap()
                    .unchecked_into::<web_sys::HtmlElement>()
                    .style()
                    .get_property_value("width")
                    .unwrap(),
                width
            );
        }
        assert!(
            panel
                .query_selector(".context-breakdown-bar")
                .unwrap()
                .is_some()
        );
        let token_bar = panel
            .query_selector(".context-breakdown-bar")
            .unwrap()
            .unwrap()
            .unchecked_into::<web_sys::HtmlElement>();
        assert!(
            token_bar.offset_height() >= 18,
            "The token bar must not shrink in the scrollable dialog"
        );
        close_dialog();
        wait_until("typing after statistics", || {
            document().active_element().is_some_and(|active| {
                active.is_same_node(Some(mounted.element(".composer-input").as_ref()))
            })
        })
        .await;
        mounted.click("button[aria-label='View generation statistics']");
        wait_until("reopened statistics", || dialog().is_some()).await;
        dialog()
            .unwrap()
            .query_selector(".modal-footer .btn.ghost")
            .unwrap()
            .unwrap()
            .unchecked_into::<web_sys::HtmlElement>()
            .click();
        wait_until("context navigation", || {
            document()
                .query_selector(".context-usage")
                .unwrap()
                .is_some()
        })
        .await;
        assert!(
            document()
                .query_selector(".generation-statistics")
                .unwrap()
                .is_none()
        );
        dialog()
            .unwrap()
            .query_selector(".modal-footer .btn.ghost")
            .unwrap()
            .unwrap()
            .unchecked_into::<web_sys::HtmlElement>()
            .click();
        wait_until("statistics navigation", || {
            document()
                .query_selector(".generation-statistics")
                .unwrap()
                .is_some()
        })
        .await;
        mounted.state.chat.active_session.set(None);
        wait_until("session change closes statistics", || dialog().is_none()).await;
        assert!(!mounted.state.ui.generation_open.get_untracked());
        mounted.click("button[aria-label='View generation statistics']");
        wait_until("draft statistics", || dialog().is_some()).await;
        mounted
            .state
            .auth
            .generation
            .update(|generation| *generation += 1);
        wait_until("account change closes statistics", || dialog().is_none()).await;
        if mode.is_some() {
            mounted.click("button[aria-label='View generation statistics']");
            wait_until("project statistics", || dialog().is_some()).await;
            mounted.state.projects.active_project.set(None);
            wait_until("project change closes statistics", || dialog().is_none()).await;
        }
        settle().await;
    }
}

#[wasm_bindgen_test]
async fn empty_statistics_keep_unknown_timings_unknown_and_do_not_start_a_run() {
    let mounted = mount_test(chat_view);
    settle().await;
    mounted.click("button[aria-label='View generation statistics']");
    wait_until("empty statistics", || dialog().is_some()).await;
    let panel = dialog().unwrap();
    let text = panel.text_content().unwrap();
    assert!(text.contains("-- t/s"));
    assert!(text.contains("No recorded generation timings yet."));
    assert!(text.contains('—'));
    assert!(panel.query_selector(".generation-chart").unwrap().is_none());
    assert!(!text.contains("NaN"));
    assert!(!mounted.state.chat.streaming.get_untracked());
    close_dialog();
    wait_until("empty statistics closed", || dialog().is_none()).await;
}
