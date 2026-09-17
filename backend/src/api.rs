//! API handlers.

use http_body_util::BodyExt;
use openwebide_core::{ChatRequest, Health, NewConnection, SystemPrompt};
use openwebide_llm::{LlmProvider, registry::Provider};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::json;
use spin_sdk::http::Request;

use crate::error::{ApiError, JsonResp};
use crate::http_client::SpinHttpClient;
use crate::state::AppState;

fn json_response(status: u16, value: &impl serde::Serialize) -> JsonResp {
    let body = serde_json::to_string(value).unwrap_or_else(|_| "{}".into());
    spin_sdk::http::Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(body)
        .expect("valid status and headers")
}

async fn read_body(req: Request) -> Result<String, ApiError> {
    let body = req.into_body();
    let collected = body
        .collect()
        .await
        .map_err(|e| ApiError::bad_request(format!("read request body: {e}")))?;
    let bytes = collected.to_bytes();
    String::from_utf8(bytes.to_vec())
        .map_err(|_| ApiError::bad_request("request body is not valid UTF-8"))
}

fn parse_json<T: DeserializeOwned>(body: String) -> Result<T, ApiError> {
    serde_json::from_str(&body).map_err(|e| ApiError::bad_request(format!("invalid JSON: {e}")))
}

fn path_id(path: &str, prefix: &str) -> Result<i64, ApiError> {
    path.strip_prefix(prefix)
        .and_then(|rest| rest.strip_prefix('/'))
        .and_then(|id| id.parse::<i64>().ok())
        .ok_or_else(|| ApiError::bad_request(format!("expected a numeric id after {prefix}/")))
}

// -- health ------------------------------------------------------------------

pub fn health() -> JsonResp {
    json_response(
        200,
        &Health {
            status: "ok".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        },
    )
}

// -- connections ---------------------------------------------------------------

pub async fn list_connections(state: &AppState) -> Result<JsonResp, ApiError> {
    let connections = state.store.list_connections().await?;
    Ok(json_response(200, &connections))
}

pub async fn create_connection(req: Request, state: &AppState) -> Result<JsonResp, ApiError> {
    let body = read_body(req).await?;
    let new: NewConnection = parse_json(body)?;
    let connection = state.store.insert_connection(&new).await?;
    Ok(json_response(201, &connection))
}

pub async fn update_connection(
    req: Request,
    state: &AppState,
    path: &str,
) -> Result<JsonResp, ApiError> {
    let id = path_id(path, "/api/connections")?;
    let body = read_body(req).await?;
    let mut connection: openwebide_core::Connection = parse_json(body)?;
    connection.id = id;
    state.store.update_connection(&connection).await?;
    Ok(json_response(200, &connection))
}

pub async fn delete_connection(state: &AppState, path: &str) -> Result<JsonResp, ApiError> {
    let id = path_id(path, "/api/connections")?;
    state.store.delete_connection(id).await?;
    Ok(json_response(200, &json!({ "deleted": id })))
}

// -- settings ------------------------------------------------------------------

pub async fn get_settings(state: &AppState) -> Result<JsonResp, ApiError> {
    let settings = state.store.all_settings().await?;
    Ok(json_response(200, &settings))
}

#[derive(Deserialize)]
struct SettingBody {
    key: String,
    value: String,
}

pub async fn set_setting(req: Request, state: &AppState) -> Result<JsonResp, ApiError> {
    let body = read_body(req).await?;
    let setting: SettingBody = parse_json(body)?;
    state
        .store
        .set_setting(&setting.key, &setting.value)
        .await?;
    Ok(json_response(200, &json!({ "key": setting.key })))
}

// -- system prompts --------------------------------------------------------------

pub async fn list_system_prompts(state: &AppState) -> Result<JsonResp, ApiError> {
    let prompts = state.store.list_system_prompts().await?;
    Ok(json_response(200, &prompts))
}

#[derive(Deserialize)]
struct PromptBody {
    name: String,
    content: String,
}

pub async fn create_system_prompt(req: Request, state: &AppState) -> Result<JsonResp, ApiError> {
    let body = read_body(req).await?;
    let prompt_body: PromptBody = parse_json(body)?;
    let prompt: SystemPrompt = state
        .store
        .insert_system_prompt(&prompt_body.name, &prompt_body.content)
        .await?;
    Ok(json_response(201, &prompt))
}

pub async fn delete_system_prompt(state: &AppState, path: &str) -> Result<JsonResp, ApiError> {
    let id = path_id(path, "/api/system-prompts")?;
    state.store.delete_system_prompt(id).await?;
    Ok(json_response(200, &json!({ "deleted": id })))
}

// -- models & chat ----------------------------------------------------------------

pub async fn list_models(req: Request, state: &AppState) -> Result<JsonResp, ApiError> {
    let query = req.uri().query().unwrap_or_default();
    let id = query_param(query, "connection_id")
        .and_then(|s| s.parse::<i64>().ok())
        .ok_or_else(|| ApiError::bad_request("missing ?connection_id=<id>"))?;
    let connection = state.store.get_connection(id).await?;
    let provider = Provider::for_connection(&connection, SpinHttpClient);
    let models = provider.list_models().await?;
    Ok(json_response(200, &models))
}

pub async fn chat(req: Request, state: &AppState) -> Result<JsonResp, ApiError> {
    let body = read_body(req).await?;
    let request: ChatRequest = parse_json(body)?;
    let connection = state.store.get_connection(request.connection_id).await?;
    let provider = Provider::for_connection(&connection, SpinHttpClient);
    let reply = provider.chat(&request).await?;
    Ok(json_response(200, &json!({ "reply": reply })))
}

/// Minimal `key=value&...` query-string lookup (avoids a dependency).
fn query_param<'a>(query: &'a str, key: &str) -> Option<&'a str> {
    query.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=')?;
        (k == key).then_some(v)
    })
}
