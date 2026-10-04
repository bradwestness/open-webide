//! One notification policy for every run transport; the host only exposes browser primitives.
use futures::future::LocalBoxFuture;
use leptos::prelude::*;
use openwebide_core::{ApprovalMode, RunEvent};
use std::{collections::HashSet, rc::Rc};

use crate::state::{auth::AuthState, chat::ChatState, settings::SettingsState, ui::UiState};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotificationPermission {
    Unsupported,
    Default,
    Denied,
    Granted,
}

pub trait NotificationHost {
    fn permission(&self) -> NotificationPermission;
    fn attended(&self) -> bool;
    fn request_permission(&self)
    -> LocalBoxFuture<'static, Result<NotificationPermission, String>>;
    fn show(&self, title: &str, body: &str, tag: &str, open: Rc<dyn Fn()>) -> Result<(), String>;
    fn close(&self);
}

#[derive(Clone, Copy)]
pub struct NotificationContext {
    pub auth: AuthState,
    pub chat: ChatState,
    pub settings: SettingsState,
    pub ui: UiState,
    pub chat_visible: bool,
}

#[derive(Clone, Copy)]
pub struct RunNotifications {
    host: StoredValue<Rc<dyn NotificationHost>, LocalStorage>,
    seen: StoredValue<HashSet<(i64, String)>>,
    account: StoredValue<u64>,
    pub configuring: RwSignal<bool>,
    pub permission: RwSignal<NotificationPermission>,
    pub revision: StoredValue<u64>,
    open: Callback<i64>,
}

impl RunNotifications {
    pub fn new(host: Rc<dyn NotificationHost>, auth: AuthState, open: Callback<i64>) -> Self {
        let notifications = Self {
            permission: RwSignal::new(host.permission()),
            host: StoredValue::new_local(host),
            seen: StoredValue::new(HashSet::new()),
            account: StoredValue::new(auth.generation.get_untracked()),
            configuring: RwSignal::new(false),
            revision: StoredValue::new(0),
            open,
        };
        #[cfg(target_arch = "wasm32")]
        Effect::new(move |_| {
            let account = auth.generation.get();
            if notifications.account.get_value() != account {
                notifications.account.set_value(account);
                notifications.seen.update_value(HashSet::clear);
                notifications.configuring.set(false);
                notifications.revision.update_value(|value| *value += 1);
                notifications.host.with_value(|host| host.close());
            }
        });
        on_cleanup(move || notifications.host.with_value(|host| host.close()));
        notifications
    }

    pub fn from_context() -> Self {
        if let Some(notifications) = use_context::<Self>() {
            return notifications;
        }
        let auth = expect_context::<AuthState>();
        let chat = expect_context::<ChatState>();
        let notifications = Self::new(
            Rc::new(BrowserNotificationHost::default()),
            auth,
            Callback::new(move |session| chat.active_session.set(Some(session))),
        );
        provide_context(notifications);
        notifications
    }

    pub fn refresh_permission(self) {
        self.permission
            .set(self.host.with_value(|host| host.permission()));
    }

    pub fn host(self) -> Rc<dyn NotificationHost> {
        self.host.get_value()
    }

    pub fn event(
        self,
        context: NotificationContext,
        session: i64,
        event: &RunEvent,
        replayed: bool,
    ) {
        let NotificationContext {
            auth,
            chat,
            settings,
            ui,
            chat_visible,
        } = context;
        if replayed {
            return;
        }
        let (key, title, action) = match event {
            RunEvent::Done { message } if message.session_id == session => (
                format!("done:{}", message.id),
                "Run finished",
                "has finished".to_string(),
            ),
            RunEvent::PermissionRequest { id, name, .. } => {
                let mode = chat
                    .approval_mode
                    .with_untracked(|modes| modes.get(&session).copied().unwrap_or_default());
                if mode == ApprovalMode::AlwaysForSession && mode.auto_approves(name) {
                    return;
                }
                (
                    format!("permission:{id}"),
                    "Approval needed",
                    format!("needs approval for {name}"),
                )
            }
            _ => return,
        };
        let generation = auth.generation.get_untracked();
        if self.account.get_value() != generation {
            self.account.set_value(generation);
            self.seen.update_value(HashSet::clear);
            self.host.with_value(|host| host.close());
        }
        let Some(name) = chat.sessions.with_untracked(|sessions| {
            sessions
                .iter()
                .find(|item| item.id == session)
                .map(|item| item.name.clone())
        }) else {
            return;
        };
        let mut fresh = false;
        self.seen
            .update_value(|seen| fresh = seen.insert((session, key.clone())));
        if !fresh || !settings.browser_notifications.get_untracked() {
            return;
        }
        self.refresh_permission();
        if self.permission.get_untracked() != NotificationPermission::Granted {
            return;
        }
        if chat_visible
            && chat.active_session.get_untracked() == Some(session)
            && self.host.with_value(|host| host.attended())
        {
            return;
        }
        let open = Rc::new(move || {
            if auth.generation.try_get_untracked() == Some(generation)
                && chat
                    .sessions
                    .try_with_untracked(|sessions| sessions.iter().any(|item| item.id == session))
                    == Some(true)
            {
                self.open.run(session);
            }
        });
        if let Err(error) = self.host.with_value(|host| {
            host.show(
                title,
                &format!("{name} {action}."),
                &format!("openwebide-{session}-{key}"),
                open,
            )
        }) {
            ui.notify(format!("Browser notification unavailable: {error}"));
        }
    }
}

