use super::support::{chat_view, mount_test, settle};
use futures::future::LocalBoxFuture;
use leptos::prelude::*;
use openwebide_core::{ChatCompletion, ChatResponse, StopReason, WorkspaceMode};
use openwebide_frontend::{
    components::Settings,
    notifications::{NotificationHost, NotificationPermission, RunNotifications},
    state_actions::settings::{SettingsActionContext, build_settings_actions},
    util::sleep_ms,
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

type PermissionReply = futures::channel::oneshot::Receiver<Result<NotificationPermission, String>>;
struct Host {
    permission: Rc<Cell<NotificationPermission>>,
    requests: Cell<usize>,
    pending: RefCell<Option<PermissionReply>>,
    shown: RefCell<Vec<(String, String)>>,
}
impl Host {
    fn new(permission: NotificationPermission) -> Rc<Self> {
        Rc::new(Self {
            permission: Rc::new(Cell::new(permission)),
            requests: Cell::new(0),
            pending: RefCell::new(None),
            shown: RefCell::new(vec![]),
        })
    }
}
impl NotificationHost for Host {
    fn permission(&self) -> NotificationPermission {
        self.permission.get()
    }
    fn attended(&self) -> bool {
        false
    }
    fn request_permission(
        &self,
    ) -> LocalBoxFuture<'static, Result<NotificationPermission, String>> {
        self.requests.set(self.requests.get() + 1);
        let pending = self.pending.borrow_mut().take();
        let permission = self.permission.clone();
        Box::pin(async move {
            let result = match pending {
                Some(reply) => reply.await.map_err(|error| error.to_string())?,
                None => Ok(NotificationPermission::Granted),
            };
            if let Ok(value) = result {
                permission.set(value);
            }
            result
        })
    }
    fn show(&self, title: &str, body: &str, _: &str, _: Rc<dyn Fn()>) -> Result<(), String> {
        self.shown.borrow_mut().push((title.into(), body.into()));
        Ok(())
    }
    fn close(&self) {}
}

#[wasm_bindgen_test]
async fn notification_setting_requests_permission_only_on_click_and_persists_opt_in() {
    for permission in [
        NotificationPermission::Default,
        NotificationPermission::Denied,
        NotificationPermission::Unsupported,
    ] {
        let host = Host::new(permission);
        let installed = host.clone();
        let mounted = mount_test(move |state| {
            let auth = expect_context();
            provide_context(RunNotifications::new(
                installed,
                auth,
                Callback::new(|_| ()),
            ));
            let actions = build_settings_actions(SettingsActionContext {
                api: state.api,
                settings: state.settings,
                ui: state.ui,
            });
            view! { <Settings on_set_theme=actions.on_set_theme on_set_notifications=actions.on_set_notifications on_set_default_prompt=actions.on_set_default_prompt on_set_bridge_url=actions.on_set_bridge_url /> }
        });
        settle().await;
        assert_eq!(host.requests.get(), 0);
        mounted.click(".notification-toggle");
        sleep_ms(5).await;
        settle().await;
        if permission == NotificationPermission::Default {
            assert_eq!(host.requests.get(), 1);
            assert!(mounted.state.settings.browser_notifications.get_untracked());
            assert_eq!(
                mounted
                    .state
                    .fake
                    .settings
                    .borrow()
                    .get("browser_notifications")
                    .map(String::as_str),
                Some("true")
            );
            mounted.click(".notification-toggle");
            sleep_ms(5).await;
            settle().await;
            assert!(!mounted.state.settings.browser_notifications.get_untracked());
            assert_eq!(
                mounted
                    .state
                    .fake
                    .settings
                    .borrow()
                    .get("browser_notifications")
                    .map(String::as_str),
                Some("false")
            );
        } else {
            assert_eq!(host.requests.get(), 0);
            assert!(!mounted.state.settings.browser_notifications.get_untracked());
            assert!(
                !mounted
                    .state
                    .fake
                    .settings
                    .borrow()
                    .contains_key("browser_notifications")
            );
            assert!(mounted.state.ui.toast.get_untracked().is_some());
        }
    }
}

