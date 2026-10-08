//! Session objectives stay open until the user confirms completion.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalStatus {
    Active,
    Paused,
    Completed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Goal {
    pub session_id: i64,
    pub objective: String,
    pub status: GoalStatus,
    pub revision: u64,
    pub updated_at: i64,
    /// Absent on older saved goals; never invent an elapsed duration for them.
    #[serde(default)]
    pub started_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum GoalCommand {
    Start { objective: String },
    Pause,
    Resume,
    Complete,
}

impl GoalCommand {
    pub fn parse(args: &str) -> Option<Self> {
        match args.trim() {
            "" | "status" => None,
            "pause" | "stop" => Some(Self::Pause),
            "resume" => Some(Self::Resume),
            "complete" => Some(Self::Complete),
            text => Some(Self::Start {
                objective: text.strip_prefix("start ").unwrap_or(text).trim().into(),
            }),
        }
    }
}
impl Goal {
    pub fn transition(
        existing: Option<&Self>,
        session: i64,
        command: GoalCommand,
        now: i64,
    ) -> Result<Self, String> {
        let revision = existing
            .map_or(0, |goal| goal.revision)
            .checked_add(1)
            .ok_or("Goal revision exhausted")?;
        let mut goal = match command {
            GoalCommand::Start { objective } => {
                if existing.is_some_and(|goal| goal.status != GoalStatus::Completed) {
                    return Err("Complete the current goal before starting another one.".into());
                }
                let objective = objective.trim().to_string();
                if objective.is_empty() || objective.len() > 16_000 {
                    return Err("A goal needs an objective of 1–16000 bytes.".into());
                }
                Self {
                    session_id: session,
                    objective,
                    status: GoalStatus::Active,
                    revision,
                    updated_at: now,
                    started_at: Some(now),
                }
            }
            control => {
                let mut goal = existing
                    .cloned()
                    .ok_or("Start a goal with /goal <objective> first.")?;
                if goal.session_id != session {
                    return Err("The goal belongs to another session.".into());
                }
                if goal.status == GoalStatus::Completed {
                    return Err("This goal is already completed. Start a new goal.".into());
                }
                goal.status = match control {
                    GoalCommand::Pause => GoalStatus::Paused,
                    GoalCommand::Resume => GoalStatus::Active,
                    GoalCommand::Complete => GoalStatus::Completed,
                    GoalCommand::Start { .. } => unreachable!(),
                };
                goal
            }
        };
        goal.revision = revision;
        goal.updated_at = now;
        Ok(goal)
    }
    pub fn completed_duration_seconds(&self) -> Option<u64> {
        if self.status != GoalStatus::Completed {
            return None;
        }
        self.started_at
            .and_then(|start| self.updated_at.checked_sub(start))
            .and_then(|elapsed| u64::try_from(elapsed).ok())
    }
    pub fn prompt(&self) -> String {
        format!(
            "Continue working toward this goal using the saved conversation and current workspace:\n\n{}\n\nUse the plan tool to track progress when available. Verify your work and report what is complete, what remains, and any blocker. Do not claim success without evidence.",
            self.objective
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lifecycle_is_explicit_and_never_infers_completion_from_a_reply() {
        let goal = Goal::transition(
            None,
            1,
            GoalCommand::Start {
                objective: "  Fix the test  ".into(),
            },
            10,
        )
        .unwrap();
        assert_eq!(goal.objective, "Fix the test");
        assert_eq!(goal.status, GoalStatus::Active);
        assert!(
            Goal::transition(
                Some(&goal),
                1,
                GoalCommand::Start {
                    objective: "replace".into()
                },
                11
            )
            .is_err()
        );
        assert!(Goal::transition(Some(&goal), 2, GoalCommand::Resume, 11).is_err());
        let paused = Goal::transition(Some(&goal), 1, GoalCommand::Pause, 11).unwrap();
        let resumed = Goal::transition(Some(&paused), 1, GoalCommand::Resume, 12).unwrap();
        assert_eq!(resumed.objective, goal.objective);
        assert!(resumed.prompt().contains("Verify your work"));
        let completed = Goal::transition(Some(&resumed), 1, GoalCommand::Complete, 13).unwrap();
        assert_eq!(completed.status, GoalStatus::Completed);
        assert_eq!(completed.started_at, Some(10));
        assert_eq!(completed.completed_duration_seconds(), Some(3));
        assert!(Goal::transition(Some(&completed), 1, GoalCommand::Resume, 14).is_err());
        assert!(
            Goal::transition(
                None,
                1,
                GoalCommand::Start {
                    objective: " ".into()
                },
                14
            )
            .is_err()
        );
        assert!(
            Goal::transition(
                None,
                1,
                GoalCommand::Start {
                    objective: "x".repeat(16_001)
                },
                14
            )
            .is_err()
        );
    }
    #[test]
    fn legacy_goals_preserve_unknown_duration_and_restart_the_clock_for_new_objectives() {
        let legacy: Goal = serde_json::from_str(r#"{"session_id":1,"objective":"Legacy","status":"paused","revision":1,"updated_at":10}"#).unwrap();
        assert_eq!(legacy.started_at, None);
        let completed = Goal::transition(Some(&legacy), 1, GoalCommand::Complete, 100).unwrap();
        assert_eq!(completed.completed_duration_seconds(), None);
        let started = Goal::transition(
            Some(&completed),
            1,
            GoalCommand::Start {
                objective: "New".into(),
            },
            200,
        )
        .unwrap();
        assert_eq!(started.started_at, Some(200));
        assert_eq!(started.completed_duration_seconds(), None);
        let reversed = Goal::transition(Some(&started), 1, GoalCommand::Complete, 199).unwrap();
        assert_eq!(reversed.completed_duration_seconds(), None);
    }
}
