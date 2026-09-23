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
    // Agent tool steps, persisted so a session's steps survive a tab switch.
    // Kept out of `messages` (which feeds the LLM context); `anchor_message_id`
    // is the user message that started the turn, so steps render right after it.
    "CREATE TABLE IF NOT EXISTS tool_steps (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
        anchor_message_id INTEGER NOT NULL,
        tool_call_id TEXT NOT NULL,
        name TEXT NOT NULL,
        summary TEXT NOT NULL,
        ok INTEGER,
        result_summary TEXT,
        diff TEXT,
        created_at INTEGER NOT NULL,
        UNIQUE (session_id, tool_call_id)
    )",
    "CREATE INDEX IF NOT EXISTS idx_tool_steps_session ON tool_steps (session_id)",
    "CREATE TABLE IF NOT EXISTS run_cancels (
        session_id INTEGER PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE
    )",
    "CREATE TABLE IF NOT EXISTS tool_permissions (
        session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
        tool_call_id TEXT NOT NULL,
        decision INTEGER NOT NULL,
        PRIMARY KEY (session_id, tool_call_id)
    )",
    "CREATE TABLE IF NOT EXISTS user_settings (
        user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
        key TEXT NOT NULL,
        value TEXT NOT NULL,
        PRIMARY KEY (user_id, key)
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
    dedup_duplicate_projects(db).await?;
    migrate_legacy_settings_to_user_settings(db).await?;
    add_message_usage_columns(db).await?;
    add_connection_context_limit_column(db).await?;
    Ok(())
}

/// `ALTER TABLE ... ADD COLUMN` is not idempotent, so probe first.
async fn add_message_usage_columns<D: Db>(db: &D) -> Result<(), StorageError> {
    for column in [
        "prompt_tokens",
        "completion_tokens",
        "eval_duration_ms",
        "usage_estimated",
    ] {
        let res = db
            .execute(
                &format!("SELECT 1 FROM pragma_table_info('messages') WHERE name = '{column}'"),
                &[],
            )
            .await?;
        if res.rows.is_empty() {
            db.execute(
                &format!("ALTER TABLE messages ADD COLUMN {column} INTEGER"),
                &[],
            )
            .await?;
        }
    }
    Ok(())
}

/// `ALTER TABLE ... ADD COLUMN` is not idempotent, so probe first.
async fn add_connection_context_limit_column<D: Db>(db: &D) -> Result<(), StorageError> {
    let res = db
        .execute(
            "SELECT 1 FROM pragma_table_info('connections') WHERE name = 'context_limit'",
            &[],
        )
        .await?;
    if res.rows.is_empty() {
        db.execute(
            "ALTER TABLE connections ADD COLUMN context_limit INTEGER",
            &[],
        )
        .await?;
    }
    Ok(())
}

/// Migrate existing global preferences (theme, default_connection, default_prompt)
/// into user_settings for existing users.
async fn migrate_legacy_settings_to_user_settings<D: Db>(db: &D) -> Result<(), StorageError> {
    db.execute(
        "INSERT OR IGNORE INTO user_settings (user_id, key, value)
         SELECT u.id, s.key, s.value
         FROM users u
         CROSS JOIN settings s
         WHERE s.key != 'auth_secret'",
        &[],
    )
    .await?;
    Ok(())
}

/// Deduplicate any historical duplicate projects sharing (user_id, mode, path),
/// reassigning their sessions to the preserved project before removing the duplicate.
async fn dedup_duplicate_projects<D: Db>(db: &D) -> Result<(), StorageError> {
    use crate::db::DbValue;
    let res = db
        .execute(
            "SELECT COALESCE(user_id, 0), mode, path, MIN(id) as keep_id, COUNT(*) as cnt
             FROM projects
             WHERE path IS NOT NULL
             GROUP BY COALESCE(user_id, 0), mode, path
             HAVING cnt > 1",
            &[],
        )
        .await?;

    for row in res.rows {
        let user_id = row.get_int(0)?;
        let mode = row.get_text(1)?.to_string();
        let path = row.get_text(2)?.to_string();
        let keep_id = row.get_int(3)?;

        let dups = db
            .execute(
                "SELECT id FROM projects
                 WHERE COALESCE(user_id, 0) = ? AND mode = ? AND path = ? AND id != ?",
                &[
                    DbValue::Int(user_id),
                    DbValue::Text(mode),
                    DbValue::Text(path),
                    DbValue::Int(keep_id),
                ],
            )
            .await?;

        for dup_row in dups.rows {
            let dup_id = dup_row.get_int(0)?;
            db.execute(
                "UPDATE sessions SET project_id = ? WHERE project_id = ?",
                &[DbValue::Int(keep_id), DbValue::Int(dup_id)],
            )
            .await?;
            db.execute("DELETE FROM projects WHERE id = ?", &[DbValue::Int(dup_id)])
                .await?;
        }
    }
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
