use std::{any::Any, cell::RefCell, rc::Rc};

use leptos::prelude::*;
use openwebide_core::{ChatSession, Connection, Project, ProviderKind, WorkspaceMode};
use openwebide_frontend::{
    backend::Api,
    components::{ChatPane, ConfirmDialog, Editor},
    state::{
        auth::AuthState,
        chat::ChatState,
        git::GitState,
        layout::LayoutState,
        projects::ProjectsState,
        settings::{SettingsState, Theme},
        ui::UiState,
        workspace::WorkspaceState,
    },
    state_actions::{
        chat::{ChatActionContext, ChatActions},
        workspace::WorkspaceActions,
    },
    testing::fake_backend::FakeBackend,
};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;

#[derive(Clone)]
pub struct TestState {
    pub auth: AuthState,
    pub api: Api,
    pub fake: Rc<FakeBackend>,
    pub chat: ChatState,
    pub projects: ProjectsState,
    pub workspace: WorkspaceState,
    pub settings: SettingsState,
    pub ui: UiState,
    pub git: GitState,
    pub bridge: RwSignal<Option<openwebide_frontend::bridge::BridgeConn>, LocalStorage>,
}

impl TestState {
    fn new() -> Self {
        Self::with_backend(Rc::new(FakeBackend::default()))
    }

    fn with_backend(fake: Rc<FakeBackend>) -> Self {
        let api: Api = StoredValue::new_local(fake.clone());
        let ui = UiState::new();
        let projects = ProjectsState::new();
        let workspace = WorkspaceState::with_active_project(projects.active_project);
        let git = GitState::with_active_project(projects.active_project);
        let chat = ChatState::with_active_session_and_toast(workspace.active_session, ui.toast);
        let settings = SettingsState::new(Theme::Dark, "ws://localhost:3001".into());
        provide_context(api);
        provide_context(ui);
        let auth = AuthState::new();
        provide_context(auth);
        provide_context(LayoutState::with_active_project(projects.active_project));
        provide_context(projects);
        provide_context(workspace);
        provide_context(git);
        provide_context(chat);
        provide_context(settings);
        provide_context(openwebide_frontend::project_git::ProjectGit::new(
            api,
            projects,
            settings,
            expect_context::<AuthState>(),
        ));
        Self {
            auth,
            api,
            fake,
            chat,
            projects,
            workspace,
            settings,
            ui,
            git,
            bridge: RwSignal::new_local(None),
        }
    }

    pub fn seed_project(&self) {
        let project = Project {
            id: 1,
            name: "test".into(),
            mode: WorkspaceMode::Remote,
            path: Some("test".into()),
            user_id: None,
            created_at: 0,
        };
        self.fake.projects.borrow_mut().push(project.clone());
        self.projects.projects.set(vec![project]);
        self.projects.active_project.set(Some(1));
    }

    pub fn seed_connection(&self) {
        let connection = Connection {
            id: 1,
            name: "Ollama".into(),
            kind: ProviderKind::Ollama,
            base_url: "http://localhost:11434".into(),
            model: Some("qwen3:8b".into()),
            enabled: true,
            context_limit: Some(8192),
            tool_stream_unsupported: false,
            tool_stream_revision: 0,
        };
        self.fake.connections.borrow_mut().push(connection.clone());
        self.settings.connections.set(vec![connection]);
        self.settings.default_connection.set(Some(1));
    }

    pub fn seed_session(&self) {
        let session = ChatSession {
            id: 1,
            name: "test".into(),
            connection_id: Some(1),
            system_prompt_id: None,
            project_id: Some(1),
            user_id: None,
            created_at: 0,
        };
        self.fake.sessions.borrow_mut().push(session.clone());
        self.chat.sessions.set(vec![session]);
        self.chat.active_session.set(Some(1));
    }
}

pub struct Mounted {
    pub root: web_sys::HtmlElement,
    pub state: TestState,
    unmount: Option<Box<dyn Any>>,
}

impl Drop for Mounted {
    fn drop(&mut self) {
        self.unmount.take();
        self.root.remove();
    }
}

pub fn mount_test<N: IntoView + 'static>(view: impl FnOnce(TestState) -> N + 'static) -> Mounted {
    mount_backend(None, view)
}

pub fn mount_test_with_backend<N: IntoView + 'static>(
    fake: Rc<FakeBackend>,
    view: impl FnOnce(TestState) -> N + 'static,
) -> Mounted {
    mount_backend(Some(fake), view)
}

