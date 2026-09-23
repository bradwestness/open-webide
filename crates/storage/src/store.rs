//! Typed repositories over a [`Db`].

use std::collections::BTreeMap;

use openwebide_core::{
    ChatMessage, ChatSession, Connection, ConversationEntry, FileDiff, NewConnection, NewProject,
    Project, ProviderKind, Role, SystemPrompt, ToolStep, TurnTelemetry, User, UserRole,
    WorkspaceMode,
};

use crate::db::{Db, DbValue, QueryRow};
use crate::{StorageError, migrations};

pub struct Store<D: Db> {
    db: D,
}

/// A user row including the password hash. The hash is internal to storage;
/// the API never returns it (it maps to [`User`], which omits it).
#[derive(Debug, Clone)]
pub struct UserRecord {
    pub id: i64,
    pub username: String,
    pub password_hash: String,
    pub role: UserRole,
    pub created_at: i64,
}

impl UserRecord {
    /// The public form of this account, without the password hash.
    pub fn public(&self) -> User {
        User {
            id: self.id,
            username: self.username.clone(),
            role: self.role,
            created_at: self.created_at,
        }
    }
}

impl<D: Db> Store<D> {
    pub fn new(db: D) -> Self {
        Self { db }
    }

    /// Apply idempotent schema migrations.
    pub async fn migrate(&self) -> Result<(), StorageError> {
        migrations::apply(&self.db).await
    }

    // -- users -------------------------------------------------------------

    const USER_COLUMNS: &'static str = "id, username, password_hash, role, created_at";

    pub async fn insert_user(
        &self,
        username: &str,
        password_hash: &str,
        role: UserRole,
        created_at: i64,
    ) -> Result<UserRecord, StorageError> {
        self.db
            .execute(
                "INSERT INTO users (username, password_hash, role, created_at)
                 VALUES (?, ?, ?, ?)",
                &[
                    DbValue::Text(username.into()),
                    DbValue::Text(password_hash.into()),
                    DbValue::Text(role.as_str().into()),
                    DbValue::Int(created_at),
                ],
            )
            .await?;
        self.get_user_by_username(username).await?.ok_or_else(|| {
            StorageError::NotFound(format!("user {username} not found after insert"))
        })
    }

    pub async fn get_user_by_username(
        &self,
        username: &str,
    ) -> Result<Option<UserRecord>, StorageError> {
        let res = self
            .db
            .execute(
                &format!(
                    "SELECT {} FROM users WHERE username = ?",
                    Self::USER_COLUMNS
                ),
                &[DbValue::Text(username.into())],
            )
            .await?;
        res.rows.first().map(user_from_row).transpose()
    }

    pub async fn get_user(&self, id: i64) -> Result<Option<UserRecord>, StorageError> {
        let res = self
            .db
            .execute(
                &format!("SELECT {} FROM users WHERE id = ?", Self::USER_COLUMNS),
                &[DbValue::Int(id)],
            )
            .await?;
        res.rows.first().map(user_from_row).transpose()
    }

    pub async fn count_users(&self) -> Result<i64, StorageError> {
        let res = self.db.execute("SELECT COUNT(*) FROM users", &[]).await?;
        Ok(res
            .rows
            .first()
            .and_then(|r| r.values.first())
            .and_then(|v| match v {
                DbValue::Int(i) => Some(*i),
                _ => None,
            })
            .unwrap_or(0))
    }

    pub async fn list_users(&self) -> Result<Vec<UserRecord>, StorageError> {
        let res = self
            .db
            .execute(
                &format!("SELECT {} FROM users ORDER BY id", Self::USER_COLUMNS),
                &[],
            )
            .await?;
        res.rows.iter().map(user_from_row).collect()
    }

    /// Give the first registered user ownership of any projects created
    /// before accounts existed.
    pub async fn reassign_orphaned_projects(&self, user_id: i64) -> Result<u64, StorageError> {
        let res = self
            .db
            .execute(
                "UPDATE projects SET user_id = ? WHERE user_id IS NULL",
                &[DbValue::Int(user_id)],
            )
            .await?;
        Ok(res.changes)
    }

    /// Give the first registered user ownership of any sessions created before
    /// accounts existed, so pre-auth chat history isn't lost to scoping.
    pub async fn reassign_orphaned_sessions(&self, user_id: i64) -> Result<u64, StorageError> {
        let res = self
            .db
            .execute(
                "UPDATE sessions SET user_id = ? WHERE user_id IS NULL",
                &[DbValue::Int(user_id)],
            )
            .await?;
        Ok(res.changes)
    }

    // -- settings ----------------------------------------------------------

    pub async fn get_setting(&self, key: &str) -> Result<Option<String>, StorageError> {
        let res = self
            .db
            .execute(
                "SELECT value FROM settings WHERE key = ?",
                &[DbValue::Text(key.into())],
            )
            .await?;
        Ok(res
            .rows
            .first()
            .and_then(|r| r.get_text(0).ok().map(str::to_string)))
    }

    pub async fn set_setting(&self, key: &str, value: &str) -> Result<(), StorageError> {
        self.db
            .execute(
                "INSERT INTO settings (key, value) VALUES (?, ?)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                &[DbValue::Text(key.into()), DbValue::Text(value.into())],
            )
            .await?;
        Ok(())
    }

    pub async fn all_settings(&self) -> Result<BTreeMap<String, String>, StorageError> {
        let res = self
            .db
            .execute("SELECT key, value FROM settings ORDER BY key", &[])
            .await?;
        let mut map = BTreeMap::new();
        for row in &res.rows {
            map.insert(row.get_text(0)?.to_string(), row.get_text(1)?.to_string());
        }
        Ok(map)
    }

    // -- user settings -----------------------------------------------------

    pub async fn get_user_setting(
        &self,
        user_id: i64,
        key: &str,
    ) -> Result<Option<String>, StorageError> {
        let res = self
            .db
            .execute(
                "SELECT value FROM user_settings WHERE user_id = ? AND key = ?",
                &[DbValue::Int(user_id), DbValue::Text(key.into())],
            )
            .await?;
        Ok(res
            .rows
            .first()
            .and_then(|r| r.get_text(0).ok().map(str::to_string)))
    }

    pub async fn set_user_setting(
        &self,
        user_id: i64,
        key: &str,
        value: &str,
    ) -> Result<(), StorageError> {
        self.db
            .execute(
                "INSERT INTO user_settings (user_id, key, value) VALUES (?, ?, ?)
                 ON CONFLICT(user_id, key) DO UPDATE SET value = excluded.value",
                &[
                    DbValue::Int(user_id),
                    DbValue::Text(key.into()),
                    DbValue::Text(value.into()),
                ],
            )
            .await?;
        Ok(())
    }

    pub async fn all_user_settings(
        &self,
        user_id: i64,
    ) -> Result<BTreeMap<String, String>, StorageError> {
        let res = self
            .db
            .execute(
                "SELECT key, value FROM user_settings WHERE user_id = ? ORDER BY key",
                &[DbValue::Int(user_id)],
            )
            .await?;
        let mut map = BTreeMap::new();
        for row in &res.rows {
            map.insert(row.get_text(0)?.to_string(), row.get_text(1)?.to_string());
        }
        Ok(map)
    }

    // -- connections -------------------------------------------------------

