use crate::{
    state::{chat::ChatState, reviews::ReviewsState},
    state_actions::reviews::ReviewActions,
};
use leptos::prelude::*;
use openwebide_core::{EditDecision, ReviewRequest, RunChange};
#[component]
pub fn RunChangesPanel(message: i64) -> impl IntoView {
    let state = use_context::<ReviewsState>();
    let actions = use_context::<ReviewActions>();
    let chat = expect_context::<ChatState>();
    let records = Memo::new(move |_| {
        state.map_or_else(Vec::new, |state| {
            state.records.with(|records| {
                records
                    .iter()
                    .filter(|record| {
                        record.session_id == chat.active_session.get().unwrap_or_default()
                            && record.message_id == message
                    })
                    .cloned()
                    .collect::<Vec<_>>()
            })
        })
    });
    view! { <Show when=move || !records.get().is_empty()>
        <details class="run-changes" data-message-id=message>
            <summary>{move || { let records = records.get(); let pending: usize = records.iter().map(RunChange::pending).sum(); format!("{} files changed · {pending} pending hunks", records.len()) }}</summary>
            <For each=move || records.get() key=|record| (record.file.path.clone(), record.revision, record.available) children=move |record| {
                let path = record.file.path.clone();
                let open_path = path.clone();
                let disabled = Signal::derive(move || chat.streaming.get() || chat.rewinding.get() || state.is_some_and(|state| state.busy.get().is_some()) || !record.available || actions.is_none());
                let request = {
                    let path = path.clone();
                    move |decision, hunk| ReviewRequest { session_id: record.session_id, message_id: record.message_id, path: path.clone(), revision: record.revision, decision, hunk }
                };
                let accept_request = request(EditDecision::Accepted, None);
                let accept = Callback::new(move |_: leptos::ev::MouseEvent| { if let Some(actions) = actions { actions.review.run(accept_request.clone()); } });
                let reject_request = request(EditDecision::Rejected, None);
                let reject = Callback::new(move |_: leptos::ev::MouseEvent| { if let Some(actions) = actions { actions.review.run(reject_request.clone()); } });
                let pending = record.pending();
                let status = if !record.available && pending > 0 { "Changed again" } else if pending == 0 { "Reviewed" } else if record.file.binary_after.is_some() || record.file.binary_before.is_some() { "Binary" } else if record.file.deleted { "Deleted" } else if record.file.before.is_none() { "Created" } else { "Modified" };
                let hunks = record.hunks();
                view! { <section class="run-change" data-path=path.clone()>
                    <div class="run-change-header">
                        <button class="btn ghost run-change-path" on:click=move |_| { if let Some(actions) = actions { actions.open.run(open_path.clone()); } }>{record.file.path}</button>
                        <span class="muted">{status}</span>
                        <Show when={move || pending > 0}>
                            <button class="btn approve" disabled=disabled on:click=move |event| accept.run(event)>"Accept file"</button>
                            <button class="btn deny" disabled=disabled on:click=move |event| reject.run(event)>"Reject file"</button>
                        </Show>
                    </div>
                    {hunks.into_iter().map(|hunk| {
                        let accept_request = request(EditDecision::Accepted, Some(hunk.index));
                        let accept = Callback::new(move |_: leptos::ev::MouseEvent| { if let Some(actions) = actions { actions.review.run(accept_request.clone()); } });
                        let reject_request = request(EditDecision::Rejected, Some(hunk.index));
                        let reject = Callback::new(move |_: leptos::ev::MouseEvent| { if let Some(actions) = actions { actions.review.run(reject_request.clone()); } });
                        let pending = hunk.decision == EditDecision::Pending;
                        let label = match hunk.decision { EditDecision::Pending => "Pending", EditDecision::Accepted => "Accepted", EditDecision::Rejected => "Rejected" };
                        view! { <details class="run-change-hunk" data-hunk=hunk.index>
                            <summary>{format!("Line {} · {label}", hunk.new_line)}</summary>
                            <Show when=move || pending>
                                <div class="run-change-actions">
                                    <button class="btn approve" disabled=disabled on:click=move |event| accept.run(event)>"Accept hunk"</button>
                                    <button class="btn deny" disabled=disabled on:click=move |event| reject.run(event)>"Reject hunk"</button>
                                </div>
                            </Show>
                            <pre class="run-change-removed">{hunk.old}</pre><pre class="run-change-added">{hunk.new}</pre>
                        </details> }
                    }).collect_view()}
                </section> }
            } />
        </details>
    </Show> }
}
