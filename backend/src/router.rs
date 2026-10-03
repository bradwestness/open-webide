//! Segment routing for the Spin HTTP entry point.
use crate::api;
use crate::error::{ApiError, JsonResp};
use crate::state::AppState;
use bytes::Bytes;
use spin_sdk::http::{FullBody, Request, Response, box_body};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Route {
    Health,
    ThemeScript,
    Register,
    Login,
    Me,
    Logout,
    BridgeToken,
    ListConnections,
    ModelSetup,
    ModelDefaults,
    ModelProfile,
    ServerSettings,
    SaveServerSettings,
    ModelRuntime,
    DetectModel,
    InspectServer,
    DiscoverServers,
    CreateConnection,
    UpdateConnection,
    DeleteConnection,
    SetToolStreamUnsupported,
    GetSettings,
    SetSetting,
    ListSystemPrompts,
    CreateSystemPrompt,
    UpdateSystemPrompt,
    DeleteSystemPrompt,
    ListProjects,
    CreateProject,
    ListPendingEdits,
    ResolvePendingEdit,
    RenameProject,
    DeleteProject,
    Browse,
    ListSessions,
    CreateSession,
    RenameSession,
    SetSessionConnection,
    DeleteSession,
    SetToolPermission,
    ApprovalCheck,
    ListMessages,
    SendSessionMessage,
    RunPlan,
    CancelSession,
    PersistMessage,
    UpsertToolStep,
    CompleteToolStep,
    ListModels,
    ModelContext,
    Chat,
    ChatTools,
    WebSearch,
    WebFetch,
    FilesGet,
    FilesPut,
    FilesPost,
    FilesDelete,
    GitGet,
    GitPost,
}

impl Route {
    fn is_public(self) -> bool {
        matches!(
            self,
            Self::Health | Self::Register | Self::Login | Self::Logout
        )
    }
}

fn numeric_id(id: &str) -> bool {
    !id.is_empty() && id.bytes().all(|byte| byte.is_ascii_digit())
}

