use super::*;
// -- projects --------------------------------------------------------------------

pub(crate) async fn list_projects(
    state: &AppState,
    user: AuthedUser,
) -> Result<JsonResp, ApiError> {
    let user_id = user.id;
    let projects = state.store.list_projects(user_id).await?;
    Ok(json_response(200, &projects))
}

pub(super) fn normalize_project_path(
    mode: WorkspaceMode,
    path: Option<String>,
) -> Result<Option<String>, ApiError> {
    match mode {
        WorkspaceMode::Local => Ok(path),
        WorkspaceMode::Remote => {
            let Some(p) = path else { return Ok(None) };
            if p.is_empty() {
                return Ok(Some(String::new()));
            }
            openwebide_core::normalize_vfs_path(&p)
                .map(Some)
                .map_err(|_| ApiError::bad_request("project path escapes the workspace root"))
        }
    }
}

pub(crate) async fn create_project(
    req: Request,
    state: &AppState,
    user: AuthedUser,
) -> Result<JsonResp, ApiError> {
    let user_id = user.id;
    let body = read_body(req, JSON_BODY_LIMIT).await?;
    let mut new: NewProject = parse_json(body)?;
    new.path = normalize_project_path(new.mode, new.path)?;
    let project = state.store.create_project(&new, user_id, now()).await?;
    Ok(json_response(201, &project))
}

#[derive(Deserialize)]
pub(super) struct RenameProjectBody {
    name: String,
}

pub(crate) async fn rename_project(
    req: Request,
    state: &AppState,
    path: &str,
    user: AuthedUser,
) -> Result<JsonResp, ApiError> {
    let user_id = user.id;
    let id = path_id(path, "/api/projects")?;
    let body = read_body(req, JSON_BODY_LIMIT).await?;
    let rename: RenameProjectBody = parse_json(body)?;
    let project = state
        .store
        .rename_project(id, &rename.name, user_id)
        .await?;
    Ok(json_response(200, &project))
}

pub(crate) async fn delete_project(
    state: &AppState,
    path: &str,
    user: AuthedUser,
) -> Result<JsonResp, ApiError> {
    let user_id = user.id;
    let id = path_id(path, "/api/projects")?;
    state.store.delete_project(id, user_id).await?;
    Ok(json_response(200, &json!({ "deleted": id })))
}

// -- host file browser (Phase 10) --------------------------------------------------

/// List a directory of the host mount (the preopen root, e.g. `~/source`)
/// for the remote file browser. `?path=` is relative to the mount root;
/// empty = the root itself. Paths that would escape the root are rejected by
/// the filesystem layer, so this only ever exposes the mounted folder.
pub(crate) async fn browse(
    req: Request,
    _state: &AppState,
    _user: AuthedUser,
) -> Result<JsonResp, ApiError> {
    let params = query(&req);
    let rel = params.get("path").cloned().unwrap_or_default();
    let entries = crate::files::list(&rel).await?;
    Ok(json_response(200, &entries))
}
