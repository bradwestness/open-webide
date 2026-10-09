use super::*;
use openwebide_core::SkillCommand;

pub(crate) async fn get(
    state: &AppState,
    path: &str,
    user: AuthedUser,
    session: bool,
) -> Result<JsonResp, ApiError> {
    let id = path_id(
        path.strip_suffix("/skills")
            .ok_or_else(|| ApiError::bad_request("Expected skills path"))?,
        if session {
            "/api/sessions"
        } else {
            "/api/projects"
        },
    )?;
    let skills = if session {
        state.store.session_skills(user.id, id).await?
    } else {
        state.store.project_skills(user.id, id).await?
    };
    Ok(json_response(200, &skills))
}
pub(crate) async fn command(
    req: Request,
    state: &AppState,
    path: &str,
    user: AuthedUser,
    session: bool,
) -> Result<JsonResp, ApiError> {
    let id = path_id(
        path.strip_suffix("/skills")
            .ok_or_else(|| ApiError::bad_request("Expected skills path"))?,
        if session {
            "/api/sessions"
        } else {
            "/api/projects"
        },
    )?;
    let command: SkillCommand = parse_json(read_body(req, 1024 * 1024).await?)?;
    let result = if session {
        state
            .store
            .session_skill_command(user.id, id, &command, now())
            .await?
    } else {
        state
            .store
            .skill_command(user.id, id, &command, false, now())
            .await?
    };
    Ok(json_response(200, &result))
}
