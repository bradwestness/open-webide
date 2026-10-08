//! Durable session goals and compare-and-swap conversation summaries.
use super::*;

impl<D: Db> Store<D> {
    pub async fn save_manual_compaction(
        &self,
        user: UserId,
        session: i64,
        through: i64,
        compaction: &openwebide_core::Compaction,
        started_ms: i64,
        now: i64,
    ) -> Result<ChatMessage, StorageError> {
        let mut compaction = compaction.clone();
        compaction.through_message_id = through;
        let content = compaction
            .stored_content()
            .map_err(|error| StorageError::InvalidValue(error.to_string()))?;
        self.db
            .transaction(|tx| async move {
                let store = Store::new(tx);
                store.get_session(session, user).await?;
                store.ensure_not_rewinding(session).await?;
                if store.cancel_requested_since(session, started_ms).await? {
                    return Err(StorageError::Conflict(
                        "Compaction stopped. Original history has been retained.".into(),
                    ));
                }
                let latest = store
                    .db
                    .execute(
                        "SELECT MAX(id) FROM messages WHERE session_id = ?",
                        &[DbValue::Int(session)],
                    )
                    .await?;
                if latest.rows.first().and_then(|row| row.get_int(0).ok()) != Some(through) {
                    return Err(StorageError::Conflict(
                        "The conversation changed while compacting; retry when the run is idle."
                            .into(),
                    ));
                }
                store
                    .insert_interim_message_unlocked(
                        session,
                        Role::System,
                        &content,
                        now,
                        None,
                        None,
                    )
                    .await
            })
            .await
    }
}

impl<D: Db> Store<D> {
    pub async fn get_goal(
        &self,
        user: UserId,
        session: i64,
    ) -> Result<Option<openwebide_core::Goal>, StorageError> {
        self.get_session(session, user).await?;
        let result = self
            .db
            .execute(
                "SELECT goal FROM session_goals WHERE session_id = ?",
                &[DbValue::Int(session)],
            )
            .await?;
        result
            .rows
            .first()
            .map(|row| {
                serde_json::from_str(row.get_text(0)?)
                    .map_err(|error| StorageError::Db(error.to_string()))
            })
            .transpose()
    }
    pub async fn update_goal(
        &self,
        user: UserId,
        session: i64,
        expected_revision: u64,
        command: openwebide_core::GoalCommand,
        now: i64,
    ) -> Result<openwebide_core::Goal, StorageError> {
        self.db.transaction(|tx| async move {
            let store = Store::new(tx);
            let existing = store.get_goal(user, session).await?;
            store.ensure_not_rewinding(session).await?;
            if existing.as_ref().map_or(0, |goal| goal.revision) != expected_revision {
                return Err(StorageError::Conflict("The goal changed in another window; refresh before updating it.".into()));
            }
            let goal = openwebide_core::Goal::transition(existing.as_ref(), session, command, now).map_err(StorageError::InvalidValue)?;
            let json = serde_json::to_string(&goal).map_err(|error| StorageError::InvalidValue(error.to_string()))?;
            store.db.execute("INSERT INTO session_goals (session_id, goal) VALUES (?, ?) ON CONFLICT(session_id) DO UPDATE SET goal = excluded.goal", &[DbValue::Int(session), DbValue::Text(json)]).await?;
            Ok(goal)
        }).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rusqlite_db::RusqliteDb;
    use openwebide_core::{GoalCommand, GoalStatus};
    #[test]
    fn goals_and_manual_summaries_are_owned_atomic_and_durable_in_both_modes() {
        futures::executor::block_on(async {
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
                            name: "p".into(),
                            mode,
                            path: Some("p".into()),
                        },
                        user,
                        1,
                    )
                    .await
                    .unwrap();
                let session = store
                    .create_session("chat", None, None, Some(project.id), user, 1)
                    .await
                    .unwrap()
                    .id;
                assert!(store.get_goal(user, session).await.unwrap().is_none());
                let goal = store
                    .update_goal(
                        user,
                        session,
                        0,
                        GoalCommand::Start {
                            objective: "Fix tests".into(),
                        },
                        1,
                    )
                    .await
                    .unwrap();
                assert_eq!(
                    store.get_goal(user, session).await.unwrap(),
                    Some(goal.clone())
                );
                assert!(store.get_goal(other, session).await.is_err());
                assert!(
                    store
                        .update_goal(other, session, 1, GoalCommand::Pause, 2)
                        .await
                        .is_err()
                );
                assert!(matches!(
                    store
                        .update_goal(user, session, 0, GoalCommand::Complete, 2)
                        .await,
                    Err(StorageError::Conflict(_))
                ));
                let paused = store
                    .update_goal(user, session, 1, GoalCommand::Pause, 2)
                    .await
                    .unwrap();
                assert_eq!(paused.status, GoalStatus::Paused);
                let resumed = store
                    .update_goal(user, session, 2, GoalCommand::Resume, 3)
                    .await
                    .unwrap();
                assert_eq!(resumed.objective, goal.objective);
                let complete = store
                    .update_goal(user, session, 3, GoalCommand::Complete, 4)
                    .await
                    .unwrap();
                assert_eq!(complete.status, GoalStatus::Completed);
                assert_eq!(complete.started_at, Some(1));
                assert_eq!(
                    store
                        .get_goal(user, session)
                        .await
                        .unwrap()
                        .unwrap()
                        .completed_duration_seconds(),
                    Some(3)
                );
                let original = store
                    .insert_message(session, Role::User, "original prompt", 1)
                    .await
                    .unwrap();
                let summary = openwebide_core::Compaction {
                    summary: "Progress saved".into(),
                    retained: vec![original.clone()],
                    through_message_id: 0,
                };
                assert!(
                    store
                        .save_manual_compaction(other, session, original.id, &summary, 0, 2)
                        .await
                        .is_err()
                );
                let later = store
                    .insert_message(session, Role::Assistant, "answer", 2)
                    .await
                    .unwrap();
                assert!(matches!(
                    store
                        .save_manual_compaction(user, session, original.id, &summary, 0, 3)
                        .await,
                    Err(StorageError::Conflict(_))
                ));
                store.request_cancel(session, 10).await.unwrap();
                assert!(
                    store
                        .save_manual_compaction(user, session, later.id, &summary, 9, 3)
                        .await
                        .is_err()
                );
                let saved = store
                    .save_manual_compaction(user, session, later.id, &summary, 11, 3)
                    .await
                    .unwrap();
                assert_eq!(
                    openwebide_core::Compaction::parse(&saved.content)
                        .unwrap()
                        .through_message_id,
                    later.id
                );
                let messages = store.list_messages(session).await.unwrap();
                assert_eq!(messages[0], original);
                assert_eq!(messages[1], later);
                assert_eq!(messages.len(), 3);
                store.delete_session(session, user).await.unwrap();
                assert!(store.get_goal(user, session).await.is_err());
            }
        });
    }
}