    pub async fn list_connections(&self) -> Result<Vec<Connection>, StorageError> {
        let res = self
            .db
            .execute(
                "SELECT id, name, kind, base_url, model, enabled, context_limit
                 FROM connections ORDER BY id",
                &[],
            )
            .await?;
        res.rows.iter().map(connection_from_row).collect()
    }

    pub async fn get_connection(&self, id: i64) -> Result<Connection, StorageError> {
        let res = self
            .db
            .execute(
                "SELECT id, name, kind, base_url, model, enabled, context_limit
                 FROM connections WHERE id = ?",
                &[DbValue::Int(id)],
            )
            .await?;
        res.rows
            .first()
            .map(connection_from_row)
            .transpose()?
            .ok_or_else(|| StorageError::NotFound(format!("connection {id}")))
    }

    pub async fn insert_connection(&self, new: &NewConnection) -> Result<Connection, StorageError> {
        let res = self
            .db
            .execute(
                "INSERT INTO connections (name, kind, base_url, model, context_limit)
                 VALUES (?, ?, ?, ?, ?)",
                &[
                    DbValue::Text(new.name.clone()),
                    DbValue::Text(new.kind.as_str().into()),
                    DbValue::Text(new.base_url.clone()),
                    new.model
                        .as_ref()
                        .map(|m| DbValue::Text(m.clone()))
                        .unwrap_or(DbValue::Null),
                    new.context_limit
                        .map(|n| DbValue::Int(n as i64))
                        .unwrap_or(DbValue::Null),
                ],
            )
            .await?;
        self.get_connection(res.last_insert_rowid).await
    }

    pub async fn update_connection(&self, conn: &Connection) -> Result<(), StorageError> {
        let res = self
            .db
            .execute(
                "UPDATE connections
                 SET name = ?, kind = ?, base_url = ?, model = ?, enabled = ?, context_limit = ?
                 WHERE id = ?",
                &[
                    DbValue::Text(conn.name.clone()),
                    DbValue::Text(conn.kind.as_str().into()),
                    DbValue::Text(conn.base_url.clone()),
                    conn.model
                        .as_ref()
                        .map(|m| DbValue::Text(m.clone()))
                        .unwrap_or(DbValue::Null),
                    DbValue::Int(conn.enabled as i64),
                    conn.context_limit
                        .map(|n| DbValue::Int(n as i64))
                        .unwrap_or(DbValue::Null),
                    DbValue::Int(conn.id),
                ],
            )
            .await?;
        if res.changes == 0 {
            return Err(StorageError::NotFound(format!("connection {}", conn.id)));
        }
        Ok(())
    }

    pub async fn delete_connection(&self, id: i64) -> Result<(), StorageError> {
        let res = self
            .db
            .execute("DELETE FROM connections WHERE id = ?", &[DbValue::Int(id)])
            .await?;
        if res.changes == 0 {
            return Err(StorageError::NotFound(format!("connection {id}")));
        }
        Ok(())
    }

    // -- system prompts ------------------------------------------------------

    pub async fn list_system_prompts(&self) -> Result<Vec<SystemPrompt>, StorageError> {
        let res = self
            .db
            .execute(
                "SELECT id, name, content FROM system_prompts ORDER BY id",
                &[],
            )
            .await?;
        res.rows.iter().map(prompt_from_row).collect()
    }

    pub async fn insert_system_prompt(
        &self,
        name: &str,
        content: &str,
    ) -> Result<SystemPrompt, StorageError> {
        let res = self
            .db
            .execute(
                "INSERT INTO system_prompts (name, content) VALUES (?, ?)",
                &[DbValue::Text(name.into()), DbValue::Text(content.into())],
            )
            .await?;
        let row = res.rows.into_iter().next();
        let id = res.last_insert_rowid;
        let _ = row;
        self.db
            .execute(
                "SELECT id, name, content FROM system_prompts WHERE id = ?",
                &[DbValue::Int(id)],
            )
            .await
            .and_then(|res| {
                res.rows
                    .first()
                    .map(prompt_from_row)
                    .transpose()?
                    .ok_or_else(|| StorageError::NotFound(format!("system prompt {id}")))
            })
    }

    pub async fn get_system_prompt(&self, id: i64) -> Result<SystemPrompt, StorageError> {
        let res = self
            .db
            .execute(
                "SELECT id, name, content FROM system_prompts WHERE id = ?",
                &[DbValue::Int(id)],
            )
            .await?;
        res.rows
            .first()
            .map(prompt_from_row)
            .transpose()?
            .ok_or_else(|| StorageError::NotFound(format!("system prompt {id}")))
    }

    pub async fn update_system_prompt(
        &self,
        id: i64,
        name: &str,
        content: &str,
    ) -> Result<SystemPrompt, StorageError> {
        let res = self
            .db
            .execute(
                "UPDATE system_prompts SET name = ?, content = ? WHERE id = ?",
                &[
                    DbValue::Text(name.into()),
                    DbValue::Text(content.into()),
                    DbValue::Int(id),
                ],
            )
            .await?;
        if res.changes == 0 {
            return Err(StorageError::NotFound(format!("system prompt {id}")));
        }
        self.get_system_prompt(id).await
    }

    pub async fn delete_system_prompt(&self, id: i64) -> Result<(), StorageError> {
        let res = self
            .db
            .execute(
                "DELETE FROM system_prompts WHERE id = ?",
                &[DbValue::Int(id)],
            )
            .await?;
        if res.changes == 0 {
            return Err(StorageError::NotFound(format!("system prompt {id}")));
        }
        Ok(())
    }

    // -- projects ------------------------------------------------------------
    //
    // Every project is owned by a user; these methods always filter by
    // `user_id` so one account never sees another's projects.

    const PROJECT_COLUMNS: &'static str = "id, name, mode, path, user_id, created_at";

    pub async fn list_projects(&self, user_id: i64) -> Result<Vec<Project>, StorageError> {
        let res = self
            .db
            .execute(
                &format!(
                    "SELECT {} FROM projects WHERE user_id = ? ORDER BY id",
                    Self::PROJECT_COLUMNS
                ),
                &[DbValue::Int(user_id)],
            )
            .await?;
        res.rows.iter().map(project_from_row).collect()
    }

    pub async fn get_project(&self, id: i64, user_id: i64) -> Result<Project, StorageError> {
        let res = self
            .db
            .execute(
                &format!(
                    "SELECT {} FROM projects WHERE id = ? AND user_id = ?",
                    Self::PROJECT_COLUMNS
                ),
                &[DbValue::Int(id), DbValue::Int(user_id)],
            )
            .await?;
        res.rows
            .first()
            .map(project_from_row)
            .transpose()?
            .ok_or_else(|| StorageError::NotFound(format!("project {id}")))
    }

