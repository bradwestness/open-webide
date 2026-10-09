//! Shared control orchestration; model I/O and goal persistence use the backend,
//! while ordinary runs retain the ProjectRuns browser/server adapters.
use crate::{
    backend::Api,
    state::{auth::AuthState, chat::ChatState, projects::ProjectsState},
};
use leptos::{prelude::*, task::spawn_local};
use openwebide_core::GoalCommand;

#[derive(Clone, Copy)]
pub struct ChatControls {
    pub goal: Callback<Option<String>>,
    pub compact: Callback<()>,
    pub refresh: Callback<()>,
}

pub fn install(
    api: Api,
    chat: ChatState,
    projects: ProjectsState,
    start_goal: Callback<String>,
    stop: Callback<()>,
) -> ChatControls {
    let auth = expect_context::<AuthState>();
    let host = expect_context::<crate::project_host::ProjectHost>();
    let generation = StoredValue::new(0u64);
    let refresh = Callback::new(move |()| {
        let Some(session) = chat.active_session.get_untracked() else {
            return;
        };
        let account = auth.generation.get_untracked();
        let project = projects.active_project.get_untracked();
        generation.update_value(|value| *value += 1);
        let ticket = generation.get_value();
        spawn_local(async move {
            let result = api.with_value(Clone::clone).get_goal(session).await;
            if auth.generation.try_get_untracked() != Some(account)
                || generation.try_get_value() != Some(ticket)
                || projects.active_project.try_get_untracked() != Some(project)
                || chat.active_session.try_get_untracked() != Some(Some(session))
            {
                return;
            }
            match result {
                Ok(goal) => {
                    if goal.as_ref().map_or(0, |goal| goal.revision)
                        < chat
                            .goal
                            .with_untracked(|goal| goal.as_ref().map_or(0, |goal| goal.revision))
                    {
                        return;
                    }
                    chat.goal.set(goal);
                    chat.goal_error.set(None);
                }
                Err(error) => chat.goal_error.set(Some(error)),
            }
        });
    });
    Effect::new(move |_| {
        auth.generation.get();
        projects.active_project.get();
        chat.active_session.get();
        generation.update_value(|value| *value += 1);
        chat.goal.set(None);
        chat.goal_error.set(None);
        chat.goal_busy.set(false);
        chat.compacting.set(false);
        chat.compact_epoch.update_value(|epoch| *epoch += 1);
        refresh.run(());
    });
    let poll = set_interval_with_handle(
        move || {
            if !chat.goal_busy.get_untracked()
                && chat.goal.with_untracked(|goal| {
                    goal.as_ref().is_some_and(|goal| {
                        goal.worker && goal.status != openwebide_core::GoalStatus::Completed
                    })
                })
            {
                refresh.run(());
            }
        },
        std::time::Duration::from_secs(5),
    )
    .ok();
    on_cleanup(move || {
        if let Some(poll) = poll {
            poll.clear();
        }
    });
    let goal = Callback::new(move |args: Option<String>| {
        let Some(command) = GoalCommand::parse(args.as_deref().unwrap_or_default()) else {
            chat.notify(chat.goal.with_untracked(|goal| {
                goal.as_ref().map_or_else(
                    || "No goal. Use /goal <objective> to start one.".into(),
                    |goal| format!("Goal ({:?}): {}", goal.status, goal.objective),
                )
            }));
            if !chat.goal_busy.get_untracked() {
                refresh.run(());
            }
            return;
        };
        if chat.goal_busy.get_untracked()
            || chat.compacting.get_untracked()
            || chat.rewinding.get_untracked()
            || chat.branching.get_untracked()
            || chat.loading_history.get_untracked().is_some()
        {
            return;
        }
        if let GoalCommand::Start { objective } = command {
            if chat.streaming.get_untracked() {
                chat.notify("Stop the current run before starting a goal.");
                return;
            }
            if let Err(error) = openwebide_core::Goal::transition(
                chat.goal.get_untracked().as_ref(),
                chat.active_session.get_untracked().unwrap_or(0),
                GoalCommand::Start {
                    objective: objective.clone(),
                },
                0,
            ) {
                chat.goal_error.set(Some(error));
                return;
            }
            start_goal.run(objective);
            return;
        }
        if chat.streaming.get_untracked() && command != GoalCommand::Pause {
            chat.notify("Pause the current run before changing this goal.");
            return;
        }
        let Some(session) = chat.active_session.get_untracked() else {
            return;
        };
        let revision = chat
            .goal
            .with_untracked(|goal| goal.as_ref().map_or(0, |goal| goal.revision));
        let account = auth.generation.get_untracked();
        let project = projects.active_project.get_untracked();
        generation.update_value(|value| *value += 1);
        let ticket = generation.get_value();
        chat.goal_busy.set(true);
        spawn_local(async move {
            let resume = command == GoalCommand::Resume;
            let pause = command == GoalCommand::Pause;
            let current = move || {
                auth.generation.try_get_untracked() == Some(account)
                    && generation.try_get_value() == Some(ticket)
                    && projects.active_project.try_get_untracked() == Some(project)
                    && chat.active_session.try_get_untracked() == Some(Some(session))
            };
            let binding = if resume {
                match host.background_binding(project, current).await {
                    Ok(binding) => binding,
                    Err(error) => {
                        if current() {
                            chat.goal_busy.set(false);
                            chat.goal_error.set(Some(error));
                        }
                        return;
                    }
                }
            } else {
                None
            };
            let result = api
                .with_value(Clone::clone)
                .dispatch_goal(session, revision, &command, binding.as_ref())
                .await;
            if auth.generation.try_get_untracked() != Some(account)
                || generation.try_get_value() != Some(ticket)
                || projects.active_project.try_get_untracked() != Some(project)
                || chat.active_session.try_get_untracked() != Some(Some(session))
            {
                return;
            }
            chat.goal_busy.set(false);
            match result {
                Ok(goal) => {
                    chat.goal.set(Some(goal));
                    chat.goal_error.set(None);
                    if pause && chat.streaming_session.get_untracked() == Some(session) {
                        stop.run(());
                    }
                }
                Err(error) => {
                    chat.goal_error.set(Some(error));
                    refresh.run(());
                }
            }
        });
    });
    let compact = Callback::new(move |()| {
        if chat.streaming.get_untracked()
            || chat.compacting.get_untracked()
            || chat.goal_busy.get_untracked()
            || chat.rewinding.get_untracked()
            || chat.branching.get_untracked()
            || chat.loading_history.get_untracked().is_some()
        {
            chat.notify("Wait for the current operation to finish before compacting.");
            return;
        }
        let Some(session) = chat.active_session.get_untracked() else {
            chat.notify("Start a conversation before compacting.");
            return;
        };
        let account = auth.generation.get_untracked();
        let project = projects.active_project.get_untracked();
        let history = chat.history_gen.get_value();
        let model = chat
            .session_model
            .with_untracked(|models| models.get(&session).cloned().flatten())
            .or_else(|| chat.selected_model.get_untracked());
        chat.compact_epoch.update_value(|epoch| *epoch += 1);
        let epoch = chat.compact_epoch.get_value();
        chat.compacting.set(true);
        spawn_local(async move {
            let result = api
                .with_value(Clone::clone)
                .compact_session(session, model.as_deref())
                .await;
            if auth.generation.try_get_untracked() != Some(account)
                || projects.active_project.try_get_untracked() != Some(project)
                || chat.active_session.try_get_untracked() != Some(Some(session))
                || chat.compact_epoch.try_get_value() != Some(epoch)
            {
                return;
            }
            chat.compacting.set(false);
            if chat.history_gen.try_get_value() != Some(history) {
                return;
            }
            match result {
                Ok(message) => {
                    chat.messages
                        .push(crate::conversation::ConversationItem::Message(message));
                    chat.notify("Conversation context compacted. Original messages are retained.");
                }
                Err(error) => chat.notify(format!("Compaction failed: {error}")),
            }
        });
    });
    let controls = ChatControls {
        goal,
        compact,
        refresh,
    };
    provide_context(controls);
    controls
}
