//! Shared UI facade; async results belong to one account, project and session.
use crate::{
    backend::Api,
    state::{auth::AuthState, chat::ChatState, projects::ProjectsState},
};
use leptos::prelude::*;
use openwebide_core::questions::{AgentQuestion, QuestionCommand, QuestionReply};

#[derive(Clone, Copy, PartialEq, Eq)]
struct Scope {
    account: u64,
    session: Option<i64>,
    project: Option<i64>,
}
#[derive(Clone, Copy)]
pub struct QuestionsState {
    pub questions: RwSignal<Vec<AgentQuestion>>,
    pub busy: RwSignal<bool>,
    pub error: RwSignal<Option<String>>,
    api: Api,
    auth: AuthState,
    chat: ChatState,
    projects: ProjectsState,
    generation: RwSignal<u64>,
}
impl QuestionsState {
    pub fn new(api: Api, auth: AuthState, chat: ChatState, projects: ProjectsState) -> Self {
        let state = Self {
            questions: RwSignal::new(vec![]),
            busy: RwSignal::new(false),
            error: RwSignal::new(None),
            api,
            auth,
            chat,
            projects,
            generation: RwSignal::new(0),
        };
        Effect::new(move |previous: Option<Scope>| {
            let scope = Scope {
                account: auth.generation.get(),
                session: chat.active_session.get(),
                project: projects.active_project.get(),
            };
            if previous.is_some_and(|previous| previous != scope) {
                state.generation.update(|generation| *generation += 1);
                state.questions.set(vec![]);
                state.busy.set(false);
                state.error.set(None);
            }
            chat.sessions.track();
            state.refresh();
            scope
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
    fn bound_session(self, scope: Scope) -> Option<i64> {
        self.chat.sessions.with_untracked(|sessions| {
            sessions
                .iter()
                .find(|session| {
                    Some(session.id) == scope.session && session.project_id == scope.project
                })
                .map(|session| session.id)
        })
    }
    fn current(self, scope: Scope, generation: u64) -> bool {
        self.auth.generation.try_get_untracked().is_some()
            && self.scope() == scope
            && self.bound_session(scope) == scope.session
            && self.generation.get_untracked() == generation
    }
    pub fn refresh(self) {
        let scope = self.scope();
        let Some(session) = self.bound_session(scope) else {
            return;
        };
        if self.busy.get_untracked() {
            return;
        }
        self.busy.set(true);
        self.generation.update(|generation| *generation += 1);
        let generation = self.generation.get_untracked();
        wasm_bindgen_futures::spawn_local(async move {
            let result = self
                .api
                .get_value()
                .question_command(session, &QuestionCommand::List)
                .await;
            if !self.current(scope, generation) {
                return;
            }
            match result {
                Ok(result) => {
                    self.questions.set(result.questions);
                    self.error.set(None);
                }
                Err(error) => self.error.set(Some(error)),
            }
            self.busy.set(false);
        });
    }
    pub fn responder(self, id: String, session: i64) -> Callback<QuestionReply> {
        let scope = self.scope();
        Callback::new(move |reply| {
            if self.auth.generation.try_get_untracked().is_some()
                && self.scope() == scope
                && scope.session == Some(session)
            {
                self.reply(id.clone(), reply);
            }
        })
    }
    fn reply(self, id: String, reply: QuestionReply) {
        let scope = self.scope();
        let Some(session) = self.bound_session(scope) else {
            return;
        };
        self.generation.update(|generation| *generation += 1);
        let generation = self.generation.get_untracked();
        self.busy.set(true);
        self.error.set(None);
        wasm_bindgen_futures::spawn_local(async move {
            let result = self
                .api
                .get_value()
                .question_command(
                    session,
                    &QuestionCommand::Reply {
                        id: id.clone(),
                        reply,
                    },
                )
                .await;
            if !self.current(scope, generation) {
                return;
            }
            match result {
                Ok(_) => {
                    self.questions
                        .update(|questions| questions.retain(|question| question.id != id));
                    // The live executor resumes by reading the same durable reply. An interrupted
                    // browser run uses the existing Resume action with the completed tool result.
                    if !self.chat.streaming.get_untracked()
                        && self.chat.active_run.get_untracked().is_none()
                    {
                        let history = self.api.get_value().list_messages(session).await;
                        if !self.current(scope, generation) {
                            return;
                        }
                        if let Ok(history) = history {
                            let resumable = self.chat.interrupted_run.get_untracked().is_some();
                            self.chat.messages.install_history(
                                crate::state_actions::chat::history_items(history),
                            );
                            self.chat.detect_interrupted_run(resumable);
                        }
                    }
                }
                Err(error) => self.error.set(Some(error)),
            }
            self.busy.set(false);
        });
    }
}