#[wasm_bindgen_test]
async fn old_permission_response_cannot_enable_or_save_notifications_for_a_new_account() {
    let host = Host::new(NotificationPermission::Default);
    let (sender, receiver) = futures::channel::oneshot::channel();
    *host.pending.borrow_mut() = Some(receiver);
    let mounted = mount_test(move |state| {
        provide_context(RunNotifications::new(
            host,
            expect_context(),
            Callback::new(|_| ()),
        ));
        let actions = build_settings_actions(SettingsActionContext {
            api: state.api,
            settings: state.settings,
            ui: state.ui,
        });
        view! { <Settings on_set_theme=actions.on_set_theme on_set_notifications=actions.on_set_notifications on_set_default_prompt=actions.on_set_default_prompt on_set_bridge_url=actions.on_set_bridge_url /> }
    });
    settle().await;
    mounted.click(".notification-toggle");
    settle().await;
    mounted.state.auth.generation.update(|value| *value += 1);
    settle().await;
    sender.send(Ok(NotificationPermission::Granted)).unwrap();
    settle().await;
    assert!(!mounted.state.settings.browser_notifications.get_untracked());
    assert!(
        !mounted
            .state
            .fake
            .settings
            .borrow()
            .contains_key("browser_notifications")
    );
}

#[wasm_bindgen_test]
async fn actual_local_remote_and_projectless_run_completions_use_the_shared_notification_facade() {
    for mode in [
        Some(WorkspaceMode::Local),
        Some(WorkspaceMode::Remote),
        None,
    ] {
        let host = Host::new(NotificationPermission::Granted);
        let installed = host.clone();
        let mounted = mount_test(move |state| {
            if let Some(mode) = mode {
                state.seed_project();
                state
                    .projects
                    .projects
                    .update(|projects| projects[0].mode = mode);
                if mode == WorkspaceMode::Local {
                    state.projects.local_handles.update(|handles| {
                        handles.insert(1, super::local_bridge::probe_folder().unchecked_into());
                    });
                }
            }
            state.seed_connection();
            state.seed_session();
            if mode.is_none() {
                state
                    .chat
                    .sessions
                    .update(|sessions| sessions[0].project_id = None);
                state.fake.sessions.borrow_mut()[0].project_id = None;
            }
            state.settings.browser_notifications.set(true);
            if mode != Some(WorkspaceMode::Local) {
                state.fake.scripted_events.borrow_mut().push_back(vec![
                    openwebide_core::RunEvent::Done {
                        message: openwebide_core::ChatMessage {
                            id: 5,
                            session_id: 1,
                            role: openwebide_core::Role::Assistant,
                            content: "reply".into(),
                            created_at: 0,
                            tool_calls: None,
                            tool_call_id: None,
                            usage: None,
                        },
                    },
                ]);
            }
            state
                .fake
                .scripted_completions
                .borrow_mut()
                .push_back(ChatCompletion {
                    response: ChatResponse::Text("reply".into()),
                    preamble: String::new(),
                    reasoning: String::new(),
                    stop_reason: StopReason::Complete,
                    usage: None,
                });
            provide_context(RunNotifications::new(
                installed,
                expect_context(),
                Callback::new(|_| ()),
            ));
            chat_view(state)
        });
        settle().await;
        mounted.input("notify when complete");
        mounted.key("Enter", "Enter", false);
        for _ in 0..200 {
            sleep_ms(5).await;
            settle().await;
            if !mounted.state.chat.streaming.get_untracked() {
                break;
            }
        }
        assert!(!mounted.state.chat.streaming.get_untracked());
        assert_eq!(
            host.shown.borrow().len(),
            1,
            "{mode:?}: {:?}",
            mounted.state.chat.error.get_untracked()
        );
        assert_eq!(
            host.shown.borrow()[0].0,
            if mode.is_some() {
                "Run finished · test"
            } else {
                "Run finished"
            }
        );
    }
}