fn resolve(method: &str, segments: &[&str]) -> Option<Route> {
    match (method, segments) {
        ("GET", ["theme.js"]) => Some(Route::ThemeScript),
        ("GET", ["health"]) => Some(Route::Health),
        ("POST", ["auth", "register"]) => Some(Route::Register),
        ("POST", ["auth", "login"]) => Some(Route::Login),
        ("GET", ["auth", "me"]) => Some(Route::Me),
        ("POST", ["auth", "logout"]) => Some(Route::Logout),
        ("POST", ["bridge", "token"]) => Some(Route::BridgeToken),
        ("POST", ["model-setup", "detect"]) => Some(Route::DetectModel),
        ("POST", ["model-setup", "inspect"]) => Some(Route::InspectServer),
        ("POST", ["model-setup", "discover"]) => Some(Route::DiscoverServers),
        ("GET", ["model-setup"]) => Some(Route::ModelSetup),
        ("PUT", ["model-setup", "defaults"]) => Some(Route::ModelDefaults),
        ("PUT", ["model-setup", "profile"]) => Some(Route::ModelProfile),
        ("GET", ["connections", id, "settings"]) if numeric_id(id) => Some(Route::ServerSettings),
        ("PUT", ["connections", id, "settings"]) if numeric_id(id) => {
            Some(Route::SaveServerSettings)
        }
        ("GET", ["connections", id, "runtime"]) if numeric_id(id) => Some(Route::ModelRuntime),
        ("GET", ["connections"]) => Some(Route::ListConnections),
        ("POST", ["connections"]) => Some(Route::CreateConnection),
        ("PUT", ["connections", _]) => Some(Route::UpdateConnection),
        ("DELETE", ["connections", _]) => Some(Route::DeleteConnection),
        ("POST", ["connections", _, "tool-stream-unsupported"]) => {
            Some(Route::SetToolStreamUnsupported)
        }
        ("GET", ["settings"]) => Some(Route::GetSettings),
        ("PUT", ["settings"]) => Some(Route::SetSetting),
        ("GET", ["system-prompts"]) => Some(Route::ListSystemPrompts),
        ("POST", ["system-prompts"]) => Some(Route::CreateSystemPrompt),
        ("PUT", ["system-prompts", _]) => Some(Route::UpdateSystemPrompt),
        ("DELETE", ["system-prompts", _]) => Some(Route::DeleteSystemPrompt),
        ("GET", ["projects"]) => Some(Route::ListProjects),
        ("POST", ["projects"]) => Some(Route::CreateProject),
        ("PUT", ["projects", _]) => Some(Route::RenameProject),
        ("DELETE", ["projects", _]) => Some(Route::DeleteProject),
        ("GET", ["projects", id, "pending-edits"]) if numeric_id(id) => {
            Some(Route::ListPendingEdits)
        }
        ("POST", ["projects", id, "pending-edits", "resolve"]) if numeric_id(id) => {
            Some(Route::ResolvePendingEdit)
        }
        ("GET", ["browse"]) => Some(Route::Browse),
        ("GET", ["sessions"]) => Some(Route::ListSessions),
        ("POST", ["sessions"]) => Some(Route::CreateSession),
        ("PUT", ["sessions", id]) if numeric_id(id) => Some(Route::RenameSession),
        ("PUT", ["sessions", id, "connection"]) if numeric_id(id) => {
            Some(Route::SetSessionConnection)
        }
        ("DELETE", ["sessions", id]) if numeric_id(id) => Some(Route::DeleteSession),
        ("POST", ["sessions", _, "approval-check"]) => Some(Route::ApprovalCheck),
        ("POST", ["sessions", _, "permissions", _]) => Some(Route::SetToolPermission),
        ("GET", ["sessions", _, "messages"]) => Some(Route::ListMessages),
        ("POST", ["sessions", _, "messages"]) => Some(Route::SendSessionMessage),
        ("POST", ["sessions", _, "run-plan"]) => Some(Route::RunPlan),
        ("POST", ["sessions", _, "cancel"]) => Some(Route::CancelSession),
        ("POST", ["sessions", _, "messages", "persist"]) => Some(Route::PersistMessage),
        ("POST", ["sessions", _, "tool-steps", "upsert"]) => Some(Route::UpsertToolStep),
        ("POST", ["sessions", _, "tool-steps", "complete"]) => Some(Route::CompleteToolStep),
        ("GET", ["models"]) => Some(Route::ListModels),
        ("GET", ["models", "context"]) => Some(Route::ModelContext),
        ("POST", ["chat"]) => Some(Route::Chat),
        ("POST", ["chat-tools"]) => Some(Route::ChatTools),
        ("GET", ["web", "search"]) => Some(Route::WebSearch),
        ("GET", ["web", "fetch"]) => Some(Route::WebFetch),
        ("GET", ["projects", _, "files"]) => Some(Route::FilesGet),
        ("GET", ["projects", _, "files", "context"]) => Some(Route::FilesGet),
        ("GET", ["projects", _, "files", "read"]) => Some(Route::FilesGet),
        ("GET", ["projects", _, "files", "raw"]) => Some(Route::FilesGet),
        ("GET", ["projects", _, "files", "search"]) => Some(Route::FilesGet),
        ("GET", ["projects", _, "files", "content-search"]) => Some(Route::FilesGet),
        ("PUT", ["projects", _, "files", "write"]) => Some(Route::FilesPut),
        ("POST", ["projects", _, "files", "create"]) => Some(Route::FilesPost),
        ("POST", ["projects", _, "files", "copy"]) => Some(Route::FilesPost),
        ("DELETE", ["projects", _, "files", "delete"]) => Some(Route::FilesDelete),
        ("GET", ["git", "status"]) => Some(Route::GitGet),
        ("GET", ["projects", _, "git", "status"]) => Some(Route::GitGet),
        ("GET", ["git", "diff"]) => Some(Route::GitGet),
        ("GET", ["projects", _, "git", "diff"]) => Some(Route::GitGet),
        ("GET", ["git", "branches"]) => Some(Route::GitGet),
        ("GET", ["projects", _, "git", "branches"]) => Some(Route::GitGet),
        ("GET", ["git", "show"]) => Some(Route::GitGet),
        ("GET", ["projects", _, "git", "show"]) => Some(Route::GitGet),
        ("POST", ["git", "status"]) => Some(Route::GitPost),
        ("POST", ["projects", _, "git", "status"]) => Some(Route::GitPost),
        ("POST", ["git", "diff"]) => Some(Route::GitPost),
        ("POST", ["projects", _, "git", "diff"]) => Some(Route::GitPost),
        ("POST", ["git", "branches"]) => Some(Route::GitPost),
        ("POST", ["projects", _, "git", "branches"]) => Some(Route::GitPost),
        ("POST", ["git", "commit"]) => Some(Route::GitPost),
        ("POST", ["projects", _, "git", "commit"]) => Some(Route::GitPost),
        ("POST", ["git", "checkout"]) => Some(Route::GitPost),
        ("POST", ["projects", _, "git", "checkout"]) => Some(Route::GitPost),
        ("POST", ["git", "sync"]) => Some(Route::GitPost),
        ("POST", ["projects", _, "git", "sync"]) => Some(Route::GitPost),
        _ => None,
    }
}

