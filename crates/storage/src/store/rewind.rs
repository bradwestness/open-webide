//! Durable rewind preparation and atomic conversation restoration.
use super::*;
use openwebide_core::RewindPlan;

impl<D: Db> Store<D> {
    pub async fn save_project_checkpoint(
        &self,
        session: i64,
        id: &str,
        checkpoint: &openwebide_core::rewind::ProjectCheckpoint,
    ) -> Result<(), StorageError> {
        self.ensure_not_rewinding(session).await?;
        let json =
            serde_json::to_string(checkpoint).map_err(|e| StorageError::Db(e.to_string()))?;
        let result = self.db.execute("UPDATE tool_steps SET checkpoint = ? WHERE session_id = ? AND tool_call_id = ? AND completion_applied = 0", &[DbValue::Text(json), DbValue::Int(session), DbValue::Text(id.into())]).await?;
        if result.changes != 1 {
            return Err(StorageError::Conflict(
                "Tool checkpoint is no longer pending".into(),
            ));
        }
        Ok(())
    }

    /// Block new runs and tool writes for the entire project while files are
    /// being restored. Projectless chats are locked only by session.
    pub async fn ensure_not_rewinding(&self, session: i64) -> Result<(), StorageError> {
        let result = self.db.execute(
            "SELECT 1 FROM session_rewinds r JOIN sessions locked ON locked.id = r.session_id
             JOIN sessions current ON current.id = ?
             WHERE locked.id = current.id OR (locked.project_id IS NOT NULL AND locked.project_id = current.project_id)",
            &[DbValue::Int(session)],
        ).await?;
        let reviews = self.db.execute("SELECT 1 FROM project_reviews r JOIN sessions locked ON locked.id = r.session_id JOIN sessions current ON current.id = ? WHERE locked.id = current.id OR locked.project_id = current.project_id", &[DbValue::Int(session)]).await?;
        if !result.rows.is_empty() || !reviews.rows.is_empty() {
            return Err(StorageError::Conflict(
                "Finish the prepared review or rewind before starting another run in this project"
                    .into(),
            ));
        }
        Ok(())
    }

    pub async fn prepare_rewind(
        &self,
        user: UserId,
        session: i64,
        message: i64,
    ) -> Result<RewindPlan, StorageError> {
        self.db.transaction(|tx| async move {
            let store = Store::new(tx);
            let owned = store.get_session(session, user).await?;
            let existing = store.db.execute("SELECT message_id, plan FROM session_rewinds WHERE session_id = ?", &[DbValue::Int(session)]).await?;
            if let Some(row) = existing.rows.first() {
                if row.get_int(0)? != message {
                    return Err(StorageError::Conflict("Resume the already prepared rewind first".into()));
                }
                return serde_json::from_str(row.get_text(1)?).map_err(|e| StorageError::Db(e.to_string()));
            }
            store.ensure_not_rewinding(session).await?;
            let pending = store.db.execute(
                "SELECT 1 FROM tool_steps t JOIN sessions s ON s.id = t.session_id
                 WHERE t.ok IS NULL AND (s.id = ? OR s.project_id = ?) LIMIT 1",
                &[DbValue::Int(session), owned.project_id.map(DbValue::Int).unwrap_or(DbValue::Null)],
            ).await?;
            if !pending.rows.is_empty() {
                return Err(StorageError::Conflict("Wait for pending tools in this project to finish before rewinding".into()));
            }
            let conversation = store.list_conversation(session).await?;
            let plan = RewindPlan::from_conversation(&conversation, message).map_err(StorageError::Conflict)?;
            let conversation = serde_json::to_string(&conversation).map_err(|e| StorageError::Db(e.to_string()))?;
            let json = serde_json::to_string(&plan).map_err(|e| StorageError::Db(e.to_string()))?;
            store.db.execute("INSERT INTO session_rewinds (session_id, message_id, conversation, plan) VALUES (?, ?, ?, ?)",
                &[DbValue::Int(session), DbValue::Int(message), DbValue::Text(conversation), DbValue::Text(json)]).await?;
            Ok(plan)
        }).await
    }

