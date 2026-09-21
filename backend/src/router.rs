//! Manual routing: Spin's `http_service` gives us one entry point, so we
//! dispatch on (method, path) ourselves.

use bytes::Bytes;
use spin_sdk::http::{FullBody, Request, Response, box_body};

use crate::api;
use crate::error::{ApiError, JsonResp};
use crate::state::AppState;

pub async fn route(req: Request) -> JsonResp {
    let path = req.uri().path().to_string();
    let method = req.method().clone();
    // Read the token before the match below moves `req`.
    let token = bearer_token(&req);

    let resp = if method.as_str() == "OPTIONS" {
        preflight()
    } else {
        let mut state = match AppState::new().await {
            Ok(state) => state,
            Err(e) => return ApiError::internal(e.to_string()).into_response(),
        };

        // Every non-public route requires a valid bearer token. Public routes
        // (health, register, login, logout) run without one; a valid token on
        // any other route sets `state.current_user` for the handlers.
        if !is_public(&path) {
            match crate::auth::authenticate(&state, token.as_deref()).await {
                Ok(user) => state.current_user = Some(user),
                Err(e) => return with_cors(e.into_response()),
            }
        }

        let result: Result<JsonResp, ApiError> = match (method.as_str(), path.as_str()) {
            ("GET", "/api/health") => Ok(api::health()),
            ("POST", "/api/auth/register") => api::register(req, &state).await,
            ("POST", "/api/auth/login") => api::login(req, &state).await,
            ("GET", "/api/auth/me") => api::me(&state).await,
            ("POST", "/api/auth/logout") => Ok(api::logout()),
            ("GET", "/api/connections") => api::list_connections(&state).await,
            ("POST", "/api/connections") => api::create_connection(req, &state).await,
            ("PUT", p) if p.starts_with("/api/connections/") => {
                api::update_connection(req, &state, p).await
            }
            ("DELETE", p) if p.starts_with("/api/connections/") => {
                api::delete_connection(&state, p).await
            }
            ("GET", "/api/settings") => api::get_settings(&state).await,
            ("PUT", "/api/settings") => api::set_setting(req, &state).await,
            ("GET", "/api/system-prompts") => api::list_system_prompts(&state).await,
            ("POST", "/api/system-prompts") => api::create_system_prompt(req, &state).await,
            ("PUT", p) if p.starts_with("/api/system-prompts/") => {
                api::update_system_prompt(req, &state, p).await
            }
            ("DELETE", p) if p.starts_with("/api/system-prompts/") => {
                api::delete_system_prompt(&state, p).await
            }
            ("GET", "/api/projects") => api::list_projects(&state).await,
            ("POST", "/api/projects") => api::create_project(req, &state).await,
            // Project file routes must precede the generic PUT/DELETE
            // project routes, which also match `/api/projects/...`.
            ("GET", p) if is_project_files(p) => api::files_get(req, &state, p).await,
            ("PUT", p) if is_project_files(p) => api::files_put(req, &state, p).await,
            ("POST", p) if is_project_files(p) => api::files_post(req, &state, p).await,
            ("DELETE", p) if is_project_files(p) => api::files_delete(req, &state, p).await,
            ("PUT", p) if p.starts_with("/api/projects/") => {
                api::rename_project(req, &state, p).await
            }
            ("DELETE", p) if p.starts_with("/api/projects/") => {
                api::delete_project(&state, p).await
            }
            ("GET", "/api/browse") => api::browse(req, &state).await,
            ("GET", "/api/sessions") => api::list_sessions(&state).await,
            ("POST", "/api/sessions") => api::create_session(req, &state).await,
            ("PUT", p) if p.starts_with("/api/sessions/") && !p.contains("/messages") => {
                api::rename_session(req, &state, p).await
            }
            ("DELETE", p) if p.starts_with("/api/sessions/") && !p.contains("/messages") => {
                api::delete_session(&state, p).await
            }
            ("GET", p) if p.starts_with("/api/sessions/") && p.ends_with("/messages") => {
                api::list_messages(&state, p).await
            }
            ("POST", p) if p.starts_with("/api/sessions/") && p.ends_with("/messages") => {
                // Moves `state`: the store is consumed by the SSE stream.
                api::send_session_message(req, state, p).await
            }
            ("GET", "/api/models") => api::list_models(req, &state).await,
            ("POST", "/api/chat") => api::chat(req, &state).await,
            _ => Err(ApiError::not_found(format!("no route for {method} {path}"))),
        };

        match result {
            Ok(resp) => resp,
            Err(e) => e.into_response(),
        }
    };

    with_cors(resp)
}

/// Match the project file routes: `/api/projects/<id>/files...`.
fn is_project_files(p: &str) -> bool {
    p.starts_with("/api/projects/") && p.contains("/files")
}

/// Routes that run without a bearer token.
fn is_public(path: &str) -> bool {
    matches!(
        path,
        "/api/health" | "/api/auth/register" | "/api/auth/login" | "/api/auth/logout"
    )
}

/// Extract the bearer token from the `Authorization` header, if present.
fn bearer_token(req: &Request) -> Option<String> {
    req.headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|v| v.trim().to_string())
}

fn preflight() -> JsonResp {
    Response::builder()
        .status(204)
        .body(box_body(FullBody::new(Bytes::new())))
        .expect("valid status and headers")
}

/// Add permissive CORS headers so the frontend can be served from another
/// origin during development (e.g. `trunk serve` on port 8080).
fn with_cors(mut resp: JsonResp) -> JsonResp {
    let headers = resp.headers_mut();
    headers.insert("access-control-allow-origin", "*".parse().unwrap());
    headers.insert(
        "access-control-allow-methods",
        "GET, POST, PUT, DELETE, OPTIONS".parse().unwrap(),
    );
    headers.insert(
        "access-control-allow-headers",
        "content-type, authorization".parse().unwrap(),
    );
    resp
}
