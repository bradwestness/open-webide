//! Schema migrations.
//!
//! Spin has no automatic migration runner, so the app re-applies this
//! idempotent DDL at startup; every statement is a no-op once its object
//! exists.

use crate::StorageError;
use crate::db::Db;

pub const MIGRATIONS: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS settings (
        key TEXT PRIMARY KEY,
        value TEXT NOT NULL
    )",
    "CREATE TABLE IF NOT EXISTS connections (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        name TEXT NOT NULL UNIQUE,
        kind TEXT NOT NULL CHECK (kind IN ('ollama', 'llamacpp')),
        base_url TEXT NOT NULL,
        model TEXT,
        enabled INTEGER NOT NULL DEFAULT 1
    )",
    "CREATE TABLE IF NOT EXISTS system_prompts (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        name TEXT NOT NULL UNIQUE,
        content TEXT NOT NULL
    )",
    "CREATE TABLE IF NOT EXISTS sessions (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        name TEXT NOT NULL,
        connection_id INTEGER REFERENCES connections(id) ON DELETE SET NULL,
        created_at INTEGER NOT NULL
    )",
    "CREATE TABLE IF NOT EXISTS messages (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
        role TEXT NOT NULL CHECK (role IN ('system', 'user', 'assistant')),
        content TEXT NOT NULL,
        created_at INTEGER NOT NULL
    )",
    "CREATE INDEX IF NOT EXISTS idx_messages_session ON messages (session_id)",
];

pub async fn apply<D: Db>(db: &D) -> Result<(), StorageError> {
    for stmt in MIGRATIONS {
        db.execute(stmt, &[]).await?;
    }
    Ok(())
}
