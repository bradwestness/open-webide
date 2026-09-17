//! Per-request application state.

use anyhow::Context;
use openwebide_storage::{Store, spin_db::SpinDb};

/// Spin components are stateless: each request gets a fresh connection to
/// the `sqlite` capability. The schema is created with idempotent DDL, so
/// re-applying migrations per request is cheap.
pub struct AppState {
    pub store: Store<SpinDb>,
}

impl AppState {
    pub async fn new() -> anyhow::Result<Self> {
        let db = SpinDb::open_default()
            .await
            .context("open default sqlite database")?;
        let store = Store::new(db);
        store.migrate().await.context("apply schema migrations")?;
        Ok(Self { store })
    }
}

/// Current unix time in seconds, for `created_at` columns.
pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
