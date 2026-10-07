use crate::{state::chat::ChatState, state_actions::chat_controls::ChatControls};
use leptos::prelude::*;
use openwebide_core::GoalStatus;

#[component]
pub fn GoalPanel() -> impl IntoView {
    let chat = expect_context::<ChatState>();
    let controls = use_context::<ChatControls>();
    view! {
        <Show when=move || chat.goal.get().is_some()>
            <section class="tui-goal" aria-label="Session goal">
                <strong>{move || chat.goal.with(|goal| goal.as_ref().map(|goal| match goal.status { GoalStatus::Completed => "Goal completed", GoalStatus::Paused => "Goal paused", GoalStatus::Active if chat.streaming.get() && chat.streaming_session.get() == chat.active_session.get() => "Working on goal", GoalStatus::Active => "Goal · review or continue" }))}</strong>
                <p>{move || chat.goal.with(|goal| goal.as_ref().map(|goal| goal.objective.clone()))}</p>
                {controls.map(|controls| view! {
                    <div class="ui-actions">
                        <Show when=move || chat.goal.with(|goal| goal.as_ref().is_some_and(|goal| goal.status != GoalStatus::Completed))>
                            <button class="btn" disabled=move || chat.goal_busy.get() || chat.compacting.get() || chat.streaming.get() on:click=move |_| controls.goal.run(Some("resume".into()))>"Continue"</button>
                            <button class="btn stop" disabled=move || chat.goal_busy.get() || chat.compacting.get() on:click=move |_| controls.goal.run(Some("pause".into()))>"Pause"</button>
                            <button class="btn" disabled=move || chat.goal_busy.get() || chat.compacting.get() || chat.streaming.get() on:click=move |_| controls.goal.run(Some("complete".into()))>"Mark complete"</button>
                        </Show>
                    </div>
                })}
            </section>
        </Show>
        <Show when=move || chat.goal_error.get().is_some()><div class="todo-notice" role="alert">{move || chat.goal_error.get()}{controls.map(|controls| view! { <button class="btn" on:click=move |_| controls.refresh.run(())>"Refresh goal"</button> })}</div></Show>
    }
}
