use super::*;
pub(crate) async fn command(
    req: Request,
    state: &AppState,
    path: &str,
    user: AuthedUser,
) -> Result<JsonResp, ApiError> {
    command_body(
        state,
        session_id(path)?,
        user,
        read_body(req, JSON_BODY_LIMIT).await?,
    )
    .await
}
async fn command_body(
    state: &AppState,
    session: i64,
    user: AuthedUser,
    body: String,
) -> Result<JsonResp, ApiError> {
    let command: openwebide_core::questions::QuestionCommand = parse_json(body)?;
    let result = state
        .store
        .question_command(user.id, session, &command, now())
        .await?;
    Ok(json_response(200, &result))
}

#[cfg(test)]
mod tests {
    use super::*;
    use openwebide_core::{Role, UserRole, questions::*};
    #[test]
    fn question_endpoint_uses_the_same_owned_contract_for_every_session_scope() {
        futures::executor::block_on(async {
            for mode in [
                Some(openwebide_core::WorkspaceMode::Local),
                Some(openwebide_core::WorkspaceMode::Remote),
                None,
            ] {
                let state = AppState::new().await.unwrap();
                let owner = state
                    .store
                    .insert_user("owner", "hash", UserRole::Admin, 1)
                    .await
                    .unwrap();
                let other = state
                    .store
                    .insert_user("other", "hash", UserRole::User, 1)
                    .await
                    .unwrap();
                let project = if let Some(mode) = mode {
                    Some(
                        state
                            .store
                            .create_project(
                                &NewProject {
                                    name: "project".into(),
                                    mode,
                                    path: Some("test".into()),
                                },
                                owner.id,
                                1,
                            )
                            .await
                            .unwrap()
                            .id,
                    )
                } else {
                    None
                };
                let session = state
                    .store
                    .create_session("chat", None, None, project, owner.id, 1)
                    .await
                    .unwrap()
                    .id;
                let anchor = state
                    .store
                    .insert_message(session, Role::User, "Task", 1)
                    .await
                    .unwrap()
                    .id;
                let id = format!("a{anchor}t1c0");
                state
                    .store
                    .upsert_tool_step(session, anchor, &id, TOOL_NAME, "Ask", 1, None)
                    .await
                    .unwrap();
                let send = |value: &QuestionCommand| serde_json::to_string(value).unwrap();
                let create = QuestionCommand::Create {
                    id: id.clone(),
                    anchor,
                    request: QuestionRequest {
                        questions: vec![Question {
                            id: "path".into(),
                            title: "Which directory?".into(),
                            options: vec![],
                        }],
                    },
                };
                let user = AuthedUser {
                    id: owner.id,
                    role: owner.role,
                };
                assert_eq!(
                    command_body(&state, session, user, send(&create))
                        .await
                        .unwrap()
                        .status(),
                    200
                );
                let foreign = AuthedUser {
                    id: other.id,
                    role: other.role,
                };
                assert_eq!(
                    command_body(&state, session, foreign, send(&QuestionCommand::List))
                        .await
                        .unwrap_err()
                        .into_response()
                        .status(),
                    404
                );
                let reply = QuestionCommand::Reply {
                    id,
                    reply: QuestionReply::Answer {
                        answers: vec![QuestionAnswer {
                            id: "path".into(),
                            value: AnswerValue::Text {
                                text: "/srv/media".into(),
                            },
                        }],
                    },
                };
                assert_eq!(
                    command_body(&state, session, user, send(&reply))
                        .await
                        .unwrap()
                        .status(),
                    200
                );
                assert_eq!(
                    command_body(&state, session, user, send(&reply))
                        .await
                        .unwrap_err()
                        .into_response()
                        .status(),
                    409
                );
                assert!(
                    state.store.list_tool_steps(session).await.unwrap()[0]
                        .result_summary
                        .as_ref()
                        .unwrap()
                        .contains("/srv/media")
                );
            }
        });
    }
}
