//! The same durable worker contract runs against server and paired-host bindings.
use super::*;
use crate::rusqlite_db::RusqliteDb;
use openwebide_core::{
    GoalCommand, GoalStatus,
    goal::{GoalEvaluation, GoalVerdict},
    scheduled::{DispatchResult, ExecutionHost, HostBinding, TaskDelivery},
};

struct Fixture {
    store: Store<RusqliteDb>,
    user: UserId,
    other: UserId,
    session: i64,
    project: Option<i64>,
    host: ExecutionHost,
    binding: Option<HostBinding>,
}
impl Fixture {
    async fn new(mode: Option<WorkspaceMode>) -> Self {
        Self::with_db(mode, RusqliteDb::open_in_memory().unwrap()).await
    }
    async fn with_db(mode: Option<WorkspaceMode>, db: RusqliteDb) -> Self {
        let store = Store::new(db);
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
                            name: "project".into(),
                            mode,
                            path: Some("project".into()),
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
            .create_session("goal", None, None, project, user, 1)
            .await
            .unwrap()
            .id;
        let local = mode == Some(WorkspaceMode::Local);
        let host = ExecutionHost {
            id: if local { "paired" } else { "server" }.into(),
            name: "Host".into(),
            last_seen: 0,
        };
        let binding = local.then(|| HostBinding {
            host_id: host.id.clone(),
            path: "project".into(),
        });
        Self {
            store,
            user,
            other,
            session,
            project,
            host,
            binding,
        }
    }
    async fn start(&self, now: i64) -> openwebide_core::Goal {
        self.store
            .dispatch_goal(
                self.user,
                self.session,
                0,
                GoalCommand::Start {
                    objective: "Make tests pass and verify them".into(),
                },
                self.binding.as_ref(),
                now,
            )
            .await
            .unwrap()
    }
    async fn inject(&self, delivery: &TaskDelivery, now: i64) -> i64 {
        let token = format!("scheduled-{}", delivery.run_id);
        self.store
            .session_run_lease(self.user, self.session, &token, false, now)
            .await
            .unwrap();
        let anchor = self
            .store
            .consume_queued_prompt(
                self.user,
                self.session,
                delivery.prompt.key(),
                &delivery.prompt.content,
                now,
            )
            .await
            .unwrap()
            .id;
        self.store
            .session_run_lease(self.user, self.session, &token, true, now)
            .await
            .unwrap();
        anchor
    }
    async fn finish(&self, delivery: &TaskDelivery, verdict: GoalVerdict, tools: bool, now: i64) {
        let message = self
            .store
            .insert_message(
                self.session,
                Role::Assistant,
                "Evidence: cargo test passed",
                now,
            )
            .await
            .unwrap();
        let assessment = GoalTurnAssessment {
            evaluation: GoalEvaluation {
                verdict,
                reason: "Observed test evidence".into(),
            },
            last_message: message.id,
            used_tools: tools,
        };
        self.store
            .scheduled_result_evaluated(
                &self.host.id,
                &DispatchResult {
                    run_id: delivery.run_id,
                    status: "complete".into(),
                    detail: "Tests passed".into(),
                    permission_id: None,
                },
                Some(&assessment),
                now,
            )
            .await
            .unwrap();
    }
    async fn goal(&self) -> openwebide_core::Goal {
        self.store
            .get_goal(self.user, self.session)
            .await
            .unwrap()
            .unwrap()
    }
}

