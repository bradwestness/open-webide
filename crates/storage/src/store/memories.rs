//! Shared memory persistence and ownership policy for browser, Spin and bridge callers.
use super::*;
use openwebide_core::{MemoryCommand, ProjectMemories, ProjectMemory};

impl<D: Db> Store<D> {
    pub async fn project_memories(
        &self,
        user: UserId,
        project: i64,
    ) -> Result<ProjectMemories, StorageError> {
        self.get_project(project, user).await?;
        let enabled = self
            .get_user_setting(user, &format!("project_memory_{project}"))
            .await?
            .as_deref()
            != Some("false");
        let rows = self.db.execute("SELECT id, title, content, revision, updated_at FROM project_memories WHERE user_id = ? AND project_id = ? ORDER BY updated_at DESC, id DESC", &[DbValue::Int(user.get()), DbValue::Int(project)]).await?;
        let entries = rows
            .rows
            .iter()
            .map(|row| {
                Ok(ProjectMemory {
                    id: row.get_int(0)?,
                    title: row.get_text(1)?.into(),
                    content: row.get_text(2)?.into(),
                    revision: row.get_int(3)?,
                    updated_at: row.get_int(4)?,
                })
            })
            .collect::<Result<Vec<_>, StorageError>>()?;
        Ok(ProjectMemories { enabled, entries })
    }
    pub async fn session_memories(
        &self,
        user: UserId,
        session: i64,
    ) -> Result<ProjectMemories, StorageError> {
        let session = self.get_session(session, user).await?;
        match session.project_id {
            Some(project) => self.project_memories(user, project).await,
            None => Ok(ProjectMemories {
                enabled: false,
                entries: Vec::new(),
            }),
        }
    }
    pub fn memory_command<'a>(
        &'a self,
        user: UserId,
        project: i64,
        command: &'a MemoryCommand,
        agent: bool,
        now: i64,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<ProjectMemories, StorageError>> + Send + 'a>,
    > {
        Box::pin(async move {
            command.validate().map_err(StorageError::InvalidRequest)?;
            self.db.transaction(|tx| async move {
                let store = Store::new(tx);
                let current = store.project_memories(user, project).await?;
                if agent && (!current.enabled || matches!(command, MemoryCommand::SetEnabled { .. })) { return Err(StorageError::InvalidRequest("Project memory is disabled or this operation is unavailable to agents".into())); }
                let scope = [DbValue::Int(user.get()), DbValue::Int(project)];
                match command {
                    MemoryCommand::Create { title, content } => {
                        if current.entries.len() >= openwebide_core::memory::MAX_MEMORIES { return Err(StorageError::InvalidRequest("This project already has 100 memories".into())); }
                        store.db.execute("INSERT INTO project_memories (user_id, project_id, title, content, updated_at) VALUES (?, ?, ?, ?, ?)", &[scope[0].clone(), scope[1].clone(), DbValue::Text(title.trim().into()), DbValue::Text(content.trim().into()), DbValue::Int(now)]).await?;
                    }
                    MemoryCommand::Update { id, revision, title, content } => {
                        let changed = store.db.execute("UPDATE project_memories SET title = ?, content = ?, revision = revision + 1, updated_at = ? WHERE id = ? AND user_id = ? AND project_id = ? AND revision = ?", &[DbValue::Text(title.trim().into()), DbValue::Text(content.trim().into()), DbValue::Int(now), DbValue::Int(*id), scope[0].clone(), scope[1].clone(), DbValue::Int(*revision)]).await?;
                        if changed.changes != 1 { return Err(StorageError::Conflict("Memory changed or was removed. Refresh before editing.".into())); }
                    }
                    MemoryCommand::Delete { id, revision } => {
                        let changed = store.db.execute("DELETE FROM project_memories WHERE id = ? AND user_id = ? AND project_id = ? AND revision = ?", &[DbValue::Int(*id), scope[0].clone(), scope[1].clone(), DbValue::Int(*revision)]).await?;
                        if changed.changes != 1 { return Err(StorageError::Conflict("Memory changed or was removed. Refresh before deleting.".into())); }
                    }
                    MemoryCommand::SetEnabled { enabled } => store.set_user_setting(user, &format!("project_memory_{project}"), if *enabled { "true" } else { "false" }).await?,
                    MemoryCommand::Search { query } => {
                        let query = query.to_lowercase();
                        return Ok(ProjectMemories { enabled: current.enabled, entries: current.entries.into_iter().filter(|entry| entry.title.to_lowercase().contains(&query) || entry.content.to_lowercase().contains(&query)).take(20).collect() });
                    }
                    MemoryCommand::Read { id } => {
                        let entry = current.entries.into_iter().find(|entry| entry.id == *id).ok_or_else(|| StorageError::NotFound("memory".into()))?;
                        return Ok(ProjectMemories { enabled: current.enabled, entries: vec![entry] });
                    }
                }
                store.project_memories(user, project).await
            }).await
        })
    }
    pub async fn session_memory_command(
        &self,
        user: UserId,
        session: i64,
        command: &MemoryCommand,
        now: i64,
    ) -> Result<ProjectMemories, StorageError> {
        let project = self
            .get_session(session, user)
            .await?
            .project_id
            .ok_or_else(|| {
                StorageError::InvalidRequest("Project memory requires a project".into())
            })?;
        self.memory_command(user, project, command, true, now).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rusqlite_db::RusqliteDb;
    use futures::executor::block_on;
    #[test]
    fn project_memory_contract_is_owned_durable_shared_opt_out_and_revision_safe_in_both_modes() {
        block_on(async {
            for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
                let store = Store::new(RusqliteDb::open_in_memory().unwrap());
                store.migrate().await.unwrap();
                store.migrate().await.unwrap();
                let user = store
                    .insert_user("owner", "hash", UserRole::Admin, 0)
                    .await
                    .unwrap()
                    .id;
                let other = store
                    .insert_user("other", "hash", UserRole::User, 0)
                    .await
                    .unwrap()
                    .id;
                let project = store
                    .create_project(
                        &NewProject {
                            name: "project".into(),
                            mode,
                            path: Some("test".into()),
                        },
                        user,
                        0,
                    )
                    .await
                    .unwrap()
                    .id;
                let second = store
                    .create_project(
                        &NewProject {
                            name: "second".into(),
                            mode,
                            path: Some("second".into()),
                        },
                        user,
                        0,
                    )
                    .await
                    .unwrap()
                    .id;
                let first = store
                    .create_session("first", None, None, Some(project), user, 0)
                    .await
                    .unwrap()
                    .id;
                let next = store
                    .create_session("next", None, None, Some(project), user, 0)
                    .await
                    .unwrap()
                    .id;
                assert_eq!(
                    store.project_memories(user, project).await.unwrap(),
                    ProjectMemories::default()
                );
                let create = MemoryCommand::Create {
                    title: "Build".into(),
                    content: "Use cargo test 🦀".into(),
                };
                let saved = store
                    .session_memory_command(user, first, &create, 1)
                    .await
                    .unwrap();
                let entry = &saved.entries[0];
                assert_eq!(store.session_memories(user, next).await.unwrap(), saved);
                assert!(store.project_memories(other, project).await.is_err());
                assert!(
                    store
                        .session_memory_command(other, first, &create, 2)
                        .await
                        .is_err()
                );
                assert!(
                    store
                        .project_memories(user, second)
                        .await
                        .unwrap()
                        .entries
                        .is_empty()
                );
                let changed = store
                    .memory_command(
                        user,
                        project,
                        &MemoryCommand::Update {
                            id: entry.id,
                            revision: entry.revision,
                            title: "Build".into(),
                            content: "Use cargo test --offline".into(),
                        },
                        false,
                        2,
                    )
                    .await
                    .unwrap();
                assert_eq!(changed.entries[0].revision, 2);
                assert!(matches!(
                    store
                        .session_memory_command(
                            user,
                            first,
                            &MemoryCommand::Delete {
                                id: entry.id,
                                revision: 1
                            },
                            3
                        )
                        .await,
                    Err(StorageError::Conflict(_))
                ));
                assert!(
                    store
                        .session_memory_command(
                            user,
                            first,
                            &MemoryCommand::Read { id: entry.id },
                            3
                        )
                        .await
                        .unwrap()
                        .entries[0]
                        .content
                        .contains("offline")
                );
                assert_eq!(
                    store
                        .session_memory_command(
                            user,
                            next,
                            &MemoryCommand::Search {
                                query: "OFFLINE".into()
                            },
                            3
                        )
                        .await
                        .unwrap()
                        .entries
                        .len(),
                    1
                );
                assert!(
                    store
                        .memory_command(
                            user,
                            second,
                            &MemoryCommand::Delete {
                                id: entry.id,
                                revision: 2
                            },
                            false,
                            3
                        )
                        .await
                        .is_err()
                );
                store
                    .memory_command(
                        user,
                        project,
                        &MemoryCommand::SetEnabled { enabled: false },
                        false,
                        3,
                    )
                    .await
                    .unwrap();
                for command in [
                    &create,
                    &MemoryCommand::Search {
                        query: String::new(),
                    },
                    &MemoryCommand::Read { id: entry.id },
                    &MemoryCommand::SetEnabled { enabled: true },
                ] {
                    assert!(matches!(
                        store.session_memory_command(user, next, command, 4).await,
                        Err(StorageError::InvalidRequest(_))
                    ));
                }
                let disabled = store.project_memories(user, project).await.unwrap();
                assert!(!disabled.enabled);
                assert_eq!(disabled.entries, changed.entries);
                assert!(
                    store
                        .memory_command(
                            user,
                            project,
                            &MemoryCommand::Create {
                                title: String::new(),
                                content: "invalid".into()
                            },
                            false,
                            4
                        )
                        .await
                        .is_err()
                );
                store.delete_session(first, user).await.unwrap();
                assert_eq!(store.session_memories(user, next).await.unwrap(), disabled);
                store
                    .memory_command(
                        user,
                        project,
                        &MemoryCommand::Delete {
                            id: entry.id,
                            revision: 2,
                        },
                        false,
                        5,
                    )
                    .await
                    .unwrap();
                assert!(
                    store
                        .project_memories(user, project)
                        .await
                        .unwrap()
                        .entries
                        .is_empty()
                );
                store
                    .memory_command(
                        user,
                        project,
                        &MemoryCommand::SetEnabled { enabled: true },
                        false,
                        6,
                    )
                    .await
                    .unwrap();
                for _ in 0..openwebide_core::memory::MAX_MEMORIES {
                    store
                        .session_memory_command(user, next, &create, 7)
                        .await
                        .unwrap();
                }
                assert!(
                    store
                        .session_memory_command(user, next, &create, 8)
                        .await
                        .is_err()
                );
                assert_eq!(
                    store
                        .project_memories(user, project)
                        .await
                        .unwrap()
                        .entries
                        .len(),
                    100
                );
                store.delete_project(project, user).await.unwrap();
                let rows = store
                    .db
                    .execute("SELECT COUNT(*) FROM project_memories", &[])
                    .await
                    .unwrap();
                assert_eq!(rows.rows[0].get_int(0).unwrap(), 0);
            }
        });
    }
}
