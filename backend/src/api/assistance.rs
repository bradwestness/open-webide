//! Shared assistance facade; model transport and persistence are host primitives.
use super::*;

pub(crate) async fn generate(
    req: Request,
    state: &AppState,
    user: AuthedUser,
) -> Result<JsonResp, ApiError> {
    let request: openwebide_core::AssistanceRequest =
        parse_json(read_body(req, JSON_BODY_LIMIT).await?)?;
    let result = execute(&state.store, user.id, &request).await?;
    Ok(json_response(200, &result))
}

pub(crate) async fn execute(
    store: &openwebide_storage::Store<crate::state::AppDb>,
    user: UserId,
    request: &openwebide_core::AssistanceRequest,
) -> Result<Option<String>, ApiError> {
    request.validate().map_err(ApiError::bad_request)?;
    if let Some(project) = request.project_id {
        store.get_project(project, user).await?;
    }
    if let Some(session) = request.session_id {
        let session = store.get_session(session, user).await?;
        if session.project_id != request.project_id {
            return Err(ApiError::bad_request(
                "Session belongs to a different project",
            ));
        }
    }
    // Refresh cached text when its generation instructions change.
    let key = serde_json::to_string(&(request, request.kind.instruction()))
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    if let Some(result) = store.cached_assistance(user, &key).await? {
        return Ok(Some(result));
    }
    let runtime =
        super::model_setup::runtime_store(store, user, request.connection_id, None).await?;
    let source = super::model_operations::ModelSource { store, user };
    let result =
        openwebide_agent::assistance::generate_text(&source, runtime, request.kind, &request.input)
            .await
            .ok();
    if let Some(result) = &result {
        store.cache_assistance(user, &key, result, now()).await?;
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use openwebide_core::{AssistanceKind, AssistanceRequest, UserRole};

    #[test]
    fn summary_cache_refreshes_when_instructions_change() {
        futures::executor::block_on(async {
            let state = AppState::new().await.unwrap();
            let user = state
                .store
                .insert_user("owner", "hash", UserRole::Admin, 1)
                .await
                .unwrap()
                .id;
            for kind in [AssistanceKind::Recap, AssistanceKind::Completion] {
                let request = AssistanceRequest {
                    kind,
                    connection_id: 999,
                    session_id: None,
                    project_id: None,
                    input: "Shared science puns, dad jokes and animal jokes.".into(),
                };
                let old_key = serde_json::to_string(&request).unwrap();
                let older_instruction_key =
                    serde_json::to_string(&(&request, "Earlier summary instructions")).unwrap();
                for key in [&old_key, &older_instruction_key] {
                    state
                        .store
                        .cache_assistance(
                            user,
                            key,
                            "The user successfully received jokes with no blockers.",
                            1,
                        )
                        .await
                        .unwrap();
                }
                // No model connection exists: stale text must miss the cache and reach runtime lookup.
                assert!(execute(&state.store, user, &request).await.is_err());
                let current_key = serde_json::to_string(&(&request, kind.instruction())).unwrap();
                state
                    .store
                    .cache_assistance(
                        user,
                        &current_key,
                        "Science puns, dad jokes and animal jokes.",
                        2,
                    )
                    .await
                    .unwrap();
                assert_eq!(
                    execute(&state.store, user, &request)
                        .await
                        .unwrap()
                        .as_deref(),
                    Some("Science puns, dad jokes and animal jokes.")
                );
            }
        });
    }
}