#[test]
fn host_goals_claim_continue_complete_and_preserve_ownership_in_all_modes() {
    futures::executor::block_on(async {
        for mode in [
            Some(WorkspaceMode::Local),
            Some(WorkspaceMode::Remote),
            None,
        ] {
            let f = Fixture::new(mode).await;
            if mode == Some(WorkspaceMode::Local) {
                assert!(
                    f.store
                        .dispatch_goal(
                            f.user,
                            f.session,
                            0,
                            GoalCommand::Start {
                                objective: "work".into()
                            },
                            None,
                            1
                        )
                        .await
                        .is_err()
                );
                assert!(
                    f.store.get_goal(f.user, f.session).await.unwrap().is_none(),
                    "activation must roll back with the host binding"
                );
            }
            assert!(
                f.store
                    .dispatch_goal(
                        f.other,
                        f.session,
                        0,
                        GoalCommand::Start {
                            objective: "work".into()
                        },
                        f.binding.as_ref(),
                        1
                    )
                    .await
                    .is_err()
            );
            let goal = f.start(1).await;
            assert!(goal.worker);
            assert!(
                f.store
                    .scheduled_tasks(f.user, f.project, 1)
                    .await
                    .unwrap()
                    .is_empty(),
                "internal worker is not a user scheduled task"
            );
            let wrong = ExecutionHost {
                id: "other-host".into(),
                ..f.host.clone()
            };
            assert!(f.store.due_scheduled(&wrong, 2).await.unwrap().is_empty());
            let delivery = f.store.due_scheduled(&f.host, 2).await.unwrap().remove(0);
            assert_eq!(delivery.binding, f.binding);
            assert!(
                f.store.due_scheduled(&f.host, 3).await.unwrap().is_empty(),
                "only one claim per turn"
            );
            assert!(
                f.store
                    .consume_queued_prompt(
                        f.user,
                        f.session,
                        delivery.prompt.key(),
                        &delivery.prompt.content,
                        3
                    )
                    .await
                    .is_err(),
                "browser cannot steal the host prompt"
            );
            let anchor = f.inject(&delivery, 3).await;
            assert_eq!(
                f.store
                    .goal_run_context(&f.host.id, delivery.run_id)
                    .await
                    .unwrap()
                    .unwrap()
                    .2,
                anchor
            );
            assert!(
                f.store
                    .goal_run_context(&wrong.id, delivery.run_id)
                    .await
                    .unwrap()
                    .is_none()
            );
            f.finish(&delivery, GoalVerdict::Continue, true, 4).await;
            assert_eq!(f.goal().await.status, GoalStatus::Active);
            // Duplicate terminal acknowledgements cannot enqueue another turn.
            f.finish(&delivery, GoalVerdict::Complete, true, 4).await;
            assert_eq!(f.goal().await.status, GoalStatus::Active);
            let next = f.store.due_scheduled(&f.host, 5).await.unwrap().remove(0);
            assert_ne!(next.run_id, delivery.run_id);
            assert!(next.prompt.content.contains("Latest goal evaluation"));
            f.inject(&next, 6).await;
            f.finish(&next, GoalVerdict::Complete, true, 7).await;
            let completed = f.goal().await;
            assert_eq!(completed.status, GoalStatus::Completed);
            assert_eq!(completed.completed_duration_seconds(), Some(6));
            assert!(
                f.store
                    .due_scheduled(&f.host, 500)
                    .await
                    .unwrap()
                    .is_empty()
            );
        }
    });
}

#[test]
fn host_goals_recover_claims_and_continue_consumed_turns_without_replaying_in_all_modes() {
    futures::executor::block_on(async {
        for mode in [
            Some(WorkspaceMode::Local),
            Some(WorkspaceMode::Remote),
            None,
        ] {
            let f = Fixture::new(mode).await;
            f.start(1).await;
            let first = f.store.due_scheduled(&f.host, 2).await.unwrap().remove(0);
            let reclaimed = f.store.due_scheduled(&f.host, 123).await.unwrap().remove(0);
            assert_eq!(first.run_id, reclaimed.run_id);
            assert_eq!(first.prompt.key(), reclaimed.prompt.key());
            f.inject(&reclaimed, 124).await;
            let recovered = f.store.due_scheduled(&f.host, 245).await.unwrap().remove(0);
            assert_ne!(recovered.run_id, first.run_id);
            assert_ne!(recovered.prompt.key(), first.prompt.key());
            assert_eq!(
                f.store.list_messages(f.session).await.unwrap().len(),
                1,
                "consumed turn remains in history exactly once"
            );
            f.store
                .dispatch_goal(f.user, f.session, 1, GoalCommand::Pause, None, 246)
                .await
                .unwrap();
            assert!(
                f.store
                    .list_queued_prompts(f.user, f.session)
                    .await
                    .unwrap()
                    .is_empty()
            );
            assert!(
                f.store
                    .due_scheduled(&f.host, 1000)
                    .await
                    .unwrap()
                    .is_empty()
            );
            assert!(
                f.store
                    .goal_run_cancelled(
                        f.user,
                        f.session,
                        &format!("scheduled-{}", recovered.run_id)
                    )
                    .await
                    .unwrap()
            );
        }
    });
}

