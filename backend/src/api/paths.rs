use super::*;
pub(super) fn project_files_path(path: &str) -> Result<(i64, &str), ApiError> {
    let rest = path
        .strip_prefix("/api/projects/")
        .ok_or_else(|| ApiError::bad_request("expected /api/projects/<id>/files..."))?;
    let (id, sub) = rest
        .split_once('/')
        .ok_or_else(|| ApiError::bad_request("expected /api/projects/<id>/files..."))?;
    let id = id
        .parse::<i64>()
        .map_err(|_| ApiError::bad_request("expected a numeric project id"))?;
    if sub.split('/').next() != Some("files") {
        return Err(ApiError::bad_request("expected a /files sub-path"));
    }
    Ok((id, sub))
}

pub(super) fn session_id(path: &str) -> Result<i64, ApiError> {
    path.strip_prefix("/api/sessions/")
        .and_then(|rest| rest.split('/').next())
        .and_then(|id| id.parse::<i64>().ok())
        .ok_or_else(|| ApiError::bad_request("expected /api/sessions/<id>..."))
}

pub(super) fn permission_path(path: &str) -> Result<(i64, &str), ApiError> {
    let id = session_id(path)?;
    let tool_call_id = path
        .strip_prefix("/api/sessions/")
        .and_then(|rest| rest.split_once('/'))
        .and_then(|(_, sub)| sub.strip_prefix("permissions/"))
        .filter(|rest| !rest.is_empty() && !rest.contains('/'))
        .ok_or_else(|| {
            ApiError::bad_request("expected /api/sessions/<id>/permissions/<tool_call_id>")
        })?;
    Ok((id, tool_call_id))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_paths() {
        assert_eq!(
            project_files_path("/api/projects/7/files/raw").unwrap(),
            (7, "files/raw")
        );
        assert!(project_files_path("/api/projects/x/files").is_err());
        assert_eq!(session_id("/api/sessions/5/messages").unwrap(), 5);
        assert!(session_id("/api/sessions/no/messages").is_err());
        assert_eq!(
            permission_path("/api/sessions/5/permissions/a1t1c0").unwrap(),
            (5, "a1t1c0")
        );
        assert!(permission_path("/api/sessions/5/permissions/a/b").is_err());
    }
}
