//! Owned session metadata, search and compare-and-set automatic titles.
use super::*;
use openwebide_core::SessionPreferences;

impl<D: Db> Store<D> {
    pub async fn export_session(
        &self,
        user: UserId,
        session: i64,
    ) -> Result<openwebide_core::SessionExport, StorageError> {
        self.db
            .transaction(|tx| async move {
                let store = Store::new(tx);
                let session = store.get_session(session, user).await?;
                let entries = store.list_conversation(session.id).await?;
                Ok(openwebide_core::SessionExport {
                    filename: openwebide_core::session_markdown_filename(&session),
                    markdown: openwebide_core::session_markdown(&session, &entries),
                })
            })
            .await
    }
    pub async fn search_sessions(
        &self,
        user: UserId,
        project: Option<i64>,
        query: &str,
        archived: bool,
    ) -> Result<Vec<ChatSession>, StorageError> {
        if let Some(project) = project {
            self.get_project(project, user).await?;
        }
        let query = query.trim().to_ascii_lowercase();
        openwebide_core::SessionSearch {
            project_id: project,
            query: query.clone(),
            archived,
        }
        .validate()
        .map_err(StorageError::InvalidValue)?;
        let rows = self.db.execute(&format!("SELECT {} FROM sessions WHERE user_id = ? AND project_id IS ? AND archived = ? AND (? = '' OR instr(lower(name), ?) > 0 OR EXISTS (SELECT 1 FROM messages WHERE messages.session_id = sessions.id AND instr(lower(messages.content), ?) > 0)) ORDER BY pinned DESC, id DESC", Self::SESSION_COLUMNS), &[
            DbValue::Int(user.get()), project.map(DbValue::Int).unwrap_or(DbValue::Null), DbValue::Int(i64::from(archived)), DbValue::Text(query.clone()), DbValue::Text(query.clone()), DbValue::Text(query),
        ]).await?;
        rows.rows.iter().map(session_from_row).collect()
    }
    pub async fn set_session_preferences(
        &self,
        user: UserId,
        session: i64,
        preferences: &SessionPreferences,
    ) -> Result<ChatSession, StorageError> {
        self.db.transaction(|tx| async move {
            let store = Store::new(tx);
            store.get_session(session, user).await?;
            store.db.execute("UPDATE sessions SET pinned = coalesce(?, pinned), archived = coalesce(?, archived) WHERE id = ? AND user_id = ?", &[
                preferences.pinned.map(|value| DbValue::Int(i64::from(value))).unwrap_or(DbValue::Null),
                preferences.archived.map(|value| DbValue::Int(i64::from(value))).unwrap_or(DbValue::Null),
                DbValue::Int(session), DbValue::Int(user.get()),
            ]).await?;
            store.get_session(session, user).await
        }).await
    }
    /// A user rename, completed title or newer title request wins over stale model output.
    pub async fn apply_session_title(
        &self,
        user: UserId,
        session: i64,
        revision: i64,
        name: &str,
    ) -> Result<Option<ChatSession>, StorageError> {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 80 || name.chars().any(char::is_control) {
            return Err(StorageError::InvalidValue(
                "Automatic titles must contain 1–80 characters on one line".into(),
            ));
        }
        self.db.transaction(|tx| async move {
            let store = Store::new(tx);
            store.get_session(session, user).await?;
            let result = store.db.execute("UPDATE sessions SET name = ?, auto_title = 0, title_revision = title_revision + 1 WHERE id = ? AND user_id = ? AND auto_title = 1 AND title_revision = ?", &[
                DbValue::Text(name.into()), DbValue::Int(session), DbValue::Int(user.get()), DbValue::Int(revision),
            ]).await?;
            if result.changes == 0 { Ok(None) } else { store.get_session(session, user).await.map(Some) }
        }).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rusqlite_db::RusqliteDb;
    use futures::executor::block_on;
    #[test]
    fn sessions_search_pin_archive_and_titles_are_owned_durable_and_mode_independent() {
        block_on(async {
            for mode in [
                Some(WorkspaceMode::Local),
                Some(WorkspaceMode::Remote),
                None,
            ] {
                let store = Store::new(RusqliteDb::open_in_memory().unwrap());
                store.migrate().await.unwrap();
                store.migrate().await.unwrap();
                let user = store
                    .insert_user("owner", "hash", UserRole::Admin, 1)
                    .await
                    .unwrap()
                    .id;
                let other = store
                    .insert_user("other", "hash", UserRole::User, 1)
                    .await
                    .unwrap()
                    .id;
                let project = if let Some(mode) = mode {
                    Some(
                        store
                            .create_project(
                                &NewProject {
                                    name: "p".into(),
                                    mode,
                                    path: Some("p".into()),
                                },
                                user,
                                1,
                            )
                            .await
                            .unwrap()
                            .id,
                    )
                } else {
                    None
                };
                let request = NewSession {
                    auto_title: true,
                    name: "New chat".into(),
                    connection_id: None,
                    system_prompt_id: None,
                    project_id: project,
                };
                let first = store
                    .create_session_request(&request, user, 1)
                    .await
                    .unwrap();
                let second = store
                    .create_session_request(&request, user, 2)
                    .await
                    .unwrap();
                let foreign = store
                    .create_session_request(
                        &NewSession {
                            project_id: None,
                            ..request.clone()
                        },
                        other,
                        3,
                    )
                    .await
                    .unwrap();
                assert!(first.auto_title);
                assert!(!first.pinned && !first.archived);
                store
                    .insert_message(first.id, Role::User, "Unique NuGet package issue", 1)
                    .await
                    .unwrap();
                store
                    .insert_message(foreign.id, Role::User, "Unique NuGet package issue", 1)
                    .await
                    .unwrap();
                let found = store
                    .search_sessions(user, project, "nuget", false)
                    .await
                    .unwrap();
                assert_eq!(found.iter().map(|s| s.id).collect::<Vec<_>>(), [first.id]);
                let pinned = store
                    .set_session_preferences(
                        user,
                        first.id,
                        &SessionPreferences {
                            pinned: Some(true),
                            archived: None,
                        },
                    )
                    .await
                    .unwrap();
                assert!(pinned.pinned);
                assert_eq!(
                    store
                        .search_sessions(user, project, "", false)
                        .await
                        .unwrap()
                        .iter()
                        .map(|s| s.id)
                        .collect::<Vec<_>>(),
                    [first.id, second.id]
                );
                let archived = store
                    .set_session_preferences(
                        user,
                        first.id,
                        &SessionPreferences {
                            pinned: None,
                            archived: Some(true),
                        },
                    )
                    .await
                    .unwrap();
                assert!(archived.pinned && archived.archived);
                assert_eq!(
                    store
                        .search_sessions(user, project, "", false)
                        .await
                        .unwrap()
                        .len(),
                    1
                );
                assert_eq!(
                    store
                        .search_sessions(user, project, "nuget", true)
                        .await
                        .unwrap(),
                    [archived]
                );
                assert!(matches!(
                    store
                        .set_session_preferences(other, first.id, &SessionPreferences::default())
                        .await,
                    Err(StorageError::NotFound(_))
                ));
                assert!(matches!(
                    store
                        .apply_session_title(other, first.id, 0, "Forbidden")
                        .await,
                    Err(StorageError::NotFound(_))
                ));
                let titled = store
                    .apply_session_title(user, first.id, 0, "Update NuGet dependencies")
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(titled.name, "Update NuGet dependencies");
                assert!(!titled.auto_title);
                assert!(titled.pinned && titled.archived);
                let exported = store.export_session(user, first.id).await.unwrap();
                assert!(exported.markdown.contains("Unique NuGet package issue"));
                assert_eq!(exported.filename, "Update-NuGet-dependencies.md");
                assert!(matches!(
                    store.export_session(other, first.id).await,
                    Err(StorageError::NotFound(_))
                ));
                assert!(
                    store
                        .apply_session_title(user, first.id, 0, "Stale model answer")
                        .await
                        .unwrap()
                        .is_none()
                );
                let renamed = store
                    .rename_session(second.id, "My chosen title", user)
                    .await
                    .unwrap();
                assert!(!renamed.auto_title);
                assert!(
                    store
                        .apply_session_title(user, second.id, 0, "Overwrite rename")
                        .await
                        .unwrap()
                        .is_none()
                );
                assert_eq!(store.get_session(second.id, user).await.unwrap(), renamed);
                store
                    .rename_session(second.id, "Üñîçode title", user)
                    .await
                    .unwrap();
                assert_eq!(
                    store
                        .search_sessions(user, project, "Üñîçode", false)
                        .await
                        .unwrap()
                        .iter()
                        .map(|session| session.id)
                        .collect::<Vec<_>>(),
                    [second.id]
                );
                store
                    .set_session_preferences(
                        user,
                        first.id,
                        &SessionPreferences {
                            pinned: Some(false),
                            archived: Some(false),
                        },
                    )
                    .await
                    .unwrap();
                assert_eq!(
                    store
                        .search_sessions(user, project, "nuget", false)
                        .await
                        .unwrap()
                        .len(),
                    1
                );
                assert!(
                    store
                        .search_sessions(user, project, &"x".repeat(257), false)
                        .await
                        .is_err()
                );
                assert!(
                    store
                        .apply_session_title(user, first.id, 0, "bad\ntitle")
                        .await
                        .is_err()
                );
                let legacy:ChatSession=serde_json::from_value(serde_json::json!({"id":99,"name":"legacy","connection_id":null,"created_at":0})).unwrap();
                assert!(!legacy.auto_title && !legacy.pinned && !legacy.archived);
            }
        });
    }
}