pub async fn route(req: Request) -> JsonResp {
    let path = req.uri().path().to_string();
    let method = req.method().clone();
    let origin_header = req
        .headers()
        .get("origin")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let origin = origin_header.as_deref();
    if method.as_str() == "OPTIONS" {
        return with_cors(preflight(origin), origin);
    }
    let segments: Vec<&str> = path
        .strip_prefix("/api/")
        .unwrap_or("")
        .split('/')
        .collect();
    let route = resolve(method.as_str(), &segments);
    let state = match AppState::new().await {
        Ok(state) => state,
        Err(e) => {
            let error = ApiError::internal(format!("{e:#}"));
            error.log_for_route(method.as_str(), &path);
            return with_cors(error.into_response(), origin);
        }
    };
    let user = match authenticate_route(&state, req.headers(), route, &path).await {
        Ok(user) => user,
        Err(e) => {
            e.log_for_route(method.as_str(), &path);
            return with_cors(e.into_response(), origin);
        }
    };
    // Native bridge requests already passed shared-secret authentication above.
    let bridge_authenticated = req.headers().contains_key("authorization") && user.is_some();
    if !csrf_allowed(method.as_str(), &path, req.headers()) && !bridge_authenticated {
        return with_cors(
            ApiError::unauthorized("not signed in").into_response(),
            origin,
        );
    }
    let result = match (route, user) {
        (Some(Route::ThemeScript), user) => api::settings::theme_script(&state, user).await,
        (Some(Route::Health), None) => Ok(api::auth::health()),
        (Some(Route::Register), None) => api::auth::register(req, &state).await,
        (Some(Route::Login), None) => api::auth::login(req, &state).await,
        (Some(Route::Logout), None) => api::auth::logout(req, &state).await,
        (Some(Route::Me), Some(user)) => api::auth::me(&state, user).await,
        (Some(Route::BridgeToken), Some(user)) => api::bridge::bridge_token(&state, user).await,
        (Some(Route::DetectModel), Some(user)) => api::model_setup::detect(req, &state, user).await,
        (Some(Route::InspectServer), Some(_)) => api::model_setup::inspect(req).await,
        (Some(Route::DiscoverServers), Some(_)) => api::model_setup::discover().await,
        (Some(Route::ModelSetup), Some(user)) => api::model_setup::get(&state, user).await,
        (Some(Route::ModelDefaults), Some(user)) => {
            api::model_setup::defaults(req, &state, user).await
        }
        (Some(Route::ModelProfile), Some(user)) => {
            api::model_setup::profile(req, &state, user).await
        }
        (Some(Route::ServerSettings), Some(_)) => {
            api::model_setup::settings(req, &state, &path, false).await
        }
        (Some(Route::SaveServerSettings), Some(_)) => {
            api::model_setup::settings(req, &state, &path, true).await
        }
        (Some(Route::ModelRuntime), Some(user)) => {
            api::model_setup::native_runtime(req, &state, &path, user, bridge_authenticated).await
        }
        (Some(Route::ListConnections), Some(user)) => {
            api::connections::list_connections(&state, user).await
        }
        (Some(Route::CreateConnection), Some(user)) => {
            api::connections::create_connection(req, &state, user).await
        }
        (Some(Route::UpdateConnection), Some(user)) => {
            api::connections::update_connection(req, &state, &path, user).await
        }
        (Some(Route::DeleteConnection), Some(user)) => {
            api::connections::delete_connection(&state, &path, user).await
        }
        (Some(Route::SetToolStreamUnsupported), Some(user)) => {
            api::connections::set_tool_stream_unsupported(req, &state, &path, user).await
        }
        (Some(Route::GetSettings), Some(user)) => api::settings::get_settings(&state, user).await,
        (Some(Route::SetSetting), Some(user)) => {
            api::settings::set_setting(req, &state, user).await
        }
        (Some(Route::ListSystemPrompts), Some(user)) => {
            api::prompts::list_system_prompts(&state, user).await
        }
        (Some(Route::CreateSystemPrompt), Some(user)) => {
            api::prompts::create_system_prompt(req, &state, user).await
        }
        (Some(Route::UpdateSystemPrompt), Some(user)) => {
            api::prompts::update_system_prompt(req, &state, &path, user).await
        }
        (Some(Route::DeleteSystemPrompt), Some(user)) => {
            api::prompts::delete_system_prompt(&state, &path, user).await
        }
        (Some(Route::ListProjects), Some(user)) => api::projects::list_projects(&state, user).await,
        (Some(Route::CreateProject), Some(user)) => {
            api::projects::create_project(req, &state, user).await
        }
        (Some(Route::RenameProject), Some(user)) => {
            api::projects::rename_project(req, &state, &path, user).await
        }
        (Some(Route::DeleteProject), Some(user)) => {
            api::projects::delete_project(&state, &path, user).await
        }
        (Some(Route::ListPendingEdits), Some(user)) => {
            api::projects::list_pending_edits(&state, &path, user).await
        }
        (Some(Route::ResolvePendingEdit), Some(user)) => {
            api::projects::resolve_pending_edit(req, &state, &path, user).await
        }
        (Some(Route::Browse), Some(user)) => api::projects::browse(req, &state, user).await,
        (Some(Route::ListSessions), Some(user)) => api::sessions::list_sessions(&state, user).await,
        (Some(Route::CreateSession), Some(user)) => {
            api::sessions::create_session(req, &state, user).await
        }
        (Some(Route::RenameSession), Some(user)) => {
            api::sessions::rename_session(req, &state, &path, user).await
        }
        (Some(Route::SetSessionConnection), Some(user)) => {
            api::sessions::set_session_connection(req, &state, &path, user).await
        }
        (Some(Route::DeleteSession), Some(user)) => {
            api::sessions::delete_session(&state, &path, user).await
        }
        (Some(Route::ApprovalCheck), Some(user)) => {
            api::approvals::check(req, &state, &path, user).await
        }
        (Some(Route::SetToolPermission), Some(user)) => {
            api::sessions::set_tool_permission(req, &state, &path, user).await
        }
        (Some(Route::ListMessages), Some(user)) => {
            api::sessions::list_messages(&state, &path, user).await
        }
        (Some(Route::SendSessionMessage), Some(user)) => {
            api::sessions::send_session_message(req, state, &path, user).await
        }
        (Some(Route::RunPlan), Some(user)) => {
            api::sessions::run_plan(req, &state, &path, user).await
        }
        (Some(Route::CancelSession), Some(user)) => {
            api::sessions::cancel_session(&state, &path, user).await
        }
        (Some(Route::PersistMessage), Some(user)) => {
            api::sessions::persist_message(req, &state, &path, user).await
        }
        (Some(Route::UpsertToolStep), Some(user)) => {
            api::sessions::upsert_tool_step(req, &state, &path, user).await
        }
        (Some(Route::CompleteToolStep), Some(user)) => {
            api::sessions::complete_tool_step(req, &state, &path, user).await
        }
        (Some(Route::ListModels), Some(user)) => api::chat::list_models(req, &state, user).await,
        (Some(Route::ModelContext), Some(user)) => {
            api::chat::model_context(req, &state, user).await
        }
        (Some(Route::Chat), Some(user)) => api::chat::chat(req, &state, user).await,
        (Some(Route::ChatTools), Some(user)) => api::chat::chat_tools(req, &state, user).await,
        (Some(Route::WebSearch), Some(user)) => api::web::web_search(req, user).await,
        (Some(Route::WebFetch), Some(user)) => api::web::web_fetch(req, user).await,
        (Some(Route::FilesGet), Some(user)) => {
            api::files::files_get(req, &state, &path, user).await
        }
        (Some(Route::FilesPut), Some(user)) => {
            api::files::files_put(req, &state, &path, user).await
        }
        (Some(Route::FilesPost), Some(user)) => {
            api::files::files_post(req, &state, &path, user).await
        }
        (Some(Route::FilesDelete), Some(user)) => {
            api::files::files_delete(req, &state, &path, user).await
        }
        (Some(Route::GitGet), Some(user)) => api::git::git_get(req, &state, &path, user).await,
        (Some(Route::GitPost), Some(user)) => api::git::git_post(req, &state, &path, user).await,
        _ => Err(ApiError::not_found(format!("no route for {method} {path}"))),
    };
    let resp = match result {
        Ok(resp) => resp,
        Err(e) => {
            e.log_for_route(method.as_str(), &path);
            e.into_response()
        }
    };
    with_cors(resp, origin)
}

