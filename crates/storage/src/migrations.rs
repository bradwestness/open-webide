//! Schema migrations.
//!
//! Spin has no automatic migration runner, so the app applies migrations
//! on every request. Migrations are version-gated with `PRAGMA
//! user_version`: each numbered step runs exactly once, and a database
//! whose version is newer than this build's [`SCHEMA_VERSION`] refuses to
//! start (a rollback deploy fails loudly).
//!
//! Rules for changing the schema:
//! - Append a new numbered step at the end of [`apply_step`] and bump
//!   [`SCHEMA_VERSION`].
//! - Every step must stay idempotent: pre-versioning databases start at
//!   version 0 and replay every step.
//! - Never edit or reorder a shipped step.

use crate::StorageError;
use crate::db::Db;

/// The highest schema version this build knows how to apply.
pub const SCHEMA_VERSION: i64 = 11;

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

/// Run one numbered migration step (1-based, in landing order). `probe`
/// answers "does this mount-relative directory exist?" and is only
/// consulted by step 11.
async fn apply_step<D: Db>(
    db: &D,
    step: i64,
    probe: &(dyn Fn(&str) -> bool + Send + Sync),
) -> Result<(), StorageError> {
    match step {
        1 => {
            for stmt in MIGRATIONS {
                db.execute(stmt, &[]).await?;
            }
            Ok(())
        }
        2 => add_session_system_prompt_column(db).await,
        3 => add_session_project_column(db).await,
        4 => add_project_user_column(db).await,
        5 => add_session_user_column(db).await,
        6 => dedup_duplicate_projects(db).await,
        7 => migrate_legacy_settings_to_user_settings(db).await,
        8 => add_message_usage_columns(db).await,
        9 => add_connection_context_limit_column(db).await,
        10 => create_project_path_index(db).await,
        11 => rewrite_docker_workspace_paths(db, probe).await,
        other => Err(StorageError::Db(format!("unknown migration step {other}"))),
    }
}

/// Read the database's current schema version.
async fn read_user_version<D: Db>(db: &D) -> Result<i64, StorageError> {
    let res = db.execute("PRAGMA user_version", &[]).await?;
    Ok(res
        .rows
        .first()
        .map(|row| row.get_int(0))
        .transpose()?
        .unwrap_or(0))
}

/// Apply all pending migration steps up to [`SCHEMA_VERSION`].
///
/// `probe` answers "does this mount-relative directory exist?" for the
/// steps that need the filesystem (step 11); pass `&|_| false` when there
/// is no filesystem.
pub async fn apply<D: Db>(
    db: &D,
    probe: &(dyn Fn(&str) -> bool + Send + Sync),
) -> Result<(), StorageError> {
    let version = read_user_version(db).await?;
    if version == SCHEMA_VERSION {
        return Ok(());
    }
    if version > SCHEMA_VERSION {
        return Err(StorageError::Db(format!(
            "database schema version {version} is newer than this build ({SCHEMA_VERSION})"
        )));
    }
    db.transaction(|tx| async move {
        // Re-read inside the transaction: a concurrent first request may
        // have already migrated while we waited for the write lock.
        let version = read_user_version(&tx).await?;
        if version > SCHEMA_VERSION {
            return Err(StorageError::Db(format!(
                "database schema version {version} is newer than this build ({SCHEMA_VERSION})"
            )));
        }
        for step in (version + 1)..=SCHEMA_VERSION {
            apply_step(&tx, step, probe).await?;
        }
        // PRAGMA takes no bound parameters, so the version is formatted in.
        tx.execute(&format!("PRAGMA user_version = {SCHEMA_VERSION}"), &[])
            .await?;
        Ok(())
    })
    .await
}

/// Run steps `1..=target` without touching `user_version`, simulating a
/// pre-versioning database that an old build already migrated.
#[cfg(test)]
pub(crate) async fn apply_through<D: Db>(db: &D, target: i64) -> Result<(), StorageError> {
    for step in 1..=target {
        apply_step(db, step, &|_| false).await?;
    }
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
         WHERE s.key IN ('theme', 'default_connection', 'default_prompt')",
        &[],
    )
    .await?;
    Ok(())
}

/// Unique index backing `create_project`'s dedup: one project per
/// (owner, mode, path). Pathless projects are excluded, and SQLite treats
/// NULL `user_id` values as distinct in unique indexes.
async fn create_project_path_index<D: Db>(db: &D) -> Result<(), StorageError> {
    db.execute(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_projects_owner_path
         ON projects (user_id, mode, path) WHERE path IS NOT NULL",
        &[],
    )
    .await?;
    Ok(())
}

/// Rewrite pre-`/workspace`-mount Docker project paths. Before the mount,
/// remote paths were stored relative to the container root
/// (`workspace/foo`); now the mount root *is* `/workspace`, so those rows
/// point at `/workspace/workspace/foo`. A row is rewritten to its
/// `workspace/`-stripped form only when the stale directory is gone and
/// the stripped one exists on the mount (so a local install that really
/// has a `workspace/` folder is left alone). Rows whose owner already has
/// a project at the stripped path are skipped (the unique index).
async fn rewrite_docker_workspace_paths<D: Db>(
    db: &D,
    probe: &(dyn Fn(&str) -> bool + Send + Sync),
) -> Result<(), StorageError> {
    use crate::db::DbValue;
    let res = db
        .execute(
            "SELECT id, path FROM projects WHERE mode = 'remote' AND path LIKE 'workspace/%'",
            &[],
        )
        .await?;

    for row in res.rows {
        let id = row.get_int(0)?;
        let path = row.get_text(1)?.to_string();
        let Some(stripped) = path.strip_prefix("workspace/") else {
            continue;
        };
        if probe(&path) || !probe(stripped) {
            continue;
        }
        match db
            .execute(
                "UPDATE projects SET path = ? WHERE id = ?",
                &[DbValue::Text(stripped.to_string()), DbValue::Int(id)],
            )
            .await
        {
            Ok(_) => {}
            // Another project of the same owner already has the stripped
            // path (unique index); leave this row as is.
            Err(StorageError::Conflict(_)) => {}
            Err(e) => return Err(e),
        }
    }
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