#[wasm_bindgen::prelude::wasm_bindgen(inline_js = r#"
let originalNotification;
let notices;
export function installNotificationMock() {
  originalNotification = window.Notification;
  notices = [];
  window.Notification = class {
    static permission = 'default';
    static requestPermission() { this.permission = 'granted'; return Promise.resolve('granted'); }
    constructor(title, options) {
      if (title === 'fail') throw new Error('constructor unavailable');
      this.title = title; this.options = options; this.closed = false;
      notices.push(this);
    }
    close() { this.closed = true; }
  };
}
export function notificationMockCount() { return notices.length; }
export function clickNotificationMock() { notices[0].onclick(); }
export function notificationMockClosed() { return notices.every(n => n.closed && n.onclick == null); }
export function restoreNotificationMock() { window.Notification = originalNotification; notices = []; }
"#)]
extern "C" {
    #[wasm_bindgen::prelude::wasm_bindgen(js_name = installNotificationMock)]
    fn install_mock();
    #[wasm_bindgen::prelude::wasm_bindgen(js_name = notificationMockCount)]
    fn mock_count() -> usize;
    #[wasm_bindgen::prelude::wasm_bindgen(js_name = clickNotificationMock)]
    fn mock_click();
    #[wasm_bindgen::prelude::wasm_bindgen(js_name = notificationMockClosed)]
    fn mock_closed() -> bool;
    #[wasm_bindgen::prelude::wasm_bindgen(js_name = restoreNotificationMock)]
    fn restore_mock();
}

#[wasm_bindgen_test]
async fn browser_notification_adapter_requests_shows_clicks_closes_and_reports_failure() {
    use openwebide_frontend::notifications::BrowserNotificationHost;
    install_mock();
    struct Restore;
    impl Drop for Restore {
        fn drop(&mut self) {
            restore_mock();
        }
    }
    let _restore = Restore;
    let host = BrowserNotificationHost::default();
    assert_eq!(host.permission(), NotificationPermission::Default);
    assert_eq!(
        host.request_permission().await.unwrap(),
        NotificationPermission::Granted
    );
    assert_eq!(host.permission(), NotificationPermission::Granted);
    let clicked = Rc::new(Cell::new(false));
    let open = clicked.clone();
    host.show(
        "Run finished",
        "chat has finished",
        "tag",
        Rc::new(move || open.set(true)),
    )
    .unwrap();
    assert_eq!(mock_count(), 1);
    mock_click();
    assert!(clicked.get());
    assert!(host.show("fail", "body", "tag", Rc::new(|| {})).is_err());
    host.close();
    assert!(mock_closed());
}

#[wasm_bindgen_test]
async fn background_push_avoids_duplicate_alerts_in_every_workspace() {
    for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
        let host = Host::new(NotificationPermission::Granted);
        let installed = host.clone();
        let mounted = mount_test(move |state| {
            state.seed_project();
            state.seed_session();
            state
                .projects
                .projects
                .update(|projects| projects[0].mode = mode);
            state.settings.browser_notifications.set(true);
            let notifications = RunNotifications::new(installed, state.auth, Callback::new(|_| ()));
            notifications.push_ready.set(true);
            notifications.event(
                openwebide_frontend::notifications::NotificationContext {
                    auth: state.auth,
                    chat: state.chat,
                    settings: state.settings,
                    ui: state.ui,
                    chat_visible: false,
                },
                1,
                &openwebide_core::RunEvent::Done {
                    message: openwebide_core::ChatMessage {
                        id: 5,
                        session_id: 1,
                        role: openwebide_core::Role::Assistant,
                        content: "done".into(),
                        created_at: 1,
                        tool_calls: None,
                        tool_call_id: None,
                        usage: None,
                    },
                },
                false,
            );
            view! { <div /> }
        });
        settle().await;
        assert_eq!(host.shown.borrow().len(), 0);
        drop(mounted);
    }
}