fn csrf_allowed(method: &str, path: &str, headers: &spin_sdk::http::HeaderMap) -> bool {
    (method == "GET" && path == "/api/theme.js") || crate::auth::csrf_header_ok(headers)
}

async fn authenticate_route(
    state: &AppState,
    headers: &spin_sdk::http::HeaderMap,
    route: Option<Route>,
    path: &str,
) -> Result<Option<crate::auth::AuthedUser>, ApiError> {
    if route == Some(Route::ThemeScript) {
        return crate::auth::theme_user(state, headers)
            .await
            .map(|user| user.map(Into::into));
    }
    if route.is_some_and(Route::is_public)
        || (route.is_none()
            && matches!(
                path,
                "/api/health" | "/api/auth/register" | "/api/auth/login" | "/api/auth/logout"
            ))
    {
        return Ok(None);
    }
    crate::auth::authenticate(state, headers)
        .await
        .map(|user| Some(user.into()))
}

fn preflight(_origin: Option<&str>) -> JsonResp {
    Response::builder()
        .status(204)
        .body(box_body(FullBody::new(Bytes::new())))
        .expect("valid status and headers")
}

/// Add permissive CORS headers so the frontend can be served from another
/// origin during development (e.g. `trunk serve` on port 8080).
fn with_cors(mut resp: JsonResp, origin: Option<&str>) -> JsonResp {
    let dev_origins = ["http://localhost:8080", "http://127.0.0.1:8080"];
    if let Some(o) = origin
        && dev_origins.contains(&o)
    {
        let headers = resp.headers_mut();
        headers.insert("access-control-allow-origin", o.parse().unwrap());
        headers.insert("access-control-allow-credentials", "true".parse().unwrap());
        headers.insert("vary", "origin".parse().unwrap());
        headers.insert(
            "access-control-allow-methods",
            "GET, POST, PUT, DELETE, OPTIONS".parse().unwrap(),
        );
        headers.insert(
            "access-control-allow-headers",
            "content-type, x-openwebide".parse().unwrap(),
        );
    }
    resp
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_exact_theme_get_skips_csrf_header() {
        let mut headers = spin_sdk::http::HeaderMap::new();
        assert!(csrf_allowed("GET", "/api/theme.js", &headers));
        for method in ["GET", "HEAD", "POST", "PUT", "DELETE", "PATCH"] {
            for path in [
                "/api/theme.js",
                "/api/theme.js/",
                "/api/theme.js/extra",
                "/api/settings",
                "/api/health",
                "/api/auth/login",
                "/api/unknown",
            ] {
                if method == "GET" && path == "/api/theme.js" {
                    continue;
                }
                assert!(!csrf_allowed(method, path, &headers), "{method} {path}");
            }
        }
        headers.insert("x-openwebide", "1".parse().unwrap());
        assert!(csrf_allowed("POST", "/api/theme.js", &headers));
        assert!(csrf_allowed("GET", "/api/settings", &headers));
    }

    #[test]
    fn theme_route_uses_only_valid_session_cookie_and_owned_setting() {
        futures::executor::block_on(async {
            use http_body_util::BodyExt;
            let state = AppState::new().await.unwrap();
            let mut invalid_cookie = spin_sdk::http::HeaderMap::new();
            invalid_cookie.insert("cookie", "owide_session=invalid".parse().unwrap());
            assert!(
                crate::auth::theme_user(&state, &invalid_cookie)
                    .await
                    .unwrap()
                    .is_none()
            );
            assert!(
                state
                    .store
                    .get_setting("auth_secret")
                    .await
                    .unwrap()
                    .is_none()
            );
            let alice = state
                .store
                .insert_user("alice", "hash", openwebide_core::UserRole::User, 1)
                .await
                .unwrap();
            let bob = state
                .store
                .insert_user("bob", "hash", openwebide_core::UserRole::User, 1)
                .await
                .unwrap();
            state
                .store
                .set_user_setting(bob.id, "theme", "dark")
                .await
                .unwrap();
            let no_theme = state
                .store
                .insert_user("no-theme", "hash", openwebide_core::UserRole::User, 1)
                .await
                .unwrap();
            let no_theme_token = crate::auth::issue_token(&state, &no_theme).await.unwrap();
            let token = crate::auth::issue_token(&state, &alice).await.unwrap();
            let mut headers = spin_sdk::http::HeaderMap::new();
            for (cookie, saved, expected) in [
                (Some(token.as_str()), Some("light"), "'light'"),
                (Some(token.as_str()), Some("dark"), "'dark'"),
                (Some(token.as_str()), Some("system"), "matchMedia"),
                (
                    Some(token.as_str()),
                    Some("invalid';alert(1)"),
                    "matchMedia",
                ),
                (None, Some("light"), "matchMedia"),
                (Some("invalid"), Some("dark"), "matchMedia"),
                (Some(no_theme_token.as_str()), None, "matchMedia"),
            ] {
                headers.remove("cookie");
                if let Some(cookie) = cookie {
                    headers.insert("cookie", format!("owide_session={cookie}").parse().unwrap());
                }
                state
                    .store
                    .set_user_setting(alice.id, "theme", saved.unwrap_or(""))
                    .await
                    .unwrap();
                let route = resolve("GET", &["theme.js"]);
                assert_eq!(route, Some(Route::ThemeScript));
                assert!(csrf_allowed("GET", "/api/theme.js", &headers));
                let user = authenticate_route(&state, &headers, route, "/api/theme.js")
                    .await
                    .unwrap();
                let response = api::settings::theme_script(&state, user).await.unwrap();
                assert_eq!(response.headers()["content-type"], "application/javascript");
                assert_eq!(response.headers()["cache-control"], "no-store");
                let body = response.into_body().collect().await.unwrap().to_bytes();
                let body = std::str::from_utf8(&body).unwrap();
                assert!(body.contains(expected), "{body}");
                assert_eq!(body.lines().count(), 1);
                assert!(!body.contains("alert"));
            }
            headers.insert("cookie", format!("owide_session={token}").parse().unwrap());
            for (method, path) in [
                ("GET", "/api/settings"),
                ("POST", "/api/theme.js"),
                ("HEAD", "/api/theme.js"),
                ("GET", "/api/theme.js/"),
            ] {
                let segments: Vec<_> = path.strip_prefix("/api/").unwrap().split('/').collect();
                assert!(
                    authenticate_route(&state, &headers, resolve(method, &segments), path)
                        .await
                        .is_err()
                );
            }
        });
    }

    #[test]
    fn unsupported_methods_on_public_paths_skip_authentication() {
        futures::executor::block_on(async {
            let state = AppState::new().await.unwrap();
            let headers = spin_sdk::http::HeaderMap::new();
            for (method, path, public) in [
                ("GET", "/api/auth/login", true),
                ("HEAD", "/api/health", true),
                ("DELETE", "/api/auth/logout", true),
                ("GET", "/api/auth/register", true),
                ("GET", "/api/auth/login/extra", false),
                ("GET", "/api/auth/login/", false),
                ("GET", "/api/unknown", false),
                ("HEAD", "/api/settings", false),
            ] {
                let segments: Vec<_> = path.strip_prefix("/api/").unwrap().split('/').collect();
                let route = resolve(method, &segments);
                assert_eq!(route, None, "{method} {path}");
                let result = authenticate_route(&state, &headers, route, path).await;
                if public {
                    assert!(result.unwrap().is_none(), "{method} {path}");
                } else {
                    assert_eq!(result.unwrap_err().into_response().status().as_u16(), 401);
                }
            }
        });
    }

    #[test]
    fn protected_routes_require_authentication() {
        futures::executor::block_on(async {
            let state = AppState::new().await.unwrap();
            let headers = spin_sdk::http::HeaderMap::new();
            for route in [Route::Health, Route::Register, Route::Login, Route::Logout] {
                assert!(
                    authenticate_route(&state, &headers, Some(route), "")
                        .await
                        .unwrap()
                        .is_none()
                );
            }
            for route in [
                Route::Me,
                Route::ListConnections,
                Route::FilesGet,
                Route::SendSessionMessage,
                Route::WebFetch,
            ] {
                let error = authenticate_route(&state, &headers, Some(route), "")
                    .await
                    .unwrap_err();
                assert_eq!(error.into_response().status().as_u16(), 401);
            }
        });
    }

    #[test]
    fn route_table() {
        for (method, path, expected) in [
            ("GET", "theme.js", Route::ThemeScript),
            ("GET", "health", Route::Health),
            ("POST", "auth/register", Route::Register),
            ("POST", "auth/login", Route::Login),
            ("GET", "auth/me", Route::Me),
            ("POST", "auth/logout", Route::Logout),
            ("POST", "bridge/token", Route::BridgeToken),
            ("GET", "connections", Route::ListConnections),
            ("POST", "connections", Route::CreateConnection),
            ("PUT", "connections/5", Route::UpdateConnection),
            ("DELETE", "connections/5", Route::DeleteConnection),
            (
                "POST",
                "connections/5/tool-stream-unsupported",
                Route::SetToolStreamUnsupported,
            ),
            ("GET", "settings", Route::GetSettings),
            ("PUT", "settings", Route::SetSetting),
            ("GET", "system-prompts", Route::ListSystemPrompts),
            ("POST", "system-prompts", Route::CreateSystemPrompt),
            ("PUT", "system-prompts/5", Route::UpdateSystemPrompt),
            ("DELETE", "system-prompts/5", Route::DeleteSystemPrompt),
            ("GET", "projects", Route::ListProjects),
            ("POST", "projects", Route::CreateProject),
            ("GET", "projects/5/pending-edits", Route::ListPendingEdits),
            (
                "POST",
                "projects/5/pending-edits/resolve",
                Route::ResolvePendingEdit,
            ),
            ("PUT", "projects/5", Route::RenameProject),
            ("DELETE", "projects/5", Route::DeleteProject),
            ("GET", "browse", Route::Browse),
            ("GET", "sessions", Route::ListSessions),
            ("POST", "sessions", Route::CreateSession),
            ("PUT", "sessions/5", Route::RenameSession),
            ("PUT", "sessions/5/connection", Route::SetSessionConnection),
            ("DELETE", "sessions/5", Route::DeleteSession),
            (
                "POST",
                "sessions/5/permissions/a1t1c0",
                Route::SetToolPermission,
            ),
            ("GET", "sessions/5/messages", Route::ListMessages),
            ("POST", "sessions/5/messages", Route::SendSessionMessage),
            ("POST", "sessions/5/run-plan", Route::RunPlan),
            ("POST", "sessions/5/cancel", Route::CancelSession),
            ("POST", "sessions/5/messages/persist", Route::PersistMessage),
            (
                "POST",
                "sessions/5/tool-steps/upsert",
                Route::UpsertToolStep,
            ),
            (
                "POST",
                "sessions/5/tool-steps/complete",
                Route::CompleteToolStep,
            ),
            ("GET", "models", Route::ListModels),
            ("GET", "models/context", Route::ModelContext),
            ("POST", "chat", Route::Chat),
            ("POST", "chat-tools", Route::ChatTools),
            ("GET", "web/search", Route::WebSearch),
            ("GET", "web/fetch", Route::WebFetch),
            ("GET", "projects/5/files", Route::FilesGet),
            ("GET", "projects/5/files/read", Route::FilesGet),
            ("GET", "projects/5/files/raw", Route::FilesGet),
            ("GET", "projects/5/files/search", Route::FilesGet),
            ("GET", "projects/5/files/content-search", Route::FilesGet),
            ("PUT", "projects/5/files/write", Route::FilesPut),
            ("POST", "projects/5/files/create", Route::FilesPost),
            ("POST", "projects/5/files/copy", Route::FilesPost),
            ("DELETE", "projects/5/files/delete", Route::FilesDelete),
            ("GET", "git/status", Route::GitGet),
            ("GET", "projects/5/git/status", Route::GitGet),
            ("GET", "git/diff", Route::GitGet),
            ("GET", "projects/5/git/diff", Route::GitGet),
            ("GET", "git/branches", Route::GitGet),
            ("GET", "projects/5/git/branches", Route::GitGet),
            ("GET", "git/show", Route::GitGet),
            ("GET", "projects/5/git/show", Route::GitGet),
            ("POST", "git/status", Route::GitPost),
            ("POST", "projects/5/git/status", Route::GitPost),
            ("POST", "git/diff", Route::GitPost),
            ("POST", "projects/5/git/diff", Route::GitPost),
            ("POST", "git/branches", Route::GitPost),
            ("POST", "projects/5/git/branches", Route::GitPost),
            ("POST", "git/commit", Route::GitPost),
            ("POST", "projects/5/git/commit", Route::GitPost),
            ("POST", "git/checkout", Route::GitPost),
            ("POST", "projects/5/git/checkout", Route::GitPost),
            ("POST", "git/sync", Route::GitPost),
            ("POST", "projects/5/git/sync", Route::GitPost),
        ] {
            let segments: Vec<_> = path.split('/').collect();
            assert_eq!(
                resolve(method, &segments),
                Some(expected),
                "{method} {path}"
            );
            assert_eq!(
                csrf_allowed(
                    method,
                    &format!("/api/{path}"),
                    &spin_sdk::http::HeaderMap::new()
                ),
                expected == Route::ThemeScript,
                "{method} {path}"
            );
            assert_eq!(
                expected.is_public(),
                matches!(
                    path,
                    "health" | "auth/register" | "auth/login" | "auth/logout"
                )
            );
        }
        for (method, path) in [
            ("DELETE", "sessions/5/anything"),
            ("GET", "unknown"),
            ("GET", "projects/5/pending-edits/extra"),
            ("GET", "projects/no/pending-edits"),
            ("GET", "projects/5/pending-edits/resolve"),
            ("POST", "projects/5/pending-edits/resolve/extra"),
            ("PUT", "projects/5/pending-edits/resolve"),
            ("PUT", "projects/5/files/wrong"),
            ("POST", "sessions/5/permissions/x/anything"),
        ] {
            assert_eq!(resolve(method, &path.split('/').collect::<Vec<_>>()), None);
        }
    }
}
