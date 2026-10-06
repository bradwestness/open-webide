//! User-scoped recovery uses database settings with optimistic window revisions.
use super::*;
use openwebide_core::editor::{EditorRecovery, EditorRecoveryRecord, EditorRecoveryRoot};

impl<D: Db> Store<D> {
    pub async fn editor_recovery(
        &self,
        user: UserId,
        project: i64,
    ) -> Result<EditorRecoveryRecord, StorageError> {
        self.get_project(project, user).await?;
        let result = self
            .db
            .execute(
                "SELECT revision, value FROM user_settings WHERE user_id = ? AND key = ?",
                &[
                    DbValue::Int(user.get()),
                    DbValue::Text(format!("editor_recovery_{project}")),
                ],
            )
            .await?;
        let Some(row) = result.rows.first() else {
            return Ok(EditorRecoveryRecord::default());
        };
        let state: EditorRecovery = serde_json::from_str(row.get_text(1)?)
            .map_err(|error| StorageError::InvalidValue(error.to_string()))?;
        state.validate().map_err(StorageError::InvalidValue)?;
        Ok(EditorRecoveryRecord {
            revision: row.get_int(0)?,
            state,
        })
    }

    pub async fn save_editor_recovery(
        &self,
        user: UserId,
        project: i64,
        expected_revision: i64,
        state: &EditorRecovery,
    ) -> Result<i64, StorageError> {
        state.validate().map_err(StorageError::InvalidValue)?;
        if expected_revision < 0 {
            return Err(StorageError::InvalidValue(
                "Invalid editor recovery revision".into(),
            ));
        }
        let revision = expected_revision.checked_add(1).ok_or_else(|| {
            StorageError::Conflict("Editor recovery revision limit reached".into())
        })?;
        let value = serde_json::to_string(state)
            .map_err(|error| StorageError::InvalidValue(error.to_string()))?;
        self.db.transaction(|tx| async move {
            let store = Store::new(tx);
            let current_project = store.get_project(project, user).await?;
            if state.root.as_ref().is_some_and(|root| *root != EditorRecoveryRoot::for_project(&current_project)) {
                return Err(StorageError::Conflict("Project folder changed; keep the recovered drafts before reopening files".into()));
            }
            let key = format!("editor_recovery_{project}");
            let current = store.db.execute("SELECT revision FROM user_settings WHERE user_id = ? AND key = ?", &[DbValue::Int(user.get()), DbValue::Text(key.clone())]).await?;
            let existing = current.rows.first().map(|row| row.get_int(0)).transpose()?.unwrap_or(0);
            if existing != expected_revision { return Err(StorageError::Conflict("Editor recovery changed in another window; keep your drafts and reload recovery before saving".into())); }
            store.db.execute("INSERT INTO user_settings (user_id, key, value, revision) VALUES (?, ?, ?, ?) ON CONFLICT(user_id, key) DO UPDATE SET value = excluded.value, revision = excluded.revision", &[DbValue::Int(user.get()), DbValue::Text(key), DbValue::Text(value), DbValue::Int(revision)]).await?;
            Ok(revision)
        }).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rusqlite_db::RusqliteDb;
    use futures::executor::block_on;
    use openwebide_core::editor::{Document, EditorRecoveryFile, RecoveryScroll};

    #[test]
    fn recovery_is_scoped_durable_revision_guarded_and_removed_with_projects_in_both_modes() {
        block_on(async {
            for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
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
                let project = store
                    .create_project(
                        &NewProject {
                            name: "workspace".into(),
                            mode,
                            path: Some("project".into()),
                        },
                        user,
                        1,
                    )
                    .await
                    .unwrap();
                let mut document = Document::new("original");
                document.replace_selections("new ", None).unwrap();
                let state = EditorRecovery {
                    format: 1,
                    root: Some(EditorRecoveryRoot::for_project(&project)),
                    selected: Some("file.rs".into()),
                    files: vec![EditorRecoveryFile {
                        path: "file.rs".into(),
                        document: Some(document.recovery()),
                        scroll: RecoveryScroll::default(),
                        read_only: false,
                    }],
                };
                assert_eq!(
                    store.editor_recovery(user, project.id).await.unwrap(),
                    EditorRecoveryRecord::default()
                );
                assert_eq!(
                    store
                        .save_editor_recovery(user, project.id, 0, &state)
                        .await
                        .unwrap(),
                    1
                );
                assert_eq!(
                    store.editor_recovery(user, project.id).await.unwrap().state,
                    state
                );
                assert!(store.all_user_settings(user).await.unwrap().is_empty());
                assert!(matches!(
                    store.editor_recovery(other, project.id).await,
                    Err(StorageError::NotFound(_))
                ));
                assert!(matches!(
                    store
                        .save_editor_recovery(other, project.id, 1, &state)
                        .await,
                    Err(StorageError::NotFound(_))
                ));
                assert!(matches!(
                    store
                        .save_editor_recovery(user, project.id, 0, &EditorRecovery::default())
                        .await,
                    Err(StorageError::Conflict(_))
                ));
                assert!(matches!(
                    store
                        .set_user_setting(user, &format!("editor_recovery_{}", project.id), "bad")
                        .await,
                    Err(StorageError::InvalidValue(_))
                ));
                let mut wrong_root = state.clone();
                wrong_root.root.as_mut().unwrap().path = Some("different".into());
                assert!(matches!(
                    store
                        .save_editor_recovery(user, project.id, 1, &wrong_root)
                        .await,
                    Err(StorageError::Conflict(_))
                ));
                // Closing every tab retains a revision tombstone against stale windows.
                assert_eq!(
                    store
                        .save_editor_recovery(user, project.id, 1, &EditorRecovery::default())
                        .await
                        .unwrap(),
                    2
                );
                assert!(matches!(
                    store
                        .save_editor_recovery(user, project.id, 1, &state)
                        .await,
                    Err(StorageError::Conflict(_))
                ));
                assert_eq!(
                    store
                        .editor_recovery(user, project.id)
                        .await
                        .unwrap()
                        .revision,
                    2
                );
                store.delete_project(project.id, user).await.unwrap();
                assert!(
                    store
                        .get_user_setting(user, &format!("editor_recovery_{}", project.id))
                        .await
                        .unwrap()
                        .is_none()
                );
            }
        });
    }
}
