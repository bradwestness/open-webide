use super::*;
// -- connections ---------------------------------------------------------------

pub(crate) async fn list_connections(
    state: &AppState,
    _user: AuthedUser,
) -> Result<JsonResp, ApiError> {
    let connections = state.store.list_connections().await?;
    Ok(json_response(200, &connections))
}

pub(super) fn validate_context_limit(limit: Option<usize>) -> Result<(), ApiError> {
    if limit == Some(0) {
        return Err(ApiError::bad_request(
            "context limit must be a positive number of tokens",
        ));
    }
    Ok(())
}

pub(crate) async fn create_connection(
    req: Request,
    state: &AppState,
    _user: AuthedUser,
) -> Result<JsonResp, ApiError> {
    let body = read_body(req, JSON_BODY_LIMIT).await?;
    let new: NewConnection = parse_json(body)?;
    validate_context_limit(new.context_limit)?;
    let connection = state.store.insert_connection(&new).await?;
    Ok(json_response(201, &connection))
}

pub(crate) async fn update_connection(
    req: Request,
    state: &AppState,
    path: &str,
    _user: AuthedUser,
) -> Result<JsonResp, ApiError> {
    let id = path_id(path, "/api/connections")?;
    let body = read_body(req, JSON_BODY_LIMIT).await?;
    let mut connection: openwebide_core::Connection = parse_json(body)?;
    validate_context_limit(connection.context_limit)?;
    connection.id = id;
    state.store.update_connection(&connection).await?;
    let connection = state.store.get_connection(id).await?;
    Ok(json_response(200, &connection))
}

pub(crate) async fn set_tool_stream_unsupported(
    req: Request,
    state: &AppState,
    path: &str,
    _user: AuthedUser,
) -> Result<JsonResp, ApiError> {
    let root = path
        .strip_suffix("/tool-stream-unsupported")
        .ok_or_else(|| ApiError::bad_request("invalid connection route"))?;
    let id = path_id(root, "/api/connections")?;
    #[derive(Deserialize)]
    struct MemoBody {
        tool_stream_revision: i64,
    }
    let body: MemoBody = parse_json(read_body(req, JSON_BODY_LIMIT).await?)?;
    state
        .store
        .set_tool_stream_unsupported(id, body.tool_stream_revision)
        .await?;
    Ok(json_response(200, &json!({})))
}

pub(crate) async fn delete_connection(
    state: &AppState,
    path: &str,
    _user: AuthedUser,
) -> Result<JsonResp, ApiError> {
    let id = path_id(path, "/api/connections")?;
    state.store.delete_connection(id).await?;
    Ok(json_response(200, &json!({ "deleted": id })))
}
