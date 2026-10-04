use super::*;
use openwebide_core::ReviewRequest;
fn project_id(path: &str) -> Result<i64, ApiError> {
    let path = path
        .strip_prefix("/api/projects/")
        .ok_or_else(|| ApiError::bad_request("invalid project path"))?;
    let id = path.split('/').next().unwrap_or_default();
    path_id(&format!("/api/projects/{id}"), "/api/projects")
}
pub(crate) async fn list(
    state: &AppState,
    path: &str,
    user: AuthedUser,
) -> Result<JsonResp, ApiError> {
    Ok(json_response(
        200,
        &state
            .store
            .list_run_changes(user.id, project_id(path)?)
            .await?,
    ))
}
pub(crate) async fn review(
    req: Request,
    state: &AppState,
    path: &str,
    user: AuthedUser,
) -> Result<JsonResp, ApiError> {
    let project = project_id(path)?;
    let request: ReviewRequest = parse_json(read_body(req, JSON_BODY_LIMIT).await?)?;
    if path.ends_with("/complete") {
        return Ok(json_response(
            200,
            &state
                .store
                .complete_run_review(user.id, project, &request)
                .await?,
        ));
    }
    let plan = if path.ends_with("/preview") {
        state
            .store
            .preview_run_review(user.id, project, &request)
            .await?
    } else {
        state
            .store
            .prepare_run_review(user.id, project, &request)
            .await?
    };
    Ok(json_response(200, &plan))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn review_routes_extract_the_project_from_the_full_api_path() {
        for suffix in [
            "run-changes",
            "reviews/preview",
            "reviews/prepare",
            "reviews/complete",
        ] {
            assert_eq!(project_id(&format!("/api/projects/7/{suffix}")).unwrap(), 7);
        }
        assert!(project_id("/api/projects/nope/run-changes").is_err());
        assert!(project_id("/projects/7/run-changes").is_err());
    }
}
