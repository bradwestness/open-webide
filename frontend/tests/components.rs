#![cfg(target_arch = "wasm32")]

wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

#[path = "components/models.rs"]
mod models;
#[path = "components/pending_edits.rs"]
mod pending_edits;
#[path = "components/permissions.rs"]
mod permissions;
#[path = "components/streaming.rs"]
mod streaming;
#[path = "components/support.rs"]
mod support;
