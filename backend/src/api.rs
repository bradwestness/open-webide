//! API handlers.

use std::sync::Arc;

use bytes::Bytes;
use http_body_util::BodyExt;
use openwebide_agent::AgentConfig;
use openwebide_core::{
    ChatRequest, EditorContext, FileDiff, FileEntry, GitCheckoutRequest, GitCommitRequest,
    GitSyncRequest, Health, NewConnection, NewProject, Role, SearchHit, SystemPrompt,
    TurnTelemetry, UserRole, WorkspaceMode,
};
use openwebide_llm::{LlmProvider, registry::Provider};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::json;
use spin_sdk::http::{FullBody, Request, Response, box_body};

use crate::agent::{CancelFlag, PermissionPoller, agent_stream, workspace_tools};
use crate::auth;
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

/// The authenticated user id for this request, or 401 if unauthenticated.
fn current_user_id(state: &AppState) -> Result<i64, ApiError> {
    state
        .current_user
        .as_ref()
        .map(|u| u.id)
        .ok_or_else(|| ApiError::unauthorized("authentication required"))
}

// -- auth --------------------------------------------------------------------

#[derive(Deserialize)]
struct RegisterBody {
    username: String,
    password: String,
}

/// Create the first (admin) account. Registration closes once any account
/// exists, so this returns 403 after the first user signs up.
pub async fn register(req: Request, state: &AppState) -> Result<JsonResp, ApiError> {
    let body = read_body(req).await?;
    let reg: RegisterBody = parse_json(body)?;
    let username = reg.username.trim();
    if username.is_empty() {
        return Err(ApiError::bad_request("username is required"));
    }
    if reg.password.len() < 8 {
        return Err(ApiError::bad_request(
            "password must be at least 8 characters",
        ));
    }
    if state.store.count_users().await? > 0 {
        return Err(ApiError::forbidden(
            "registration is closed; an account already exists",
        ));
    }
    let password_hash = auth::hash_password(&reg.password)?;
    let user = state
        .store
        .insert_user(username, &password_hash, UserRole::Admin, now())
        .await?;
    // Pre-auth projects and sessions (user_id NULL) belong to whoever signs
    // up first, so nothing created before accounts existed is lost to scoping.
    state.store.reassign_orphaned_projects(user.id).await?;
    state.store.reassign_orphaned_sessions(user.id).await?;
    let token = auth::issue_token(state, user.id).await?;
    Ok(json_response(
        201,
        &json!({ "user": user.public(), "token": token }),
    ))
}

#[derive(Deserialize)]
struct LoginBody {
    username: String,
    password: String,
}

pub async fn login(req: Request, state: &AppState) -> Result<JsonResp, ApiError> {
    let body = read_body(req).await?;
    let creds: LoginBody = parse_json(body)?;
    let user = state
        .store
        .get_user_by_username(creds.username.trim())
        .await?
        .ok_or_else(|| ApiError::unauthorized("invalid username or password"))?;
    if !auth::verify_password(&creds.password, &user.password_hash) {
        return Err(ApiError::unauthorized("invalid username or password"));
    }
    let token = auth::issue_token(state, user.id).await?;
    Ok(json_response(
        200,
        &json!({ "user": user.public(), "token": token }),
    ))
}

/// The authenticated account (set by the router from the bearer token).
pub async fn me(state: &AppState) -> Result<JsonResp, ApiError> {
    let user = current_user_id(state)?;
    let user = state.store.get_user(user).await?.map(|u| u.public());
    Ok(json_response(200, &json!({ "user": user })))
}

