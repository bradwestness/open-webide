use crate::state::{chat::ChatState, reviews::ReviewsState};
use leptos::prelude::*;

#[component]
pub fn TurnSummary(message: i64) -> impl IntoView {
    let chat = expect_context::<ChatState>();
    let reviews = use_context::<ReviewsState>();
    let summary = Memo::new(move |_| {
        chat.turn_summaries
            .with(|summaries| summaries.get(&message).cloned())
    });
    let files = Memo::new(move |_| {
        let mut files = summary.with(|summary| {
            summary
                .as_ref()
                .map(|summary| summary.files.clone())
                .unwrap_or_default()
        });
        if let Some(reviews) = reviews {
            reviews.records.with(|records| {
                for record in records {
                    if Some(record.session_id) == chat.active_session.get()
                        && record.message_id == message
                    {
                        files.insert(record.file.path.clone());
                    }
                }
            });
        }
        files.len()
    });
    view! { <Show when=move || summary.with(|summary| summary.as_ref().is_some_and(|summary| summary.tool_count() > 0))>
        <div class="tui-turn-summary muted" data-message-id=message title="Tool steps and unique files changed during this prompt. Time sums recorded model generation and tool steps; approval waits are excluded. ≥ marks incomplete or still-running timing.">
            {move || summary.with(|summary| summary.as_ref().map(|summary| summary.label(files.get())).unwrap_or_default())}
        </div>
    </Show> }
}
