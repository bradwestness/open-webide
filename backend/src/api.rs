//! API handlers.

use bytes::Bytes;
use http_body_util::BodyExt;
use openwebide_core::{
    ChatRequest, FileEntry, Health, NewConnection, NewProject, Role, SystemPrompt,
};
use openwebide_llm::{LlmProvider, registry::Provider};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::json;
use spin_sdk::http::{FullBody, Request, Response, box_body};

use crate::error::{ApiError, JsonResp};
use crate::http_client::SpinHttpClient;
use crate::sse::{SseBody, message_stream};
use crate::state::{AppState, now};

fn json_response(status: u16, value: &impl serde::Serialize) -> JsonResp {
    let body = serde_json::to_string(value).unwrap_or_else(|_| "{}".into());
    Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(box_body(FullBody::new(Bytes::from(body))))
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

// -- projects --------------------------------------------------------------------

pub async fn list_projects(state: &AppState) -> Result<JsonResp, ApiError> {
    let projects = state.store.list_projects().await?;
    Ok(json_response(200, &projects))
}

pub async fn create_project(req: Request, state: &AppState) -> Result<JsonResp, ApiError> {
    let body = read_body(req).await?;
    let new: NewProject = parse_json(body)?;
    let project = state.store.create_project(&new, now()).await?;
    Ok(json_response(201, &project))
}

#[derive(Deserialize)]
struct RenameProjectBody {
    name: String,
}

pub async fn rename_project(
    req: Request,
    state: &AppState,
    path: &str,
) -> Result<JsonResp, ApiError> {
    let id = path_id(path, "/api/projects")?;
    let body = read_body(req).await?;
    let rename: RenameProjectBody = parse_json(body)?;
    let project = state.store.rename_project(id, &rename.name).await?;
    Ok(json_response(200, &project))
}

pub async fn delete_project(state: &AppState, path: &str) -> Result<JsonResp, ApiError> {
    let id = path_id(path, "/api/projects")?;
    state.store.delete_project(id).await?;
    Ok(json_response(200, &json!({ "deleted": id })))
}

// -- project files (remote mode) ---------------------------------------------------

/// Parse `/api/projects/<id>/files...` into the project id and the
/// `files...` sub-path (e.g. `files`, `files/read`, `files/write`).
fn project_files_path(path: &str) -> Result<(i64, &str), ApiError> {
    let rest = path
        .strip_prefix("/api/projects/")
        .ok_or_else(|| ApiError::bad_request("expected /api/projects/<id>/files..."))?;
    let (id, sub) = rest
        .split_once('/')
        .ok_or_else(|| ApiError::bad_request("expected /api/projects/<id>/files..."))?;
    let id = id
        .parse::<i64>()
        .map_err(|_| ApiError::bad_request("expected a numeric project id"))?;
    if !sub.starts_with("files") {
        return Err(ApiError::bad_request("expected a /files sub-path"));
    }
    Ok((id, sub))
}

/// Load a remote-mode project and join its workspace path with a
/// project-relative path. Returns the full preopen-relative path (for
/// filesystem access) and the project's base path (so returned entry paths
/// can be stripped back to project-relative form).
async fn remote_project_path(
    state: &AppState,
    id: i64,
    rel: &str,
) -> Result<(String, String), ApiError> {
    let project = state.store.get_project(id).await?;
    if project.mode != openwebide_core::WorkspaceMode::Remote {
        return Err(ApiError::bad_request(
            "file access is only available for remote-mode projects",
        ));
    }
    let base = project.path.unwrap_or_default();
    let full = if base.is_empty() {
        rel.to_string()
    } else if rel.is_empty() {
        base.clone()
    } else {
        format!("{base}/{rel}")
    };
    Ok((full, base))
}

/// Strip the project's base-path prefix from each entry's path so the
/// returned paths are project-relative — matching what the read, write,
/// create, and search endpoints expect in `?path=`.
fn strip_base(base: &str, entries: Vec<FileEntry>) -> Vec<FileEntry> {
    if base.is_empty() {
        return entries;
    }
    let prefix = format!("{base}/");
    entries
        .into_iter()
        .map(|e| {
            let path = e
                .path
                .strip_prefix(prefix.as_str())
                .unwrap_or(e.path.as_str())
                .to_string();
            FileEntry { path, ..e }
        })
        .collect()
}

fn files_query(req: &Request, key: &str) -> Option<String> {
    req.uri().query()?.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=')?;
        (k == key).then(|| urldecode(v))
    })
}

