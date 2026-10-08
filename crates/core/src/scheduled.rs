//! Saved prompts and scheduling policy, shared by every execution host.
use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};
use std::str::FromStr;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Schedule {
    Once {
        at: i64,
    },
    Cron {
        expression: String,
        timezone: String,
    },
}
impl Schedule {
    pub fn next_after(&self, after: i64) -> Result<Option<i64>, String> {
        match self {
            Self::Once { at } => Ok((*at > after).then_some(*at)),
            Self::Cron {
                expression,
                timezone,
            } => {
                if expression.len() > 128 || expression.split_whitespace().count() != 5 {
                    return Err(
                        "Use a five-field cron expression (minute hour day month weekday).".into(),
                    );
                }
                let zone = Tz::from_str(timezone).map_err(|_| "Choose a valid timezone.")?;
                let start = DateTime::<Utc>::from_timestamp(after, 0)
                    .ok_or("Invalid schedule timestamp.")?
                    .with_timezone(&zone);
                let cron = croner::Cron::from_str(expression)
                    .map_err(|error| format!("Invalid schedule: {error}"))?;
                let next = cron
                    .find_next_occurrence(&start, false)
                    .map_err(|error| format!("Schedule has no next occurrence: {error}"))?;
                Ok(Some(next.timestamp()))
            }
        }
    }
    pub fn validate(&self, now: i64) -> Result<(), String> {
        if self.next_after(now)?.is_none() {
            return Err("Choose a future date and time.".into());
        }
        Ok(())
    }
}
/// Session policy is resolved when an occurrence becomes due, not when saved.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionTarget {
    #[default]
    Existing,
    New,
    Latest,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskDraft {
    #[serde(default)]
    pub session_target: SessionTarget,
    #[serde(default)]
    pub auto_title: bool,
    #[serde(default)]
    pub title: String,
    pub prompt: String,
    #[serde(default)]
    pub session_id: i64,
    pub schedule: Schedule,
    pub enabled: bool,
}
impl TaskDraft {
    pub fn validate(&self, now: i64) -> Result<(), String> {
        if (!self.auto_title && self.title.trim().is_empty()) || self.title.chars().count() > 120 {
            return Err("Task titles need 1–120 characters.".into());
        }
        if self.prompt.trim().is_empty() || self.prompt.len() > 32 * 1024 {
            return Err("Task prompts need 1–32 KiB of text.".into());
        }
        if self.session_target == SessionTarget::Existing && self.session_id <= 0 {
            return Err("Choose a session for this task.".into());
        }
        self.schedule.validate(now)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum TaskCommand {
    List,
    Create {
        draft: TaskDraft,
    },
    Update {
        id: i64,
        revision: i64,
        draft: TaskDraft,
    },
    SetEnabled {
        id: i64,
        revision: i64,
        enabled: bool,
    },
    Delete {
        id: i64,
        revision: i64,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionHost {
    pub id: String,
    pub name: String,
    pub last_seen: i64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostBinding {
    pub host_id: String,
    pub path: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScheduledTask {
    pub id: i64,
    pub revision: i64,
    pub project_id: Option<i64>,
    pub draft: TaskDraft,
    pub next_run: Option<i64>,
    pub host_id: String,
    pub host_available: bool,
    pub last_run: Option<TaskRun>,
}
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TaskRun {
    #[serde(default)]
    pub session_id: Option<i64>,
    pub id: i64,
    pub task_id: i64,
    pub due_at: i64,
    pub status: String,
    pub detail: String,
    pub message_id: Option<i64>,
    pub permission_id: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaskDelivery {
    pub run_id: i64,
    pub task_id: i64,
    pub user_id: i64,
    pub session_id: i64,
    pub prompt: crate::QueuedPrompt,
    pub binding: Option<HostBinding>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DispatchResult {
    #[serde(default)]
    pub permission_id: Option<String>,
    pub run_id: i64,
    pub status: String,
    pub detail: String,
}
pub fn calendar_cron(expression: &str) -> Option<(String, Vec<u8>)> {
    let fields = expression.split_whitespace().collect::<Vec<_>>();
    if fields.len() != 5 || fields[2] != "*" || fields[3] != "*" {
        return None;
    }
    let minute = fields[0].parse::<u8>().ok()?;
    let hour = fields[1].parse::<u8>().ok()?;
    if minute > 59 || hour > 23 {
        return None;
    }
    let days = if fields[4] == "*" {
        (0..7).collect()
    } else {
        fields[4]
            .split(',')
            .map(str::parse::<u8>)
            .collect::<Result<Vec<_>, _>>()
            .ok()?
    };
    if days.is_empty() || days.iter().any(|day| *day > 6) {
        return None;
    }
    Some((format!("{hour:02}:{minute:02}"), days))
}
pub fn weekly_cron(time: &str, weekdays: &[u8]) -> Result<String, String> {
    let (hour, minute) = time.split_once(':').ok_or("Choose a time.")?;
    let hour: u8 = hour.parse().map_err(|_| "Choose a time.")?;
    let minute: u8 = minute.parse().map_err(|_| "Choose a time.")?;
    if hour > 23 || minute > 59 || weekdays.is_empty() || weekdays.iter().any(|day| *day > 6) {
        return Err("Choose a time and at least one weekday.".into());
    }
    let mut days = weekdays.to_vec();
    days.sort_unstable();
    days.dedup();
    Ok(format!(
        "{minute} {hour} * * {}",
        days.iter().map(u8::to_string).collect::<Vec<_>>().join(",")
    ))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn calendar_choices_and_cron_keep_wall_time_across_dst() {
        assert_eq!(weekly_cron("09:30", &[5, 1, 1]).unwrap(), "30 9 * * 1,5");
        assert!(weekly_cron("24:00", &[1]).is_err());
        let schedule = Schedule::Cron {
            expression: "0 9 * * *".into(),
            timezone: "America/Chicago".into(),
        };
        let before = DateTime::parse_from_rfc3339("2026-03-07T16:00:00Z")
            .unwrap()
            .timestamp();
        assert_eq!(
            schedule.next_after(before).unwrap(),
            Some(
                DateTime::parse_from_rfc3339("2026-03-08T14:00:00Z")
                    .unwrap()
                    .timestamp()
            )
        );
        assert!(
            Schedule::Cron {
                expression: "* * * * * *".into(),
                timezone: "UTC".into()
            }
            .validate(before)
            .is_err()
        );
        assert_eq!(Schedule::Once { at: 100 }.next_after(100).unwrap(), None);
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RunControl {
    pub cancelled: bool,
    pub approved: Option<bool>,
}
