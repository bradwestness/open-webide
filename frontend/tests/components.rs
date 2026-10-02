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

#[path = "components/terminal.rs"]
mod terminal;

#[path = "components/runs.rs"]
mod runs;

#[path = "components/local_bridge.rs"]
mod local_bridge;

#[path = "components/persistence.rs"]
mod persistence;

#[path = "components/editor.rs"]
mod editor;

#[path = "components/search.rs"]
mod search;

#[path = "components/idb.rs"]
mod idb;