/// Percent-decode a query parameter value (e.g. `src%2Fhello.rs` ->
/// `src/hello.rs`). The frontend percent-encodes every non-unreserved byte in
/// a path, including `/`, so the backend must decode before using it.
fn urldecode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && bytes[i + 1].is_ascii_hexdigit()
            && bytes[i + 2].is_ascii_hexdigit()
        {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap();
            if let Ok(b) = u8::from_str_radix(hex, 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub async fn files_get(req: Request, state: &AppState, path: &str) -> Result<JsonResp, ApiError> {
    let (id, sub) = project_files_path(path)?;
    match sub {
        "files" => {
            let rel = files_query(&req, "path").unwrap_or_default();
            let (full, base) = remote_project_path(state, id, &rel).await?;
            let entries = crate::files::list(&full).await?;
            Ok(json_response(200, &strip_base(&base, entries)))
        }
        "files/read" => {
            let rel =
                files_query(&req, "path").ok_or_else(|| ApiError::bad_request("missing ?path="))?;
            let (full, _) = remote_project_path(state, id, &rel).await?;
            let content = crate::files::read(&full).await?;
            Ok(json_response(
                200,
                &json!({ "path": rel, "content": content }),
            ))
        }
        "files/search" => {
            let q = files_query(&req, "q").ok_or_else(|| ApiError::bad_request("missing ?q="))?;
            let rel = files_query(&req, "path").unwrap_or_default();
            let (full, base) = remote_project_path(state, id, &rel).await?;
            let entries = crate::files::search(&full, &q).await?;
            Ok(json_response(200, &strip_base(&base, entries)))
        }
        other => Err(ApiError::not_found(format!("no file route for {other}"))),
    }
}

pub async fn files_put(req: Request, state: &AppState, path: &str) -> Result<JsonResp, ApiError> {
    let (id, sub) = project_files_path(path)?;
    if sub != "files/write" {
        return Err(ApiError::not_found(format!("no file route for {sub}")));
    }
    let rel = files_query(&req, "path").ok_or_else(|| ApiError::bad_request("missing ?path="))?;
    let (full, _) = remote_project_path(state, id, &rel).await?;
    let content = read_body(req).await?;
    crate::files::write(&full, &content).await?;
    Ok(json_response(200, &json!({ "path": rel })))
}

pub async fn files_post(req: Request, state: &AppState, path: &str) -> Result<JsonResp, ApiError> {
    let (id, sub) = project_files_path(path)?;
    if sub != "files/create" {
        return Err(ApiError::not_found(format!("no file route for {sub}")));
    }
    let rel = files_query(&req, "path").ok_or_else(|| ApiError::bad_request("missing ?path="))?;
    let kind = files_query(&req, "type").unwrap_or_else(|| "file".into());
    let is_dir = kind == "dir";
    let (full, _) = remote_project_path(state, id, &rel).await?;
    crate::files::create(&full, is_dir).await?;
    Ok(json_response(
        201,
        &json!({ "path": rel, "is_dir": is_dir }),
    ))
}

// -- sessions --------------------------------------------------------------------

pub async fn list_sessions(state: &AppState) -> Result<JsonResp, ApiError> {
    let sessions = state.store.list_sessions().await?;
    Ok(json_response(200, &sessions))
}

#[derive(Deserialize)]
struct CreateSessionBody {
    name: String,
    #[serde(default)]
    connection_id: Option<i64>,
    #[serde(default)]
    system_prompt_id: Option<i64>,
    #[serde(default)]
    project_id: Option<i64>,
}

pub async fn create_session(req: Request, state: &AppState) -> Result<JsonResp, ApiError> {
    let body = read_body(req).await?;
    let new: CreateSessionBody = parse_json(body)?;
    let session = state
        .store
        .create_session(
            &new.name,
            new.connection_id,
            new.system_prompt_id,
            new.project_id,
            now(),
        )
        .await?;
    Ok(json_response(201, &session))
}

#[derive(Deserialize)]
struct RenameSessionBody {
    name: String,
}

pub async fn rename_session(
    req: Request,
    state: &AppState,
    path: &str,
) -> Result<JsonResp, ApiError> {
    let id = session_id(path)?;
    let body = read_body(req).await?;
    let rename: RenameSessionBody = parse_json(body)?;
    let session = state.store.rename_session(id, &rename.name).await?;
    Ok(json_response(200, &session))
}

pub async fn delete_session(state: &AppState, path: &str) -> Result<JsonResp, ApiError> {
    let id = session_id(path)?;
    state.store.delete_session(id).await?;
    Ok(json_response(200, &json!({ "deleted": id })))
}

pub async fn list_messages(state: &AppState, path: &str) -> Result<JsonResp, ApiError> {
    let id = session_id(path)?;
    let messages = state.store.list_messages(id).await?;
    Ok(json_response(200, &messages))
}

#[derive(Deserialize)]
struct SendMessageBody {
    content: String,
    #[serde(default)]
    model: Option<String>,
}

/// Send a user message and stream the assistant reply back as SSE.
///
/// `state` is taken by value: the store is moved into the response body so
/// the assistant message can be persisted from inside the stream.
pub async fn send_session_message(
    req: Request,
    state: AppState,
    path: &str,
) -> Result<JsonResp, ApiError> {
    let session_id = session_id(path)?;
    let body = read_body(req).await?;
    let send: SendMessageBody = parse_json(body)?;

    let session = state.store.get_session(session_id).await?;
    let connection_id = session
        .connection_id
        .ok_or_else(|| ApiError::bad_request("session has no connection; pick one first"))?;
    let connection = state.store.get_connection(connection_id).await?;
    let system_prompt = match session.system_prompt_id {
        Some(id) => Some(state.store.get_system_prompt(id).await?.content),
        None => None,
    };
    let history = state.store.list_messages(session_id).await?;
    let user_message = state
        .store
        .insert_message(session_id, Role::User, &send.content, now())
        .await?;

    let mut messages = history;
    messages.push(user_message.clone());
    let request = ChatRequest {
        connection_id,
        system_prompt,
        model: send.model,
        messages,
    };
    let provider = Provider::for_connection(&connection, SpinHttpClient);
    let stream = message_stream(state.store, session_id, user_message, request, provider);

    Ok(Response::builder()
        .status(200)
        .header("content-type", "text/event-stream")
        .header("cache-control", "no-cache")
        .body(box_body(SseBody::new(stream)))
        .expect("valid status and headers"))
}

/// Parse `/api/sessions/<id>...` into the session id.
fn session_id(path: &str) -> Result<i64, ApiError> {
    path.strip_prefix("/api/sessions/")
        .and_then(|rest| rest.split('/').next())
        .and_then(|id| id.parse::<i64>().ok())
        .ok_or_else(|| ApiError::bad_request("expected /api/sessions/<id>..."))
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
