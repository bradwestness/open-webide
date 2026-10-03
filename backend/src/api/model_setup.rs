use super::*;
use openwebide_core::{ModelDefaults, ModelProfile, ModelRuntime, ServerSettingsUpdate};

pub(crate) async fn get(state: &AppState, user: AuthedUser) -> Result<JsonResp, ApiError> {
    Ok(json_response(200, &state.store.model_setup(user.id).await?))
}
pub(crate) async fn defaults(
    req: Request,
    state: &AppState,
    user: AuthedUser,
) -> Result<JsonResp, ApiError> {
    let defaults: ModelDefaults = parse_json(read_body(req, JSON_BODY_LIMIT).await?)?;
    state.store.save_model_defaults(user.id, &defaults).await?;
    // Keep the old default-server preference in sync for older clients.
    if let Some(primary) = &defaults.primary {
        state
            .store
            .set_user_setting(
                user.id,
                "default_connection",
                &primary.server_id.to_string(),
            )
            .await?;
    }
    get(state, user).await
}
pub(crate) async fn profile(
    req: Request,
    state: &AppState,
    user: AuthedUser,
) -> Result<JsonResp, ApiError> {
    let profile: ModelProfile = parse_json(read_body(req, JSON_BODY_LIMIT).await?)?;
    state.store.save_model_profile(&profile).await?;
    get(state, user).await
}
pub(crate) async fn settings(
    req: Request,
    state: &AppState,
    path: &str,
    write: bool,
) -> Result<JsonResp, ApiError> {
    let id = server_id(path)?;
    if write {
        let update: ServerSettingsUpdate = parse_json(read_body(req, JSON_BODY_LIMIT).await?)?;
        state.store.save_server_settings(id, &update).await?;
    }
    Ok(json_response(200, &state.store.server_settings(id).await?))
}
pub(crate) fn server_id(path: &str) -> Result<i64, ApiError> {
    path.strip_prefix("/api/connections/")
        .and_then(|rest| rest.split('/').next())
        .and_then(|id| id.parse().ok())
        .ok_or_else(|| ApiError::bad_request("Expected a server id."))
}

pub(crate) async fn runtime(
    state: &AppState,
    user: UserId,
    id: i64,
    model: Option<&str>,
) -> Result<ModelRuntime, ApiError> {
    runtime_store(&state.store, user, id, model).await
}
pub(crate) async fn runtime_store(
    store: &openwebide_storage::Store<crate::state::AppDb>,
    user: UserId,
    id: i64,
    model: Option<&str>,
) -> Result<ModelRuntime, ApiError> {
    let mut connection = store.get_connection(id).await?;
    if !connection.enabled {
        return Err(ApiError::bad_request("Server is disabled."));
    }
    let transport = store.server_transport(id).await?;
    let setup = store.model_setup(user).await?;
    let model = model
        .map(str::to_string)
        .or_else(|| {
            setup
                .defaults
                .primary
                .as_ref()
                .filter(|selection| selection.server_id == id)
                .map(|selection| selection.model.clone())
        })
        .or_else(|| connection.model.clone());
    let selected_model = model;
    let model = selected_model.clone().unwrap_or_default();
    let explicit = setup
        .profiles
        .iter()
        .any(|profile| profile.selection.server_id == id && profile.selection.model == model);
    let mut settings = setup.resolve(id, &model);
    if let Some(cached) = store
        .get_setting(&format!("model_detection_{id}_{model}"))
        .await?
        && let Ok(cached) = serde_json::from_str::<serde_json::Value>(&cached)
        && cached.get("revision").and_then(serde_json::Value::as_i64)
            == Some(connection.tool_stream_revision)
        && let Some(detected) = cached.get("detection").and_then(|value| {
            serde_json::from_value::<openwebide_core::ModelDetection>(value.clone()).ok()
        })
    {
        if settings.context_limit.is_none() {
            settings.context_limit = detected.context_limit;
        }
        if settings.tools.is_none()
            && !detected.capabilities.is_empty()
            && !detected
                .capabilities
                .iter()
                .any(|capability| capability == "tools")
        {
            settings.tools = Some(false);
        }
    }
    if !explicit && connection.model.as_deref() == Some(&model) {
        settings.context_limit = settings.context_limit.or(connection.context_limit);
    }
    connection.context_limit = settings.context_limit;
    connection.model = selected_model;
    Ok(ModelRuntime {
        connection,
        settings,
        transport,
    })
}