    pub async fn create_project(
        &self,
        new: &NewProject,
        user_id: i64,
        created_at: i64,
    ) -> Result<Project, StorageError> {
        // Re-opening a folder that already has a project returns that
        // project instead of creating a duplicate: closing a tab only hides
        // it, so the same folder can be opened again later.
        if let Some(path) = &new.path {
            let res = self
                .db
                .execute(
                    "SELECT id FROM projects
                     WHERE user_id = ? AND mode = ? AND path = ?",
                    &[
                        DbValue::Int(user_id),
                        DbValue::Text(new.mode.as_str().into()),
                        DbValue::Text(path.clone()),
                    ],
                )
                .await?;
            if let Some(row) = res.rows.first() {
                return self.get_project(row.get_int(0)?, user_id).await;
            }
        }
        let res = self
            .db
            .execute(
                "INSERT INTO projects (name, mode, path, user_id, created_at)
                 VALUES (?, ?, ?, ?, ?)",
                &[
                    DbValue::Text(new.name.clone()),
                    DbValue::Text(new.mode.as_str().into()),
                    new.path
                        .as_ref()
                        .map(|p| DbValue::Text(p.clone()))
                        .unwrap_or(DbValue::Null),
                    DbValue::Int(user_id),
                    DbValue::Int(created_at),
                ],
            )
            .await?;
        self.get_project(res.last_insert_rowid, user_id).await
    }

    pub async fn rename_project(
        &self,
        id: i64,
        name: &str,
        user_id: i64,
    ) -> Result<Project, StorageError> {
        let res = self
            .db
            .execute(
                "UPDATE projects SET name = ? WHERE id = ? AND user_id = ?",
                &[
                    DbValue::Text(name.into()),
                    DbValue::Int(id),
                    DbValue::Int(user_id),
                ],
            )
            .await?;
        if res.changes == 0 {
            return Err(StorageError::NotFound(format!("project {id}")));
        }
        self.get_project(id, user_id).await
    }

    pub async fn delete_project(&self, id: i64, user_id: i64) -> Result<(), StorageError> {
        // Remove the project's sessions and their messages first so no
        // orphans are left behind.
        self.db
            .execute(
                "DELETE FROM messages WHERE session_id IN (SELECT id FROM sessions WHERE project_id = ?)",
                &[DbValue::Int(id)],
            )
            .await?;
        self.db
            .execute(
                "DELETE FROM sessions WHERE project_id = ?",
                &[DbValue::Int(id)],
            )
            .await?;
        let res = self
            .db
            .execute(
                "DELETE FROM projects WHERE id = ? AND user_id = ?",
                &[DbValue::Int(id), DbValue::Int(user_id)],
            )
            .await?;
        if res.changes == 0 {
            return Err(StorageError::NotFound(format!("project {id}")));
        }
        Ok(())
    }

    // -- sessions ------------------------------------------------------------
    //
    // Sessions are owned by a user (set at creation); every lookup filters by
    // `user_id` so one account never reads another's conversations.

    const SESSION_COLUMNS: &'static str =
        "id, name, connection_id, system_prompt_id, project_id, user_id, created_at";

    pub async fn list_sessions(&self, user_id: i64) -> Result<Vec<ChatSession>, StorageError> {
        let res = self
            .db
            .execute(
                &format!(
                    "SELECT {} FROM sessions WHERE user_id = ? ORDER BY id",
                    Self::SESSION_COLUMNS
                ),
                &[DbValue::Int(user_id)],
            )
            .await?;
        res.rows.iter().map(session_from_row).collect()
    }

    pub async fn list_sessions_for_project(
        &self,
        project_id: i64,
        user_id: i64,
    ) -> Result<Vec<ChatSession>, StorageError> {
        let res = self
            .db
            .execute(
                &format!(
                    "SELECT {} FROM sessions WHERE project_id = ? AND user_id = ? ORDER BY id",
                    Self::SESSION_COLUMNS
                ),
                &[DbValue::Int(project_id), DbValue::Int(user_id)],
            )
            .await?;
        res.rows.iter().map(session_from_row).collect()
    }

    pub async fn get_session(&self, id: i64, user_id: i64) -> Result<ChatSession, StorageError> {
        let res = self
            .db
            .execute(
                &format!(
                    "SELECT {} FROM sessions WHERE id = ? AND user_id = ?",
                    Self::SESSION_COLUMNS
                ),
                &[DbValue::Int(id), DbValue::Int(user_id)],
            )
            .await?;
        res.rows
            .first()
            .map(session_from_row)
            .transpose()?
            .ok_or_else(|| StorageError::NotFound(format!("session {id}")))
    }

    pub async fn create_session(
        &self,
        name: &str,
        connection_id: Option<i64>,
        system_prompt_id: Option<i64>,
        project_id: Option<i64>,
        user_id: i64,
        created_at: i64,
    ) -> Result<ChatSession, StorageError> {
        let res = self
            .db
            .execute(
                "INSERT INTO sessions (name, connection_id, system_prompt_id, project_id, user_id, created_at)
                 VALUES (?, ?, ?, ?, ?, ?)",
                &[
                    DbValue::Text(name.into()),
                    connection_id.map(DbValue::Int).unwrap_or(DbValue::Null),
                    system_prompt_id.map(DbValue::Int).unwrap_or(DbValue::Null),
                    project_id.map(DbValue::Int).unwrap_or(DbValue::Null),
                    DbValue::Int(user_id),
                    DbValue::Int(created_at),
                ],
            )
            .await?;
        self.get_session(res.last_insert_rowid, user_id).await
    }

    pub async fn rename_session(
        &self,
        id: i64,
        name: &str,
        user_id: i64,
    ) -> Result<ChatSession, StorageError> {
        let res = self
            .db
            .execute(
                "UPDATE sessions SET name = ? WHERE id = ? AND user_id = ?",
                &[
                    DbValue::Text(name.into()),
                    DbValue::Int(id),
                    DbValue::Int(user_id),
                ],
            )
            .await?;
        if res.changes == 0 {
            return Err(StorageError::NotFound(format!("session {id}")));
        }
        self.get_session(id, user_id).await
    }

    pub async fn delete_session(&self, id: i64, user_id: i64) -> Result<(), StorageError> {
        let res = self
            .db
            .execute(
                "DELETE FROM sessions WHERE id = ? AND user_id = ?",
                &[DbValue::Int(id), DbValue::Int(user_id)],
            )
            .await?;
        if res.changes == 0 {
            return Err(StorageError::NotFound(format!("session {id}")));
        }
        Ok(())
    }

    // -- messages --------------------------------------------------------------

    pub async fn list_messages(&self, session_id: i64) -> Result<Vec<ChatMessage>, StorageError> {
        let res = self
            .db
            .execute(
                "SELECT id, session_id, role, content, created_at,
                        prompt_tokens, completion_tokens, eval_duration_ms, usage_estimated
                 FROM messages WHERE session_id = ? ORDER BY id",
                &[DbValue::Int(session_id)],
            )
            .await?;
        res.rows.iter().map(message_from_row).collect()
    }

    pub async fn insert_message(
        &self,
        session_id: i64,
        role: Role,
        content: &str,
        created_at: i64,
    ) -> Result<ChatMessage, StorageError> {
        self.insert_message_with_usage(session_id, role, content, created_at, None)
            .await
    }

