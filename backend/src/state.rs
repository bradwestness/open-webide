//! Per-request application state.

use anyhow::Context;
use openwebide_core::User;
use openwebide_storage::Store;

#[cfg(target_family = "wasm")]
pub type AppDb = openwebide_storage::spin_db::SpinDb;
#[cfg(not(target_family = "wasm"))]
pub type AppDb = openwebide_storage::rusqlite_db::RusqliteDb;

/// Spin components are stateless: each request gets a fresh connection to
/// the `sqlite` capability. Migrations are version-gated by `PRAGMA
/// user_version`, so an up-to-date schema costs a single read per request.
pub struct AppState {
    pub store: Store<AppDb>,
    /// The authenticated account for this request, set by the router after
    /// verifying the bearer token. `None` on public routes (health,
    /// register, login).
    pub current_user: Option<User>,
}

impl AppState {
    pub async fn new() -> anyhow::Result<Self> {
        #[cfg(target_family = "wasm")]
        let db = AppDb::open_default()
            .await
            .context("open default sqlite database")?;
        #[cfg(not(target_family = "wasm"))]
        let db = AppDb::open_in_memory().context("open in-memory sqlite database")?;
        let store = Store::new(db);
        store
            .migrate_with(&crate::files::dir_exists)
            .await
            .context("apply schema migrations")?;
        Ok(Self {
            store,
            current_user: None,
        })
    }
}

/// Current unix time in seconds, for `created_at` columns.
pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Current unix time in seconds, or `None` if the clock is unusable.
/// Expiry checks must fail closed on `None`.
pub fn unix_now_checked() -> Option<i64> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_secs()).ok())
}
