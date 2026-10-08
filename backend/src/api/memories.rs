use super::*;
use openwebide_core::MemoryCommand;

pub(crate) async fn get(
    state: &AppState,
    path: &str,
    user: AuthedUser,
    session: bool,
) -> Result<JsonResp, ApiError> {
    let id = path_id(
        path.strip_suffix("/memories")
            .ok_or_else(|| ApiError::bad_request("Expected memories path"))?,
        if session {
            "/api/sessions"
        } else {
            "/api/projects"
        },
    )?;
    let memories = if session {
        state.store.session_memories(user.id, id).await?
    } else {
        state.store.project_memories(user.id, id).await?
    };
    Ok(json_response(200, &memories))
}
pub(crate) async fn command(
    req: Request,
    state: &AppState,
    path: &str,
    user: AuthedUser,
    session: bool,
) -> Result<JsonResp, ApiError> {
    let id = path_id(
        path.strip_suffix("/memories")
            .ok_or_else(|| ApiError::bad_request("Expected memories path"))?,
        if session {
            "/api/sessions"
        } else {
            "/api/projects"
        },
    )?;
    let command: MemoryCommand = parse_json(read_body(req, 32 * 1024).await?)?;
    let project = if session {
        state
            .store
            .get_session(id, user.id)
            .await?
            .project_id
            .ok_or_else(|| ApiError::bad_request("Project memory requires a project"))?
    } else {
        id
    };
    let command = super::naming::memory(
        &state.store,
        user.id,
        project,
        session.then_some(id),
        command,
    )
    .await?;
    let result = if session {
        state
            .store
            .session_memory_command(user.id, id, &command, now())
            .await?
    } else {
        state
            .store
            .memory_command(user.id, id, &command, false, now())
            .await?
    };
    Ok(json_response(200, &result))
}
