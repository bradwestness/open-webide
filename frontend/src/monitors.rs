//! Conversation-scoped monitor facade; both modes use the same durable host dispatcher.
use crate::{
    backend::Api,
    state::{auth::AuthState, chat::ChatState, monitors::MonitorsState, projects::ProjectsState},
};
use leptos::{prelude::*, task::spawn_local};
use openwebide_core::scheduled::{MonitorCommand, TaskCommand};
#[derive(Clone, Copy)]
pub struct MonitorActions {
    pub command: Callback<MonitorCommand>,
    pub authorize: Callback<()>,
}
impl MonitorActions {
    pub fn new(
        api: Api,
        state: MonitorsState,
        auth: AuthState,
        projects: ProjectsState,
        chat: ChatState,
    ) -> Self {
        let generation = StoredValue::new(0u64);
        let command = Callback::new(move |command| {
            let Some(session) = chat.active_session.get_untracked() else {
                return;
            };
            if state.busy.get_untracked() {
                return;
            }
            let account = auth.generation.get_untracked();
            let project = projects.active_project.get_untracked();
            let ticket = generation.get_value();
            state.busy.set(true);
            spawn_local(async move {
                let result = api
                    .with_value(Clone::clone)
                    .scheduled_session_command(
                        session,
                        &TaskCommand::Monitor {
                            session_id: 0,
                            command,
                        },
                    )
                    .await;
                if generation.try_get_value() != Some(ticket)
                    || auth.generation.try_get_untracked() != Some(account)
                    || projects.active_project.try_get_untracked() != Some(project)
                    || chat.active_session.try_get_untracked() != Some(Some(session))
                {
                    return;
                }
                state.busy.set(false);
                match result {
                    Ok(entries) => {
                        state.entries.set(entries);
                        state.error.set(None);
                    }
                    Err(error) => state.error.set(Some(error)),
                }
            });
        });
        Effect::new(move |_| {
            auth.generation.get();
            projects.active_project.get();
            chat.active_session.get();
            generation.update_value(|value| *value += 1);
            state.entries.set(Vec::new());
            state.error.set(None);
            state.busy.set(false);
        });
        let host = use_context::<crate::project_host::ProjectHost>();
        let authorize = Callback::new(move |()| {
            let Some(session) = chat.active_session.get_untracked() else {
                return;
            };
            let Some(project) = projects.active_project.get_untracked() else {
                return;
            };
            let Some(host) = host else {
                state.error.set(Some("Project host unavailable".into()));
                return;
            };
            if state.busy.get_untracked() {
                return;
            }
            let account = auth.generation.get_untracked();
            let ticket = generation.get_value();
            let current = move || {
                generation.try_get_value() == Some(ticket)
                    && auth.generation.try_get_untracked() == Some(account)
                    && projects.active_project.try_get_untracked() == Some(Some(project))
                    && chat.active_session.try_get_untracked() == Some(Some(session))
            };
            state.busy.set(true);
            spawn_local(async move {
                let result = async {
                    let binding = host.scheduled_binding(project, current).await?;
                    api.with_value(Clone::clone)
                        .scheduled_command(
                            Some(project),
                            &TaskCommand::Monitor {
                                session_id: session,
                                command: MonitorCommand::List {},
                            },
                            Some(&binding),
                        )
                        .await
                }
                .await;
                if !current() {
                    return;
                }
                state.busy.set(false);
                match result {
                    Ok(entries) => {
                        state.entries.set(entries);
                        state.error.set(None);
                    }
                    Err(error) => state.error.set(Some(error)),
                }
            });
        });
        Self { command, authorize }
    }
}
