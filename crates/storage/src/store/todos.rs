//! Checklist revisions follow their prompt's lifetime, including rewind and branches.
use super::*;
use openwebide_core::{TodoPlan, TodoUpdate};

impl<D: Db> Store<D> {
    pub async fn get_todo_plan(
        &self,
        user: UserId,
        session: i64,
    ) -> Result<Option<TodoUpdate>, StorageError> {
        self.get_session(session, user).await?;
        let result = self.db.execute("SELECT id, anchor_message_id, plan, created_at FROM todo_updates WHERE session_id = ? ORDER BY id DESC LIMIT 1", &[DbValue::Int(session)]).await?;
        result
            .rows
            .first()
            .map(|row| {
                Ok(TodoUpdate {
                    id: row.get_int(0)?,
                    session_id: session,
                    anchor_message_id: row.get_int(1)?,
                    plan: serde_json::from_str(row.get_text(2)?)
                        .map_err(|error| StorageError::Db(error.to_string()))?,
                    created_at: row.get_int(3)?,
                })
            })
            .transpose()
    }

    pub fn write_todo_plan<'a>(
        &'a self,
        user: UserId,
        session: i64,
        anchor: i64,
        plan: &'a TodoPlan,
        created_at: i64,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<TodoUpdate, StorageError>> + Send + 'a>,
    > {
        Box::pin(async move {
            plan.validate().map_err(StorageError::InvalidValue)?;
            let json = serde_json::to_string(plan)
                .map_err(|error| StorageError::InvalidValue(error.to_string()))?;
            let plan = plan.clone();
            self.db.transaction(|tx| async move {
            let store = Store::new(tx);
            store.get_session(session, user).await?;
            store.ensure_not_rewinding(session).await?;
            let result = store.db.execute("SELECT id FROM messages WHERE session_id = ? AND role = 'user' ORDER BY id DESC LIMIT 1", &[DbValue::Int(session)]).await?;
            if result.rows.first().map(|row| row.get_int(0)).transpose()? != Some(anchor) {
                return Err(StorageError::Conflict("The plan belongs to an earlier or missing prompt. Refresh this session before updating it.".into()));
            }
            let result = store.db.execute("INSERT INTO todo_updates (session_id, anchor_message_id, plan, created_at) VALUES (?, ?, ?, ?)", &[DbValue::Int(session), DbValue::Int(anchor), DbValue::Text(json), DbValue::Int(created_at)]).await?;
            Ok(TodoUpdate { id: result.last_insert_rowid, session_id: session, anchor_message_id: anchor, plan: plan.clone(), created_at })
        }).await
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rusqlite_db::RusqliteDb;
    use futures::executor::block_on;
    use openwebide_core::{TodoItem, TodoStatus};

    #[test]
    fn checklist_contract_is_owned_durable_anchored_and_atomic_in_all_workspaces() {
        block_on(async {
            for mode in [
                Some(WorkspaceMode::Local),
                Some(WorkspaceMode::Remote),
                None,
            ] {
                let store = Store::new(RusqliteDb::open_in_memory().unwrap());
                store.migrate().await.unwrap();
                store.migrate().await.unwrap();
                let owner = store
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
                                    name: "project".into(),
                                    mode,
                                    path: Some("test".into()),
                                },
                                owner,
                                1,
                            )
                            .await
                            .unwrap()
                            .id,
                    )
                } else {
                    None
                };
                let session = store
                    .create_session("source", None, None, project, owner, 1)
                    .await
                    .unwrap()
                    .id;
                let sibling = store
                    .create_session("sibling", None, None, project, owner, 1)
                    .await
                    .unwrap()
                    .id;
                assert!(store.get_todo_plan(owner, session).await.unwrap().is_none());
                let first = store
                    .insert_message(session, Role::User, "first", 1)
                    .await
                    .unwrap();
                let plan = TodoPlan {
                    todos: vec![TodoItem {
                        id: "inspect".into(),
                        content: "Inspect the problem".into(),
                        status: TodoStatus::InProgress,
                    }],
                };
                let written = store
                    .write_todo_plan(owner, session, first.id, &plan, 2)
                    .await
                    .unwrap();
                assert_eq!(
                    store.get_todo_plan(owner, session).await.unwrap(),
                    Some(written.clone())
                );
                assert!(store.get_todo_plan(other, session).await.is_err());
                assert!(
                    store
                        .write_todo_plan(other, session, first.id, &plan, 2)
                        .await
                        .is_err()
                );
                assert!(
                    store
                        .write_todo_plan(owner, sibling, first.id, &plan, 2)
                        .await
                        .is_err()
                );
                let second = store
                    .insert_message(session, Role::User, "second", 3)
                    .await
                    .unwrap();
                assert!(
                    store
                        .write_todo_plan(owner, session, first.id, &TodoPlan::default(), 3)
                        .await
                        .is_err()
                );
                assert_eq!(
                    store.get_todo_plan(owner, session).await.unwrap(),
                    Some(written.clone())
                );
                let cleared = store
                    .write_todo_plan(owner, session, second.id, &TodoPlan::default(), 4)
                    .await
                    .unwrap();
                assert!(cleared.id > written.id);
                assert_eq!(
                    store.get_todo_plan(owner, session).await.unwrap(),
                    Some(cleared)
                );
                // Invalid input and a failed database write preserve the last revision.
                let invalid = TodoPlan {
                    todos: vec![TodoItem {
                        id: "bad".into(),
                        content: String::new(),
                        status: TodoStatus::Pending,
                    }],
                };
                assert!(
                    store
                        .write_todo_plan(owner, session, second.id, &invalid, 5)
                        .await
                        .is_err()
                );
                store.db.execute("CREATE TRIGGER fail_plan BEFORE INSERT ON todo_updates BEGIN SELECT RAISE(ABORT, 'forced failure'); END", &[]).await.unwrap();
                assert!(
                    store
                        .write_todo_plan(owner, session, second.id, &plan, 5)
                        .await
                        .is_err()
                );
                store
                    .db
                    .execute("DROP TRIGGER fail_plan", &[])
                    .await
                    .unwrap();
                // A branch receives only plan revisions before the selected prompt.
                let branch = store
                    .fork_session(owner, session, second.id, 6)
                    .await
                    .unwrap();
                let branched = store
                    .get_todo_plan(owner, branch.session.id)
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(branched.plan, plan);
                assert_eq!(branched.session_id, branch.session.id);
                assert_ne!(branched.anchor_message_id, first.id);
                store
                    .delete_session(branch.session.id, owner)
                    .await
                    .unwrap();
                // Rewind deletes later anchored updates and restores the preceding plan.
                store
                    .prepare_rewind(owner, session, second.id)
                    .await
                    .unwrap();
                assert!(
                    store
                        .write_todo_plan(owner, session, second.id, &plan, 7)
                        .await
                        .is_err()
                );
                store
                    .complete_rewind(owner, session, second.id)
                    .await
                    .unwrap();
                assert_eq!(
                    store.get_todo_plan(owner, session).await.unwrap(),
                    Some(written)
                );
                store.delete_session(session, owner).await.unwrap();
                assert!(
                    store
                        .db
                        .execute("SELECT 1 FROM todo_updates", &[])
                        .await
                        .unwrap()
                        .rows
                        .is_empty()
                );
            }
        });
    }
}
