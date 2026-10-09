use super::{context_usage::ContextUsage, modal::Modal};
use crate::{
    conversation::ConversationItem,
    prompt::Composer,
    state::{auth::AuthState, chat::ChatState, projects::ProjectsState, ui::UiState},
};
use leptos::prelude::*;
use openwebide_core::{Role, SessionTelemetry};

/// Chat details share ownership and focus policy in every workspace mode.
#[component]
pub fn ChatDetails() -> impl IntoView {
    let ui = expect_context::<UiState>();
    let chat = expect_context::<ChatState>();
    let auth = expect_context::<AuthState>();
    let projects = expect_context::<ProjectsState>();
    let scope = StoredValue::new((
        auth.generation.get_untracked(),
        projects.active_project.get_untracked(),
        chat.active_session.get_untracked(),
    ));
    Effect::new(move |_| {
        let current = (
            auth.generation.get(),
            projects.active_project.get(),
            chat.active_session.get(),
        );
        if scope.get_value() != current {
            ui.context_open.set(false);
            ui.generation_open.set(false);
            ui.run_context.set(None);
            scope.set_value(current);
        }
    });
    view! { <leptos::portal::Portal><ContextUsage/><RunContext/><GenerationStatistics/></leptos::portal::Portal> }
}

#[component]
fn RunContext() -> impl IntoView {
    let ui = expect_context::<UiState>();
    let chat = expect_context::<ChatState>();
    let close =
        expect_context::<Composer>().after(Callback::new(move |()| ui.run_context.set(None)));
    let content = Memo::new(move |_| {
        let id = ui.run_context.get()?;
        let handle = chat
            .run_contexts
            .with(|contexts| contexts.get(&id).copied())?;
        handle.item.with(|item| match item {
            ConversationItem::Message(message) => message
                .content
                .strip_prefix(openwebide_core::RUN_CONTEXT_PREFIX)
                .map(str::to_owned),
            _ => None,
        })
    });
    Effect::new(move |_| {
        if ui.run_context.get().is_some() && content.get().is_none() {
            ui.run_context.set(None);
        }
    });
    view! { <Show when=move || content.get().is_some()>
        <Modal title="Run context".to_string().into() on_close=close describedby="run-context-description">
            <div class="modal-body">
                <p id="run-context-description" class="form-hint">"The saved instructions and environment used for this prompt."</p>
                <pre class="tui-thinking-pre run-context-content">{move || content.get().unwrap_or_default()}</pre>
            </div>
            <div class="modal-footer"><button class="btn" on:click=move |_| close.run(())>"Close"</button></div>
        </Modal>
    </Show> }
}