fn mount_backend<N: IntoView + 'static>(
    fake: Option<Rc<FakeBackend>>,
    view: impl FnOnce(TestState) -> N + 'static,
) -> Mounted {
    let document = web_sys::window().unwrap().document().unwrap();
    let root: web_sys::HtmlElement = document.create_element("div").unwrap().unchecked_into();
    document.body().unwrap().append_child(&root).unwrap();
    let state = Rc::new(RefCell::new(None));
    let slot = state.clone();
    let handle = leptos::mount::mount_to(root.clone(), move || {
        let state = fake.map_or_else(TestState::new, TestState::with_backend);
        *slot.borrow_mut() = Some(state.clone());
        view(state)
    });
    let state = state.borrow_mut().take().unwrap();
    Mounted {
        root,
        state,
        unmount: Some(Box::new(handle)),
    }
}

pub async fn settle() {
    for _ in 0..50 {
        JsFuture::from(js_sys::Promise::resolve(&JsValue::NULL))
            .await
            .unwrap();
    }
}

pub fn chat_actions(state: TestState) -> ChatActions {
    ChatActions::new(ChatActionContext {
        api: state.api,
        chat: state.chat,
        projects: state.projects,
        workspace: state.workspace,
        settings: state.settings,
        ui: state.ui,
        git: state.git,
        bridge: state.bridge,
        request_open: Callback::new(|_| ()),
        refresh_git: Callback::new(|()| ()),
        on_sync_click: Callback::new(|()| ()),
    })
}

pub fn chat_view(state: TestState) -> impl IntoView {
    let actions = chat_actions(state);
    view! {
        <openwebide_frontend::components::ContextUsage/>
        <ChatPane
            on_select_connection_model=actions.select_connection_model
            on_send=actions.send on_resume_run=actions.resume_run on_stop=actions.stop
            on_permission=actions.permission on_permission_always=actions.permission_always on_slash_command=actions.slash_command on_rewind=actions.rewind />
    }
}

pub fn editor_view(state: TestState) -> impl IntoView {
    let read_only = RwSignal::new(false);
    let actions = WorkspaceActions::new(
        state.api,
        state.projects,
        state.workspace,
        state.ui,
        read_only,
        Callback::new(|()| ()),
    );
    view! {
        <Editor read_only=read_only.into() on_open_lossy=actions.on_open_lossy on_save=actions.on_save on_accept=actions.on_accept on_reject=actions.on_reject />
        <ConfirmDialog />
    }
}

impl Mounted {
    pub fn element(&self, selector: &str) -> web_sys::HtmlElement {
        self.root
            .query_selector(selector)
            .unwrap()
            .unwrap_or_else(|| panic!("missing {selector}"))
            .unchecked_into()
    }

    pub fn click(&self, selector: &str) {
        self.element(selector).click();
    }

    pub fn click_text(&self, text: &str) {
        fn find(parent: &web_sys::Element, text: &str) -> Option<web_sys::HtmlElement> {
            let mut child = parent.first_element_child();
            while let Some(element) = child {
                if (element.tag_name() == "BUTTON" || element.class_list().contains("recent-item"))
                    && element.text_content().unwrap_or_default().contains(text)
                {
                    return Some(element.unchecked_into());
                }
                if let Some(found) = find(&element, text) {
                    return Some(found);
                }
                child = element.next_element_sibling();
            }
            None
        }
        find(&self.root, text)
            .unwrap_or_else(|| panic!("missing control {text}"))
            .click();
    }

    pub fn input(&self, value: &str) {
        let input: web_sys::HtmlTextAreaElement = self.element(".composer-input").unchecked_into();
        input.set_value(value);
        let init = web_sys::EventInit::new();
        init.set_bubbles(true);
        input
            .dispatch_event(&web_sys::Event::new_with_event_init_dict("input", &init).unwrap())
            .unwrap();
    }

    pub fn key(&self, key: &str, code: &str, alt: bool) {
        let init = web_sys::KeyboardEventInit::new();
        init.set_key(key);
        init.set_code(code);
        init.set_alt_key(alt);
        init.set_bubbles(true);
        init.set_cancelable(true);
        self.element(".composer-input")
            .dispatch_event(
                &web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init)
                    .unwrap(),
            )
            .unwrap();
    }
}