    /// Called only after all file snapshots have been restored. Keep the old
    /// conversation as recovery data, and remove its messages, steps and stale
    /// compactions together. A repeated completion is harmless.
    pub async fn complete_rewind(
        &self,
        user: UserId,
        session: i64,
        message: i64,
    ) -> Result<Vec<ConversationEntry>, StorageError> {
        self.db.transaction(|tx| async move {
            let store = Store::new(tx);
            let owned = store.get_session(session, user).await?;
            let prepared = store.db.execute("SELECT message_id, conversation, plan FROM session_rewinds WHERE session_id = ?", &[DbValue::Int(session)]).await?;
            let Some(row) = prepared.rows.first() else {
                let done = store.db.execute("SELECT 1 FROM rewind_history WHERE session_id = ? AND message_id = ?", &[DbValue::Int(session), DbValue::Int(message)]).await?;
                if done.rows.is_empty() { return Err(StorageError::NotFound("prepared rewind".into())); }
                return store.list_conversation(session).await;
            };
            if row.get_int(0)? != message { return Err(StorageError::Conflict("Rewind checkpoint changed".into())); }
            let expected: Vec<ConversationEntry> = serde_json::from_str(row.get_text(1)?).map_err(|e| StorageError::Db(e.to_string()))?;
            if store.list_conversation(session).await? != expected {
                return Err(StorageError::Conflict("Conversation changed during rewind".into()));
            }
            let plan: RewindPlan = serde_json::from_str(row.get_text(2)?).map_err(|e| StorageError::Db(e.to_string()))?;
            // Retain pending edits from earlier turns when this rewind restores
            // their latest contents; remove a pending review when fully undone.
            if let Some(project) = owned.project_id {
                let pending = store.list_pending_edits(user, project).await?;
                for file in &plan.files {
                    if let Some(edit) = pending.iter().find(|edit| edit.path == file.path) {
                        if let Some(current) = &edit.file {
                            if current.after_bytes().map_err(StorageError::Conflict)? != file.after_bytes().map_err(StorageError::Conflict)? { return Err(StorageError::Conflict(format!("{} has a newer review revision", file.path))); }
                            continue;
                        }
                        let last_diff = expected.iter().rev().find_map(|entry| match entry { ConversationEntry::ToolStep(step) => step.diff.as_ref().filter(|diff| diff.path == file.path), ConversationEntry::Message(_) => None });
                        if last_diff.is_some_and(|diff| diff.new != edit.diff.new) {
                            return Err(StorageError::Conflict(format!("{} has a newer review revision", file.path)));
                        }
                        if file.before_bytes().map_err(StorageError::Conflict)? == edit.diff.old.as_ref().map(|s| s.as_bytes().to_vec()) || file.binary_before.is_some() {
                            store.db.execute("DELETE FROM pending_edits WHERE project_id = ? AND path = ? AND user_id = ?", &[DbValue::Int(project), DbValue::Text(file.path.clone()), DbValue::Int(user.get())]).await?;
                        } else if let Some(before) = &file.before {
                            let mut diff = edit.diff.clone();
                            diff.new.clone_from(before);
                            store.db.execute("UPDATE pending_edits SET diff = ?, revision = revision + 1 WHERE project_id = ? AND path = ? AND user_id = ?", &[DbValue::Text(serde_json::to_string(&diff).map_err(|e| StorageError::Db(e.to_string()))?), DbValue::Int(project), DbValue::Text(file.path.clone()), DbValue::Int(user.get())]).await?;
                        }
                    }
                }
            }
            store.db.execute("INSERT INTO rewind_history (session_id, message_id, conversation) VALUES (?, ?, ?)", &[DbValue::Int(session), DbValue::Int(message), DbValue::Text(row.get_text(1)?.into())]).await?;
            store.db.execute("DELETE FROM tool_steps WHERE session_id = ? AND anchor_message_id >= ?", &[DbValue::Int(session), DbValue::Int(message)]).await?;
            store.db.execute("DELETE FROM messages WHERE session_id = ? AND id >= ?", &[DbValue::Int(session), DbValue::Int(message)]).await?;
            if let Some(project) = owned.project_id { store.reconcile_review_rewind(user, project, &plan.files).await?; }
            store.db.execute("DELETE FROM session_rewinds WHERE session_id = ?", &[DbValue::Int(session)]).await?;
            store.db.execute("DELETE FROM tool_permissions WHERE session_id = ?", &[DbValue::Int(session)]).await?;
            store.list_conversation(session).await
        }).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rusqlite_db::RusqliteDb;
    use futures::executor::block_on;

    #[test]
    fn rewind_is_owned_durable_atomic_and_isolates_other_projects() {
        block_on(async {
            for mode in [WorkspaceMode::Local, WorkspaceMode::Remote] {
                let store = Store::new(RusqliteDb::open_in_memory().unwrap());
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
                            name: "project".into(),
                            mode,
                            path: Some("repos/test".into()),
                        },
                        user,
                        1,
                    )
                    .await
                    .unwrap();
                let session = store
                    .create_session("chat", None, None, Some(project.id), user, 1)
                    .await
                    .unwrap();
                let sibling = store
                    .create_session("other chat", None, None, Some(project.id), user, 1)
                    .await
                    .unwrap();
                let free = store
                    .create_session("web chat", None, None, None, user, 1)
                    .await
                    .unwrap();
                let before = store
                    .insert_message(session.id, Role::User, "earlier prompt", 1)
                    .await
                    .unwrap();
                store
                    .insert_message(session.id, Role::Assistant, "earlier reply", 1)
                    .await
                    .unwrap();
                let target = store
                    .insert_message(session.id, Role::User, "edit this", 2)
                    .await
                    .unwrap();
                let diff = FileDiff {
                    path: "a.txt".into(),
                    old: Some("before".into()),
                    new: "after".into(),
                    old_unavailable: false,
                    backup_path: None,
                };
                store
                    .upsert_tool_step(
                        session.id,
                        target.id,
                        "step",
                        "write_file",
                        "a.txt",
                        2,
                        None,
                    )
                    .await
                    .unwrap();
                assert!(
                    store
                        .prepare_rewind(user, session.id, target.id)
                        .await
                        .is_err()
                );
                store
                    .complete_tool_step(user, session.id, "step", true, "done", Some(&diff))
                    .await
                    .unwrap();
                store
                    .insert_message(session.id, Role::Assistant, "done", 2)
                    .await
                    .unwrap();
                store
                    .upsert_tool_step(
                        session.id,
                        target.id,
                        "shell",
                        "run_command",
                        "shell changes",
                        2,
                        None,
                    )
                    .await
                    .unwrap();
                let checkpoint = openwebide_core::rewind::ProjectCheckpoint {
                    before: [("a.txt".into(), "YWZ0ZXI=".into())].into(),
                    after: Some(
                        [
                            ("a.txt".into(), "c2hlbGw=".into()),
                            ("binary.dat".into(), "AP8=".into()),
                        ]
                        .into(),
                    ),
                };
                store
                    .save_project_checkpoint(session.id, "shell", &checkpoint)
                    .await
                    .unwrap();
                store
                    .complete_tool_step(
                        user,
                        session.id,
                        "shell",
                        false,
                        "changed files before failing",
                        None,
                    )
                    .await
                    .unwrap();
                let original = store.list_conversation(session.id).await.unwrap();
                assert!(
                    store
                        .prepare_rewind(other, session.id, target.id)
                        .await
                        .is_err()
                );
                let plan = store
                    .prepare_rewind(user, session.id, target.id)
                    .await
                    .unwrap();
                assert_eq!(plan.files.len(), 2);
                assert_eq!(plan.files[0].before.as_deref(), Some("before"));
                assert_eq!(plan.files[0].after, "shell");
                assert_eq!(plan.files[1].after_bytes().unwrap(), Some(vec![0, 255]));
                assert_eq!(
                    store
                        .prepare_rewind(user, session.id, target.id)
                        .await
                        .unwrap(),
                    plan
                );
                assert!(
                    store
                        .prepare_rewind(user, session.id, before.id)
                        .await
                        .is_err()
                );
                assert!(
                    store
                        .insert_message(sibling.id, Role::User, "cannot run", 3)
                        .await
                        .is_err()
                );
                assert!(
                    store
                        .upsert_tool_step(sibling.id, 1, "other", "write_file", "x", 3, None)
                        .await
                        .is_err()
                );
                store
                    .insert_message(free.id, Role::User, "unrelated", 3)
                    .await
                    .unwrap();
                assert!(
                    store
                        .complete_rewind(other, session.id, target.id)
                        .await
                        .is_err()
                );
                store.db.execute("CREATE TRIGGER fail_rewind BEFORE INSERT ON rewind_history BEGIN SELECT RAISE(ABORT, 'fail'); END", &[]).await.unwrap();
                assert!(
                    store
                        .complete_rewind(user, session.id, target.id)
                        .await
                        .is_err()
                );
                assert_eq!(store.list_conversation(session.id).await.unwrap(), original);
                assert_eq!(
                    store
                        .list_pending_edits(user, project.id)
                        .await
                        .unwrap()
                        .len(),
                    2
                );
                store
                    .db
                    .execute("DROP TRIGGER fail_rewind", &[])
                    .await
                    .unwrap();
                let restored = store
                    .complete_rewind(user, session.id, target.id)
                    .await
                    .unwrap();
                assert_eq!(restored.len(), 2);
                assert_eq!(
                    store
                        .complete_rewind(user, session.id, target.id)
                        .await
                        .unwrap(),
                    restored
                );
                assert!(
                    store
                        .list_pending_edits(user, project.id)
                        .await
                        .unwrap()
                        .is_empty()
                );
                store
                    .insert_message(sibling.id, Role::User, "can run now", 3)
                    .await
                    .unwrap();
                let archived = store
                    .db
                    .execute(
                        "SELECT conversation FROM rewind_history WHERE session_id = ?",
                        &[DbValue::Int(session.id)],
                    )
                    .await
                    .unwrap();
                assert_eq!(
                    serde_json::from_str::<Vec<ConversationEntry>>(
                        archived.rows[0].get_text(0).unwrap()
                    )
                    .unwrap(),
                    original
                );
            }
        });
    }
}