#[component]
fn GenerationStatistics() -> impl IntoView {
    let ui = expect_context::<UiState>();
    let chat = expect_context::<ChatState>();
    let close =
        expect_context::<Composer>().after(Callback::new(move |()| ui.generation_open.set(false)));
    view! { <Show when=move || ui.generation_open.get()>
        <Modal title="Generation statistics".to_string().into() on_close=close class="modal modal-sm" describedby="generation-description">
            <div class="modal-body generation-statistics">
                <p id="generation-description" class="form-hint">"Model generation speed excludes tool execution and approval waits. ~ marks estimated token counts."</p>
                {move || {
                    let telemetry = chat.session_telemetry.get();
                    let calls = chat.messages.handles.with(|handles| handles.iter().filter_map(|handle| handle.item.with(|item| match item {
                        ConversationItem::Message(message) if message.role == Role::Assistant => message.usage.filter(|usage| usage.tokens_per_second().is_some()),
                        _ => None,
                    })).collect::<Vec<_>>());
                    let recent = calls.iter().enumerate().skip(calls.len().saturating_sub(8)).map(|(index, usage)| (index + 1, *usage)).collect::<Vec<_>>();
                    let max_speed = recent.iter().filter_map(|(_, usage)| usage.tokens_per_second()).fold(1.0_f64, f64::max);
                    let total = telemetry.total_prompt_tokens.saturating_add(telemetry.total_completion_tokens).max(1);
                    let approximate = SessionTelemetry::approx(telemetry.totals_estimated);
                    let last = telemetry.last_call;
                    let last_count = move |count: usize| format!("{}{count}", SessionTelemetry::approx(last.is_some_and(|usage| usage.estimated)));
                    view! {
                        <p class="muted">{telemetry.model.clone()}</p>
                        <p class="generation-rate"><span>{telemetry.speed_text()}</span><span class="form-hint">"Last measured speed"</span></p>
                        <dl class="context-breakdown-list">
                            <div><dt>"Latest input"</dt><dd>{last.map_or_else(|| "—".into(), |usage| last_count(usage.prompt_tokens))}</dd></div>
                            <div><dt>"Latest output"</dt><dd>{last.map_or_else(|| "—".into(), |usage| last_count(usage.completion_tokens))}</dd></div>
                            <div><dt>"Latest generation time"</dt><dd>{last.filter(|usage| usage.eval_duration_ms > 0).map_or_else(|| "—".into(), |usage| format!("{:.2}s", usage.eval_duration_ms as f64 / 1000.0))}</dd></div>
                            <div><dt>"Tool calls"</dt><dd>{telemetry.tool_calls_count}</dd></div>
                        </dl>
                        <h3>"Session tokens"</h3>
                        <div class="context-breakdown-bar" role="img" aria-label=format!("Session tokens: {approximate}{} input, {approximate}{} output", telemetry.total_prompt_tokens, telemetry.total_completion_tokens)>
                            <span class="context-segment context-category-3" title=format!("Input: {approximate}{} tokens", telemetry.total_prompt_tokens) style:width=format!("{}%", telemetry.total_prompt_tokens as f64 / total as f64 * 100.0)/>
                            <span class="context-segment context-generated" title=format!("Output: {approximate}{} tokens", telemetry.total_completion_tokens) style:width=format!("{}%", telemetry.total_completion_tokens as f64 / total as f64 * 100.0)/>
                        </div>
                        <dl class="context-breakdown-list">
                            <div><dt><span class="context-key context-category-3"/>"Input"</dt><dd>{format!("{approximate}{}", telemetry.total_prompt_tokens)}</dd></div>
                            <div><dt><span class="context-key context-generated"/>"Output"</dt><dd>{format!("{approximate}{}", telemetry.total_completion_tokens)}</dd></div>
                        </dl>
                        <h3>"Recent model calls"</h3>
                        {if recent.is_empty() {
                            view! { <p class="form-hint">"No recorded generation timings yet."</p> }.into_any()
                        } else {
                            view! { <div class="generation-chart" role="list" aria-label="Generation speeds for the last eight recorded model calls, in tokens per second">
                                {recent.into_iter().map(|(index, usage)| {
                                    let speed = usage.tokens_per_second().unwrap_or_default();
                                    let label = format!("{}{speed:.1} t/s", SessionTelemetry::approx(usage.estimated));
                                    let title = format!("Call {index}: {label} · {} output tokens · {:.2}s generation", usage.completion_tokens, usage.eval_duration_ms as f64 / 1000.0);
                                    view! { <div class="generation-chart-row" role="listitem" title=title>
                                        <span>{format!("Call {index}")}</span><div class="generation-chart-track" aria-hidden="true"><span style:width=format!("{}%", speed / max_speed * 100.0)/></div><span>{label}</span>
                                    </div> }
                                }).collect_view()}
                            </div> }.into_any()
                        }}
                        <p class="form-hint">"Session totals include delegated tasks. The chart uses recorded chat calls; intermediate calls may be absent after reloading."</p>
                    }
                }}
            </div>
            <div class="modal-footer"><button class="btn ghost" on:click=move |_| { ui.generation_open.set(false); ui.context_open.set(true); }>"Context"</button><button class="btn" on:click=move |_| close.run(())>"Close"</button></div>
        </Modal>
    </Show> }
}