    /// Insert a message, persisting the model call's usage alongside it (or
    /// NULLs when `usage` is `None`).
    pub async fn insert_message_with_usage(
        &self,
        session_id: i64,
        role: Role,
        content: &str,
        created_at: i64,
        usage: Option<&TurnTelemetry>,
    ) -> Result<ChatMessage, StorageError> {
        let res = self
            .db
            .execute(
                "INSERT INTO messages
                     (session_id, role, content, created_at,
                      prompt_tokens, completion_tokens, eval_duration_ms, usage_estimated)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
                &[
                    DbValue::Int(session_id),
                    DbValue::Text(role.as_str().into()),
                    DbValue::Text(content.into()),
                    DbValue::Int(created_at),
                    usage
                        .map(|u| DbValue::Int(u.prompt_tokens as i64))
                        .unwrap_or(DbValue::Null),
                    usage
                        .map(|u| DbValue::Int(u.completion_tokens as i64))
                        .unwrap_or(DbValue::Null),
                    usage
                        .map(|u| DbValue::Int(u.eval_duration_ms as i64))
                        .unwrap_or(DbValue::Null),
                    usage
                        .map(|u| DbValue::Int(u.estimated as i64))
                        .unwrap_or(DbValue::Null),
                ],
            )
            .await?;
        Ok(ChatMessage {
            id: res.last_insert_rowid,
            session_id,
            role,
            content: content.into(),
            created_at,
            tool_calls: None,
            tool_call_id: None,
            usage: usage.copied(),
        })
    }

    // -- tool steps ------------------------------------------------------------
    //
    // Agent tool steps are persisted separately from chat messages so a
    // session's steps survive a tab switch without polluting the LLM context.
    // `anchor_message_id` is the user message that started the turn; a step
    // renders right after it.

    /// Record (or refresh) a tool step as it is requested. Called for both a
    /// `tool_call` and a `permission_request` (which share an id), so a gated
    /// write is stored once.
    pub async fn upsert_tool_step(
        &self,
        session_id: i64,
        anchor_message_id: i64,
        tool_call_id: &str,
        name: &str,
        summary: &str,
        created_at: i64,
    ) -> Result<(), StorageError> {
        self.db
            .execute(
                "INSERT INTO tool_steps
                     (session_id, anchor_message_id, tool_call_id, name, summary, created_at)
                 VALUES (?, ?, ?, ?, ?, ?)
                 ON CONFLICT (session_id, tool_call_id)
                 DO UPDATE SET name = excluded.name, summary = excluded.summary",
                &[
                    DbValue::Int(session_id),
                    DbValue::Int(anchor_message_id),
                    DbValue::Text(tool_call_id.into()),
                    DbValue::Text(name.into()),
                    DbValue::Text(summary.into()),
                    DbValue::Int(created_at),
                ],
            )
            .await?;
        Ok(())
    }

    /// Fill in a tool step's outcome once it finishes.
    pub async fn complete_tool_step(
        &self,
        session_id: i64,
        tool_call_id: &str,
        ok: bool,
        result_summary: &str,
        diff: Option<&FileDiff>,
    ) -> Result<(), StorageError> {
        let diff_json = diff
            .map(|d| serde_json::to_string(d).map_err(|e| StorageError::Db(e.to_string())))
            .transpose()?;
        self.db
            .execute(
                "UPDATE tool_steps
                 SET ok = ?, result_summary = ?, diff = ?
                 WHERE session_id = ? AND tool_call_id = ?",
                &[
                    DbValue::Int(i64::from(ok)),
                    DbValue::Text(result_summary.into()),
                    diff_json.map(DbValue::Text).unwrap_or(DbValue::Null),
                    DbValue::Int(session_id),
                    DbValue::Text(tool_call_id.into()),
                ],
            )
            .await?;
        Ok(())
    }

    /// The session's tool steps in the order they were recorded.
    pub async fn list_tool_steps(&self, session_id: i64) -> Result<Vec<ToolStep>, StorageError> {
        let res = self
            .db
            .execute(
                "SELECT tool_call_id, name, summary, ok, result_summary, diff, anchor_message_id
                 FROM tool_steps WHERE session_id = ? ORDER BY id",
                &[DbValue::Int(session_id)],
            )
            .await?;
        res.rows.iter().map(tool_step_from_row).collect()
    }

    /// The session's conversation as a single ordered list of messages and
    /// tool steps: each message, then the steps anchored to it. This is what
    /// the message-list endpoint returns so a reloaded session shows its steps.
    pub async fn list_conversation(
        &self,
        session_id: i64,
    ) -> Result<Vec<ConversationEntry>, StorageError> {
        let messages = self.list_messages(session_id).await?;
        let steps = self.list_tool_steps(session_id).await?;
        let mut by_anchor: BTreeMap<i64, Vec<ToolStep>> = BTreeMap::new();
        for step in steps {
            by_anchor
                .entry(step.anchor_message_id)
                .or_default()
                .push(step);
        }
        let mut out: Vec<ConversationEntry> = Vec::new();
        for message in messages {
            let anchor = message.id;
            out.push(ConversationEntry::Message(message));
            if let Some(steps) = by_anchor.remove(&anchor) {
                out.extend(steps.into_iter().map(ConversationEntry::ToolStep));
            }
        }
        // Steps whose anchor message is gone (shouldn't happen) go at the end.
        for steps in by_anchor.into_values() {
            out.extend(steps.into_iter().map(ConversationEntry::ToolStep));
        }
        Ok(out)
    }

    // -- run cancellation ----------------------------------------------------

    /// Mark the session's in-flight run for cancellation. The streaming
    /// request polls [`Self::cancel_requested`] at step boundaries; Spin
    /// requests are stateless, so the flag lives in the database.
    pub async fn request_cancel(&self, session_id: i64) -> Result<(), StorageError> {
        self.db
            .execute(
                "INSERT OR IGNORE INTO run_cancels (session_id) VALUES (?)",
                &[DbValue::Int(session_id)],
            )
            .await?;
        Ok(())
    }

    /// Whether a cancel has been requested for the session's in-flight run.
    pub async fn cancel_requested(&self, session_id: i64) -> Result<bool, StorageError> {
        let res = self
            .db
            .execute(
                "SELECT 1 FROM run_cancels WHERE session_id = ?",
                &[DbValue::Int(session_id)],
            )
            .await?;
        Ok(!res.rows.is_empty())
    }

    /// Clear the cancel flag, called when a run finishes or a new one starts.
    pub async fn clear_cancel(&self, session_id: i64) -> Result<(), StorageError> {
        self.db
            .execute(
                "DELETE FROM run_cancels WHERE session_id = ?",
                &[DbValue::Int(session_id)],
            )
            .await?;
        Ok(())
    }

    // -- tool permissions ----------------------------------------------------

    /// Record the user's decision on a gated tool call. The in-flight run
    /// polls [`Self::tool_permission`] until it arrives; Spin requests are
    /// stateless, so the decision lives in the database.
    pub async fn set_tool_permission(
        &self,
        session_id: i64,
        tool_call_id: &str,
        approved: bool,
    ) -> Result<(), StorageError> {
        self.db
            .execute(
                "INSERT OR REPLACE INTO tool_permissions \
                 (session_id, tool_call_id, decision) VALUES (?, ?, ?)",
                &[
                    DbValue::Int(session_id),
                    DbValue::Text(tool_call_id.to_string()),
                    DbValue::Int(approved as i64),
                ],
            )
            .await?;
        Ok(())
    }

    /// The user's decision on a gated tool call, if one has been recorded.
    pub async fn tool_permission(
        &self,
        session_id: i64,
        tool_call_id: &str,
    ) -> Result<Option<bool>, StorageError> {
        let res = self
            .db
            .execute(
                "SELECT decision FROM tool_permissions \
                 WHERE session_id = ? AND tool_call_id = ?",
                &[
                    DbValue::Int(session_id),
                    DbValue::Text(tool_call_id.to_string()),
                ],
            )
            .await?;
        Ok(res
            .rows
            .first()
            .map(|row| row.get_int(0).map(|d| d != 0).unwrap_or(false)))
    }

