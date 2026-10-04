//! Shared checklist loading and freshness policy for every project and run transport.
use crate::{
    backend::Api,
    state::{auth::AuthState, chat::ChatState, projects::ProjectsState},
};
use leptos::{prelude::*, task::spawn_local};

#[derive(Clone, Copy)]
pub struct TodoActions {
    pub refresh: Callback<i64>,
}

pub fn install(api: Api, chat: ChatState, projects: ProjectsState) -> TodoActions {
    let auth = expect_context::<AuthState>();
    let generation = StoredValue::new(0u64);
    let refresh = Callback::new(move |session: i64| {
        if chat.active_session.get_untracked() != Some(session) {
            return;
        }
        let account = auth.generation.get_untracked();
        let project = projects.active_project.get_untracked();
        generation.update_value(|value| *value += 1);
        let ticket = generation.get_value();
        let history = chat.history_gen.get_value();
        chat.todo_loading.set(true);
        spawn_local(async move {
            let result = api.with_value(Clone::clone).get_todo_plan(session).await;
            if auth.generation.try_get_untracked() != Some(account)
                || generation.try_get_value() != Some(ticket)
                || projects.active_project.try_get_untracked() != Some(project)
                || chat.active_session.try_get_untracked() != Some(Some(session))
                || chat.history_gen.try_get_value() != Some(history)
            {
                return;
            }
            chat.todo_loading.set(false);
            match result {
                Ok(plan) => {
                    chat.todo_plan.set(plan);
                    chat.todo_error.set(None);
                }
                Err(error) => chat
                    .todo_error
                    .set(Some(format!("Could not load plan: {error}"))),
            }
        });
    });
    Effect::new(move |_| {
        auth.generation.get();
        projects.active_project.get();
        let session = chat.active_session.get();
        generation.update_value(|value| *value += 1);
        chat.todo_plan.set(None);
        chat.todo_error.set(None);
        chat.todo_loading.set(false);
        if let Some(session) = session {
            refresh.run(session);
        }
    });
    Effect::new(move |_| {
        if chat.loading_history.get().is_none()
            && let Some(session) = chat.active_session.get_untracked()
        {
            refresh.run(session);
        }
    });
    let actions = TodoActions { refresh };
    provide_context(actions);
    actions
}
