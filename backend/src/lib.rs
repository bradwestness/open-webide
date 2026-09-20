//! Open WebIDE backend: a Spin HTTP component exposing the REST API over
//! Spin's `sqlite` capability.

mod agent;
mod api;
mod auth;
mod error;
mod files;
mod http_client;
mod router;
mod sse;
mod state;

use spin_sdk::http::{IntoResponse, Request};
use spin_sdk::http_service;

// Generate bindings for the extra component dependencies declared in
// `spin-dependencies.wit` (the WASI filesystem, used for remote-mode projects).
spin_sdk::dependencies!();

#[http_service]
async fn handle(req: Request) -> impl IntoResponse {
    router::route(req).await
}