#[test]
fn host_goals_yield_to_foreground_and_ignore_paused_or_stale_evaluations_in_all_modes() {
    futures::executor::block_on(async {
        for mode in [
            Some(WorkspaceMode::Local),
            Some(WorkspaceMode::Remote),
            None,
        ] {
            let f = Fixture::new(mode).await;
            f.start(1).await;
            let prompt = f
                .store
                .enqueue_prompt(f.user, f.session, "User work first", 2)
                .await
                .unwrap();
            assert!(f.store.due_scheduled(&f.host, 3).await.unwrap().is_empty());
            f.store
                .remove_queued_prompt(f.user, f.session, prompt.key())
                .await
                .unwrap();
            f.store
                .session_run_lease(f.user, f.session, "foreground", false, 4)
                .await
                .unwrap();
            assert!(f.store.due_scheduled(&f.host, 5).await.unwrap().is_empty());
            f.store
                .session_run_lease(f.user, f.session, "foreground", true, 6)
                .await
                .unwrap();
            let first = f.store.due_scheduled(&f.host, 7).await.unwrap().remove(0);
            f.inject(&first, 8).await;
            let message = f
                .store
                .insert_message(f.session, Role::Assistant, "Tests pass", 9)
                .await
                .unwrap();
            let assessment = GoalTurnAssessment {
                evaluation: GoalEvaluation {
                    verdict: GoalVerdict::Complete,
                    reason: "test result".into(),
                },
                last_message: message.id,
                used_tools: true,
            };
            f.store
                .insert_message(f.session, Role::User, "Also check another test", 10)
                .await
                .unwrap();
            f.store
                .scheduled_result_evaluated(
                    &f.host.id,
                    &DispatchResult {
                        run_id: first.run_id,
                        status: "complete".into(),
                        detail: String::new(),
                        permission_id: None,
                    },
                    Some(&assessment),
                    11,
                )
                .await
                .unwrap();
            assert_eq!(
                f.goal().await.status,
                GoalStatus::Active,
                "old evidence cannot complete after new user input"
            );
            let second = f.store.due_scheduled(&f.host, 12).await.unwrap().remove(0);
            f.inject(&second, 13).await;
            let paused = f
                .store
                .dispatch_goal(f.user, f.session, 1, GoalCommand::Pause, None, 14)
                .await
                .unwrap();
            let resumed = f
                .store
                .dispatch_goal(
                    f.user,
                    f.session,
                    paused.revision,
                    GoalCommand::Resume,
                    f.binding.as_ref(),
                    15,
                )
                .await
                .unwrap();
            f.finish(&second, GoalVerdict::Complete, true, 16).await;
            assert_eq!(
                f.goal().await,
                resumed,
                "an old run cannot complete a resumed goal"
            );
            assert!(!f.store.due_scheduled(&f.host, 17).await.unwrap().is_empty());
        }
    });
}

#[test]
fn host_goals_pause_on_blockers_errors_missing_evaluation_and_stalls_in_all_modes() {
    futures::executor::block_on(async {
        for mode in [
            Some(WorkspaceMode::Local),
            Some(WorkspaceMode::Remote),
            None,
        ] {
            for status in [
                "failed",
                "cancelled",
                "complete",
                "blocked-verdict",
                "stall",
            ] {
                let f = Fixture::new(mode).await;
                f.start(1).await;
                let mut now = 2;
                for turn in 0..if status == "stall" { 3 } else { 1 } {
                    let delivery = f.store.due_scheduled(&f.host, now).await.unwrap().remove(0);
                    f.inject(&delivery, now + 1).await;
                    if status == "blocked-verdict" || status == "stall" {
                        f.finish(
                            &delivery,
                            if status == "stall" {
                                GoalVerdict::Continue
                            } else {
                                GoalVerdict::Blocked
                            },
                            false,
                            now + 2,
                        )
                        .await;
                    } else {
                        f.store
                            .scheduled_result(
                                &f.host.id,
                                &DispatchResult {
                                    run_id: delivery.run_id,
                                    status: status.into(),
                                    detail: "Needs review".into(),
                                    permission_id: None,
                                },
                                now + 2,
                            )
                            .await
                            .unwrap();
                    }
                    if status == "stall" && turn < 2 {
                        assert_eq!(f.goal().await.status, GoalStatus::Active);
                    }
                    now += 3;
                }
                assert_eq!(
                    f.goal().await.status,
                    GoalStatus::Paused,
                    "{mode:?}: {status}"
                );
                assert!(f.goal().await.note.is_some());
                assert!(
                    f.store
                        .due_scheduled(&f.host, 1000)
                        .await
                        .unwrap()
                        .is_empty()
                );
            }
        }
    });
}

