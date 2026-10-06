//! Recovery persistence is independent of the project's filesystem adapter.
use super::*;
use openwebide_core::editor::{EditorRecoveryRecord, recovery_body_limit};

fn project_id(path: &str) -> Result<i64, ApiError> {
    let path = path
        .strip_suffix("/editor-recovery")
        .ok_or_else(|| ApiError::bad_request("Invalid editor recovery path"))?;
    path_id(path, "/api/projects")
}

pub(crate) async fn get(
    state: &AppState,
    path: &str,
    user: AuthedUser,
) -> Result<JsonResp, ApiError> {
    let record = state
        .store
        .editor_recovery(user.id, project_id(path)?)
        .await?;
    Ok(json_response(200, &record))
}

pub(crate) async fn save(
    req: Request,
    state: &AppState,
    path: &str,
    user: AuthedUser,
) -> Result<JsonResp, ApiError> {
    let project = project_id(path)?;
    // Check ownership before accepting a potentially large draft body.
    state.store.get_project(project, user.id).await?;
    let body = read_body(req, recovery_body_limit()).await?;
    let record: EditorRecoveryRecord = parse_json(body)?;
    save_record(state, project, user, record).await
}

pub(super) async fn save_record(
    state: &AppState,
    project: i64,
    user: AuthedUser,
    record: EditorRecoveryRecord,
) -> Result<JsonResp, ApiError> {
    record.state.validate().map_err(ApiError::bad_request)?;
    if record.revision < 0 {
        return Err(ApiError::bad_request("Invalid editor recovery revision"));
    }
    let revision = state
        .store
        .save_editor_recovery(user.id, project, record.revision, &record.state)
        .await?;
    Ok(json_response(200, &json!({ "revision": revision })))
}
