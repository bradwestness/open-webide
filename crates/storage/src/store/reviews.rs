//! Durable per-run file reviews shared by all execution hosts.
use super::*;
use openwebide_core::{ReviewPlan, ReviewRequest, RewindFile, RunChange};
use serde::Serialize;
fn json(value: &impl Serialize) -> Result<String, StorageError> {
    serde_json::to_string(value).map_err(|e| StorageError::Db(e.to_string()))
}
fn record(text: &str) -> Result<RunChange, StorageError> {
    serde_json::from_str(text).map_err(|e| StorageError::Db(e.to_string()))
}
impl<D: Db> Store<D> {
    pub(super) async fn record_run_changes(
        &self,
        user: UserId,
        project: i64,
        session: i64,
        anchor: i64,
        source_step: i64,
        checkpoint: &openwebide_core::rewind::ProjectCheckpoint,
    ) -> Result<(), StorageError> {
        let rows = self.db.execute("SELECT id FROM messages WHERE session_id = ? AND role = 'user' AND id <= ? ORDER BY id DESC LIMIT 1", &[DbValue::Int(session), DbValue::Int(anchor)]).await?;
        let Some(prompt) = rows.rows.first() else {
            return Ok(());
        };
        let message = prompt.get_int(0)?;
        for file in checkpoint.changes().map_err(StorageError::Conflict)? {
            let existing = self.db.execute("SELECT state FROM run_changes WHERE session_id = ? AND message_id = ? AND path = ?", &[DbValue::Int(session), DbValue::Int(message), DbValue::Text(file.path.clone())]).await?;
            let mut change = if let Some(row) = existing.rows.first() {
                let mut change = record(row.get_text(0)?)?;
                change.coalesce(file).map_err(StorageError::Conflict)?;
                change
            } else {
                RunChange::new(session, message, file).map_err(StorageError::Conflict)?
            };
            change.source_step = source_step;
            self.db.execute("INSERT INTO run_changes (project_id, session_id, message_id, path, state) VALUES (?, ?, ?, ?, ?) ON CONFLICT(session_id, message_id, path) DO UPDATE SET state = excluded.state", &[DbValue::Int(project), DbValue::Int(session), DbValue::Int(message), DbValue::Text(change.file.path.clone()), DbValue::Text(json(&change)?)]).await?;
            self.project_review_file(user, project, &change).await?;
        }
        Ok(())
    }
    async fn project_review_file(
        &self,
        user: UserId,
        project: i64,
        change: &RunChange,
    ) -> Result<(), StorageError> {
        let file = change.pending_file().map_err(StorageError::Conflict)?;
        let decision = if change.pending() > 0 {
            EditDecision::Pending
        } else if change
            .decisions
            .iter()
            .all(|&d| d == EditDecision::Rejected)
        {
            EditDecision::Rejected
        } else {
            EditDecision::Accepted
        };
        self.db.execute("INSERT INTO pending_edits (user_id, project_id, path, revision, decision, diff, file) VALUES (?, ?, ?, 1, ?, ?, ?) ON CONFLICT(project_id, path) DO UPDATE SET revision = pending_edits.revision + 1, decision = excluded.decision, diff = excluded.diff, file = excluded.file", &[DbValue::Int(user.get()), DbValue::Int(project), DbValue::Text(file.path.clone()), DbValue::Text(edit_decision_text(decision).into()), DbValue::Text(json(&file.preview())?), DbValue::Text(json(&file)?)]).await?;
        Ok(())
    }
    async fn available_review(
        &self,
        project: i64,
        change: &mut RunChange,
    ) -> Result<(), StorageError> {
        let rows = self
            .db
            .execute(
                "SELECT file FROM pending_edits WHERE project_id = ? AND path = ?",
                &[
                    DbValue::Int(project),
                    DbValue::Text(change.file.path.clone()),
                ],
            )
            .await?;
        let file = rows
            .rows
            .first()
            .and_then(|row| row.get_text_opt(0))
            .map(serde_json::from_str::<RewindFile>)
            .transpose()
            .map_err(|e| StorageError::Db(e.to_string()))?;
        change.available = !change.conflicted
            && file
                .as_ref()
                .map(RewindFile::after_bytes)
                .transpose()
                .map_err(StorageError::Conflict)?
                .flatten()
                == change.current_bytes().map_err(StorageError::Conflict)?;
        Ok(())
    }
    pub async fn list_run_changes(
        &self,
        user: UserId,
        project: i64,
    ) -> Result<Vec<RunChange>, StorageError> {
        self.get_project(project, user).await?;
        let rows = self
            .db
            .execute(
                "SELECT state FROM run_changes WHERE project_id = ? ORDER BY message_id, path",
                &[DbValue::Int(project)],
            )
            .await?;
        let mut records = Vec::new();
        for row in &rows.rows {
            let mut change = record(row.get_text(0)?)?;
            self.available_review(project, &mut change).await?;
            records.push(change);
        }
        Ok(records)
    }
    pub async fn preview_run_review(
        &self,
        user: UserId,
        project: i64,
        request: &ReviewRequest,
    ) -> Result<ReviewPlan, StorageError> {
        let session = self.get_session(request.session_id, user).await?;
        self.get_project(project, user).await?;
        if session.project_id != Some(project) {
            return Err(StorageError::NotFound("run review".into()));
        }
        let rows = self
            .db
            .execute(
                "SELECT plan FROM project_reviews WHERE project_id = ?",
                &[DbValue::Int(project)],
            )
            .await?;
        if let Some(row) = rows.rows.first() {
            let plan: ReviewPlan = serde_json::from_str(row.get_text(0)?)
                .map_err(|e| StorageError::Db(e.to_string()))?;
            if plan.request == *request {
                return Ok(plan);
            }
            return Err(StorageError::Conflict(
                "Resume the already prepared review first".into(),
            ));
        }
        let rows = self.db.execute("SELECT state FROM run_changes WHERE project_id = ? AND session_id = ? AND message_id = ? AND path = ?", &[DbValue::Int(project), DbValue::Int(request.session_id), DbValue::Int(request.message_id), DbValue::Text(request.path.clone())]).await?;
        let mut change = rows
            .rows
            .first()
            .map(|row| record(row.get_text(0)?))
            .transpose()?
            .ok_or_else(|| StorageError::NotFound("run change".into()))?;
        self.available_review(project, &mut change).await?;
        change
            .prepare(request.clone())
            .map_err(StorageError::Conflict)
    }
    pub async fn prepare_run_review(
        &self,
        user: UserId,
        project: i64,
        request: &ReviewRequest,
    ) -> Result<ReviewPlan, StorageError> {
        self.db.transaction(|tx| async move {
            let store = Store::new(tx);
            let plan = store.preview_run_review(user, project, request).await?;
            let exists = store.db.execute("SELECT 1 FROM project_reviews WHERE project_id = ?", &[DbValue::Int(project)]).await?;
            if !exists.rows.is_empty() { return Ok(plan); }
            store.ensure_not_rewinding(request.session_id).await?;
            let pending = store.db.execute("SELECT 1 FROM tool_steps t JOIN sessions s ON s.id = t.session_id WHERE s.project_id = ? AND t.ok IS NULL LIMIT 1", &[DbValue::Int(project)]).await?;
            if !pending.rows.is_empty() { return Err(StorageError::Conflict("Wait for project tools to finish before reviewing changes".into())); }
            store.db.execute("INSERT INTO project_reviews (project_id, session_id, plan) VALUES (?, ?, ?)", &[DbValue::Int(project), DbValue::Int(request.session_id), DbValue::Text(json(&plan)?)]).await?;
            Ok(plan)
        }).await
    }
    pub async fn complete_run_review(
        &self,
        user: UserId,
        project: i64,
        request: &ReviewRequest,
    ) -> Result<RunChange, StorageError> {
        self.db.transaction(|tx| async move {
            let store = Store::new(tx);
            store.get_project(project, user).await?;
            let owned = store.get_session(request.session_id, user).await?;
            if owned.project_id != Some(project) { return Err(StorageError::NotFound("run review".into())); }
            let rows = store.db.execute("SELECT plan FROM project_reviews WHERE project_id = ?", &[DbValue::Int(project)]).await?;
            let Some(row) = rows.rows.first() else {
                let rows = store.db.execute("SELECT request, state FROM run_review_history WHERE project_id = ? AND session_id = ? AND message_id = ? AND path = ? AND revision = ?", &[DbValue::Int(project), DbValue::Int(request.session_id), DbValue::Int(request.message_id), DbValue::Text(request.path.clone()), DbValue::Int(request.revision)]).await?;
                let row = rows.rows.first().ok_or_else(|| StorageError::NotFound("prepared review".into()))?;
                if row.get_text(0)? != json(request)? { return Err(StorageError::Conflict("Review decision changed".into())); }
                return record(row.get_text(1)?);
            };
            let plan: ReviewPlan = serde_json::from_str(row.get_text(0)?).map_err(|e| StorageError::Db(e.to_string()))?;
            if plan.request != *request { return Err(StorageError::Conflict("Review request changed".into())); }
            let current = store.db.execute("SELECT state FROM run_changes WHERE session_id = ? AND message_id = ? AND path = ?", &[DbValue::Int(request.session_id), DbValue::Int(request.message_id), DbValue::Text(request.path.clone())]).await?;
            if current.rows.first().map(|row| record(row.get_text(0)?)).transpose()? != Some(plan.original.clone()) { return Err(StorageError::Conflict("Run changed during review".into())); }
            store.db.execute("UPDATE run_changes SET state = ? WHERE session_id = ? AND message_id = ? AND path = ?", &[DbValue::Text(json(&plan.reviewed)?), DbValue::Int(request.session_id), DbValue::Int(request.message_id), DbValue::Text(request.path.clone())]).await?;
            store.project_review_file(user, project, &plan.reviewed).await?;
            // A rejection is itself a project file transition. Reflect it in
            // the latest checkpoint so later rewind sees the actual contents.
            let latest = store.db.execute("SELECT state FROM run_changes WHERE project_id = ? AND path = ? ORDER BY COALESCE((SELECT execution_order FROM tool_steps WHERE id = json_extract(state, '$.source_step')), json_extract(state, '$.source_step')) DESC LIMIT 1", &[DbValue::Int(project), DbValue::Text(request.path.clone())]).await?;
            if let Some(row) = latest.rows.first() {
                let latest = record(row.get_text(0)?)?;
                let source = store.db.execute("SELECT checkpoint FROM tool_steps WHERE id = ?", &[DbValue::Int(latest.source_step)]).await?;
                if let Some(text) = source.rows.first().and_then(|row| row.get_text_opt(0)) {
                    use base64::{Engine, engine::general_purpose::STANDARD};
                    let mut checkpoint: openwebide_core::rewind::ProjectCheckpoint = serde_json::from_str(text).map_err(|e| StorageError::Db(e.to_string()))?;
                    if let Some(after) = &mut checkpoint.after {
                        match plan.reviewed.current_bytes().map_err(StorageError::Conflict)? { Some(bytes) => { after.insert(request.path.clone(), STANDARD.encode(bytes)); }, None => { after.remove(&request.path); } }
                        store.db.execute("UPDATE tool_steps SET checkpoint = ? WHERE id = ?", &[DbValue::Text(json(&checkpoint)?), DbValue::Int(latest.source_step)]).await?;
                    }
                }
            }
            store.db.execute("INSERT INTO run_review_history (project_id, session_id, message_id, path, revision, request, state) VALUES (?, ?, ?, ?, ?, ?, ?)", &[DbValue::Int(project), DbValue::Int(request.session_id), DbValue::Int(request.message_id), DbValue::Text(request.path.clone()), DbValue::Int(request.revision), DbValue::Text(json(request)?), DbValue::Text(json(&plan.reviewed)?)]).await?;
            store.db.execute("DELETE FROM project_reviews WHERE project_id = ?", &[DbValue::Int(project)]).await?;
            Ok(plan.reviewed)
        }).await
    }
    pub(super) async fn reconcile_review_rewind(
        &self,
        user: UserId,
        project: i64,
        files: &[RewindFile],
    ) -> Result<(), StorageError> {
        for file in files {
            let rows = self
                .db
                .execute(
                    "SELECT file FROM pending_edits WHERE project_id = ? AND path = ?",
                    &[DbValue::Int(project), DbValue::Text(file.path.clone())],
                )
                .await?;
            if rows
                .rows
                .first()
                .and_then(|row| row.get_text_opt(0))
                .is_none()
            {
                continue;
            }
            let rows = self.db.execute("SELECT state FROM run_changes WHERE project_id = ? AND path = ? ORDER BY COALESCE((SELECT execution_order FROM tool_steps WHERE id = json_extract(state, '$.source_step')), json_extract(state, '$.source_step')) DESC", &[DbValue::Int(project), DbValue::Text(file.path.clone())]).await?;
            let mut remaining = None;
            for row in &rows.rows {
                let record = record(row.get_text(0)?)?;
                if record.pending() > 0
                    && record.current_bytes().map_err(StorageError::Conflict)?
                        == file.before_bytes().map_err(StorageError::Conflict)?
                {
                    remaining = Some(record);
                    break;
                }
            }
            if let Some(record) = remaining {
                self.project_review_file(user, project, &record).await?;
            } else {
                self.db
                    .execute(
                        "DELETE FROM pending_edits WHERE project_id = ? AND path = ?",
                        &[DbValue::Int(project), DbValue::Text(file.path.clone())],
                    )
                    .await?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rusqlite_db::RusqliteDb;
    use base64::{Engine, engine::general_purpose::STANDARD};
    use futures::executor::block_on;
    #[test]
    fn reviews_are_owned_durable_atomic_and_keep_rewind_consistent_in_both_modes() {
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
                            path: Some("test".into()),
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
                    .create_session("sibling", None, None, Some(project.id), user, 1)
                    .await
                    .unwrap();
                let prompt = store
                    .insert_message(session.id, Role::User, "edit files", 1)
                    .await
                    .unwrap();
                let interim = store
                    .insert_message(session.id, Role::Assistant, "tools", 1)
                    .await
                    .unwrap();
                store
                    .upsert_tool_step(
                        session.id,
                        interim.id,
                        "shell",
                        "run_command",
                        "shell",
                        1,
                        None,
                    )
                    .await
                    .unwrap();
                let checkpoint = openwebide_core::rewind::ProjectCheckpoint {
                    skipped: Default::default(),
                    before: [("file.txt".into(), STANDARD.encode("one\r\nkeep\nthree"))].into(),
                    after: Some(
                        [
                            ("file.txt".into(), STANDARD.encode("ONE\r\nkeep\nTHREE\n")),
                            ("binary.dat".into(), STANDARD.encode([0, 255])),
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
                        "failed after writing",
                        None,
                    )
                    .await
                    .unwrap();
                let changes = store.list_run_changes(user, project.id).await.unwrap();
                assert_eq!(changes.len(), 2);
                assert!(changes.iter().all(|change| change.message_id == prompt.id));
                assert!(store.list_run_changes(other, project.id).await.is_err());
                let text = changes
                    .iter()
                    .find(|change| change.file.path == "file.txt")
                    .unwrap();
                let request = ReviewRequest {
                    session_id: session.id,
                    message_id: prompt.id,
                    path: "file.txt".into(),
                    revision: text.revision,
                    decision: EditDecision::Accepted,
                    hunk: Some(0),
                };
                assert!(
                    store
                        .prepare_run_review(other, project.id, &request)
                        .await
                        .is_err()
                );
                let plan = store
                    .prepare_run_review(user, project.id, &request)
                    .await
                    .unwrap();
                assert_eq!(
                    store
                        .prepare_run_review(user, project.id, &request)
                        .await
                        .unwrap(),
                    plan
                );
                assert!(
                    store
                        .insert_message(sibling.id, Role::User, "blocked", 2)
                        .await
                        .is_err()
                );
                assert!(
                    store
                        .prepare_rewind(user, session.id, prompt.id)
                        .await
                        .is_err()
                );
                store.db.execute("CREATE TRIGGER fail_review BEFORE UPDATE ON run_changes BEGIN SELECT RAISE(ABORT, 'fail'); END", &[]).await.unwrap();
                assert!(
                    store
                        .complete_run_review(user, project.id, &request)
                        .await
                        .is_err()
                );
                assert_eq!(
                    store.list_run_changes(user, project.id).await.unwrap(),
                    changes
                );
                store
                    .db
                    .execute("DROP TRIGGER fail_review", &[])
                    .await
                    .unwrap();
                let accepted = store
                    .complete_run_review(user, project.id, &request)
                    .await
                    .unwrap();
                assert_eq!(
                    store
                        .complete_run_review(user, project.id, &request)
                        .await
                        .unwrap(),
                    accepted
                );
                assert!(
                    store
                        .complete_run_review(
                            user,
                            project.id,
                            &ReviewRequest {
                                decision: EditDecision::Rejected,
                                ..request.clone()
                            }
                        )
                        .await
                        .is_err()
                );
                let pending = store.list_pending_edits(user, project.id).await.unwrap();
                assert_eq!(
                    pending
                        .iter()
                        .find(|edit| edit.path == "file.txt")
                        .unwrap()
                        .diff
                        .old
                        .as_deref(),
                    Some("ONE\r\nkeep\nthree")
                );
                let reject = ReviewRequest {
                    revision: accepted.revision,
                    decision: EditDecision::Rejected,
                    hunk: Some(1),
                    ..request
                };
                store
                    .prepare_run_review(user, project.id, &reject)
                    .await
                    .unwrap();
                let rejected = store
                    .complete_run_review(user, project.id, &reject)
                    .await
                    .unwrap();
                assert_eq!(rejected.pending(), 0);
                let binary = changes
                    .iter()
                    .find(|change| change.file.path == "binary.dat")
                    .unwrap();
                let binary_request = ReviewRequest {
                    session_id: session.id,
                    message_id: prompt.id,
                    path: binary.file.path.clone(),
                    revision: binary.revision,
                    decision: EditDecision::Rejected,
                    hunk: None,
                };
                store
                    .prepare_run_review(user, project.id, &binary_request)
                    .await
                    .unwrap();
                store
                    .complete_run_review(user, project.id, &binary_request)
                    .await
                    .unwrap();
                assert!(
                    store
                        .list_pending_edits(user, project.id)
                        .await
                        .unwrap()
                        .is_empty()
                );
                let rewind = store
                    .prepare_rewind(user, session.id, prompt.id)
                    .await
                    .unwrap();
                assert_eq!(rewind.files.len(), 1);
                assert_eq!(rewind.files[0].after, "ONE\r\nkeep\nthree");
                assert_eq!(
                    rewind.files[0].before.as_deref(),
                    Some("one\r\nkeep\nthree")
                );
                store
                    .complete_rewind(user, session.id, prompt.id)
                    .await
                    .unwrap();
                assert!(
                    store
                        .list_run_changes(user, project.id)
                        .await
                        .unwrap()
                        .is_empty()
                );
                assert!(
                    store
                        .list_pending_edits(user, project.id)
                        .await
                        .unwrap()
                        .is_empty()
                );
            }
        });
    }
}
