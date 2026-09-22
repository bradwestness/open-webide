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
    "CREATE TABLE IF NOT EXISTS users (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        username TEXT NOT NULL UNIQUE,
        password_hash TEXT NOT NULL,
        role TEXT NOT NULL CHECK (role IN ('admin', 'user')) DEFAULT 'user',
        created_at INTEGER NOT NULL
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
    "CREATE TABLE IF NOT EXISTS projects (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        name TEXT NOT NULL,
        mode TEXT NOT NULL CHECK (mode IN ('remote', 'local')),
        path TEXT,
        created_at INTEGER NOT NULL
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
    "CREATE TABLE IF NOT EXISTS run_cancels (
        session_id INTEGER PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE
    )",
];

pub async fn apply<D: Db>(db: &D) -> Result<(), StorageError> {
    for stmt in MIGRATIONS {
        db.execute(stmt, &[]).await?;
    }
    add_session_system_prompt_column(db).await?;
    add_session_project_column(db).await?;
    add_project_user_column(db).await?;
    add_session_user_column(db).await?;
    Ok(())
}

/// `ALTER TABLE ... ADD COLUMN` is not idempotent, so probe first. The
/// `users` table is created above before this runs, so the FK target exists.
async fn add_project_user_column<D: Db>(db: &D) -> Result<(), StorageError> {
    let res = db
        .execute(
            "SELECT 1 FROM pragma_table_info('projects') WHERE name = 'user_id'",
            &[],
        )
        .await?;
    if res.rows.is_empty() {
        db.execute(
            "ALTER TABLE projects ADD COLUMN user_id INTEGER REFERENCES users(id)",
            &[],
        )
        .await?;
    }
    Ok(())
}

/// `ALTER TABLE ... ADD COLUMN` is not idempotent, so probe first.
async fn add_session_user_column<D: Db>(db: &D) -> Result<(), StorageError> {
    let res = db
        .execute(
            "SELECT 1 FROM pragma_table_info('sessions') WHERE name = 'user_id'",
            &[],
        )
        .await?;
    if res.rows.is_empty() {
        db.execute(
            "ALTER TABLE sessions ADD COLUMN user_id INTEGER REFERENCES users(id)",
            &[],
        )
        .await?;
    }
    Ok(())
}

/// `ALTER TABLE ... ADD COLUMN` is not idempotent, so probe first.
async fn add_session_system_prompt_column<D: Db>(db: &D) -> Result<(), StorageError> {
    let res = db
        .execute(
            "SELECT 1 FROM pragma_table_info('sessions') WHERE name = 'system_prompt_id'",
            &[],
        )
        .await?;
    if res.rows.is_empty() {
        db.execute(
            "ALTER TABLE sessions ADD COLUMN system_prompt_id INTEGER
             REFERENCES system_prompts(id) ON DELETE SET NULL",
            &[],
        )
        .await?;
    }
    Ok(())
}

/// `ALTER TABLE ... ADD COLUMN` is not idempotent, so probe first. The
/// `projects` table is created above before this runs, so the FK target
/// exists.
async fn add_session_project_column<D: Db>(db: &D) -> Result<(), StorageError> {
    let res = db
        .execute(
            "SELECT 1 FROM pragma_table_info('sessions') WHERE name = 'project_id'",
            &[],
        )
        .await?;
    if res.rows.is_empty() {
        db.execute(
            "ALTER TABLE sessions ADD COLUMN project_id INTEGER
             REFERENCES projects(id) ON DELETE CASCADE",
            &[],
        )
        .await?;
    }
    Ok(())
}
