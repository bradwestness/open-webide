//! Structured questions and explicit replies, independent of execution host.
use serde::{Deserialize, Serialize};

pub const TOOL_NAME: &str = "ask_user_question";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuestionRequest {
    pub questions: Vec<Question>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Question {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub options: Vec<QuestionOption>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuestionOption {
    pub label: String,
    #[serde(default)]
    pub description: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuestionAnswer {
    pub id: String,
    pub value: AnswerValue,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AnswerValue {
    Choice { index: usize },
    Text { text: String },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum QuestionReply {
    Answer { answers: Vec<QuestionAnswer> },
    Cancel,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentQuestion {
    pub id: String,
    pub session_id: i64,
    pub anchor_message_id: i64,
    pub request: QuestionRequest,
    pub reply: Option<QuestionReply>,
    pub created_at: i64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum QuestionCommand {
    Create {
        id: String,
        anchor: i64,
        request: QuestionRequest,
    },
    Read {
        id: String,
    },
    List,
    Reply {
        id: String,
        reply: QuestionReply,
    },
    CancelRun {
        anchor: i64,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuestionResult {
    pub questions: Vec<AgentQuestion>,
}

pub fn ensure_resumable(questions: &[AgentQuestion], anchor: i64) -> Result<(), String> {
    if questions
        .iter()
        .any(|question| question.anchor_message_id == anchor && question.reply.is_none())
    {
        Err("Answer or cancel pending questions before resuming this run.".into())
    } else {
        Ok(())
    }
}

fn bounded(value: &str, max: usize) -> bool {
    !value.trim().is_empty() && value.len() <= max && !value.contains('\0')
}
impl QuestionRequest {
    pub fn validate(&self) -> Result<(), String> {
        if !(1..=3).contains(&self.questions.len()) {
            return Err("Ask between one and three related questions.".into());
        }
        let mut ids = std::collections::BTreeSet::new();
        for question in &self.questions {
            if !bounded(&question.id, 64)
                || !ids.insert(&question.id)
                || !bounded(&question.title, 1000)
                || question.options.len() > 6
            {
                return Err("Questions need unique IDs, a title and at most six choices.".into());
            }
            let mut labels = std::collections::BTreeSet::new();
            for option in &question.options {
                if !bounded(&option.label, 160)
                    || !labels.insert(&option.label)
                    || option.description.len() > 500
                    || option.description.contains('\0')
                {
                    return Err("Choices need distinct labels and bounded descriptions.".into());
                }
            }
        }
        Ok(())
    }
    pub fn validate_reply(&self, reply: &QuestionReply) -> Result<(), String> {
        self.validate()?;
        let QuestionReply::Answer { answers } = reply else {
            return Ok(());
        };
        if answers.len() != self.questions.len() {
            return Err("Answer every question before submitting.".into());
        }
        let mut ids = std::collections::BTreeSet::new();
        for answer in answers {
            let question = self
                .questions
                .iter()
                .find(|question| question.id == answer.id)
                .ok_or("The reply contains an unknown question.")?;
            if !ids.insert(&answer.id) {
                return Err("Answer each question once.".into());
            }
            match &answer.value {
                AnswerValue::Choice { index } if *index < question.options.len() => (),
                AnswerValue::Text { text } if bounded(text, 4000) => (),
                _ => {
                    return Err(
                        "Choose an available answer or enter a nonempty reply up to 4000 bytes."
                            .into(),
                    );
                }
            }
        }
        Ok(())
    }
    /// Persist the same explicit result the model receives, including free text.
    pub fn result_text(&self, reply: &QuestionReply) -> Result<String, String> {
        self.validate_reply(reply)?;
        let result = match reply {
            QuestionReply::Cancel => {
                serde_json::json!({"cancelled":true,"message":"The user cancelled these questions. Do not assume an answer or repeat them unchanged."})
            }
            QuestionReply::Answer { answers } => {
                serde_json::json!({"cancelled":false,"answers":answers.iter().map(|answer| {
                let question = self.questions.iter().find(|question| question.id == answer.id).expect("validated question");
                let text = match &answer.value { AnswerValue::Choice { index } => question.options[*index].label.clone(), AnswerValue::Text { text } => text.clone() };
                serde_json::json!({"id":answer.id,"question":question.title,"answer":text})
            }).collect::<Vec<_>>()})
            }
        };
        serde_json::to_string(&result).map_err(|error| error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resume_waits_only_for_unanswered_questions_in_the_requesting_run() {
        let mut question = AgentQuestion {
            id: "question".into(),
            session_id: 1,
            anchor_message_id: 2,
            request: QuestionRequest { questions: vec![] },
            reply: None,
            created_at: 0,
        };
        assert!(ensure_resumable(std::slice::from_ref(&question), 2).is_err());
        assert!(ensure_resumable(std::slice::from_ref(&question), 3).is_ok());
        question.reply = Some(QuestionReply::Cancel);
        assert!(ensure_resumable(&[question], 2).is_ok());
        assert!(ensure_resumable(&[], 2).is_ok());
    }
    #[test]
    fn questions_require_explicit_complete_valid_answers_and_preserve_user_text() {
        let request = QuestionRequest {
            questions: vec![
                Question {
                    id: "choice".into(),
                    title: "Choose a destination".into(),
                    options: vec![QuestionOption {
                        label: "Home (Recommended)".into(),
                        description: "User space".into(),
                    }],
                },
                Question {
                    id: "path".into(),
                    title: "Which directory?".into(),
                    options: vec![],
                },
            ],
        };
        request.validate().unwrap();
        let mut reply = QuestionReply::Answer { answers: vec![] };
        assert!(request.validate_reply(&reply).is_err());
        if let QuestionReply::Answer { answers } = &mut reply {
            answers.extend([
                QuestionAnswer {
                    id: "choice".into(),
                    value: AnswerValue::Choice { index: 0 },
                },
                QuestionAnswer {
                    id: "path".into(),
                    value: AnswerValue::Text {
                        text: "/srv/媒体".into(),
                    },
                },
            ]);
        }
        let result = request.result_text(&reply).unwrap();
        assert!(result.contains("/srv/媒体") && result.contains("Home (Recommended)"));
        assert!(
            request
                .result_text(&QuestionReply::Cancel)
                .unwrap()
                .contains("cancelled")
        );
        for answers in [
            vec![
                QuestionAnswer {
                    id: "choice".into(),
                    value: AnswerValue::Choice { index: 1 },
                },
                QuestionAnswer {
                    id: "path".into(),
                    value: AnswerValue::Text { text: "x".into() },
                },
            ],
            vec![
                QuestionAnswer {
                    id: "choice".into(),
                    value: AnswerValue::Choice { index: 0 }
                };
                2
            ],
        ] {
            assert!(
                request
                    .validate_reply(&QuestionReply::Answer { answers })
                    .is_err()
            );
        }
        let mut invalid = request.clone();
        invalid.questions[1].id = "choice".into();
        assert!(invalid.validate().is_err());
        invalid = request;
        let duplicate = invalid.questions[0].options[0].clone();
        invalid.questions[0].options.push(duplicate);
        assert!(invalid.validate().is_err());
    }
}
