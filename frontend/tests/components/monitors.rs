use super::support::{mount_test, settle};
use leptos::prelude::*;
use openwebide_core::{
    WorkspaceMode,
    scheduled::{MonitorCommand, Schedule, ScheduledTask, SessionTarget, TaskDraft},
};
use wasm_bindgen_test::*;
fn entry() -> ScheduledTask {
    ScheduledTask {
        id: 7,
        revision: 1,
        project_id: Some(1),
        draft: TaskDraft {
            title: "Monitor".into(),
            prompt: "Check the build".into(),
            session_id: 1,
            session_target: SessionTarget::Existing,
            auto_title: false,
            model: None,
            schedule: Schedule::Once { at: 600 },
            enabled: true,
        },
        next_run: Some(600),
        host_id: "host".into(),
        host_available: true,
        last_run: None,
    }
}
#[wasm_bindgen_test]
async fn conversation_monitors_have_status_and_cancel_without_saved_tasks_in_both_modes() {
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        let mounted = mount_test(move |state| {
            state.seed_project();
            state.seed_session();
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            state.fake.monitors.borrow_mut().insert(1, vec![entry()]);
            view! {<openwebide_frontend::components::ConversationMonitors/>}
        });
        settle().await;
        mounted.click("button[aria-label=\"View monitors\"]");
        settle().await;
        assert!(
            mounted
                .root
                .text_content()
                .unwrap()
                .contains("Monitor #7: Pending")
        );
        assert!(mounted.state.fake.scheduled.borrow().is_empty());
        mounted.click_text("Cancel future checks");
        settle().await;
        assert!(mounted.state.monitors.entries.get_untracked().is_empty());
        assert!(mounted.state.fake.monitors.borrow()[&1].is_empty());
        assert!(
            mounted
                .root
                .query_selector("button[aria-label=\"View monitors\"]")
                .unwrap()
                .is_none()
        );
    }
}
#[wasm_bindgen_test]
async fn old_monitor_replies_cannot_update_a_new_conversation_or_account() {
    for account_change in [false, true] {
        let (send, receive) = futures::channel::oneshot::channel();
        let mounted = mount_test(move |state| {
            state.seed_project();
            state.seed_session();
            state.fake.monitor_results.borrow_mut().push_back(receive);
            view! {<openwebide_frontend::components::ConversationMonitors/>}
        });
        settle().await;
        if account_change {
            mounted
                .state
                .auth
                .generation
                .update(|generation| *generation += 1);
        } else {
            mounted.state.chat.active_session.set(Some(2));
        }
        settle().await;
        send.send(Ok(vec![entry()])).unwrap();
        settle().await;
        assert!(mounted.state.monitors.entries.get_untracked().is_empty());
        mounted
            .state
            .monitor_actions
            .command
            .run(MonitorCommand::List {});
        settle().await;
        assert!(!mounted.state.monitors.busy.get_untracked());
    }
}

#[wasm_bindgen_test]
async fn monitor_indicator_is_hidden_without_active_monitors() {
    for enabled in [false, true] {
        let mounted = mount_test(move |state| {
            state.seed_project();
            state.seed_session();
            if !enabled {
                let mut monitor = entry();
                monitor.draft.enabled = false;
                state.fake.monitors.borrow_mut().insert(1, vec![monitor]);
            }
            view! {<openwebide_frontend::components::ConversationMonitors/>}
        });
        settle().await;
        assert!(
            mounted
                .root
                .query_selector("button[aria-label=\"View monitors\"]")
                .unwrap()
                .is_none()
        );
    }
}