/// Stateless logout: the token is bearer-based, so the client simply discards
/// its copy. This endpoint exists for symmetry and future revocation.
pub fn logout() -> JsonResp {
    json_response(200, &json!({ "ok": true }))
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

fn validate_context_limit(limit: Option<usize>) -> Result<(), ApiError> {
    if limit == Some(0) {
        return Err(ApiError::bad_request(
            "context limit must be a positive number of tokens",
        ));
    }
    Ok(())
}

pub async fn create_connection(req: Request, state: &AppState) -> Result<JsonResp, ApiError> {
    let body = read_body(req).await?;
    let new: NewConnection = parse_json(body)?;
    validate_context_limit(new.context_limit)?;
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
    validate_context_limit(connection.context_limit)?;
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
    let user_id = current_user_id(state)?;
    let settings = state.store.all_user_settings(user_id).await?;
    Ok(json_response(200, &settings))
}

#[derive(Deserialize)]
struct SettingBody {
    key: String,
    value: String,
}

pub async fn set_setting(req: Request, state: &AppState) -> Result<JsonResp, ApiError> {
    let user_id = current_user_id(state)?;
    let body = read_body(req).await?;
    let setting: SettingBody = parse_json(body)?;
    state
        .store
        .set_user_setting(user_id, &setting.key, &setting.value)
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

pub async fn update_system_prompt(
    req: Request,
    state: &AppState,
    path: &str,
) -> Result<JsonResp, ApiError> {
    let id = path_id(path, "/api/system-prompts")?;
    let body = read_body(req).await?;
    let prompt_body: PromptBody = parse_json(body)?;
    let prompt: SystemPrompt = state
        .store
        .update_system_prompt(id, &prompt_body.name, &prompt_body.content)
        .await?;
    Ok(json_response(200, &prompt))
}

pub async fn delete_system_prompt(state: &AppState, path: &str) -> Result<JsonResp, ApiError> {
    let id = path_id(path, "/api/system-prompts")?;
    state.store.delete_system_prompt(id).await?;
    Ok(json_response(200, &json!({ "deleted": id })))
}

// -- projects --------------------------------------------------------------------

pub async fn list_projects(state: &AppState) -> Result<JsonResp, ApiError> {
    let user_id = current_user_id(state)?;
    let projects = state.store.list_projects(user_id).await?;
    Ok(json_response(200, &projects))
}

pub async fn create_project(req: Request, state: &AppState) -> Result<JsonResp, ApiError> {
    let user_id = current_user_id(state)?;
    let body = read_body(req).await?;
    let new: NewProject = parse_json(body)?;
    let project = state.store.create_project(&new, user_id, now()).await?;
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
    let user_id = current_user_id(state)?;
    let id = path_id(path, "/api/projects")?;
    let body = read_body(req).await?;
    let rename: RenameProjectBody = parse_json(body)?;
    let project = state
        .store
        .rename_project(id, &rename.name, user_id)
        .await?;
    Ok(json_response(200, &project))
}

pub async fn delete_project(state: &AppState, path: &str) -> Result<JsonResp, ApiError> {
    let user_id = current_user_id(state)?;
    let id = path_id(path, "/api/projects")?;
    state.store.delete_project(id, user_id).await?;
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
    user_id: i64,
    id: i64,
    rel: &str,
) -> Result<(String, String), ApiError> {
    let project = state.store.get_project(id, user_id).await?;
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

/// Strip the project's base-path prefix from each search hit's path so the
/// returned paths are project-relative.
fn strip_base_hits(base: &str, hits: Vec<SearchHit>) -> Vec<SearchHit> {
    if base.is_empty() {
        return hits;
    }
    let prefix = format!("{base}/");
    hits.into_iter()
        .map(|h| {
            let path = h
                .path
                .strip_prefix(prefix.as_str())
                .unwrap_or(h.path.as_str())
                .to_string();
            SearchHit { path, ..h }
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
    let user_id = current_user_id(state)?;
    let (id, sub) = project_files_path(path)?;
    match sub {
        "files" => {
            let rel = files_query(&req, "path").unwrap_or_default();
            let (full, base) = remote_project_path(state, user_id, id, &rel).await?;
            let entries = crate::files::list(&full).await?;
            Ok(json_response(200, &strip_base(&base, entries)))
        }
        "files/read" => {
            let rel =
                files_query(&req, "path").ok_or_else(|| ApiError::bad_request("missing ?path="))?;
            let (full, _) = remote_project_path(state, user_id, id, &rel).await?;
            let content = crate::files::read(&full).await?;
            Ok(json_response(
                200,
                &json!({ "path": rel, "content": content }),
            ))
        }
        "files/raw" => {
            let rel =
                files_query(&req, "path").ok_or_else(|| ApiError::bad_request("missing ?path="))?;
            let (full, _) = remote_project_path(state, user_id, id, &rel).await?;
            let bytes = crate::files::read_bytes(&full).await?;
            let mime = crate::files::mime_type_from_path(&rel);
            let resp = Response::builder()
                .status(200)
                .header("content-type", mime)
                .header("cache-control", "private, max-age=300")
                .body(box_body(FullBody::new(Bytes::from(bytes))))
                .map_err(|e| ApiError::internal(e.to_string()))?;
            Ok(resp)
        }
        "files/search" => {
            let q = files_query(&req, "q").ok_or_else(|| ApiError::bad_request("missing ?q="))?;
            let rel = files_query(&req, "path").unwrap_or_default();
            let (full, base) = remote_project_path(state, user_id, id, &rel).await?;
            let entries = crate::files::search(&full, &q).await?;
            Ok(json_response(200, &strip_base(&base, entries)))
        }
        "files/content-search" => {
            let q = files_query(&req, "q").ok_or_else(|| ApiError::bad_request("missing ?q="))?;
            let rel = files_query(&req, "path").unwrap_or_default();
            let (full, base) = remote_project_path(state, user_id, id, &rel).await?;
            let hits = crate::files::full_text_search(&full, &q).await?;
            Ok(json_response(200, &strip_base_hits(&base, hits)))
        }
        other => Err(ApiError::not_found(format!("no file route for {other}"))),
    }
}

pub async fn files_put(req: Request, state: &AppState, path: &str) -> Result<JsonResp, ApiError> {
    let user_id = current_user_id(state)?;
    let (id, sub) = project_files_path(path)?;
    if sub != "files/write" {
        return Err(ApiError::not_found(format!("no file route for {sub}")));
    }
    let rel = files_query(&req, "path").ok_or_else(|| ApiError::bad_request("missing ?path="))?;
    let (full, _) = remote_project_path(state, user_id, id, &rel).await?;
    let content = read_body(req).await?;
    crate::files::write(&full, &content).await?;
    Ok(json_response(200, &json!({ "path": rel })))
}

pub async fn files_post(req: Request, state: &AppState, path: &str) -> Result<JsonResp, ApiError> {
    let user_id = current_user_id(state)?;
    let (id, sub) = project_files_path(path)?;
    if sub != "files/create" {
        return Err(ApiError::not_found(format!("no file route for {sub}")));
    }
    let rel = files_query(&req, "path").ok_or_else(|| ApiError::bad_request("missing ?path="))?;
    let kind = files_query(&req, "type").unwrap_or_else(|| "file".into());
    let is_dir = kind == "dir";
    let (full, _) = remote_project_path(state, user_id, id, &rel).await?;
    crate::files::create(&full, is_dir).await?;
    Ok(json_response(
        201,
        &json!({ "path": rel, "is_dir": is_dir }),
    ))
}

pub async fn files_delete(
    req: Request,
    state: &AppState,
    path: &str,
) -> Result<JsonResp, ApiError> {
    let user_id = current_user_id(state)?;
    let (id, sub) = project_files_path(path)?;
    if sub != "files/delete" {
        return Err(ApiError::not_found(format!("no file route for {sub}")));
    }
    let rel = files_query(&req, "path").ok_or_else(|| ApiError::bad_request("missing ?path="))?;
    let (full, _) = remote_project_path(state, user_id, id, &rel).await?;
    crate::files::delete(&full).await?;
    Ok(json_response(200, &json!({ "path": rel })))
}

// -- git operations (Phase 13) -----------------------------------------------------

fn project_git_path(path: &str) -> Result<(Option<i64>, &str), ApiError> {
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

pub async fn git_get(req: Request, state: &AppState, path: &str) -> Result<JsonResp, ApiError> {
    let user_id = current_user_id(state)?;
    let (project_id, sub) = project_git_path(path)?;
    let project_dir = if let Some(id) = project_id {
        let (full, _) = remote_project_path(state, user_id, id, "").await?;
        std::path::PathBuf::from(full)
    } else {
        std::path::PathBuf::new()
    };

    match sub {
        "status" => {
            let status = crate::git::repo_status(&project_dir)
                .await
                .map_err(ApiError::internal)?;
            Ok(json_response(200, &status))
        }
        "diff" => {
            let file_path = files_query(&req, "path");
            let diff = crate::git::repo_diff(file_path.as_deref())
                .await
                .map_err(ApiError::internal)?;
            Ok(json_response(200, &json!({ "diff": diff })))
        }
        "branches" => {
            let branches = crate::git::repo_branches()
                .await
                .map_err(ApiError::internal)?;
            Ok(json_response(200, &branches))
        }
        "show" => {
            let file_path = files_query(&req, "path")
                .ok_or_else(|| ApiError::bad_request("missing path query parameter"))?;
            let content = crate::git::repo_file_head(&project_dir, &file_path)
                .await
                .map_err(ApiError::internal)?;
            Ok(json_response(200, &json!({ "content": content })))
        }
        other => Err(ApiError::not_found(format!("unknown git action: {other}"))),
    }
}

pub async fn git_post(req: Request, state: &AppState, path: &str) -> Result<JsonResp, ApiError> {
    let user_id = current_user_id(state)?;
    let (project_id, sub) = project_git_path(path)?;
    let project_dir = if let Some(id) = project_id {
        let (full, _) = remote_project_path(state, user_id, id, "").await?;
        std::path::PathBuf::from(full)
    } else {
        std::path::PathBuf::new()
    };

    let body = read_body(req).await?;

    match sub {
        "status" => {
            let status = crate::git::repo_status(&project_dir)
                .await
                .map_err(ApiError::internal)?;
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
            let diff = crate::git::repo_diff(diff_req.path.as_deref())
                .await
                .map_err(ApiError::internal)?;
            Ok(json_response(200, &json!({ "diff": diff })))
        }
        "branches" => {
            let branches = crate::git::repo_branches()
                .await
                .map_err(ApiError::internal)?;
            Ok(json_response(200, &branches))
        }
        "commit" => {
            let commit_req: GitCommitRequest = parse_json(body)?;
            let result = crate::git::repo_commit(&commit_req)
                .await
                .map_err(ApiError::bad_request)?;
            Ok(json_response(200, &result))
        }
        "checkout" => {
            let checkout_req: GitCheckoutRequest = parse_json(body)?;
            let result = crate::git::repo_checkout(&checkout_req)
                .await
                .map_err(ApiError::bad_request)?;
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
            let result = crate::git::repo_sync(&sync_req)
                .await
                .map_err(ApiError::bad_request)?;
            Ok(json_response(200, &result))
        }
        other => Err(ApiError::not_found(format!("unknown git action: {other}"))),
    }
}

// -- host file browser (Phase 10) --------------------------------------------------

/// List a directory of the host mount (the preopen root, e.g. `~/source`)
/// for the remote file browser. `?path=` is relative to the mount root;
/// empty = the root itself. Paths that would escape the root are rejected by
/// the filesystem layer, so this only ever exposes the mounted folder.
pub async fn browse(req: Request, state: &AppState) -> Result<JsonResp, ApiError> {
    let _user_id = current_user_id(state)?;
    let rel = files_query(&req, "path").unwrap_or_default();
    let entries = crate::files::list(&rel).await?;
    Ok(json_response(200, &entries))
}

// -- sessions --------------------------------------------------------------------

pub async fn list_sessions(state: &AppState) -> Result<JsonResp, ApiError> {
    let user_id = current_user_id(state)?;
    let sessions = state.store.list_sessions(user_id).await?;
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
    let user_id = current_user_id(state)?;
    let body = read_body(req).await?;
    let new: CreateSessionBody = parse_json(body)?;
    let session = state
        .store
        .create_session(
            &new.name,
            new.connection_id,
            new.system_prompt_id,
            new.project_id,
            user_id,
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
    let user_id = current_user_id(state)?;
    let id = session_id(path)?;
    let body = read_body(req).await?;
    let rename: RenameSessionBody = parse_json(body)?;
    let session = state
        .store
        .rename_session(id, &rename.name, user_id)
        .await?;
    Ok(json_response(200, &session))
}

pub async fn delete_session(state: &AppState, path: &str) -> Result<JsonResp, ApiError> {
    let user_id = current_user_id(state)?;
    let id = session_id(path)?;
    state.store.delete_session(id, user_id).await?;
    Ok(json_response(200, &json!({ "deleted": id })))
}

/// Request cancellation of the session's in-flight run. The flag is picked
/// up by the streaming request at its next step boundary (before the next
/// model call or tool execution); a run that is not in flight is unaffected.
pub async fn cancel_session(state: &AppState, path: &str) -> Result<JsonResp, ApiError> {
    let user_id = current_user_id(state)?;
    let id = session_id(path)?;
    // Verify ownership before setting the flag.
    state.store.get_session(id, user_id).await?;
    state.store.request_cancel(id).await?;
    Ok(json_response(200, &json!({ "cancelled": id })))
}

#[derive(Deserialize)]
struct PermissionBody {
    approved: bool,
}

/// Record the user's decision on a gated tool call. The waiting call consumes
/// it on its next poll, so it answers that call only; a decision for a call
/// that is no longer waiting is harmless (decisions are cleared when the run
/// ends).
pub async fn set_tool_permission(
    req: Request,
    state: &AppState,
    path: &str,
) -> Result<JsonResp, ApiError> {
    let user_id = current_user_id(state)?;
    let (id, tool_call_id) = permission_path(path)?;
    // Verify ownership before recording the decision.
    state.store.get_session(id, user_id).await?;
    let body = read_body(req).await?;
    let decision: PermissionBody = parse_json(body)?;
    state
        .store
        .set_tool_permission(id, tool_call_id, decision.approved)
        .await?;
    Ok(json_response(200, &json!({ "ok": true })))
}

pub async fn list_messages(state: &AppState, path: &str) -> Result<JsonResp, ApiError> {
    let user_id = current_user_id(state)?;
    let id = session_id(path)?;
    // Verify ownership before exposing the (unscoped) message list.
    state.store.get_session(id, user_id).await?;
    // Messages plus the agent's tool steps, interleaved, so a reloaded session
    // shows its steps again.
    let conversation = state.store.list_conversation(id).await?;
    Ok(json_response(200, &conversation))
}

#[derive(Deserialize)]
struct SendMessageBody {
    content: String,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    editor_context: Option<EditorContext>,
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
    let user_id = current_user_id(&state)?;
    let session_id = session_id(path)?;
    let body = read_body(req).await?;
    let send: SendMessageBody = parse_json(body)?;

    let session = state.store.get_session(session_id, user_id).await?;
    // A new run starts: clear any stale cancel flag and permission
    // decisions from a previous run.
    let _ = state.store.clear_cancel(session_id).await;
    let _ = state.store.clear_tool_permissions(session_id).await;
    let connection_id = session
        .connection_id
        .ok_or_else(|| ApiError::bad_request("session has no connection; pick one first"))?;
    let connection = state.store.get_connection(connection_id).await?;
    let system_prompt = match session.system_prompt_id {
        Some(id) => Some(state.store.get_system_prompt(id).await?.content),
        None => None,
    };
    let system_prompt = Some(with_temporal_context(system_prompt, now()));
    let history = state.store.list_messages(session_id).await?;
    let full_content = match &send.editor_context {
        Some(ctx) => format!("{}{}", ctx.format_prompt_injection(), send.content),
        None => send.content,
    };
    let user_message = state
        .store
        .insert_message(session_id, Role::User, &full_content, now())
        .await?;

    // Remote-mode projects run the agentic loop with workspace tools;
    // everything else is plain chat.
    let (is_remote, base) = match session.project_id {
        Some(id) => match state.store.get_project(id, user_id).await {
            Ok(project) if project.mode == WorkspaceMode::Remote => {
                (true, project.path.unwrap_or_default())
            }
            _ => (false, String::new()),
        },
        None => (false, String::new()),
    };

    let mut messages = history;
    messages.push(user_message.clone());
    let request = ChatRequest {
        connection_id,
        system_prompt,
        model: send.model,
        messages,
        tools: if is_remote {
            workspace_tools()
        } else {
            Vec::new()
        },
    };
    let provider = Provider::for_connection(&connection, SpinHttpClient);
    let store = Arc::new(state.store);
    let cancel = CancelFlag::new(store.clone(), session_id);
    let gate = PermissionPoller::new(store.clone(), session_id);
    let stream = if is_remote {
        agent_stream(
            store.clone(),
            session_id,
            user_message,
            request,
            provider,
            base,
            AgentConfig::default(),
            cancel,
            gate,
        )
    } else {
        message_stream(store, session_id, user_message, request, provider)
    };

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

/// Parse `/api/sessions/<id>/permissions/<tool_call_id>`.
fn permission_path(path: &str) -> Result<(i64, &str), ApiError> {
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

/// Append current UTC date and time to the system prompt for zero-turn temporal context.
fn with_temporal_context(system_prompt: Option<String>, timestamp_secs: i64) -> String {
    let temporal = format!(
        "Current Date & Time: {}",
        openwebide_core::format_utc_timestamp(timestamp_secs)
    );
    match system_prompt {
        Some(base) if !base.trim().is_empty() => format!("{base}\n\n{temporal}"),
        _ => temporal,
    }
}

pub async fn chat(req: Request, state: &AppState) -> Result<JsonResp, ApiError> {
    let body = read_body(req).await?;
    let mut request: ChatRequest = parse_json(body)?;
    let connection = state.store.get_connection(request.connection_id).await?;
    let provider = Provider::for_connection(&connection, SpinHttpClient);
    request.system_prompt = Some(with_temporal_context(request.system_prompt, now()));
    let reply = provider.chat(&request).await?;
    Ok(json_response(200, &json!({ "reply": reply })))
}

pub async fn chat_tools(req: Request, state: &AppState) -> Result<JsonResp, ApiError> {
    let _user_id = current_user_id(state)?;
    let body = read_body(req).await?;
    let mut request: ChatRequest = parse_json(body)?;
    let connection = state.store.get_connection(request.connection_id).await?;
    let provider = Provider::for_connection(&connection, SpinHttpClient);
    request.system_prompt = Some(with_temporal_context(request.system_prompt, now()));
    let response = provider.chat_tools(&request).await?;
    Ok(json_response(200, &response))
}

#[derive(Deserialize)]
struct PersistMessageBody {
    role: Role,
    content: String,
    #[serde(default)]
    usage: Option<TurnTelemetry>,
}

pub async fn persist_message(
    req: Request,
    state: &AppState,
    path: &str,
) -> Result<JsonResp, ApiError> {
    let user_id = current_user_id(state)?;
    let id = session_id(path)?;
    state.store.get_session(id, user_id).await?;
    let body = read_body(req).await?;
    let msg: PersistMessageBody = parse_json(body)?;
    let message = state
        .store
        .insert_message_with_usage(id, msg.role, &msg.content, now(), msg.usage.as_ref())
        .await?;
    Ok(json_response(201, &message))
}

/// Resolve a connection's context window: the connection's configured value,
/// else provider discovery. A provider error surfaces as `null`, not a 5xx,
/// so an unreachable runtime doesn't raise a banner.
pub async fn model_context(req: Request, state: &AppState) -> Result<JsonResp, ApiError> {
    let query = req.uri().query().unwrap_or_default();
    let connection_id = query_param(query, "connection_id")
        .and_then(|s| s.parse::<i64>().ok())
        .ok_or_else(|| ApiError::bad_request("missing ?connection_id=<id>"))?;
    let model = query_param(query, "model").map(urldecode);
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

#[derive(Deserialize)]
struct UpsertToolStepBody {
    anchor_message_id: i64,
    tool_call_id: String,
    name: String,
    summary: String,
}

pub async fn upsert_tool_step(
    req: Request,
    state: &AppState,
    path: &str,
) -> Result<JsonResp, ApiError> {
    let user_id = current_user_id(state)?;
    let id = session_id(path)?;
    state.store.get_session(id, user_id).await?;
    let body = read_body(req).await?;
    let step: UpsertToolStepBody = parse_json(body)?;
    state
        .store
        .upsert_tool_step(
            id,
            step.anchor_message_id,
            &step.tool_call_id,
            &step.name,
            &step.summary,
            now(),
        )
        .await?;
    Ok(json_response(200, &json!({ "ok": true })))
}

#[derive(Deserialize)]
struct CompleteToolStepBody {
    tool_call_id: String,
    ok: bool,
    result_summary: String,
    #[serde(default)]
    diff: Option<FileDiff>,
}

pub async fn complete_tool_step(
    req: Request,
    state: &AppState,
    path: &str,
) -> Result<JsonResp, ApiError> {
    let user_id = current_user_id(state)?;
    let id = session_id(path)?;
    state.store.get_session(id, user_id).await?;
    let body = read_body(req).await?;
    let step: CompleteToolStepBody = parse_json(body)?;
    state
        .store
        .complete_tool_step(
            id,
            &step.tool_call_id,
            step.ok,
            &step.result_summary,
            step.diff.as_ref(),
        )
        .await?;
    Ok(json_response(200, &json!({ "ok": true })))
}

pub async fn web_search(req: Request) -> Result<JsonResp, ApiError> {
    let query = files_query(&req, "query")
        .or_else(|| files_query(&req, "q"))
        .ok_or_else(|| ApiError::bad_request("missing ?query= parameter"))?;
    let limit = files_query(&req, "limit")
        .and_then(|l| l.parse::<usize>().ok())
        .unwrap_or(5);

    match crate::web::search_web_internal(&query, limit).await {
        Ok(results) => Ok(json_response(200, &results)),
        Err(e) => Err(ApiError::internal(format!("web search failed: {e}"))),
    }
}

pub async fn web_fetch(req: Request) -> Result<JsonResp, ApiError> {
    let url =
        files_query(&req, "url").ok_or_else(|| ApiError::bad_request("missing ?url= parameter"))?;

    match crate::web::fetch_page_internal(&url).await {
        Ok(content) => Ok(json_response(
            200,
            &serde_json::json!({
                "url": url,
                "content": content,
            }),
        )),
        Err(e) => Err(ApiError::bad_request(format!("web fetch failed: {e}"))),
    }
}

/// Minimal `key=value&...` query-string lookup (avoids a dependency).
fn query_param<'a>(query: &'a str, key: &str) -> Option<&'a str> {
    query.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=')?;
        (k == key).then_some(v)
    })
}
