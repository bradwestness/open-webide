use crate::state::chat::ChatState;
use leptos::prelude::*;
use openwebide_core::TodoStatus;

/// Mounted above the composer; plan contents survive collapsed display.
#[component]
pub fn TodoPlanPanel() -> impl IntoView {
    let chat = expect_context::<ChatState>();
    let actions = use_context::<crate::state_actions::todos::TodoActions>();
    view! {
        <Show when=move || chat.todo_plan.with(|plan| plan.as_ref().is_some_and(|update| !update.plan.todos.is_empty()))>
            <details class="tui-todo-plan" open>
                <summary>{move || chat.todo_plan.with(|plan| {
                    let Some(update) = plan else { return String::new(); };
                    let complete = update.plan.todos.iter().filter(|todo| todo.status == TodoStatus::Completed).count();
                    format!("Plan · {complete}/{} completed", update.plan.todos.len())
                })}</summary>
                <div class="todo-items">{move || chat.todo_plan.get().map(|update| update.plan.todos.into_iter().map(|todo| {
                    let (status, marker, label) = match todo.status { TodoStatus::Pending => ("pending", "○", "Pending"), TodoStatus::InProgress => ("active", "◉", "In progress"), TodoStatus::Completed => ("completed", "✓", "Completed") };
                    view! { <div class=format!("todo-row {status}") data-todo-id=todo.id><span class="todo-status" title=label aria-label=label>{marker}</span><span>{todo.content}</span></div> }
                }).collect_view())}</div>
            </details>
        </Show>
        <Show when=move || chat.todo_loading.get() && chat.todo_plan.with(Option::is_none)><div class="todo-notice">"Loading plan…"</div></Show>
        <Show when=move || chat.todo_error.get().is_some()>
            <div class="todo-notice">{move || chat.todo_error.get()}
                {actions.map(|actions| view! { <button class="btn ghost" disabled=move || chat.todo_loading.get() on:click=move |_| { if let Some(session) = chat.active_session.get_untracked() { actions.refresh.run(session); } }>"Retry"</button> })}
            </div>
        </Show>
    }
}
