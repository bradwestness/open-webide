use crate::state::questions::QuestionsState;
use leptos::prelude::*;
use openwebide_core::questions::{AnswerValue, QuestionAnswer, QuestionReply, QuestionRequest};

#[component]
pub fn QuestionsPanel() -> impl IntoView {
    let state = expect_context::<QuestionsState>();
    let timer =
        set_interval_with_handle(move || state.refresh(), std::time::Duration::from_secs(2)).ok();
    on_cleanup(move || {
        if let Some(timer) = timer {
            timer.clear();
        }
    });
    view! {
        <Show when=move||!state.questions.with(Vec::is_empty)>
            <section class="agent-questions" aria-label="Agent questions">
                <p class="form-hint">"The agent is waiting for your reply. Answers are shared with the agent and saved in this conversation."</p>
                <For each=move||state.questions.get() key=|question|question.id.clone() children=move|question|view!{<QuestionForm id=question.id request=question.request session=question.session_id/>}/>
                <Show when=move||state.error.get().is_some()><p class="form-hint" role="alert">{move||state.error.get().unwrap_or_default()}</p></Show>
            </section>
        </Show>
    }
}
#[component]
fn QuestionForm(id: String, request: QuestionRequest, session: i64) -> impl IntoView {
    let state = expect_context::<QuestionsState>();
    let request = StoredValue::new(request);
    let answers = RwSignal::new(std::collections::BTreeMap::<String, AnswerValue>::new());
    let reply = move || QuestionReply::Answer {
        answers: answers.with(|answers| {
            answers
                .iter()
                .map(|(id, value)| QuestionAnswer {
                    id: id.clone(),
                    value: value.clone(),
                })
                .collect()
        }),
    };
    let respond = state.responder(id.clone(), session);
    let submit = Callback::new(move |()| {
        let reply = reply();
        if !state.busy.get_untracked()
            && request.with_value(|request| request.validate_reply(&reply).is_ok())
        {
            respond.run(reply);
        }
    });
    view! {
        <form class="agent-question-form" on:submit=move|event|{event.prevent_default();submit.run(());} on:keydown=move|event:web_sys::KeyboardEvent|{
            if event.key()=="Enter" && (event.ctrl_key()||event.meta_key()){event.prevent_default();submit.run(());}
        }>
            {request.get_value().questions.into_iter().map(|question|{
                let question_id=question.id.clone();let text_id=question.id.clone();let text_value_id=question.id.clone();let name=format!("question-{id}-{}",question.id);
                view!{
                    <fieldset class="setting-row ui-field ui-field-group">
                        <legend class="setting-label">{question.title}</legend>
                        {question.options.into_iter().enumerate().map(|(index,option)|{
                            let checked_id=question_id.clone();let change_id=question_id.clone();let name=name.clone();
                            view!{<label class="agent-question-option"><input type="radio" name=name prop:checked=move||answers.with(|answers|matches!(answers.get(&checked_id),Some(AnswerValue::Choice{index:chosen})if *chosen==index)) on:change=move|_|{answers.update(|answers|{answers.insert(change_id.clone(),AnswerValue::Choice{index});});}/><span>{option.label}<small class="form-hint">{option.description}</small></span></label>}
                        }).collect::<Vec<_>>()}
                        <label class="setting-label">"Your answer"<textarea class="form-input" rows="2" maxlength="4000" prop:value=move||answers.with(|answers|match answers.get(&text_value_id){Some(AnswerValue::Text{text})=>text.clone(),_=>String::new()}) on:input=move|event|{answers.update(|answers|{answers.insert(text_id.clone(),AnswerValue::Text{text:event_target_value(&event)});});}/></label>
                    </fieldset>
                }
            }).collect::<Vec<_>>()}
            <super::ui::InlineActions><button type="submit" class="btn send" disabled=move||state.busy.get() || request.with_value(|request|request.validate_reply(&reply()).is_err())>"Submit answers"</button><button type="button" class="btn ghost" disabled=move||state.busy.get() on:click=move|_|respond.run(QuestionReply::Cancel)>"Cancel questions"</button></super::ui::InlineActions>
        </form>
    }
}
