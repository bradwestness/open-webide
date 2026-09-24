//! Manual routing: Spin's `http_service` gives us one entry point, so we
//! dispatch on (method, path) ourselves.

use bytes::Bytes;
use spin_sdk::http::{FullBody, HeaderMap, Request, Response, box_body};

use crate::api;
use crate::error::{ApiError, JsonResp};
use crate::state::AppState;

pub async fn route(req: Request) -> JsonResp {
    let path = req.uri().path().to_string();
    let method = req.method().clone();
    // Read the token before the match below moves `req`.
    let token = token_from_headers(req.headers());

    let resp = if method.as_str() == "OPTIONS" {
        preflight()
    } else {
        let mut state = match AppState::new().await {
            Ok(state) => state,
            Err(e) => return with_cors(ApiError::internal(format!("{e:#}")).into_response()),
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
            ("GET", p) if is_project_git(p) => api::git_get(req, &state, p).await,
            ("POST", p) if is_project_git(p) => api::git_post(req, &state, p).await,
            ("GET", p) if p.starts_with("/api/git/") => api::git_get(req, &state, p).await,
            ("POST", p) if p.starts_with("/api/git/") => api::git_post(req, &state, p).await,
            ("PUT", p) if p.starts_with("/api/projects/") => {
                api::rename_project(req, &state, p).await
            }
            ("DELETE", p) if p.starts_with("/api/projects/") => {
                api::delete_project(&state, p).await
            }
            ("GET", "/api/browse") => api::browse(req, &state).await,
            ("GET", "/api/sessions") => api::list_sessions(&state).await,
            ("POST", "/api/sessions") => api::create_session(req, &state).await,
            ("PUT", p) if is_session_root(p) => api::rename_session(req, &state, p).await,
            ("DELETE", p) if is_session_root(p) => api::delete_session(&state, p).await,
            ("POST", p) if p.starts_with("/api/sessions/") && p.contains("/permissions/") => {
                api::set_tool_permission(req, &state, p).await
            }
            ("GET", p) if p.starts_with("/api/sessions/") && p.ends_with("/messages") => {
                api::list_messages(&state, p).await
            }
            ("POST", p) if p.starts_with("/api/sessions/") && p.ends_with("/messages") => {
                // Moves `state`: the store is consumed by the SSE stream.
                api::send_session_message(req, state, p).await
            }
            ("POST", p) if p.starts_with("/api/sessions/") && p.ends_with("/cancel") => {
                api::cancel_session(&state, p).await
            }
            ("POST", p) if p.starts_with("/api/sessions/") && p.ends_with("/messages/persist") => {
                api::persist_message(req, &state, p).await
            }
            ("POST", p) if p.starts_with("/api/sessions/") && p.ends_with("/tool-steps/upsert") => {
                api::upsert_tool_step(req, &state, p).await
            }
            ("POST", p)
                if p.starts_with("/api/sessions/") && p.ends_with("/tool-steps/complete") =>
            {
                api::complete_tool_step(req, &state, p).await
            }
            ("GET", "/api/models") => api::list_models(req, &state).await,
            ("GET", "/api/models/context") => api::model_context(req, &state).await,
            ("POST", "/api/chat") => api::chat(req, &state).await,
            ("POST", "/api/chat-tools") => api::chat_tools(req, &state).await,
            ("GET", "/api/web/search") => api::web_search(req).await,
            ("GET", "/api/web/fetch") => api::web_fetch(req).await,
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

/// Match the project git routes: `/api/projects/<id>/git/...`.
fn is_project_git(p: &str) -> bool {
    p.starts_with("/api/projects/") && p.contains("/git/")
}

/// Match a session root: `/api/sessions/<id>` where `<id>` is all ASCII digits.
fn is_session_root(p: &str) -> bool {
    p.strip_prefix("/api/sessions/")
        .is_some_and(|rest| !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()))
}

/// Routes that run without a bearer token.
fn is_public(path: &str) -> bool {
    matches!(
        path,
        "/api/health" | "/api/auth/register" | "/api/auth/login" | "/api/auth/logout"
    )
}

/// Extract the bearer token from request headers.
fn token_from_headers(headers: &HeaderMap) -> Option<String> {
    bearer_token_from(headers.get("authorization").and_then(|v| v.to_str().ok()))
}

/// Extract the bearer token from the `Authorization` header.
fn bearer_token_from(authorization: Option<&str>) -> Option<String> {
    authorization
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bearer_token_from() {
        assert_eq!(
            bearer_token_from(Some("Bearer foo")),
            Some("foo".to_string())
        );
        assert_eq!(
            bearer_token_from(Some("Bearer  foo  ")),
            Some("foo".to_string())
        );
        assert_eq!(bearer_token_from(Some("foo")), None);
        assert_eq!(bearer_token_from(None), None);
    }

    #[test]
    fn test_is_session_root() {
        assert!(is_session_root("/api/sessions/5"));
        assert!(!is_session_root("/api/sessions/5/messages"));
        assert!(!is_session_root("/api/sessions/5/tool-steps/x"));
        assert!(!is_session_root("/api/sessions/"));
        assert!(!is_session_root("/api/sessions/abc"));
    }

    #[test]
    fn test_url_token_unauthenticated() {
        // Build a request with `?token=...` query string and no `Authorization` header
        let req = spin_sdk::http::Request::builder()
            .method("GET")
            .uri("/api/sessions?token=fake")
            .body(())
            .unwrap();

        // Run it through the actual auth-checking path used by routes:
        // 1. extract the token
        let token = token_from_headers(req.headers());

        // 2. check authentication
        let state = futures::executor::block_on(crate::state::AppState::new()).unwrap();
        let auth_res =
            futures::executor::block_on(crate::auth::authenticate(&state, token.as_deref()));

        // Assert it is treated as unauthenticated
        assert!(auth_res.is_err());
    }
}
