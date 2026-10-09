use super::support::{mount_test, settle};
use leptos::prelude::*;
use openwebide_core::questions::*;
use openwebide_frontend::{
    components::questions::QuestionsPanel, state::questions::QuestionsState,
};
use std::{cell::Cell, rc::Rc};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

fn question() -> AgentQuestion {
    AgentQuestion {
        id: "a1t1c0".into(),
        session_id: 1,
        anchor_message_id: 1,
        created_at: 1,
        reply: None,
        request: QuestionRequest {
            questions: vec![
                Question {
                    id: "destination".into(),
                    title: "Where should this run?".into(),
                    options: vec![
                        QuestionOption {
                            label: "Home (Recommended)".into(),
                            description: "Use your own account".into(),
                        },
                        QuestionOption {
                            label: "Shared server".into(),
                            description: "Use the server directory".into(),
                        },
                    ],
                },
                Question {
                    id: "path".into(),
                    title: "Which directory?".into(),
                    options: vec![],
                },
            ],
        },
    }
}
#[wasm_bindgen_test]
async fn questions_require_an_explicit_choice_and_text_preserve_drafts_during_polling_and_fit_a_phone()
 {
    let slot = Rc::new(Cell::new(None));
    let capture = slot.clone();
    let mounted = mount_test(move |state| {
        state.seed_session();
        state
            .chat
            .sessions
            .update(|sessions| sessions[0].project_id = None);
        state.chat.active_session.set(Some(1));
        state.chat.streaming.set(true);
        *state.fake.questions.borrow_mut() = vec![question()];
        let questions = QuestionsState::new(state.api, state.auth, state.chat, state.projects);
        provide_context(questions);
        capture.set(Some(questions));
        view! {<style>{include_str!("../../styles.css")}</style><QuestionsPanel/>}
    });
    settle().await;
    mounted.root.style().set_property("width", "360px").unwrap();
    let state = slot.get().unwrap();
    let submit = mounted
        .element("button[type='submit']")
        .unchecked_into::<web_sys::HtmlButtonElement>();
    assert!(submit.disabled());
    let choice = mounted
        .element("input[type='radio']")
        .unchecked_into::<web_sys::HtmlInputElement>();
    assert!(!choice.checked());
    choice.click();
    settle().await;
    assert!(submit.disabled());
    let text = mounted
        .element("fieldset:nth-of-type(2) textarea")
        .unchecked_into::<web_sys::HtmlTextAreaElement>();
    text.set_value("/srv/media");
    text.dispatch_event(&web_sys::Event::new("input").unwrap())
        .unwrap();
    settle().await;
    assert!(!submit.disabled());
    state.refresh();
    settle().await;
    assert_eq!(text.value(), "/srv/media");
    assert!(
        mounted.root.scroll_width() <= 362,
        "Question form overflowed the phone width"
    );
    let init = web_sys::KeyboardEventInit::new();
    init.set_key("Enter");
    init.set_ctrl_key(true);
    init.set_bubbles(true);
    text.dispatch_event(
        &web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap(),
    )
    .unwrap();
    settle().await;
    let commands = mounted.state.fake.question_commands.borrow();
    let replies = commands
        .iter()
        .filter_map(|(_, command)| match command {
            QuestionCommand::Reply { reply, .. } => Some(reply),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(replies.len(), 1);
    assert_eq!(
        *replies[0],
        QuestionReply::Answer {
            answers: vec![
                QuestionAnswer {
                    id: "destination".into(),
                    value: AnswerValue::Choice { index: 0 }
                },
                QuestionAnswer {
                    id: "path".into(),
                    value: AnswerValue::Text {
                        text: "/srv/media".into()
                    }
                }
            ]
        }
    );
    assert!(state.questions.get_untracked().is_empty());
    assert!(mounted.state.chat.draft.get_untracked().is_empty());
}
#[wasm_bindgen_test]
async fn question_replies_and_delayed_lists_cannot_cross_session_project_or_account_changes() {
    let slot = Rc::new(Cell::new(None));
    let capture = slot.clone();
    let mounted = mount_test(move |state| {
        state.seed_session();
        state
            .chat
            .sessions
            .update(|sessions| sessions[0].project_id = None);
        state.chat.active_session.set(Some(1));
        *state.fake.questions.borrow_mut() = vec![question()];
        let questions = QuestionsState::new(state.api, state.auth, state.chat, state.projects);
        provide_context(questions);
        capture.set(Some(questions));
        view! {<QuestionsPanel/>}
    });
    settle().await;
    let state = slot.get().unwrap();
    for transition in 0..3 {
        mounted.state.seed_session();
        mounted
            .state
            .chat
            .sessions
            .update(|sessions| sessions[0].project_id = None);
        mounted.state.chat.active_session.set(Some(1));
        mounted.state.projects.active_project.set(None);
        settle().await;
        let respond = state.responder("a1t1c0".into(), 1);
        let (sender, receiver) = futures::channel::oneshot::channel();
        mounted
            .state
            .fake
            .question_loads
            .borrow_mut()
            .push_back(receiver);
        state.refresh();
        settle().await;
        match transition {
            0 => mounted.state.chat.active_session.set(Some(2)),
            1 => mounted.state.projects.active_project.set(Some(8)),
            _ => mounted
                .state
                .auth
                .generation
                .update(|generation| *generation += 1),
        }
        // A saved form callback must be rejected even before the invalidation effect runs.
        respond.run(QuestionReply::Cancel);
        settle().await;
        sender
            .send(Ok(QuestionResult {
                questions: vec![question()],
            }))
            .unwrap();
        settle().await;
        if transition == 0 {
            assert!(state.questions.get_untracked().is_empty());
        }
    }
    assert!(
        !mounted
            .state
            .fake
            .question_commands
            .borrow()
            .iter()
            .any(|(_, command)| matches!(command, QuestionCommand::Reply { .. }))
    );
}
#[wasm_bindgen_test]
async fn cancelling_questions_is_a_distinct_explicit_reply() {
    let mounted = mount_test(move |state| {
        state.seed_session();
        state
            .chat
            .sessions
            .update(|sessions| sessions[0].project_id = None);
        state.chat.active_session.set(Some(1));
        state.chat.streaming.set(true);
        *state.fake.questions.borrow_mut() = vec![question()];
        provide_context(QuestionsState::new(
            state.api,
            state.auth,
            state.chat,
            state.projects,
        ));
        view! {<QuestionsPanel/>}
    });
    settle().await;
    mounted.click_text("Cancel questions");
    settle().await;
    assert!(
        mounted
            .state
            .fake
            .question_commands
            .borrow()
            .iter()
            .any(|(_, command)| matches!(
                command,
                QuestionCommand::Reply {
                    reply: QuestionReply::Cancel,
                    ..
                }
            ))
    );
}
