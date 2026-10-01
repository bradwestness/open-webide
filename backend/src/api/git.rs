use super::files::remote_project_path;
use super::*;
// -- git operations (Phase 13) -----------------------------------------------------

pub(super) fn project_git_path(path: &str) -> Result<(Option<i64>, &str), ApiError> {
    if let Some(rest) = path.strip_prefix("/api/projects/") {
        let (id_str, sub) = rest
            .split_once('/')
            .ok_or_else(|| ApiError::bad_request("expected /api/projects/<id>/git/..."))?;
        let id = id_str
            .parse::<i64>()
            .map_err(|_| ApiError::bad_request("expected a numeric project id"))?;
        let git_sub = sub
            .strip_prefix("git/")
            .ok_or_else(|| ApiError::bad_request("expected /git/ sub-path"))?;
        Ok((Some(id), git_sub))
    } else if let Some(git_sub) = path.strip_prefix("/api/git/") {
        Ok((None, git_sub))
    } else {
        Err(ApiError::bad_request("unrecognized git path"))
    }
}

pub(crate) async fn git_get(
    req: Request,
    state: &AppState,
    path: &str,
    user: AuthedUser,
) -> Result<JsonResp, ApiError> {
    let params = query(&req);
    let user_id = user.id;
    let (project_id, sub) = project_git_path(path)?;
    let project_dir = if let Some(id) = project_id {
        let (full, _) = remote_project_path(state, user_id, id, "").await?;
        full
    } else {
        String::new()
    };

    match sub {
        "status" => {
            let status = crate::git::repo_status(&state.store, &project_dir).await?;
            Ok(json_response(200, &status))
        }
        "diff" => {
            let file_path = params.get("path").cloned();
            let diff =
                crate::git::repo_diff(&state.store, &project_dir, file_path.as_deref()).await?;
            Ok(json_response(200, &json!({ "diff": diff })))
        }
        "branches" => {
            let branches = crate::git::repo_branches(&state.store, &project_dir).await?;
            Ok(json_response(200, &branches))
        }
        "show" => {
            let file_path = params
                .get("path")
                .cloned()
                .ok_or_else(|| ApiError::bad_request("missing path query parameter"))?;
            let content =
                crate::git::repo_file_head(&state.store, &project_dir, &file_path).await?;
            match content {
                crate::git::Blob::Text(content) => {
                    Ok(json_response(200, &json!({ "content": content })))
                }
                crate::git::Blob::Binary(_bytes) => Err(ApiError::new(415, "binary file at HEAD")),
            }
        }
        other => Err(ApiError::not_found(format!("unknown git action: {other}"))),
    }
}

pub(crate) async fn git_post(
    req: Request,
    state: &AppState,
    path: &str,
    user: AuthedUser,
) -> Result<JsonResp, ApiError> {
    let user_id = user.id;
    let (project_id, sub) = project_git_path(path)?;
    let project_dir = if let Some(id) = project_id {
        let (full, _) = remote_project_path(state, user_id, id, "").await?;
        full
    } else {
        String::new()
    };

    let body = read_body(req, JSON_BODY_LIMIT).await?;

    match sub {
        "status" => {
            let status = crate::git::repo_status(&state.store, &project_dir).await?;
            Ok(json_response(200, &status))
        }
        "diff" => {
            #[derive(Deserialize, Default)]
            struct DiffReq {
                path: Option<String>,
            }
            let diff_req: DiffReq = if body.trim().is_empty() {
                DiffReq::default()
            } else {
                parse_json(body)?
            };
            let diff =
                crate::git::repo_diff(&state.store, &project_dir, diff_req.path.as_deref()).await?;
            Ok(json_response(200, &json!({ "diff": diff })))
        }
        "branches" => {
            let branches = crate::git::repo_branches(&state.store, &project_dir).await?;
            Ok(json_response(200, &branches))
        }
        "commit" => {
            let commit_req: GitCommitRequest = parse_json(body)?;
            let result = crate::git::repo_commit(&state.store, &project_dir, &commit_req).await?;
            Ok(json_response(200, &result))
        }
        "checkout" => {
            let checkout_req: GitCheckoutRequest = parse_json(body)?;
            let result =
                crate::git::repo_checkout(&state.store, &project_dir, &checkout_req).await?;
            Ok(json_response(200, &result))
        }
        "sync" => {
            let sync_req: GitSyncRequest = if body.trim().is_empty() {
                GitSyncRequest {
                    action: "sync".into(),
                    remote: None,
                    branch: None,
                }
            } else {
                parse_json(body)?
            };
            let result = crate::git::repo_sync(&state.store, &project_dir, &sync_req).await?;
            Ok(json_response(200, &result))
        }
        other => Err(ApiError::not_found(format!("unknown git action: {other}"))),
    }
}
