use super::*;
use openwebide_core::scheduled::{DispatchResult, ExecutionHost, HostBinding, TaskCommand};
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CommandBody {
    project_id: Option<i64>,
    command: TaskCommand,
    #[serde(default)]
    binding: Option<HostBinding>,
}
pub(crate) async fn list(
    state: &AppState,
    query: Option<&str>,
    user: AuthedUser,
) -> Result<JsonResp, ApiError> {
    let project = query
        .and_then(|query| query.strip_prefix("project_id="))
        .map(str::parse::<i64>)
        .transpose()
        .map_err(|_| ApiError::bad_request("Invalid project"))?;
    Ok(json_response(
        200,
        &state.store.scheduled_tasks(user.id, project, now()).await?,
    ))
}
pub(crate) async fn command(
    req: Request,
    state: &AppState,
    user: AuthedUser,
) -> Result<JsonResp, ApiError> {
    let body: CommandBody = parse_json(read_body(req, 64 * 1024).await?)?;
    let command =
        super::naming::task(&state.store, user.id, body.project_id, body.command, false).await?;
    Ok(json_response(
        200,
        &state
            .store
            .scheduled_command(
                user.id,
                body.project_id,
                &command,
                body.binding.as_ref(),
                false,
                now(),
            )
            .await?,
    ))
}
pub(crate) async fn session_command(
    req: Request,
    state: &AppState,
    path: &str,
    user: AuthedUser,
) -> Result<JsonResp, ApiError> {
    let command: TaskCommand = parse_json(read_body(req, 64 * 1024).await?)?;
    let session = session_id(path)?;
    let project = state.store.get_session(session, user.id).await?.project_id;
    let mut command = command;
    if let TaskCommand::Create { draft } | TaskCommand::Update { draft, .. } = &mut command
        && draft.session_target == openwebide_core::scheduled::SessionTarget::Existing
        && draft.session_id == 0
    {
        draft.session_id = session;
    }
    let command = super::naming::task(&state.store, user.id, project, command, true).await?;
    Ok(json_response(
        200,
        &state
            .store
            .scheduled_session_command(user.id, session_id(path)?, &command, now())
            .await?,
    ))
}
pub(crate) async fn due(req: Request, state: &AppState) -> Result<JsonResp, ApiError> {
    let host: ExecutionHost = parse_json(read_body(req, 4096).await?)?;
    Ok(json_response(
        200,
        &state.store.due_scheduled(&host, now()).await?,
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResultBody {
    host_id: String,
    result: DispatchResult,
}
pub(crate) async fn result(req: Request, state: &AppState) -> Result<JsonResp, ApiError> {
    let mut body: ResultBody = parse_json(read_body(req, 4096).await?)?;
    if body.result.status == "complete"
        && let Some((user, session, anchor)) = state
            .store
            .scheduled_run_session(&body.host_id, body.result.run_id)
            .await?
    {
        let messages = state.store.list_messages(session).await?;
        let final_message = super::completion::after_prompt(&messages, anchor);
        if let Some(message) = final_message
            && let Some(summary) =
                super::completion::summary(&state.store, user, session, Some(message)).await
        {
            body.result.detail = summary;
        }
    }
    state
        .store
        .scheduled_result(&body.host_id, &body.result, now())
        .await?;
    Ok(json_response(200, &json!({"ok":true})))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LeaseBody {
    token: String,
    release: bool,
    #[serde(default)]
    since: i64,
    #[serde(default)]
    permission_id: Option<String>,
}
pub(crate) async fn lease(
    req: Request,
    state: &AppState,
    path: &str,
    user: AuthedUser,
) -> Result<JsonResp, ApiError> {
    let body: LeaseBody = parse_json(read_body(req, 4096).await?)?;
    let session = session_id(path)?;
    state
        .store
        .session_run_lease(user.id, session, &body.token, body.release, now())
        .await?;
    let cancelled = state
        .store
        .cancel_requested_since(session, body.since)
        .await?;
    let approved = if let Some(id) = body.permission_id {
        state.store.take_tool_permission(session, &id).await?
    } else {
        None
    };
    Ok(json_response(
        200,
        &openwebide_core::scheduled::RunControl {
            cancelled,
            approved,
        },
    ))
}
