//! Owned, immutable questions. Replies atomically complete their original tool step.
use super::*;
use openwebide_core::questions::{AgentQuestion, QuestionCommand, QuestionReply, QuestionResult};

fn decode(row: &crate::db::QueryRow, session: i64) -> Result<AgentQuestion, StorageError> {
    Ok(AgentQuestion {
        id: row.get_text(0)?.into(),
        session_id: session,
        anchor_message_id: row.get_int(1)?,
        request: serde_json::from_str(row.get_text(2)?)
            .map_err(|error| StorageError::Db(error.to_string()))?,
        reply: row
            .get_text_opt(3)
            .map(serde_json::from_str)
            .transpose()
            .map_err(|error| StorageError::Db(error.to_string()))?,
        created_at: row.get_int(4)?,
    })
}

impl<D: Db> Store<D> {
    pub fn question_command<'a>(
        &'a self,
        user: UserId,
        session: i64,
        command: &'a QuestionCommand,
        now: i64,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<QuestionResult, StorageError>> + Send + 'a>,
    > {
        Box::pin(async move {
            self.db.transaction(|tx| async move {
                let store = Store::new(tx);
                store.get_session(session,user).await?;
                store.ensure_not_rewinding(session).await?;
                match command {
                    QuestionCommand::Create { id, anchor, request } => {
                        request.validate().map_err(StorageError::InvalidValue)?;
                        if id.is_empty() || id.len()>256 { return Err(StorageError::InvalidValue("Invalid question call ID.".into())); }
                        store.require_current_question_step(session,id,Some(*anchor)).await?;
                        let request_json = serde_json::to_string(request).map_err(|error|StorageError::Db(error.to_string()))?;
                        store.db.execute("INSERT INTO agent_questions(session_id,tool_call_id,step_id,anchor_message_id,request,created_at) SELECT ?,?,id,?,?,? FROM tool_steps WHERE session_id=? AND tool_call_id=? ON CONFLICT(session_id,tool_call_id) DO NOTHING", &[DbValue::Int(session),DbValue::Text(id.clone()),DbValue::Int(*anchor),DbValue::Text(request_json),DbValue::Int(now),DbValue::Int(session),DbValue::Text(id.clone())]).await?;
                        let question=store.read_question(session,id).await?;
                        if question.request != *request || question.anchor_message_id != *anchor { return Err(StorageError::Conflict("The question ID already belongs to another request.".into())); }
                        Ok(QuestionResult{questions:vec![question]})
                    }
                    QuestionCommand::Read { id } => {
                        let question=store.read_question(session,id).await?;
                        if question.reply.is_none() {store.require_current_question_step(session,id,Some(question.anchor_message_id)).await?;}
                        Ok(QuestionResult{questions:vec![question]})
                    },
                    QuestionCommand::List => {
                        let rows=store.db.execute("SELECT q.tool_call_id,q.anchor_message_id,q.request,q.reply,q.created_at FROM agent_questions q JOIN tool_steps s ON s.id=q.step_id WHERE q.session_id=? AND q.reply IS NULL AND s.completion_applied=0 AND q.anchor_message_id=(SELECT MAX(id) FROM messages WHERE session_id=? AND role='user') ORDER BY q.created_at,q.tool_call_id LIMIT 16", &[DbValue::Int(session),DbValue::Int(session)]).await?;
                        Ok(QuestionResult{questions:rows.rows.iter().map(|row|decode(row,session)).collect::<Result<_,_>>()?})
                    }
                    QuestionCommand::Reply { id, reply } => {
                        let question=store.read_question(session,id).await?;
                        if question.reply.is_some() { return Err(StorageError::Conflict("This question has already been answered or cancelled.".into())); }
                        store.require_current_question_step(session,id,Some(question.anchor_message_id)).await?;
                        let result=question.request.result_text(reply).map_err(StorageError::InvalidValue)?;
                        let reply_json=serde_json::to_string(reply).map_err(|error|StorageError::Db(error.to_string()))?;
                        store.db.execute("UPDATE agent_questions SET reply=? WHERE session_id=? AND tool_call_id=? AND reply IS NULL", &[DbValue::Text(reply_json),DbValue::Int(session),DbValue::Text(id.clone())]).await?;
                        store.db.execute("UPDATE tool_steps SET ok=?,result_summary=?,completion_applied=1 WHERE session_id=? AND tool_call_id=? AND completion_applied=0", &[DbValue::Int(i64::from(matches!(reply,QuestionReply::Answer{..}))),DbValue::Text(result),DbValue::Int(session),DbValue::Text(id.clone())]).await?;
                        Ok(QuestionResult{questions:vec![store.read_question(session,id).await?]})
                    }
                    QuestionCommand::CancelRun { anchor } => {
                        let result=serde_json::json!({"cancelled":true,"message":"The run stopped before a reply was received. No answer was assumed."}).to_string();
                        store.db.execute("UPDATE tool_steps SET ok=0,result_summary=?,completion_applied=1 WHERE id IN (SELECT step_id FROM agent_questions WHERE session_id=? AND anchor_message_id=? AND reply IS NULL) AND completion_applied=0", &[DbValue::Text(result),DbValue::Int(session),DbValue::Int(*anchor)]).await?;
                        let reply=serde_json::to_string(&QuestionReply::Cancel).expect("reply serializes");
                        store.db.execute("UPDATE agent_questions SET reply=? WHERE session_id=? AND anchor_message_id=? AND reply IS NULL", &[DbValue::Text(reply),DbValue::Int(session),DbValue::Int(*anchor)]).await?;
                        Ok(QuestionResult{questions:vec![]})
                    }
                }
            }).await
        })
    }
    async fn read_question(&self, session: i64, id: &str) -> Result<AgentQuestion, StorageError> {
        let rows=self.db.execute("SELECT tool_call_id,anchor_message_id,request,reply,created_at FROM agent_questions WHERE session_id=? AND tool_call_id=?", &[DbValue::Int(session),DbValue::Text(id.into())]).await?;
        decode(
            rows.rows
                .first()
                .ok_or_else(|| StorageError::NotFound("question".into()))?,
            session,
        )
    }
    async fn require_current_question_step(
        &self,
        session: i64,
        id: &str,
        anchor: Option<i64>,
    ) -> Result<(), StorageError> {
        let rows=self.db.execute("SELECT (SELECT MAX(id) FROM messages WHERE session_id=? AND role='user'),completion_applied FROM tool_steps WHERE session_id=? AND tool_call_id=? AND name='ask_user_question' AND anchor_message_id >= (SELECT MAX(id) FROM messages WHERE session_id=? AND role='user')", &[DbValue::Int(session),DbValue::Int(session),DbValue::Text(id.into()),DbValue::Int(session)]).await?;
        let row = rows.rows.first().ok_or_else(|| {
            StorageError::Conflict(
                "This question belongs to an earlier or missing run. Refresh the session.".into(),
            )
        })?;
        let run_anchor = openwebide_core::parse_step_id(id.split('.').next().unwrap_or(id))
            .map(|(anchor, _, _)| anchor);
        if run_anchor != Some(row.get_int(0)?)
            || row.get_int(1)? != 0
            || anchor.is_some_and(|anchor| row.get_int(0).ok() != Some(anchor))
        {
            return Err(StorageError::Conflict(
                "This question is no longer waiting for a reply.".into(),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::executor::block_on;
    use openwebide_core::questions::*;
    #[test]
    fn questions_are_owned_durable_immutable_and_replies_complete_the_original_call_in_all_modes() {
        block_on(async {
            for mode in [
                Some(WorkspaceMode::Local),
                Some(WorkspaceMode::Remote),
                None,
            ] {
                let database = tempfile::NamedTempFile::new().unwrap();
                let store =
                    Store::new(crate::rusqlite_db::RusqliteDb::open(database.path()).unwrap());
                store.migrate().await.unwrap();
                store.migrate().await.unwrap();
                let owner = store
                    .insert_user("owner", "hash", UserRole::Admin, 1)
                    .await
                    .unwrap()
                    .id;
                let other = store
                    .insert_user("other", "hash", UserRole::User, 1)
                    .await
                    .unwrap()
                    .id;
                let project = if let Some(mode) = mode {
                    Some(
                        store
                            .create_project(
                                &NewProject {
                                    name: "project".into(),
                                    mode,
                                    path: Some("test".into()),
                                },
                                owner,
                                1,
                            )
                            .await
                            .unwrap()
                            .id,
                    )
                } else {
                    None
                };
                let session = store
                    .create_session("chat", None, None, project, owner, 1)
                    .await
                    .unwrap()
                    .id;
                let prompt = store
                    .insert_message(session, Role::User, "Help me configure this", 1)
                    .await
                    .unwrap();
                let wire = openwebide_core::ToolCall {
                    id: "wire-call".into(),
                    name: TOOL_NAME.into(),
                    arguments: "{}".into(),
                };
                let turn = store
                    .insert_interim_message(
                        session,
                        Role::Assistant,
                        "",
                        2,
                        None,
                        Some(std::slice::from_ref(&wire)),
                    )
                    .await
                    .unwrap();
                let id = format!("a{}t1c0", prompt.id);
                store
                    .upsert_tool_step(session, turn.id, &id, TOOL_NAME, "Ask user", 2, None)
                    .await
                    .unwrap();
                let request = QuestionRequest {
                    questions: vec![Question {
                        id: "path".into(),
                        title: "What directory?".into(),
                        options: vec![],
                    }],
                };
                let create = QuestionCommand::Create {
                    id: id.clone(),
                    anchor: prompt.id,
                    request: request.clone(),
                };
                let first = store
                    .question_command(owner, session, &create, 2)
                    .await
                    .unwrap();
                assert!(first.questions[0].reply.is_none());
                assert_eq!(
                    store
                        .question_command(owner, session, &create, 3)
                        .await
                        .unwrap(),
                    first
                );
                let mut different = request.clone();
                different.questions[0].title = "Replace the prompt?".into();
                assert!(
                    store
                        .question_command(
                            owner,
                            session,
                            &QuestionCommand::Create {
                                id: id.clone(),
                                anchor: prompt.id,
                                request: different
                            },
                            3
                        )
                        .await
                        .is_err()
                );
                assert!(
                    store
                        .question_command(other, session, &QuestionCommand::List, 3)
                        .await
                        .is_err()
                );
                assert!(
                    store
                        .question_command(
                            owner,
                            session,
                            &QuestionCommand::Reply {
                                id: id.clone(),
                                reply: QuestionReply::Answer { answers: vec![] }
                            },
                            3
                        )
                        .await
                        .is_err()
                );
                drop(store);
                let store =
                    Store::new(crate::rusqlite_db::RusqliteDb::open(database.path()).unwrap());
                assert_eq!(
                    store
                        .question_command(owner, session, &QuestionCommand::List, 3)
                        .await
                        .unwrap(),
                    first
                );
                let reply = QuestionReply::Answer {
                    answers: vec![QuestionAnswer {
                        id: "path".into(),
                        value: AnswerValue::Text {
                            text: "/srv/media".into(),
                        },
                    }],
                };
                let saved = store
                    .question_command(
                        owner,
                        session,
                        &QuestionCommand::Reply {
                            id: id.clone(),
                            reply: reply.clone(),
                        },
                        3,
                    )
                    .await
                    .unwrap();
                assert_eq!(saved.questions[0].reply, Some(reply.clone()));
                assert!(
                    store
                        .question_command(
                            owner,
                            session,
                            &QuestionCommand::Reply {
                                id: id.clone(),
                                reply: reply.clone()
                            },
                            3
                        )
                        .await
                        .is_err()
                );
                let steps = store.list_tool_steps(session).await.unwrap();
                assert_eq!(steps[0].ok, Some(true));
                let history = openwebide_core::tool_history(vec![turn], &steps);
                assert_eq!(history.last().unwrap().role, Role::Tool);
                assert_eq!(
                    history.last().unwrap().tool_call_id.as_deref(),
                    Some("wire-call")
                );
                assert!(history.last().unwrap().content.contains("/srv/media"));
                // Late agent completion cannot replace the durable user answer.
                store
                    .complete_tool_step(owner, session, &id, false, "cancelled", None)
                    .await
                    .unwrap();
                assert_eq!(
                    store.list_tool_steps(session).await.unwrap()[0].result_summary,
                    steps[0].result_summary
                );
                let second = format!("a{}t2c0", prompt.id);
                store
                    .upsert_tool_step(session, prompt.id, &second, TOOL_NAME, "Ask", 4, None)
                    .await
                    .unwrap();
                store
                    .question_command(
                        owner,
                        session,
                        &QuestionCommand::Create {
                            id: second.clone(),
                            anchor: prompt.id,
                            request: request.clone(),
                        },
                        4,
                    )
                    .await
                    .unwrap();
                store
                    .question_command(
                        owner,
                        session,
                        &QuestionCommand::CancelRun { anchor: prompt.id },
                        5,
                    )
                    .await
                    .unwrap();
                assert_eq!(
                    store
                        .question_command(
                            owner,
                            session,
                            &QuestionCommand::Read { id: second.clone() },
                            5
                        )
                        .await
                        .unwrap()
                        .questions[0]
                        .reply,
                    Some(QuestionReply::Cancel)
                );
                assert!(
                    store
                        .question_command(
                            owner,
                            session,
                            &QuestionCommand::Reply {
                                id: second,
                                reply: reply.clone()
                            },
                            5
                        )
                        .await
                        .is_err()
                );
                let stale = format!("a{}t3c0", prompt.id);
                store
                    .upsert_tool_step(session, prompt.id, &stale, TOOL_NAME, "Ask", 6, None)
                    .await
                    .unwrap();
                store
                    .question_command(
                        owner,
                        session,
                        &QuestionCommand::Create {
                            id: stale.clone(),
                            anchor: prompt.id,
                            request,
                        },
                        6,
                    )
                    .await
                    .unwrap();
                store
                    .insert_message(session, Role::User, "New run", 7)
                    .await
                    .unwrap();
                assert!(
                    store
                        .question_command(
                            owner,
                            session,
                            &QuestionCommand::Reply {
                                id: stale.clone(),
                                reply
                            },
                            7
                        )
                        .await
                        .is_err()
                );
                assert!(
                    store
                        .question_command(owner, session, &QuestionCommand::Read { id: stale }, 7)
                        .await
                        .is_err()
                );
                assert!(
                    store
                        .question_command(owner, session, &QuestionCommand::List, 7)
                        .await
                        .unwrap()
                        .questions
                        .is_empty()
                );
            }
        });
    }
}
