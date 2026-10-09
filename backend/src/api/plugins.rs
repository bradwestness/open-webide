//! Authenticated database and remote bridge transport for plugin installation.
use super::*;
use openwebide_core::plugins::{PluginSource, PreparedPlugin, RecordPlugin};

pub(crate) async fn list(state: &AppState, user: AuthedUser) -> Result<JsonResp, ApiError> {
    Ok(json_response(
        200,
        &state.store.plugin_installations(user.id).await?,
    ))
}
pub(crate) async fn record(
    req: Request,
    state: &AppState,
    user: AuthedUser,
) -> Result<JsonResp, ApiError> {
    let request: RecordPlugin = parse_json(read_body(req, 256 * 1024).await?)?;
    Ok(json_response(
        200,
        &state.store.record_plugin(user.id, &request, now()).await?,
    ))
}
pub(crate) async fn prepare(
    req: Request,
    state: &AppState,
    path: &str,
    user: AuthedUser,
) -> Result<JsonResp, ApiError> {
    let project = path_id(
        path.strip_suffix("/plugins/prepare")
            .ok_or_else(|| ApiError::bad_request("Expected plugin path"))?,
        "/api/projects",
    )?;
    super::files::remote_project_path(state, user.id, project, "").await?;
    let source: PluginSource = parse_json(read_body(req, 16 * 1024).await?)?;
    source
        .validate()
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    let (status, body) = crate::bridge::send(
        &state.store,
        "/plugins/prepare",
        json!({"source":source,"user":user.id.get()}).to_string(),
    )
    .await?;
    if status != 200 {
        return Err(if status == 400 {
            ApiError::bad_request(String::from_utf8_lossy(&body))
        } else {
            ApiError::bad_gateway("Plugin preparation failed on the execution host.")
        });
    }
    let prepared: PreparedPlugin =
        serde_json::from_slice(&body).map_err(|error| ApiError::internal(error.to_string()))?;
    prepared
        .validate()
        .map_err(|error| ApiError::internal(error.to_string()))?;
    if prepared.source != source {
        return Err(ApiError::internal(
            "Plugin host returned a different source.",
        ));
    }
    Ok(json_response(200, &prepared))
}