pub(crate) async fn native_runtime(
    req: Request,
    state: &AppState,
    path: &str,
    user: AuthedUser,
    native: bool,
) -> Result<JsonResp, ApiError> {
    let params = query(&req);
    let runtime = runtime(
        state,
        user.id,
        server_id(path)?,
        params.get("model").map(String::as_str),
    )
    .await?;
    Ok(runtime_response(runtime, native))
}

pub(crate) fn runtime_response(mut runtime: ModelRuntime, native: bool) -> JsonResp {
    if !native {
        runtime.transport = Default::default();
    }
    json_response(200, &runtime)
}

#[derive(Deserialize)]
struct DetectBody {
    server_id: i64,
    model: String,
}
pub(crate) async fn detect(
    req: Request,
    state: &AppState,
    _user: AuthedUser,
) -> Result<JsonResp, ApiError> {
    let body: DetectBody = parse_json(read_body(req, JSON_BODY_LIMIT).await?)?;
    let connection = state.store.get_connection(body.server_id).await?;
    let mut transport = state.store.server_transport(body.server_id).await?;
    transport.timeout_seconds = transport.timeout_seconds.min(5);
    let detection = openwebide_llm::discovery::detect(
        &connection,
        &body.model,
        SpinHttpClient::default().with_transport(transport),
    )
    .await?;
    state
        .store
        .set_setting(
            &format!("model_detection_{}_{}", connection.id, body.model),
            &serde_json::to_string(&serde_json::json!({"revision": connection.tool_stream_revision, "detection": detection})).map_err(|e| ApiError::internal(e.to_string()))?,
        )
        .await?;
    Ok(json_response(200, &detection))
}
#[derive(Deserialize)]
struct InspectBody {
    base_url: String,
    kind: Option<openwebide_core::ProviderKind>,
}
pub(crate) async fn inspect(req: Request) -> Result<JsonResp, ApiError> {
    let body: InspectBody = parse_json(read_body(req, JSON_BODY_LIMIT).await?)?;
    validate_url(&body.base_url)?;
    let transport = openwebide_core::ServerTransport {
        timeout_seconds: 3,
        ..Default::default()
    };
    let discovery = openwebide_llm::discovery::inspect(
        &body.base_url,
        body.kind,
        SpinHttpClient::default().with_transport(transport),
    )
    .await?;
    Ok(json_response(200, &discovery))
}
pub(crate) fn validate_url(url: &str) -> Result<(), ApiError> {
    let uri = url
        .parse::<spin_sdk::http::Uri>()
        .map_err(|_| ApiError::bad_request("Enter an HTTP or HTTPS server URL."))?;
    if !matches!(uri.scheme_str(), Some("http" | "https"))
        || uri.host().is_none()
        || uri
            .authority()
            .is_some_and(|value| value.as_str().contains('@'))
        || uri.query().is_some()
        || url.contains('#')
    {
        return Err(ApiError::bad_request(
            "Use an HTTP or HTTPS base URL without credentials, query parameters or fragments.",
        ));
    }
    Ok(())
}
pub(crate) async fn discover() -> Result<JsonResp, ApiError> {
    let transport = openwebide_core::ServerTransport {
        timeout_seconds: 2,
        ..Default::default()
    };
    let found =
        openwebide_llm::discovery::discover(SpinHttpClient::default().with_transport(transport))
            .await;
    Ok(json_response(200, &found))
}
