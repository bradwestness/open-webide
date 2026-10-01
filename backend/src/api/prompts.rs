use super::*;
// -- system prompts --------------------------------------------------------------

pub(crate) async fn list_system_prompts(
    state: &AppState,
    _user: AuthedUser,
) -> Result<JsonResp, ApiError> {
    let prompts = state.store.list_system_prompts().await?;
    Ok(json_response(200, &prompts))
}

#[derive(Deserialize)]
pub(super) struct PromptBody {
    name: String,
    content: String,
}

pub(crate) async fn create_system_prompt(
    req: Request,
    state: &AppState,
    _user: AuthedUser,
) -> Result<JsonResp, ApiError> {
    let body = read_body(req, JSON_BODY_LIMIT).await?;
    let prompt_body: PromptBody = parse_json(body)?;
    let prompt: SystemPrompt = state
        .store
        .insert_system_prompt(&prompt_body.name, &prompt_body.content)
        .await?;
    Ok(json_response(201, &prompt))
}

pub(crate) async fn update_system_prompt(
    req: Request,
    state: &AppState,
    path: &str,
    _user: AuthedUser,
) -> Result<JsonResp, ApiError> {
    let id = path_id(path, "/api/system-prompts")?;
    let body = read_body(req, JSON_BODY_LIMIT).await?;
    let prompt_body: PromptBody = parse_json(body)?;
    let prompt: SystemPrompt = state
        .store
        .update_system_prompt(id, &prompt_body.name, &prompt_body.content)
        .await?;
    Ok(json_response(200, &prompt))
}

pub(crate) async fn delete_system_prompt(
    state: &AppState,
    path: &str,
    _user: AuthedUser,
) -> Result<JsonResp, ApiError> {
    let id = path_id(path, "/api/system-prompts")?;
    state.store.delete_system_prompt(id).await?;
    Ok(json_response(200, &json!({ "deleted": id })))
}