#[derive(Default)]
pub struct BrowserNotificationHost {
    #[cfg(target_arch = "wasm32")]
    active: std::cell::RefCell<Vec<web_sys::Notification>>,
}

impl NotificationHost for BrowserNotificationHost {
    fn permission(&self) -> NotificationPermission {
        #[cfg(target_arch = "wasm32")]
        {
            use wasm_bindgen::JsValue;
            let Some(window) = web_sys::window() else {
                return NotificationPermission::Unsupported;
            };
            if !window.is_secure_context()
                || !js_sys::Reflect::has(window.as_ref(), &JsValue::from_str("Notification"))
                    .unwrap_or(false)
            {
                return NotificationPermission::Unsupported;
            }
            match web_sys::Notification::permission() {
                web_sys::NotificationPermission::Granted => NotificationPermission::Granted,
                web_sys::NotificationPermission::Denied => NotificationPermission::Denied,
                _ => NotificationPermission::Default,
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            NotificationPermission::Unsupported
        }
    }
    fn attended(&self) -> bool {
        #[cfg(target_arch = "wasm32")]
        {
            web_sys::window()
                .and_then(|window| window.document())
                .is_some_and(|document| !document.hidden() && document.has_focus().unwrap_or(false))
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            true
        }
    }
    fn request_permission(
        &self,
    ) -> LocalBoxFuture<'static, Result<NotificationPermission, String>> {
        // Invoke synchronously while the settings click still carries user activation.
        #[cfg(target_arch = "wasm32")]
        let requested = web_sys::Notification::request_permission();
        Box::pin(async move {
            #[cfg(target_arch = "wasm32")]
            {
                let promise = requested.map_err(|error| format!("{error:?}"))?;
                let permission = wasm_bindgen_futures::JsFuture::from(promise)
                    .await
                    .map_err(|error| format!("{error:?}"))?;
                Ok(match permission.as_string().as_deref() {
                    Some("granted") => NotificationPermission::Granted,
                    Some("denied") => NotificationPermission::Denied,
                    _ => NotificationPermission::Default,
                })
            }
            #[cfg(not(target_arch = "wasm32"))]
            Ok(NotificationPermission::Unsupported)
        })
    }
    fn show(&self, title: &str, body: &str, tag: &str, open: Rc<dyn Fn()>) -> Result<(), String> {
        #[cfg(target_arch = "wasm32")]
        {
            use wasm_bindgen::{JsCast, closure::Closure};
            let options = web_sys::NotificationOptions::new();
            options.set_body(body);
            options.set_tag(tag);
            let notification = web_sys::Notification::new_with_options(title, &options)
                .map_err(|error| format!("{error:?}"))?;
            let clicked = notification.clone();
            let handler = Closure::<dyn FnMut()>::new(move || {
                clicked.close();
                if let Some(window) = web_sys::window() {
                    let _ = window.focus();
                }
                open();
            })
            .into_js_value();
            notification.set_onclick(Some(handler.unchecked_ref()));
            let mut active = self.active.borrow_mut();
            if active.len() >= 32 {
                let oldest = active.remove(0);
                oldest.set_onclick(None);
                oldest.close();
            }
            active.push(notification);
            Ok(())
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = (title, body, tag, open);
            Err("Notifications are unavailable on this platform".into())
        }
    }
    fn close(&self) {
        #[cfg(target_arch = "wasm32")]
        for notification in self.active.borrow_mut().drain(..) {
            notification.set_onclick(None);
            notification.close();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::settings::Theme;
    use openwebide_core::{ChatMessage, ChatSession, Role, WorkspaceMode};
    use std::cell::{Cell, RefCell};

    struct FakeHost {
        permission: Cell<NotificationPermission>,
        attended: Cell<bool>,
        shown: RefCell<Vec<String>>,
        opens: RefCell<Vec<Rc<dyn Fn()>>>,
        failed: Cell<bool>,
        closed: Cell<usize>,
    }
    impl NotificationHost for FakeHost {
        fn permission(&self) -> NotificationPermission {
            self.permission.get()
        }
        fn attended(&self) -> bool {
            self.attended.get()
        }
        fn request_permission(
            &self,
        ) -> LocalBoxFuture<'static, Result<NotificationPermission, String>> {
            let permission = self.permission.get();
            Box::pin(async move { Ok(permission) })
        }
        fn show(&self, title: &str, _: &str, tag: &str, open: Rc<dyn Fn()>) -> Result<(), String> {
            if self.failed.get() {
                return Err("unavailable".into());
            }
            self.shown.borrow_mut().push(format!("{title}:{tag}"));
            self.opens.borrow_mut().push(open);
            Ok(())
        }
        fn close(&self) {
            self.closed.set(self.closed.get() + 1);
        }
    }
    fn done(id: i64) -> RunEvent {
        RunEvent::Done {
            message: ChatMessage {
                id,
                session_id: 1,
                role: Role::Assistant,
                content: "reply".into(),
                tool_calls: None,
                tool_call_id: None,
                usage: None,
                created_at: 0,
            },
        }
    }
    fn permission(id: &str) -> RunEvent {
        RunEvent::PermissionRequest {
            id: id.into(),
            name: "write_file".into(),
            summary: "private arguments".into(),
            diff: None,
            note: None,
        }
    }

    #[test]
    fn notifications_share_the_same_event_contract_for_all_workspaces() {
        for mode in [
            Some(WorkspaceMode::Local),
            Some(WorkspaceMode::Remote),
            None,
        ] {
            Owner::new().with(|| {
                let auth = AuthState::new();
                let chat = ChatState::new();
                let settings = SettingsState::new(Theme::Dark, String::new());
                let ui = UiState::new();
                chat.sessions.set(vec![ChatSession {
                    id: 1,
                    name: "chat".into(),
                    project_id: mode.map(|_| 1),
                    connection_id: None,
                    system_prompt_id: None,
                    user_id: None,
                    created_at: 0,
                }]);
                chat.active_session.set(Some(1));
                let host = Rc::new(FakeHost {
                    permission: Cell::new(NotificationPermission::Granted),
                    attended: Cell::new(false),
                    shown: RefCell::new(vec![]),
                    opens: RefCell::new(vec![]),
                    failed: Cell::new(false),
                    closed: Cell::new(0),
                });
                let opened = RwSignal::new(None);
                let notifications = RunNotifications::new(
                    host.clone(),
                    auth,
                    Callback::new(move |id| opened.set(Some(id))),
                );
                let send = |event: &RunEvent, replayed| {
                    notifications.event(
                        NotificationContext {
                            auth,
                            chat,
                            settings,
                            ui,
                            chat_visible: true,
                        },
                        1,
                        event,
                        replayed,
                    );
                };
                send(&done(1), false);
                assert!(host.shown.borrow().is_empty());
                settings.browser_notifications.set(true);
                send(&done(2), true);
                assert!(host.shown.borrow().is_empty());
                send(&done(3), false);
                send(&done(3), false);
                send(&permission("tool"), false);
                send(&permission("tool"), false);
                assert_eq!(host.shown.borrow().len(), 2);
                host.opens.borrow()[0]();
                assert_eq!(opened.get_untracked(), Some(1));
                host.attended.set(true);
                send(&done(4), false);
                assert_eq!(host.shown.borrow().len(), 2);
                chat.active_session.set(Some(2));
                send(&done(5), false);
                assert_eq!(host.shown.borrow().len(), 3);
                chat.set_approval_mode(1, ApprovalMode::AlwaysForSession);
                send(&permission("auto"), false);
                assert_eq!(host.shown.borrow().len(), 3);
                for (index, permission) in [
                    NotificationPermission::Denied,
                    NotificationPermission::Default,
                    NotificationPermission::Unsupported,
                ]
                .into_iter()
                .enumerate()
                {
                    host.permission.set(permission);
                    send(&done(10 + i64::try_from(index).unwrap()), false);
                }
                assert_eq!(host.shown.borrow().len(), 3);
                host.permission.set(NotificationPermission::Granted);
                host.failed.set(true);
                send(&done(20), false);
                assert!(ui.toast.get_untracked().unwrap().contains("unavailable"));
                assert_eq!(host.shown.borrow().len(), 3);
                opened.set(None);
                auth.generation.update(|value| *value += 1);
                host.opens.borrow()[0]();
                assert_eq!(opened.get_untracked(), None);
                host.failed.set(false);
                send(&done(3), false);
                assert_eq!(host.shown.borrow().len(), 4);
                assert!(host.closed.get() > 0);
                chat.sessions.set(vec![]);
                send(&done(99), false);
                assert_eq!(host.shown.borrow().len(), 4);
            });
        }
    }
}
