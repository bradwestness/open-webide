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
    let key = serde_json::to_string(&request)
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