    /// Clear recorded decisions, called when a run finishes or a new one
    /// starts.
    pub async fn clear_tool_permissions(&self, session_id: i64) -> Result<(), StorageError> {
        self.db
            .execute(
                "DELETE FROM tool_permissions WHERE session_id = ?",
                &[DbValue::Int(session_id)],
            )
            .await?;
        Ok(())
    }
}

fn connection_from_row(row: &QueryRow) -> Result<Connection, StorageError> {
    let kind = row.get_text(2)?;
    Ok(Connection {
        id: row.get_int(0)?,
        name: row.get_text(1)?.to_string(),
        kind: ProviderKind::parse(kind)
            .ok_or_else(|| StorageError::InvalidValue(format!("unknown provider kind: {kind}")))?,
        base_url: row.get_text(3)?.to_string(),
        model: row.get_text_opt(4).map(str::to_string),
        enabled: row.get_int(5)? != 0,
        context_limit: row.get_int_opt(6).filter(|&n| n > 0).map(|n| n as usize),
    })
}

fn prompt_from_row(row: &QueryRow) -> Result<SystemPrompt, StorageError> {
    Ok(SystemPrompt {
        id: row.get_int(0)?,
        name: row.get_text(1)?.to_string(),
        content: row.get_text(2)?.to_string(),
    })
}

fn opt_int(row: &QueryRow, idx: usize, field: &str) -> Result<Option<i64>, StorageError> {
    match &row.values[idx] {
        DbValue::Int(i) => Ok(Some(*i)),
        DbValue::Null => Ok(None),
        other => Err(StorageError::InvalidValue(format!(
            "{field} is not an integer: {other:?}"
        ))),
    }
}

fn session_from_row(row: &QueryRow) -> Result<ChatSession, StorageError> {
    Ok(ChatSession {
        id: row.get_int(0)?,
        name: row.get_text(1)?.to_string(),
        connection_id: opt_int(row, 2, "connection_id")?,
        system_prompt_id: opt_int(row, 3, "system_prompt_id")?,
        project_id: opt_int(row, 4, "project_id")?,
        user_id: opt_int(row, 5, "user_id")?,
        created_at: row.get_int(6)?,
    })
}

fn project_from_row(row: &QueryRow) -> Result<Project, StorageError> {
    let mode = row.get_text(2)?;
    Ok(Project {
        id: row.get_int(0)?,
        name: row.get_text(1)?.to_string(),
        mode: WorkspaceMode::parse(mode)
            .ok_or_else(|| StorageError::InvalidValue(format!("unknown workspace mode: {mode}")))?,
        path: row.get_text_opt(3).map(str::to_string),
        user_id: opt_int(row, 4, "user_id")?,
        created_at: row.get_int(5)?,
    })
}

fn user_from_row(row: &QueryRow) -> Result<UserRecord, StorageError> {
    let role = row.get_text(3)?;
    Ok(UserRecord {
        id: row.get_int(0)?,
        username: row.get_text(1)?.to_string(),
        password_hash: row.get_text(2)?.to_string(),
        role: UserRole::parse(role)
            .ok_or_else(|| StorageError::InvalidValue(format!("unknown role: {role}")))?,
        created_at: row.get_int(4)?,
    })
}

fn message_from_row(row: &QueryRow) -> Result<ChatMessage, StorageError> {
    let role = row.get_text(2)?;
    let prompt_tokens = row.get_int_opt(5);
    let completion_tokens = row.get_int_opt(6);
    let usage = if prompt_tokens.is_some() || completion_tokens.is_some() {
        Some(TurnTelemetry {
            prompt_tokens: prompt_tokens.unwrap_or(0) as usize,
            completion_tokens: completion_tokens.unwrap_or(0) as usize,
            eval_duration_ms: row.get_int_opt(7).unwrap_or(0) as u64,
            estimated: row.get_int_opt(8).map(|v| v != 0).unwrap_or(false),
        })
    } else {
        None
    };
    Ok(ChatMessage {
        id: row.get_int(0)?,
        session_id: row.get_int(1)?,
        role: Role::parse(role)
            .ok_or_else(|| StorageError::InvalidValue(format!("unknown role: {role}")))?,
        content: row.get_text(3)?.to_string(),
        created_at: row.get_int(4)?,
        tool_calls: None,
        tool_call_id: None,
        usage,
    })
}

