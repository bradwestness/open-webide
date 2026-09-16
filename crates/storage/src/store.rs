//! Typed repositories over a [`Db`].

use std::collections::BTreeMap;

use openwebide_core::{
    ChatMessage, ChatSession, Connection, NewConnection, ProviderKind, Role, SystemPrompt,
};

use crate::db::{Db, DbValue, QueryRow};
use crate::{StorageError, migrations};

pub struct Store<D: Db> {
    db: D,
}

impl<D: Db> Store<D> {
    pub fn new(db: D) -> Self {
        Self { db }
    }

    /// Apply idempotent schema migrations.
    pub async fn migrate(&self) -> Result<(), StorageError> {
        migrations::apply(&self.db).await
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

    // -- connections -------------------------------------------------------

    pub async fn list_connections(&self) -> Result<Vec<Connection>, StorageError> {
        let res = self
            .db
            .execute(
                "SELECT id, name, kind, base_url, model, enabled
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
                "SELECT id, name, kind, base_url, model, enabled
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
                "INSERT INTO connections (name, kind, base_url, model) VALUES (?, ?, ?, ?)",
                &[
                    DbValue::Text(new.name.clone()),
                    DbValue::Text(new.kind.as_str().into()),
                    DbValue::Text(new.base_url.clone()),
                    new.model
                        .as_ref()
                        .map(|m| DbValue::Text(m.clone()))
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
                 SET name = ?, kind = ?, base_url = ?, model = ?, enabled = ?
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

    // -- sessions ------------------------------------------------------------

    pub async fn list_sessions(&self) -> Result<Vec<ChatSession>, StorageError> {
        let res = self
            .db
            .execute(
                "SELECT id, name, connection_id, created_at
                 FROM sessions ORDER BY id",
                &[],
            )
            .await?;
        res.rows.iter().map(session_from_row).collect()
    }

    pub async fn create_session(
        &self,
        name: &str,
        connection_id: Option<i64>,
        created_at: i64,
    ) -> Result<ChatSession, StorageError> {
        let res = self
            .db
            .execute(
                "INSERT INTO sessions (name, connection_id, created_at) VALUES (?, ?, ?)",
                &[
                    DbValue::Text(name.into()),
                    connection_id.map(DbValue::Int).unwrap_or(DbValue::Null),
                    DbValue::Int(created_at),
                ],
            )
            .await?;
        let id = res.last_insert_rowid;
        Ok(ChatSession {
            id,
            name: name.into(),
            connection_id,
            created_at,
        })
    }

    pub async fn delete_session(&self, id: i64) -> Result<(), StorageError> {
        let res = self
            .db
            .execute("DELETE FROM sessions WHERE id = ?", &[DbValue::Int(id)])
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
                "SELECT id, session_id, role, content, created_at
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
        let res = self
            .db
            .execute(
                "INSERT INTO messages (session_id, role, content, created_at)
                 VALUES (?, ?, ?, ?)",
                &[
                    DbValue::Int(session_id),
                    DbValue::Text(role.as_str().into()),
                    DbValue::Text(content.into()),
                    DbValue::Int(created_at),
                ],
            )
            .await?;
        Ok(ChatMessage {
            id: res.last_insert_rowid,
            session_id,
            role,
            content: content.into(),
            created_at,
        })
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
    })
}

fn prompt_from_row(row: &QueryRow) -> Result<SystemPrompt, StorageError> {
    Ok(SystemPrompt {
        id: row.get_int(0)?,
        name: row.get_text(1)?.to_string(),
        content: row.get_text(2)?.to_string(),
    })
}

fn session_from_row(row: &QueryRow) -> Result<ChatSession, StorageError> {
    Ok(ChatSession {
        id: row.get_int(0)?,
        name: row.get_text(1)?.to_string(),
        connection_id: match &row.values[2] {
            DbValue::Int(i) => Some(*i),
            DbValue::Null => None,
            other => {
                return Err(StorageError::InvalidValue(format!(
                    "connection_id is not an integer: {other:?}"
                )));
            }
        },
        created_at: row.get_int(3)?,
    })
}

fn message_from_row(row: &QueryRow) -> Result<ChatMessage, StorageError> {
    let role = row.get_text(2)?;
    Ok(ChatMessage {
        id: row.get_int(0)?,
        session_id: row.get_int(1)?,
        role: Role::parse(role)
            .ok_or_else(|| StorageError::InvalidValue(format!("unknown role: {role}")))?,
        content: row.get_text(3)?.to_string(),
        created_at: row.get_int(4)?,
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
    fn connection_crud() {
        let store = test_store();
        block_on(async {
            let conn = store
                .insert_connection(&NewConnection {
                    name: "local-ollama".into(),
                    kind: ProviderKind::Ollama,
                    base_url: "http://localhost:11434".into(),
                    model: Some("qwen2.5-coder:7b".into()),
                })
                .await
                .unwrap();
            assert!(conn.id > 0);
            assert!(conn.enabled);
            assert_eq!(conn.model.as_deref(), Some("qwen2.5-coder:7b"));

            let mut conn = store.get_connection(conn.id).await.unwrap();
            conn.name = "renamed".into();
            conn.enabled = false;
            store.update_connection(&conn).await.unwrap();
            let reloaded = store.get_connection(conn.id).await.unwrap();
            assert_eq!(reloaded.name, "renamed");
            assert!(!reloaded.enabled);

            assert_eq!(store.list_connections().await.unwrap().len(), 1);

            store.delete_connection(conn.id).await.unwrap();
            assert!(store.get_connection(conn.id).await.is_err());
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

            store.delete_system_prompt(prompt.id).await.unwrap();
            assert!(store.list_system_prompts().await.unwrap().is_empty());
        });
    }

    #[test]
    fn sessions_and_messages() {
        let store = test_store();
        block_on(async {
            let session = store
                .create_session("first session", None, 1_700_000_000)
                .await
                .unwrap();
            assert!(session.id > 0);

            let msg = store
                .insert_message(session.id, Role::User, "hello", 1_700_000_001)
                .await
                .unwrap();
            assert_eq!(msg.id, 1);

            let messages = store.list_messages(session.id).await.unwrap();
            assert_eq!(messages.len(), 1);
            assert_eq!(messages[0].role, Role::User);
            assert_eq!(messages[0].content, "hello");

            store.delete_session(session.id).await.unwrap();
            assert!(store.list_messages(session.id).await.unwrap().is_empty());
        });
    }

    #[test]
    fn deleting_connection_nulls_session() {
        let store = test_store();
        block_on(async {
            let conn = store
                .insert_connection(&NewConnection {
                    name: "c".into(),
                    kind: ProviderKind::LlamaCpp,
                    base_url: "http://localhost:8080".into(),
                    model: None,
                })
                .await
                .unwrap();
            let session = store.create_session("s", Some(conn.id), 1).await.unwrap();
            assert_eq!(session.connection_id, Some(conn.id));

            store.delete_connection(conn.id).await.unwrap();

            let sessions = store.list_sessions().await.unwrap();
            assert_eq!(sessions[0].connection_id, None);
        });
    }
}
