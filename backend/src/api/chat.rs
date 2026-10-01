use super::*;
// -- models & chat ----------------------------------------------------------------

pub(crate) async fn list_models(
    req: Request,
    state: &AppState,
    _user: AuthedUser,
) -> Result<JsonResp, ApiError> {
    let params = query(&req);
    let id = params
        .get("connection_id")
        .and_then(|s| s.parse::<i64>().ok())
        .ok_or_else(|| ApiError::bad_request("missing ?connection_id=<id>"))?;
    let connection = state.store.get_connection(id).await?;
    let provider = Provider::for_connection(&connection, SpinHttpClient);
    let models = provider.list_models().await?;
    Ok(json_response(200, &models))
}

pub(crate) async fn chat(
    req: Request,
    state: &AppState,
    _user: AuthedUser,
) -> Result<JsonResp, ApiError> {
    let body = read_body(req, CHAT_BODY_LIMIT).await?;
    let mut request: ChatRequest = parse_json(body)?;
    let connection = state.store.get_connection(request.connection_id).await?;
    let provider = Provider::for_connection(&connection, SpinHttpClient);
    request.system_prompt = Some(with_temporal_context(request.system_prompt, now()));
    let reply = provider.chat(&request).await?;
    Ok(json_response(200, &json!({ "reply": reply })))
}

pub(crate) async fn chat_tools(
    req: Request,
    state: &AppState,
    _user: AuthedUser,
) -> Result<JsonResp, ApiError> {
    let body = read_body(req, CHAT_BODY_LIMIT).await?;
    let mut request: ChatRequest = parse_json(body)?;
    let connection = state.store.get_connection(request.connection_id).await?;
    let memo = ToolStreamMemo::new(connection.tool_stream_unsupported);
    let provider = Provider::for_connection_with_memo(&connection, SpinHttpClient, memo.clone());
    request.system_prompt = Some(with_temporal_context(request.system_prompt, now()));
    let response = provider.chat_tools(&request).await;
    if memo.take_unrecorded()
        && let Err(error) = state
            .store
            .set_tool_stream_unsupported(connection.id, connection.tool_stream_revision)
            .await
    {
        eprintln!(
            "connection {}: set_tool_stream_unsupported: {error}",
            connection.id
        );
    }
    let response = response?;
    Ok(json_response(200, &response))
}

pub(crate) async fn model_context(
    req: Request,
    state: &AppState,
    _user: AuthedUser,
) -> Result<JsonResp, ApiError> {
    let params = query(&req);
    let connection_id = params
        .get("connection_id")
        .and_then(|s| s.parse::<i64>().ok())
        .ok_or_else(|| ApiError::bad_request("missing ?connection_id=<id>"))?;
    let model = params.get("model").cloned();
    let connection = state.store.get_connection(connection_id).await?;
    let limit = if let Some(n) = connection.context_limit {
        Some(n)
    } else {
        let provider = Provider::for_connection(&connection, SpinHttpClient);
        provider
            .context_limit(model.as_deref())
            .await
            .unwrap_or(None)
    };
    Ok(json_response(200, &json!({ "context_limit": limit })))
}