#[test]
fn host_goals_survive_database_reopen_and_yield_claimed_prompts_to_new_input() {
    futures::executor::block_on(async {
        for mode in [
            Some(WorkspaceMode::Local),
            Some(WorkspaceMode::Remote),
            None,
        ] {
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir()
                .join(format!("openwebide-goal-{}-{stamp}.db", std::process::id()));
            let mut f = Fixture::with_db(mode, RusqliteDb::open(&path).unwrap()).await;
            f.start(1).await;
            // Drop every connection, as on backend restart. There is no browser executor.
            drop(f.store);
            f.store = Store::new(RusqliteDb::open(&path).unwrap());
            f.store.migrate().await.unwrap();
            assert!(f.goal().await.worker);
            let claimed = f.store.due_scheduled(&f.host, 2).await.unwrap().remove(0);
            let user_prompt = f
                .store
                .enqueue_prompt(f.user, f.session, "User input takes priority", 3)
                .await
                .unwrap();
            assert_eq!(
                f.store
                    .list_queued_prompts(f.user, f.session)
                    .await
                    .unwrap(),
                vec![user_prompt.clone()]
            );
            assert!(
                f.store
                    .consume_queued_prompt(
                        f.user,
                        f.session,
                        claimed.prompt.key(),
                        &claimed.prompt.content,
                        3
                    )
                    .await
                    .is_err()
            );
            f.store
                .remove_queued_prompt(f.user, f.session, user_prompt.key())
                .await
                .unwrap();
            let next = f.store.due_scheduled(&f.host, 4).await.unwrap().remove(0);
            assert_ne!(claimed.run_id, next.run_id);
            f.store
                .remove_queued_prompt(f.user, f.session, next.prompt.key())
                .await
                .unwrap();
            assert_eq!(f.goal().await.status, GoalStatus::Paused);
            assert!(
                f.store
                    .due_scheduled(&f.host, 500)
                    .await
                    .unwrap()
                    .is_empty()
            );
            drop(f);
            std::fs::remove_file(path).unwrap();
        }
    });
}

#[test]
fn host_goals_keep_approvals_pending_and_cancel_old_turns_by_revision() {
    futures::executor::block_on(async {
        for mode in [
            Some(WorkspaceMode::Local),
            Some(WorkspaceMode::Remote),
            None,
        ] {
            let f = Fixture::new(mode).await;
            let legacy = f
                .store
                .update_goal(
                    f.user,
                    f.session,
                    0,
                    GoalCommand::Start {
                        objective: "Verify tests".into(),
                    },
                    1,
                )
                .await
                .unwrap();
            assert!(!legacy.worker);
            assert!(f.store.due_scheduled(&f.host, 2).await.unwrap().is_empty());
            f.store
                .dispatch_goal(
                    f.user,
                    f.session,
                    legacy.revision,
                    GoalCommand::Resume,
                    f.binding.as_ref(),
                    3,
                )
                .await
                .unwrap();
            let delivery = f.store.due_scheduled(&f.host, 4).await.unwrap().remove(0);
            f.inject(&delivery, 5).await;
            let token = format!("scheduled-{}", delivery.run_id);
            assert!(
                !f.store
                    .goal_run_cancelled(f.user, f.session, &token)
                    .await
                    .unwrap()
            );
            f.store
                .scheduled_result(
                    &f.host.id,
                    &DispatchResult {
                        run_id: delivery.run_id,
                        status: "blocked".into(),
                        detail: "Approve shell command".into(),
                        permission_id: Some("command".into()),
                    },
                    5,
                )
                .await
                .unwrap();
            assert_eq!(f.goal().await.status, GoalStatus::Active);
            assert!(f.store.due_scheduled(&f.host, 6).await.unwrap().is_empty());
            assert!(
                f.store
                    .take_tool_permission(f.session, "command")
                    .await
                    .unwrap()
                    .is_none(),
                "the worker must never manufacture an approval"
            );
            let paused = f
                .store
                .dispatch_goal(f.user, f.session, 2, GoalCommand::Pause, None, 5)
                .await
                .unwrap();
            assert!(
                f.store
                    .goal_run_cancelled(f.user, f.session, &token)
                    .await
                    .unwrap(),
                "same-second pause is independent of the clock"
            );
            f.store
                .dispatch_goal(
                    f.user,
                    f.session,
                    paused.revision,
                    GoalCommand::Resume,
                    f.binding.as_ref(),
                    5,
                )
                .await
                .unwrap();
            assert!(
                f.store
                    .goal_run_cancelled(f.user, f.session, &token)
                    .await
                    .unwrap(),
                "old turns remain cancelled after resume"
            );
            f.store
                .scheduled_result(
                    &f.host.id,
                    &DispatchResult {
                        run_id: delivery.run_id,
                        status: "cancelled".into(),
                        detail: "Stopped".into(),
                        permission_id: None,
                    },
                    6,
                )
                .await
                .unwrap();
            let next = f.store.due_scheduled(&f.host, 7).await.unwrap().remove(0);
            f.inject(&next, 8).await;
            f.store
                .db
                .execute(
                    "UPDATE goal_workers SET turns=99 WHERE session_id=?",
                    &[DbValue::Int(f.session)],
                )
                .await
                .unwrap();
            f.finish(&next, GoalVerdict::Continue, true, 9).await;
            assert_eq!(f.goal().await.status, GoalStatus::Paused);
            assert!(f.goal().await.note.unwrap().contains("100 turns"));
        }
    });
}