fn tool_step_from_row(row: &QueryRow) -> Result<ToolStep, StorageError> {
    let ok = match &row.values[3] {
        DbValue::Int(i) => Some(*i != 0),
        DbValue::Null => None,
        other => {
            return Err(StorageError::InvalidValue(format!(
                "tool step ok is not an integer: {other:?}"
            )));
        }
    };
    let diff = row
        .get_text_opt(5)
        .map(serde_json::from_str::<FileDiff>)
        .transpose()
        .map_err(|e| StorageError::InvalidValue(format!("bad tool step diff: {e}")))?;
    Ok(ToolStep {
        tool_call_id: row.get_text(0)?.to_string(),
        name: row.get_text(1)?.to_string(),
        summary: row.get_text(2)?.to_string(),
        ok,
        result_summary: row.get_text_opt(4).map(str::to_string),
        diff,
        anchor_message_id: row.get_int(6)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rusqlite_db::RusqliteDb;
    use futures::executor::block_on;

    fn test_store() -> Store<RusqliteDb> {
        let db = RusqliteDb::open_in_memory().unwrap();
        let store = Store::new(db);
        block_on(store.migrate()).unwrap();
        store
    }

    /// Create a user and return its id, for tests that need scoped data.
    /// Call this *before* the test's `block_on` block (it runs its own
    /// executor, so it must not be nested inside one).
    fn test_user(store: &Store<RusqliteDb>, username: &str, role: UserRole) -> i64 {
        block_on(async {
            store
                .insert_user(username, "hash", role, 1)
                .await
                .unwrap()
                .id
        })
    }

    #[test]
    fn settings_roundtrip() {
        let store = test_store();
        block_on(async {
            store.set_setting("theme", "dark").await.unwrap();
            assert_eq!(
                store.get_setting("theme").await.unwrap().as_deref(),
                Some("dark")
            );
            store.set_setting("theme", "light").await.unwrap();
            assert_eq!(
                store.get_setting("theme").await.unwrap().as_deref(),
                Some("light")
            );
            assert_eq!(store.get_setting("missing").await.unwrap(), None);
            let all = store.all_settings().await.unwrap();
            assert_eq!(all.len(), 1);
            assert_eq!(all.get("theme").map(String::as_str), Some("light"));
        });
    }

    #[test]
    fn user_settings_roundtrip_and_scoping() {
        let store = test_store();
        let alice = test_user(&store, "alice", UserRole::Admin);
        let bob = test_user(&store, "bob", UserRole::User);
        block_on(async {
            store
                .set_user_setting(alice, "open_tabs", "[1, 2]")
                .await
                .unwrap();
            store
                .set_user_setting(alice, "active_project", "2")
                .await
                .unwrap();
            store
                .set_user_setting(bob, "open_tabs", "[3]")
                .await
                .unwrap();

            assert_eq!(
                store
                    .get_user_setting(alice, "open_tabs")
                    .await
                    .unwrap()
                    .as_deref(),
                Some("[1, 2]")
            );
            assert_eq!(
                store
                    .get_user_setting(alice, "active_project")
                    .await
                    .unwrap()
                    .as_deref(),
                Some("2")
            );
            assert_eq!(
                store
                    .get_user_setting(bob, "open_tabs")
                    .await
                    .unwrap()
                    .as_deref(),
                Some("[3]")
            );
            assert_eq!(
                store.get_user_setting(bob, "active_project").await.unwrap(),
                None
            );

            let alice_all = store.all_user_settings(alice).await.unwrap();
            assert_eq!(alice_all.len(), 2);
            assert_eq!(
                alice_all.get("open_tabs").map(String::as_str),
                Some("[1, 2]")
            );
        });
    }

    #[test]
    fn connection_crud() {
        let store = test_store();
        block_on(async {
            let conn = store
                .insert_connection(&NewConnection {
                    name: "local-ollama".into(),
                    kind: ProviderKind::Ollama,
                    base_url: "http://localhost:11434".into(),
                    model: Some("qwen2.5-coder:7b".into()),
                    context_limit: Some(8192),
                })
                .await
                .unwrap();
            assert!(conn.id > 0);
            assert!(conn.enabled);
            assert_eq!(conn.model.as_deref(), Some("qwen2.5-coder:7b"));
            assert_eq!(conn.context_limit, Some(8192));

            let mut conn = store.get_connection(conn.id).await.unwrap();
            assert_eq!(conn.context_limit, Some(8192));
            conn.name = "renamed".into();
            conn.enabled = false;
            conn.context_limit = Some(32_768);
            store.update_connection(&conn).await.unwrap();
            let reloaded = store.get_connection(conn.id).await.unwrap();
            assert_eq!(reloaded.name, "renamed");
            assert!(!reloaded.enabled);
            assert_eq!(reloaded.context_limit, Some(32_768));

            assert_eq!(store.list_connections().await.unwrap().len(), 1);
            assert_eq!(
                store.list_connections().await.unwrap()[0].context_limit,
                Some(32_768)
            );

            // Clearing the field writes NULL, which round-trips as `None`.
            let mut cleared = reloaded;
            cleared.context_limit = None;
            store.update_connection(&cleared).await.unwrap();
            assert_eq!(
                store
                    .get_connection(cleared.id)
                    .await
                    .unwrap()
                    .context_limit,
                None
            );

            store.delete_connection(cleared.id).await.unwrap();
            assert!(store.get_connection(cleared.id).await.is_err());
            assert!(store.list_connections().await.unwrap().is_empty());
        });
    }

    #[test]
    fn system_prompt_crud() {
        let store = test_store();
        block_on(async {
            let prompt = store
                .insert_system_prompt("coder", "You are a coding agent.")
                .await
                .unwrap();
            assert!(prompt.id > 0);
            assert_eq!(prompt.name, "coder");

            assert_eq!(store.list_system_prompts().await.unwrap().len(), 1);

            let updated = store
                .update_system_prompt(prompt.id, "helper", "You are a helpful agent.")
                .await
                .unwrap();
            assert_eq!(updated.id, prompt.id);
            assert_eq!(updated.name, "helper");
            assert_eq!(updated.content, "You are a helpful agent.");

            assert!(store.update_system_prompt(9999, "x", "y").await.is_err());

            store.delete_system_prompt(prompt.id).await.unwrap();
            assert!(store.list_system_prompts().await.unwrap().is_empty());
        });
    }

    #[test]
    fn sessions_and_messages() {
        let store = test_store();
        let user_id = test_user(&store, "alice", UserRole::Admin);
        block_on(async {
            let session = store
                .create_session("first session", None, None, None, user_id, 1_700_000_000)
                .await
                .unwrap();
            assert!(session.id > 0);
            assert_eq!(session.system_prompt_id, None);
            assert_eq!(session.user_id, Some(user_id));

            let reloaded = store.get_session(session.id, user_id).await.unwrap();
            assert_eq!(reloaded.name, "first session");

            let renamed = store
                .rename_session(session.id, "renamed", user_id)
                .await
                .unwrap();
            assert_eq!(renamed.name, "renamed");

            let msg = store
                .insert_message(session.id, Role::User, "hello", 1_700_000_001)
                .await
                .unwrap();
            assert_eq!(msg.id, 1);

            let messages = store.list_messages(session.id).await.unwrap();
            assert_eq!(messages.len(), 1);
            assert_eq!(messages[0].role, Role::User);
            assert_eq!(messages[0].content, "hello");
            assert_eq!(messages[0].usage, None);

            store.delete_session(session.id, user_id).await.unwrap();
            assert!(store.get_session(session.id, user_id).await.is_err());
            assert!(store.list_messages(session.id).await.unwrap().is_empty());
        });
    }

    #[test]
    fn message_usage_roundtrips_through_list_messages_and_conversation() {
        let store = test_store();
        let user_id = test_user(&store, "alice", UserRole::Admin);
        block_on(async {
            let session = store
                .create_session("s", None, None, None, user_id, 1)
                .await
                .unwrap();
            let usage = TurnTelemetry {
                prompt_tokens: 120,
                completion_tokens: 30,
                eval_duration_ms: 900,
                estimated: true,
            };
            store
                .insert_message_with_usage(session.id, Role::User, "hi", 2, None)
                .await
                .unwrap();
            let assistant = store
                .insert_message_with_usage(session.id, Role::Assistant, "hello", 3, Some(&usage))
                .await
                .unwrap();
            assert_eq!(assistant.usage, Some(usage));

            let messages = store.list_messages(session.id).await.unwrap();
            assert_eq!(messages[0].usage, None);
            assert_eq!(messages[1].usage, Some(usage));

            let conversation = store.list_conversation(session.id).await.unwrap();
            let ConversationEntry::Message(last) = conversation.last().unwrap() else {
                panic!("expected a message");
            };
            assert_eq!(last.usage, Some(usage));
        });
    }

    #[test]
    fn migrate_is_idempotent_and_adds_usage_and_context_limit_columns() {
        let store = test_store();
        block_on(async {
            store.migrate().await.unwrap();

            for column in [
                "prompt_tokens",
                "completion_tokens",
                "eval_duration_ms",
                "usage_estimated",
            ] {
                let res = store
                    .db
                    .execute(
                        &format!(
                            "SELECT 1 FROM pragma_table_info('messages') WHERE name = '{column}'"
                        ),
                        &[],
                    )
                    .await
                    .unwrap();
                assert!(!res.rows.is_empty(), "missing messages.{column}");
            }
            let res = store
                .db
                .execute(
                    "SELECT 1 FROM pragma_table_info('connections') WHERE name = 'context_limit'",
                    &[],
                )
                .await
                .unwrap();
            assert!(!res.rows.is_empty(), "missing connections.context_limit");
        });
    }

    #[test]
    fn conversation_interleaves_tool_steps() {
        let store = test_store();
        let user_id = test_user(&store, "alice", UserRole::Admin);
        block_on(async {
            let session = store
                .create_session("s", None, None, None, user_id, 1)
                .await
                .unwrap();

            // Turn 1: user message, a gated write that succeeds, assistant reply.
            let user1 = store
                .insert_message(session.id, Role::User, "make a file", 2)
                .await
                .unwrap();
            store
                .upsert_tool_step(
                    session.id,
                    user1.id,
                    "call-1",
                    "write_file",
                    "write a.txt",
                    3,
                )
                .await
                .unwrap();
            let diff = FileDiff {
                path: "a.txt".into(),
                old: None,
                new: "hi".into(),
            };
            store
                .complete_tool_step(session.id, "call-1", true, "wrote a.txt", Some(&diff))
                .await
                .unwrap();
            store
                .insert_message(session.id, Role::Assistant, "done", 4)
                .await
                .unwrap();

            // Turn 2: another message and step, to prove per-anchor ordering.
            let user2 = store
                .insert_message(session.id, Role::User, "again", 5)
                .await
                .unwrap();
            store
                .upsert_tool_step(session.id, user2.id, "call-2", "read_file", "read a.txt", 6)
                .await
                .unwrap();
            store
                .complete_tool_step(session.id, "call-2", true, "read a.txt", None)
                .await
                .unwrap();
            store
                .insert_message(session.id, Role::Assistant, "done again", 7)
                .await
                .unwrap();

            let convo = store.list_conversation(session.id).await.unwrap();
            assert_eq!(convo.len(), 6);
            assert!(matches!(
                convo[0],
                ConversationEntry::Message(ref m)
                    if m.role == Role::User && m.content == "make a file"
            ));
            match &convo[1] {
                ConversationEntry::ToolStep(ts) => {
                    assert_eq!(ts.tool_call_id, "call-1");
                    assert_eq!(ts.name, "write_file");
                    assert_eq!(ts.ok, Some(true));
                    assert_eq!(ts.result_summary.as_deref(), Some("wrote a.txt"));
                    assert_eq!(ts.anchor_message_id, user1.id);
                    assert!(ts.diff.is_some());
                }
                other => panic!("expected tool step, got {other:?}"),
            }
            assert!(matches!(
                convo[2],
                ConversationEntry::Message(ref m)
                    if m.role == Role::Assistant && m.content == "done"
            ));
            assert!(matches!(
                convo[3],
                ConversationEntry::Message(ref m) if m.role == Role::User && m.content == "again"
            ));
            match &convo[4] {
                ConversationEntry::ToolStep(ts) => {
                    assert_eq!(ts.tool_call_id, "call-2");
                    assert_eq!(ts.ok, Some(true));
                    assert!(ts.diff.is_none());
                }
                other => panic!("expected tool step, got {other:?}"),
            }
            assert!(matches!(
                convo[5],
                ConversationEntry::Message(ref m)
                    if m.role == Role::Assistant && m.content == "done again"
            ));

            // The LLM context path is unchanged: messages only, no tool steps.
            let messages = store.list_messages(session.id).await.unwrap();
            assert_eq!(messages.len(), 4);
        });
    }

    #[test]
    fn session_system_prompt_reference() {
        let store = test_store();
        let user_id = test_user(&store, "alice", UserRole::Admin);
        block_on(async {
            let prompt = store
                .insert_system_prompt("coder", "You are a coding agent.")
                .await
                .unwrap();
            let session = store
                .create_session("s", None, Some(prompt.id), None, user_id, 1)
                .await
                .unwrap();
            assert_eq!(session.system_prompt_id, Some(prompt.id));

            store.delete_system_prompt(prompt.id).await.unwrap();
            let reloaded = store.get_session(session.id, user_id).await.unwrap();
            assert_eq!(reloaded.system_prompt_id, None);
        });
    }

    #[test]
    fn deleting_connection_nulls_session() {
        let store = test_store();
        let user_id = test_user(&store, "alice", UserRole::Admin);
        block_on(async {
            let conn = store
                .insert_connection(&NewConnection {
                    name: "c".into(),
                    kind: ProviderKind::LlamaCpp,
                    base_url: "http://localhost:8080".into(),
                    model: None,
                    context_limit: None,
                })
                .await
                .unwrap();
            let session = store
                .create_session("s", Some(conn.id), None, None, user_id, 1)
                .await
                .unwrap();
            assert_eq!(session.connection_id, Some(conn.id));

            store.delete_connection(conn.id).await.unwrap();

            let sessions = store.list_sessions(user_id).await.unwrap();
            assert_eq!(sessions[0].connection_id, None);
        });
    }

    #[test]
    fn project_crud() {
        let store = test_store();
        let user_id = test_user(&store, "alice", UserRole::Admin);
        block_on(async {
            let project = store
                .create_project(
                    &NewProject {
                        name: "my-app".into(),
                        mode: WorkspaceMode::Remote,
                        path: Some("projects/my-app".into()),
                    },
                    user_id,
                    1_700_000_000,
                )
                .await
                .unwrap();
            assert!(project.id > 0);
            assert_eq!(project.mode, WorkspaceMode::Remote);
            assert_eq!(project.path.as_deref(), Some("projects/my-app"));
            assert_eq!(project.user_id, Some(user_id));

            let reloaded = store.get_project(project.id, user_id).await.unwrap();
            assert_eq!(reloaded.name, "my-app");

            let renamed = store
                .rename_project(project.id, "renamed-app", user_id)
                .await
                .unwrap();
            assert_eq!(renamed.name, "renamed-app");

            assert_eq!(store.list_projects(user_id).await.unwrap().len(), 1);

            store.delete_project(project.id, user_id).await.unwrap();
            assert!(store.get_project(project.id, user_id).await.is_err());
            assert!(store.list_projects(user_id).await.unwrap().is_empty());
        });
    }

    #[test]
    fn create_project_dedups_by_folder() {
        let store = test_store();
        let user_id = test_user(&store, "alice", UserRole::Admin);
        block_on(async {
            let new = NewProject {
                name: "my-app".into(),
                mode: WorkspaceMode::Remote,
                path: Some("projects/my-app".into()),
            };
            let first = store.create_project(&new, user_id, 1).await.unwrap();
            // The same folder again returns the existing project.
            let again = store.create_project(&new, user_id, 2).await.unwrap();
            assert_eq!(again.id, first.id);
            assert_eq!(store.list_projects(user_id).await.unwrap().len(), 1);

            // A different mode or a pathless project is a distinct project.
            let local = store
                .create_project(
                    &NewProject {
                        name: "my-app".into(),
                        mode: WorkspaceMode::Local,
                        path: Some("projects/my-app".into()),
                    },
                    user_id,
                    3,
                )
                .await
                .unwrap();
            assert_ne!(local.id, first.id);
            let pathless = store
                .create_project(
                    &NewProject {
                        name: "other".into(),
                        mode: WorkspaceMode::Local,
                        path: None,
                    },
                    user_id,
                    4,
                )
                .await
                .unwrap();
            assert_ne!(pathless.id, local.id);
            assert_eq!(store.list_projects(user_id).await.unwrap().len(), 3);
        });
    }

    #[test]
    fn test_migration_dedups_existing_projects() {
        let store = test_store();
        let user_id = test_user(&store, "alice", UserRole::Admin);
        block_on(async {
            // Directly insert two projects with the exact same path
            store.db.execute(
                "INSERT INTO projects (name, mode, path, user_id, created_at) VALUES (?, ?, ?, ?, ?)",
                &[
                    DbValue::Text("dup1".into()),
                    DbValue::Text("remote".into()),
                    DbValue::Text("repos/dup".into()),
                    DbValue::Int(user_id),
                    DbValue::Int(100),
                ],
            ).await.unwrap();
            let p1_id = store
                .db
                .execute("SELECT last_insert_rowid()", &[])
                .await
                .unwrap()
                .rows[0]
                .get_int(0)
                .unwrap();

            store.db.execute(
                "INSERT INTO projects (name, mode, path, user_id, created_at) VALUES (?, ?, ?, ?, ?)",
                &[
                    DbValue::Text("dup2".into()),
                    DbValue::Text("remote".into()),
                    DbValue::Text("repos/dup".into()),
                    DbValue::Int(user_id),
                    DbValue::Int(200),
                ],
            ).await.unwrap();
            let p2_id = store
                .db
                .execute("SELECT last_insert_rowid()", &[])
                .await
                .unwrap()
                .rows[0]
                .get_int(0)
                .unwrap();

            // Insert a session on p2
            store.db.execute(
                "INSERT INTO sessions (name, project_id, user_id, created_at) VALUES (?, ?, ?, ?)",
                &[
                    DbValue::Text("session-on-dup".into()),
                    DbValue::Int(p2_id),
                    DbValue::Int(user_id),
                    DbValue::Int(250),
                ],
            ).await.unwrap();

            assert_eq!(store.list_projects(user_id).await.unwrap().len(), 2);

            // Run migration apply
            crate::migrations::apply(&store.db).await.unwrap();

            // Now there should be only 1 project
            let projs = store.list_projects(user_id).await.unwrap();
            assert_eq!(projs.len(), 1);
            assert_eq!(projs[0].id, p1_id);

            // And the session on p2 was reassigned to p1
            let sessions = store.list_sessions(user_id).await.unwrap();
            assert_eq!(sessions.len(), 1);
            assert_eq!(sessions[0].project_id, Some(p1_id));
        });
    }

    #[test]
    fn session_belongs_to_project() {
        let store = test_store();
        let user_id = test_user(&store, "alice", UserRole::Admin);
        block_on(async {
            let project = store
                .create_project(
                    &NewProject {
                        name: "my-app".into(),
                        mode: WorkspaceMode::Local,
                        path: None,
                    },
                    user_id,
                    1,
                )
                .await
                .unwrap();
            let session = store
                .create_session("s", None, None, Some(project.id), user_id, 2)
                .await
                .unwrap();
            assert_eq!(session.project_id, Some(project.id));

            let for_project = store
                .list_sessions_for_project(project.id, user_id)
                .await
                .unwrap();
            assert_eq!(for_project.len(), 1);
            assert_eq!(for_project[0].id, session.id);

            // Deleting the project cascades to its sessions.
            store.delete_project(project.id, user_id).await.unwrap();
            assert!(store.get_session(session.id, user_id).await.is_err());
        });
    }

    #[test]
    fn users_scoping_isolates_data() {
        let store = test_store();
        let alice = test_user(&store, "alice", UserRole::Admin);
        let bob = test_user(&store, "bob", UserRole::User);
        block_on(async {
            let project = store
                .create_project(
                    &NewProject {
                        name: "alice-app".into(),
                        mode: WorkspaceMode::Local,
                        path: None,
                    },
                    alice,
                    1,
                )
                .await
                .unwrap();
            let session = store
                .create_session("s", None, None, Some(project.id), alice, 2)
                .await
                .unwrap();

            // Bob sees none of Alice's projects or sessions.
            assert!(store.list_projects(bob).await.unwrap().is_empty());
            assert!(store.list_sessions(bob).await.unwrap().is_empty());
            assert!(store.get_project(project.id, bob).await.is_err());
            assert!(store.get_session(session.id, bob).await.is_err());

            // Alice still sees her own.
            assert_eq!(store.list_projects(alice).await.unwrap().len(), 1);
            assert_eq!(store.list_sessions(alice).await.unwrap().len(), 1);
        });
    }

    #[test]
    fn first_user_inherits_orphaned_projects() {
        let store = test_store();
        let user_id = test_user(&store, "alice", UserRole::Admin);
        block_on(async {
            // A project created before accounts existed has no owner.
            let res = store
                .db
                .execute(
                    "INSERT INTO projects (name, mode, path, created_at) VALUES (?, ?, ?, ?)",
                    &[
                        DbValue::Text("legacy".into()),
                        DbValue::Text("local".into()),
                        DbValue::Null,
                        DbValue::Int(1),
                    ],
                )
                .await
                .unwrap();
            let orphan_id = res.last_insert_rowid;

            let reassigned = store.reassign_orphaned_projects(user_id).await.unwrap();
            assert_eq!(reassigned, 1);

            let reloaded = store.get_project(orphan_id, user_id).await.unwrap();
            assert_eq!(reloaded.user_id, Some(user_id));
        });
    }

    #[test]
    fn first_user_inherits_orphaned_sessions() {
        let store = test_store();
        let user_id = test_user(&store, "alice", UserRole::Admin);
        block_on(async {
            // A session created before accounts existed has no owner.
            let res = store
                .db
                .execute(
                    "INSERT INTO sessions (name, created_at) VALUES (?, ?)",
                    &[DbValue::Text("legacy".into()), DbValue::Int(1)],
                )
                .await
                .unwrap();
            let orphan_id = res.last_insert_rowid;

            let reassigned = store.reassign_orphaned_sessions(user_id).await.unwrap();
            assert_eq!(reassigned, 1);

            let reloaded = store.get_session(orphan_id, user_id).await.unwrap();
            assert_eq!(reloaded.user_id, Some(user_id));
        });
    }

    #[test]
    fn cancel_flag_lifecycle() {
        let store = test_store();
        let user_id = test_user(&store, "alice", UserRole::Admin);
        block_on(async {
            let session = store
                .create_session("s", None, None, None, user_id, 1)
                .await
                .unwrap();
            assert!(!store.cancel_requested(session.id).await.unwrap());

            store.request_cancel(session.id).await.unwrap();
            // Repeating the request is a no-op.
            store.request_cancel(session.id).await.unwrap();
            assert!(store.cancel_requested(session.id).await.unwrap());

            store.clear_cancel(session.id).await.unwrap();
            assert!(!store.cancel_requested(session.id).await.unwrap());
        });
    }

    #[test]
    fn tool_permission_lifecycle() {
        let store = test_store();
        let user_id = test_user(&store, "alice", UserRole::Admin);
        block_on(async {
            let session = store
                .create_session("s", None, None, None, user_id, 1)
                .await
                .unwrap();
            assert_eq!(
                store.tool_permission(session.id, "call-1").await.unwrap(),
                None
            );

            store
                .set_tool_permission(session.id, "call-1", true)
                .await
                .unwrap();
            assert_eq!(
                store.tool_permission(session.id, "call-1").await.unwrap(),
                Some(true)
            );

            // Other tool calls and sessions are unaffected.
            assert_eq!(
                store.tool_permission(session.id, "call-2").await.unwrap(),
                None
            );

            store.clear_tool_permissions(session.id).await.unwrap();
            assert_eq!(
                store.tool_permission(session.id, "call-1").await.unwrap(),
                None
            );
        });
    }
}
