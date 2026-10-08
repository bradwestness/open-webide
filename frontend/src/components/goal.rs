use crate::{
    state::{auth::AuthState, chat::ChatState, projects::ProjectsState},
    state_actions::chat_controls::ChatControls,
};
use leptos::prelude::*;
use openwebide_core::{Goal, GoalStatus};

fn label(goal: &Goal) -> String {
    match goal.status {
        GoalStatus::Active => "Goal active".into(),
        GoalStatus::Paused => "Goal paused".into(),
        GoalStatus::Completed => goal.completed_duration_seconds().map_or_else(
            || "Goal complete".into(),
            |seconds| {
                let duration = if seconds >= 3600 {
                    format!("{}h{}m", seconds / 3600, seconds % 3600 / 60)
                } else if seconds >= 60 {
                    format!("{}m", seconds / 60)
                } else {
                    format!("{seconds}s")
                };
                format!("Goal complete ({duration})")
            },
        ),
    }
}

#[component]
pub fn GoalStatusIndicator() -> impl IntoView {
    let chat = expect_context::<ChatState>();
    let auth = expect_context::<AuthState>();
    let projects = expect_context::<ProjectsState>();
    let open = RwSignal::new(false);
    Effect::new(move |_| {
        auth.generation.get();
        projects.active_project.get();
        chat.active_session.get();
        open.set(false);
    });
    view! {
        <Show when=move || chat.goal.with(|goal| goal.as_ref().is_some_and(|goal| Some(goal.session_id) == chat.active_session.get()))>
            <super::dropdown::Dropdown class="tui-goal-status" trigger_class="btn sm ghost tui-goal-label" menu_class="tui-goal-menu" menu_role="dialog" aria_label="View goal" open=open above=true hide_caret=true label=move || view! { <span>{move || chat.goal.with(|goal| goal.as_ref().map(label))}</span> }>
                <GoalPanel />
            </super::dropdown::Dropdown>
        </Show>
    }
}

#[component]
fn GoalPanel() -> impl IntoView {
    let chat = expect_context::<ChatState>();
    let controls = use_context::<ChatControls>();
    let running =
        move || chat.streaming.get() && chat.streaming_session.get() == chat.active_session.get();
    view! {
        <section class="tui-goal" aria-label="Session goal">
            <strong>{move || chat.goal.with(|goal| goal.as_ref().map(label))}</strong>
            <p>{move || chat.goal.with(|goal| goal.as_ref().map(|goal| goal.objective.clone()))}</p>
            <p class="form-hint">{move || chat.goal.with(|goal| goal.as_ref().map(|goal| match goal.status {
                GoalStatus::Completed => "You marked this goal complete.",
                GoalStatus::Paused => "Paused. Continue when you’re ready.",
                GoalStatus::Active if chat.awaiting_step_id.get().is_some() => "Waiting for tool approval.",
                GoalStatus::Active if running() => "The agent is working toward this goal.",
                GoalStatus::Active => "Review the latest reply or continue working toward this goal.",
            }))}</p>
            {controls.map(|controls| view! {
                <div class="ui-actions">
                    <Show when=move || chat.goal.with(|goal| goal.as_ref().is_some_and(|goal| goal.status != GoalStatus::Completed))>
                        <button class="btn" disabled=move || chat.goal_busy.get() || chat.compacting.get() || chat.streaming.get() on:click=move |_| controls.goal.run(Some("resume".into()))>"Continue"</button>
                        <Show when=move || chat.goal.with(|goal| goal.as_ref().is_some_and(|goal| goal.status == GoalStatus::Active))>
                            <button class="btn stop" disabled=move || chat.goal_busy.get() || chat.compacting.get() on:click=move |_| controls.goal.run(Some("pause".into()))>"Pause"</button>
                        </Show>
                        <button class="btn" disabled=move || chat.goal_busy.get() || chat.compacting.get() || chat.streaming.get() on:click=move |_| controls.goal.run(Some("complete".into()))>"Mark complete"</button>
                    </Show>
                    <button class="btn ghost" disabled=move || chat.goal_busy.get() on:click=move |_| controls.refresh.run(())>"Refresh goal"</button>
                </div>
            })}
        </section>
    }
}

#[component]
pub fn GoalNotice() -> impl IntoView {
    let chat = expect_context::<ChatState>();
    let controls = use_context::<ChatControls>();
    view! { <Show when=move || chat.goal_error.get().is_some()><div class="todo-notice" role="alert">{move || chat.goal_error.get()}{controls.map(|controls| view! { <button class="btn" on:click=move |_| controls.refresh.run(())>"Refresh goal"</button> })}</div></Show> }
}
