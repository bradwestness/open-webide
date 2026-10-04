use crate::{
    components::modal::Modal,
    state::{auth::AuthState, chat::ChatState, projects::ProjectsState, ui::UiState},
};
use leptos::prelude::*;

#[component]
pub fn ContextUsage() -> impl IntoView {
    let ui = expect_context::<UiState>();
    let chat = expect_context::<ChatState>();
    let auth = expect_context::<AuthState>();
    let projects = expect_context::<ProjectsState>();
    let project = StoredValue::new(projects.active_project.get_untracked());
    let session = StoredValue::new(chat.active_session.get_untracked());
    let generation = StoredValue::new(auth.generation.get_untracked());
    Effect::new(move |_| {
        let current = chat.active_session.get();
        let current_project = projects.active_project.get();
        let account = auth.generation.get();
        if session.get_value() != current
            || generation.get_value() != account
            || project.get_value() != current_project
        {
            ui.context_open.set(false);
            project.set_value(current_project);
            session.set_value(current);
            generation.set_value(account);
        }
    });
    let close = Callback::new(move |()| ui.context_open.set(false));
    view! { <Show when=move || ui.context_open.get()>
        <Modal title="Context".to_string().into() on_close=close class="modal modal-sm" describedby="context-description">
            <div class="modal-body context-usage">
                <p id="context-description" class="form-hint">"Latest model request. Category counts are estimates; summaries replace compacted history. Drafts are included after sending."</p>
                {move || {
                    let telemetry = chat.session_telemetry.get();
                    match telemetry.context {
                        None => view! { <p>"No breakdown is available yet. Send a message to capture one."</p> }.into_any(),
                        Some(context) => {
                            let generated = telemetry.context_tokens.saturating_sub(context.total());
                            let free = telemetry.context_limit.saturating_sub(telemetry.context_tokens);
                            let total = telemetry.context_tokens.max(telemetry.context_limit).max(1);
                            let sections = context.sections();
                            view! {
                                <p>{format!("{} · {}{} / {}{} tokens", telemetry.model, if telemetry.context_estimated { "~" } else { "" }, telemetry.context_tokens, if telemetry.context_limit_estimated { "~" } else { "" }, telemetry.context_limit)}</p>
                                <div class="context-breakdown-bar" role="img" aria-label=format!("Context: {} used, {free} free", telemetry.context_tokens)>
                                    {sections.into_iter().enumerate().map(|(index, (label, count))| view! { <span class=format!("context-segment context-category-{index}") title=format!("{label}: ~{count} tokens") style:width=format!("{}%", count as f64 / total as f64 * 100.0) /> }).collect_view()}
                                    <span class="context-segment context-generated" title=format!("Generated reply: {generated} tokens") style:width=format!("{}%", generated as f64 / total as f64 * 100.0) />
                                </div>
                                <dl class="context-breakdown-list">
                                    {sections.into_iter().enumerate().map(|(index, (label, count))| view! { <div><dt><span class=format!("context-key context-category-{index}")/>{label}</dt><dd>{format!("~{count}")}</dd></div> }).collect_view()}
                                    <div><dt><span class="context-key context-generated"/>"Generated reply"</dt><dd>{generated}</dd></div>
                                    <div><dt>"Free context"</dt><dd>{free}</dd></div>
                                </dl>
                                <Show when={move || telemetry.context_tokens > telemetry.context_limit}><p class="form-hint">"The latest call exceeds the configured context limit."</p></Show>
                            }.into_any()
                        }
                    }
                }}
            </div>
            <div class="modal-footer"><button class="btn" on:click=move |_| close.run(())>"Close"</button></div>
        </Modal>
    </Show> }
}
