//! SQLite persistence for Open WebIDE.
//!
//! The same repository code runs on the Spin backend (wasm, via the Spin
//! `sqlite` capability) and natively (via rusqlite) for tests.

pub mod db;
pub mod error;
pub mod migrations;
pub mod store;

#[cfg(not(target_family = "wasm"))]
pub mod rusqlite_db;
#[cfg(target_family = "wasm")]
pub mod spin_db;

pub use db::{Db, DbValue, ExecResult, QueryRow};
pub use error::StorageError;
pub use store::Store;
