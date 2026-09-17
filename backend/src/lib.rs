//! Open WebIDE backend: a Spin HTTP component exposing the REST API over
//! Spin's `sqlite` capability.

mod api;
mod error;
mod http_client;
mod router;
mod state;

use spin_sdk::http::{IntoResponse, Request};
use spin_sdk::http_service;

#[http_service]
async fn handle(req: Request) -> impl IntoResponse {
    router::route(req).await
}
