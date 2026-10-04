//! Owned, durable child-agent trees; tool effects remain in the ordinary journal.
use super::*;
use openwebide_core::{TaskHistory, TaskSnapshot, tasks::task_step_scope};

impl<D: Db> Store<D> {
    pub async fn list_tasks(&self, session: i64) -> Result<Vec<TaskHistory>, StorageError> {
        let rows = self.db.execute("SELECT anchor_message_id, snapshot FROM task_runs WHERE session_id = ? ORDER BY anchor_message_id, task_id", &[DbValue::Int(session)]).await?;
        rows.rows
            .iter()
            .map(|row| {
                Ok(TaskHistory {
                    anchor_message_id: row.get_int(0)?,
                    snapshot: serde_json::from_str(row.get_text(1)?)
                        .map_err(|error| StorageError::InvalidValue(error.to_string()))?,
                })
            })
            .collect()
    }
}
fn validate(snapshot: &TaskSnapshot, depth: usize, count: &mut usize) -> Result<(), StorageError> {
    *count += 1;
    if depth > openwebide_core::tasks::MAX_TASK_DEPTH || *count > 32 {
        return Err(StorageError::InvalidValue(
            "Child task tree exceeds its run limits".into(),
        ));
    }
    let parent = &snapshot.task.parent_tool_call_id;
    let index = snapshot
        .task
        .id
        .strip_prefix(&format!("{parent}.task"))
        .and_then(|index| index.parse::<usize>().ok());
    if !index
        .is_some_and(|index| (1..=openwebide_core::tasks::MAX_TASKS_PER_REQUEST).contains(&index))
        || (depth == 1 && openwebide_core::parse_step_id(parent).is_none())
    {
        return Err(StorageError::InvalidValue(
            "Invalid child task identity".into(),
        ));
    }
    for item in &snapshot.run.items {
        if let openwebide_core::RunItem::Step(step) = item
            && task_step_scope(&step.id) != Some(snapshot.task.id.as_str())
        {
            return Err(StorageError::InvalidValue(
                "Tool step is outside its child task".into(),
            ));
        }
    }
    for child in &snapshot.children {
        if task_step_scope(&child.task.parent_tool_call_id) != Some(snapshot.task.id.as_str()) {
            return Err(StorageError::InvalidValue(
                "Nested child has another parent".into(),
            ));
        }
        validate(child, depth + 1, count)?;
    }
    Ok(())
}
impl<D: Db + Send + Sync> Store<D> {
    pub fn save_task<'a>(
        &'a self,
        user: UserId,
        session: i64,
        anchor: i64,
        snapshot: &'a TaskSnapshot,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), StorageError>> + Send + 'a>>
    {
        Box::pin(async move {
            validate(snapshot, 1, &mut 0)?;
            self.db.transaction(|tx| async move {
                let store = Store::new(tx);
                store.get_session(session, user).await?;
                store.ensure_not_rewinding(session).await?;
                let parent = store.db.execute("SELECT anchor_message_id FROM tool_steps WHERE session_id = ? AND tool_call_id = ? AND name = 'task'", &[DbValue::Int(session), DbValue::Text(snapshot.task.parent_tool_call_id.clone())]).await?;
                if parent.rows.first().is_none_or(|row| row.get_int(0).ok() != Some(anchor)) {
                    return Err(StorageError::NotFound("Parent task tool not found".into()));
                }
                let previous = store.db.execute("SELECT anchor_message_id, snapshot FROM task_runs WHERE session_id = ? AND task_id = ?", &[DbValue::Int(session), DbValue::Text(snapshot.task.id.clone())]).await?;
                if let Some(row) = previous.rows.first() {
                    let old: TaskSnapshot = serde_json::from_str(row.get_text(1)?).map_err(|error| StorageError::InvalidValue(error.to_string()))?;
                    if old == *snapshot { return Ok(()); }
                    if row.get_int(0)? != anchor || old.task.parent_tool_call_id != snapshot.task.parent_tool_call_id || old.task.description != snapshot.task.description || old.run.finished.is_some() {
                        return Err(StorageError::Conflict("Child task already finalized or ownership changed".into()));
                    }
                    if let (Some(old), Some(new)) = (old.task.timing, snapshot.task.timing) {
                        old.merge(new).map_err(|error| StorageError::Conflict(error.into()))?;
                    }
                }
                let json = serde_json::to_string(snapshot).map_err(|error| StorageError::InvalidValue(error.to_string()))?;
                store.db.execute("INSERT INTO task_runs (session_id, task_id, anchor_message_id, snapshot) VALUES (?, ?, ?, ?) ON CONFLICT(session_id, task_id) DO UPDATE SET snapshot = excluded.snapshot", &[DbValue::Int(session), DbValue::Text(snapshot.task.id.clone()), DbValue::Int(anchor), DbValue::Text(json)]).await?;
                Ok(())
            }).await
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rusqlite_db::RusqliteDb;
    use openwebide_core::{AgentTask, RunEvent, TaskStatus, ToolTiming};
    fn snapshot(parent: &str) -> TaskSnapshot {
        TaskSnapshot::new(AgentTask {
            id: format!("{parent}.task1"),
            parent_tool_call_id: parent.into(),
            description: "Inspect packages".into(),
            status: TaskStatus::Running,
            timing: Some(ToolTiming::start(1000)),
            telemetry: Some(TurnTelemetry {
                prompt_tokens: 11,
                completion_tokens: 7,
                ..Default::default()
            }),
            tool_count: 0,
            result: None,
        })
    }
    #[test]
    fn child_history_is_owned_frozen_exported_and_preserved_by_forks_and_rewind_in_all_modes() {
        futures::executor::block_on(async {
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
                                    name: "test".into(),
                                    mode,
                                    path: Some("test".into()),
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
                let session = store
                    .create_session("test", None, None, project, user, 1)
                    .await
                    .unwrap();
                let prompt = store
                    .insert_message(session.id, Role::User, "Inspect packages", 1)
                    .await
                    .unwrap();
                let parent = format!("a{}t1c0", prompt.id);
                store
                    .upsert_tool_step(session.id, prompt.id, &parent, "task", "Delegate", 1, None)
                    .await
                    .unwrap();
                let mut task = snapshot(&parent);
                assert!(
                    store
                        .save_task(other, session.id, prompt.id, &task)
                        .await
                        .is_err()
                );
                assert!(
                    store
                        .save_task(user, session.id, prompt.id + 1, &task)
                        .await
                        .is_err()
                );
                store
                    .save_task(user, session.id, prompt.id, &task)
                    .await
                    .unwrap();
                let completed_message = ChatMessage {
                    id: 0,
                    session_id: 0,
                    role: Role::Assistant,
                    content: "Package findings".into(),
                    created_at: 2,
                    tool_calls: None,
                    tool_call_id: None,
                    usage: None,
                };
                task.task.status = TaskStatus::Completed;
                task.task.result = Some("Package findings".into());
                task.task.timing = task.task.timing.map(|timing| timing.sample(2000, true));
                task.run.finished = Some(RunEvent::Done {
                    message: completed_message,
                });
                store
                    .save_task(user, session.id, prompt.id, &task)
                    .await
                    .unwrap();
                store
                    .save_task(user, session.id, prompt.id, &task)
                    .await
                    .unwrap();
                assert!(
                    store
                        .save_task(user, session.id, prompt.id, &snapshot(&parent))
                        .await
                        .is_err()
                );
                store
                    .complete_tool_step(user, session.id, &parent, true, "Done", None)
                    .await
                    .unwrap();
                let next = store
                    .insert_message(session.id, Role::User, "Next", 3)
                    .await
                    .unwrap();
                let history = store.list_conversation(session.id).await.unwrap();
                assert!(
                    matches!(&history[2], ConversationEntry::Task(task) if task.snapshot.task.status == TaskStatus::Completed)
                );
                let markdown = openwebide_core::session_markdown(&session, &history);
                assert!(
                    markdown.contains("Child task: Inspect packages")
                        && markdown.contains("Package findings")
                );
                let branch = store
                    .fork_session(user, session.id, next.id, 3)
                    .await
                    .unwrap();
                assert_eq!(
                    store.list_tasks(branch.session.id).await.unwrap()[0].snapshot,
                    task
                );
                store
                    .prepare_rewind(user, session.id, next.id)
                    .await
                    .unwrap();
                assert!(
                    store
                        .save_task(user, session.id, prompt.id, &task)
                        .await
                        .is_err()
                );
                store
                    .complete_rewind(user, session.id, next.id)
                    .await
                    .unwrap();
                assert_eq!(
                    store.list_tasks(session.id).await.unwrap()[0].snapshot,
                    task
                );
                store
                    .prepare_rewind(user, session.id, prompt.id)
                    .await
                    .unwrap();
                store
                    .complete_rewind(user, session.id, prompt.id)
                    .await
                    .unwrap();
                assert!(store.list_tasks(session.id).await.unwrap().is_empty());
                assert_eq!(store.list_tasks(branch.session.id).await.unwrap().len(), 1);
                store.delete_session(branch.session.id, user).await.unwrap();
                assert!(
                    store
                        .list_tasks(branch.session.id)
                        .await
                        .unwrap()
                        .is_empty()
                );
            }
        });
    }
    #[test]
    fn checkpoint_execution_order_overrides_parallel_request_order_for_rewind_and_review() {
        futures::executor::block_on(async {
            use base64::{Engine, engine::general_purpose::STANDARD};
            use openwebide_core::rewind::{ProjectCheckpoint, ProjectSnapshot};
            let store = Store::new(RusqliteDb::open_in_memory().unwrap());
            store.migrate().await.unwrap();
            let user = store
                .insert_user("owner", "hash", UserRole::Admin, 1)
                .await
                .unwrap()
                .id;
            let project = store
                .create_project(
                    &NewProject {
                        name: "test".into(),
                        mode: WorkspaceMode::Remote,
                        path: Some("test".into()),
                    },
                    user,
                    1,
                )
                .await
                .unwrap();
            let session = store
                .create_session("test", None, None, Some(project.id), user, 1)
                .await
                .unwrap();
            let prompt = store
                .insert_message(session.id, Role::User, "Change file", 1)
                .await
                .unwrap();
            // Requested A then B; B acquires mutation ownership first.
            for id in ["A", "B"] {
                store
                    .upsert_tool_step(session.id, prompt.id, id, "write_file", "write", 1, None)
                    .await
                    .unwrap();
            }
            let snap = |content: &str| ProjectSnapshot {
                files: [("shared.txt".into(), STANDARD.encode(content))].into(),
                skipped: BTreeMap::new(),
            };
            for (id, before, after) in [("B", "original", "B"), ("A", "B", "A")] {
                store
                    .save_project_checkpoint(
                        session.id,
                        id,
                        &ProjectCheckpoint::from_snapshots(snap(before), None),
                    )
                    .await
                    .unwrap();
                store
                    .save_project_checkpoint(
                        session.id,
                        id,
                        &ProjectCheckpoint::from_snapshots(snap(before), Some(snap(after))),
                    )
                    .await
                    .unwrap();
                store
                    .complete_tool_step(user, session.id, id, true, "written", None)
                    .await
                    .unwrap();
            }
            let steps = store.list_tool_steps(session.id).await.unwrap();
            assert_eq!(
                steps
                    .iter()
                    .map(|step| step.tool_call_id.as_str())
                    .collect::<Vec<_>>(),
                ["B", "A"]
            );
            let review = store.list_run_changes(user, project.id).await.unwrap();
            assert_eq!(review.len(), 1);
            assert_eq!(
                review[0].file.before_bytes().unwrap(),
                Some(b"original".to_vec())
            );
            assert_eq!(review[0].file.after_bytes().unwrap(), Some(b"A".to_vec()));
            let rewind = store
                .prepare_rewind(user, session.id, prompt.id)
                .await
                .unwrap();
            assert_eq!(rewind.files.len(), 1);
            assert_eq!(
                rewind.files[0].before_bytes().unwrap(),
                Some(b"original".to_vec())
            );
            assert_eq!(rewind.files[0].after_bytes().unwrap(), Some(b"A".to_vec()));
        });
    }
}
