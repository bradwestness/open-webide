//! Host view facade. Session/account/project transitions invalidate every async result.
use crate::{
    backend::Api,
    state::{auth::AuthState, chat::ChatState, projects::ProjectsState},
};
use leptos::prelude::*;
use openwebide_core::host_admin::{
    HostConnection, HostEnvironment, HostInput, HostOperation, HostRequest, HostResponse,
};
#[derive(Clone, Copy, PartialEq, Eq)]
struct Scope {
    account: u64,
    session: Option<i64>,
    project: Option<i64>,
}
#[derive(Clone, Copy)]
pub struct HostState {
    pub connection: RwSignal<Option<HostConnection>>,
    pub environment: RwSignal<Option<HostEnvironment>>,
    pub operations: RwSignal<Vec<HostOperation>>,
    pub error: RwSignal<Option<String>>,
    pub busy: RwSignal<bool>,
    pub open: RwSignal<bool>,
    generation: RwSignal<u64>,
    api: Api,
    auth: AuthState,
    chat: ChatState,
    projects: ProjectsState,
}
impl HostState {
    pub fn new(api: Api, auth: AuthState, chat: ChatState, projects: ProjectsState) -> Self {
        let state = Self {
            connection: RwSignal::new(None),
            environment: RwSignal::new(None),
            operations: RwSignal::new(Vec::new()),
            error: RwSignal::new(None),
            busy: RwSignal::new(false),
            open: RwSignal::new(false),
            generation: RwSignal::new(0),
            api,
            auth,
            chat,
            projects,
        };
        Effect::new(move |previous: Option<Scope>| {
            let scope = Scope {
                account: auth.generation.get(),
                session: chat.active_session.get(),
                project: projects.active_project.get(),
            };
            if previous.is_some_and(|previous| previous != scope) {
                state.generation.update(|generation| *generation += 1);
                state.environment.set(None);
                state.operations.set(Vec::new());
                state.error.set(None);
                state.busy.set(false);
                state.open.set(false);
            }
            scope
        });
        Effect::new(move || {
            auth.generation.track();
            state.connection.set(None);
        });
        state
    }
    fn scope(self) -> Scope {
        Scope {
            account: self.auth.generation.get_untracked(),
            session: self.chat.active_session.get_untracked(),
            project: self.projects.active_project.get_untracked(),
        }
    }
    fn current(self, scope: Scope, generation: u64) -> bool {
        self.auth.generation.try_get_untracked().is_some()
            && self.scope() == scope
            && self.generation.get_untracked() == generation
    }
    fn begin(self) -> (Scope, u64) {
        self.generation.update(|generation| *generation += 1);
        self.busy.set(true);
        self.error.set(None);
        (self.scope(), self.generation.get_untracked())
    }
    pub fn load_connection(self) {
        let (scope, generation) = self.begin();
        let api = self.api.get_value();
        wasm_bindgen_futures::spawn_local(async move {
            let result = api.host_connection().await;
            if !self.current(scope, generation) {
                return;
            }
            match result {
                Ok(connection) => self.connection.set(Some(connection)),
                Err(error) => self.error.set(Some(error)),
            }
            self.busy.set(false);
        });
    }
    pub fn save_connection(self, connection: HostConnection) {
        let (scope, generation) = self.begin();
        let api = self.api.get_value();
        wasm_bindgen_futures::spawn_local(async move {
            let result = api.save_host_connection(&connection).await;
            if !self.current(scope, generation) {
                return;
            }
            match result {
                Ok(connection) => {
                    self.connection.set(Some(connection));
                    self.environment.set(None);
                }
                Err(error) => self.error.set(Some(error)),
            }
            self.busy.set(false);
        });
    }
    pub fn test_connection(self) {
        let (scope, generation) = self.begin();
        let api = self.api.get_value();
        wasm_bindgen_futures::spawn_local(async move {
            let result = api.probe_host_connection().await;
            if !self.current(scope, generation) {
                return;
            }
            match result {
                Ok(environment) => self.environment.set(Some(environment)),
                Err(error) => self.error.set(Some(error)),
            }
            self.busy.set(false);
        });
    }
    pub fn refresh(self, inspect: bool) {
        let scope = self.scope();
        let Some(session) = scope.session.filter(|_| scope.project.is_none()) else {
            return;
        };
        if self.busy.get_untracked() {
            return;
        }
        let (_, generation) = self.begin();
        let api = self.api.get_value();
        wasm_bindgen_futures::spawn_local(async move {
            if inspect {
                let result = api.host_view(session, &HostRequest::Inspect).await;
                if !self.current(scope, generation) {
                    return;
                }
                match result {
                    Ok(HostResponse::Environment(environment)) => {
                        self.environment.set(Some(environment));
                    }
                    Ok(_) => self
                        .error
                        .set(Some("Unexpected host environment response".into())),
                    Err(error) => self.error.set(Some(error)),
                }
            }
            let result = api.host_view(session, &HostRequest::Operations).await;
            if !self.current(scope, generation) {
                return;
            }
            match result {
                Ok(HostResponse::Operations(operations)) => self.operations.set(operations),
                Ok(_) => self
                    .error
                    .set(Some("Unexpected host operation response".into())),
                Err(error) => self.error.set(Some(error)),
            }
            self.busy.set(false);
        });
    }
    pub fn reply(self, input: HostInput) {
        let scope = self.scope();
        let Some(session) = scope.session.filter(|_| scope.project.is_none()) else {
            return;
        };
        let api = self.api.get_value();
        wasm_bindgen_futures::spawn_local(async move {
            let result = api.host_input(session, &input).await;
            if self.auth.generation.try_get_untracked().is_none() || self.scope() != scope {
                return;
            }
            if let Err(error) = result {
                self.error.set(Some(error));
            }
            self.refresh(false);
        });
    }
    pub fn ask(self, resource: &openwebide_core::host_admin::HostResource) {
        if self.projects.active_project.get_untracked().is_some() {
            return;
        }
        let Some(environment) = self.environment.get_untracked() else {
            return;
        };
        self.chat.draft.update(|draft| {
            if !draft.is_empty() {
                draft.push_str("\n\n");
            }
            draft.push_str(&format!(
                "Inspect this resource on administration host {}:\n{}",
                environment.target,
                serde_json::to_string(resource).unwrap_or_default()
            ));
        });
    }
}
