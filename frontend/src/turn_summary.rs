//! Per-prompt activity derived once from the shared conversation store.
use crate::conversation::ConversationItem;
use openwebide_core::Role;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TurnSummary {
    tools: BTreeMap<String, Option<(u64, bool)>>,
    pub files: BTreeSet<String>,
    model_time_ms: u64,
    model_timing_unknown: bool,
    seen_messages: BTreeSet<i64>,
}
impl TurnSummary {
    pub fn tool_count(&self) -> usize {
        self.tools.len()
    }
    pub fn recorded_time_ms(&self) -> Option<u64> {
        let mut known = self.model_time_ms > 0;
        let mut total = self.model_time_ms;
        for (elapsed, _) in self.tools.values().flatten() {
            known = true;
            total = total.saturating_add(*elapsed);
        }
        known.then_some(total)
    }
    pub fn timing_incomplete(&self) -> bool {
        self.model_timing_unknown
            || self
                .tools
                .values()
                .any(|timing| timing.is_none_or(|(_, finished)| !finished))
    }
    pub fn label(&self, files: usize) -> String {
        let tools = self.tool_count();
        let mut label = format!(
            "{tools} {} · {files} {} changed",
            if tools == 1 { "tool" } else { "tools" },
            if files == 1 { "file" } else { "files" }
        );
        if let Some(time) = self.recorded_time_ms() {
            #[allow(
                clippy::cast_precision_loss,
                reason = "Human-scale durations use display precision"
            )]
            let seconds = time as f64 / 1000.0;
            label.push_str(&format!(
                " · {}{seconds:.1}s",
                if self.timing_incomplete() { "≥" } else { "" }
            ));
        }
        label
    }
}

#[derive(Default)]
pub struct TurnSummaries {
    current: Option<i64>,
    summaries: BTreeMap<i64, TurnSummary>,
}
impl TurnSummaries {
    pub fn observe(&mut self, item: &ConversationItem) {
        if let ConversationItem::Message(message) = item
            && message.role == Role::User
            && message.id > 0
        {
            self.current = Some(message.id);
            self.summaries.entry(message.id).or_default();
            return;
        }
        let Some(summary) = self.current.and_then(|id| self.summaries.get_mut(&id)) else {
            return;
        };
        match item {
            ConversationItem::Message(message)
                if message.role == Role::Assistant
                    && message.id > 0
                    && summary.seen_messages.insert(message.id) =>
            {
                if let Some(usage) = message.usage.filter(|usage| usage.eval_duration_ms > 0) {
                    summary.model_time_ms =
                        summary.model_time_ms.saturating_add(usage.eval_duration_ms);
                } else {
                    summary.model_timing_unknown = true;
                }
            }
            ConversationItem::ToolStep {
                id,
                timing,
                result,
                awaiting_permission,
                ..
            } if !awaiting_permission || result.is_some() => {
                // A completed denial remains a tool step, while a pending
                // permission request does not count as a started step.
                summary.tools.insert(
                    id.clone(),
                    timing.map(|timing| (timing.elapsed_ms, timing.finished)),
                );
                if let Some(diff) = result.as_ref().and_then(|result| result.diff.as_ref()) {
                    summary.files.insert(diff.path.clone());
                }
            }
            _ => {}
        }
    }
    pub fn finish(self) -> BTreeMap<i64, TurnSummary> {
        self.summaries
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversation::{ToolStepResult, local_message, next_item_nonce};
    use openwebide_core::{FileDiff, ToolTiming, TurnTelemetry};
    fn message(id: i64, role: Role, duration: u64) -> ConversationItem {
        let ConversationItem::Message(mut message) = local_message(1, "text") else {
            unreachable!()
        };
        message.id = id;
        message.role = role;
        message.usage = (duration > 0).then_some(TurnTelemetry {
            eval_duration_ms: duration,
            ..Default::default()
        });
        ConversationItem::Message(message)
    }
    fn step(
        id: &str,
        pending: bool,
        timing: Option<ToolTiming>,
        path: Option<&str>,
    ) -> ConversationItem {
        ConversationItem::ToolStep {
            timing,
            key: next_item_nonce(),
            id: id.into(),
            name: "tool".into(),
            summary: "tool".into(),
            result: (!pending).then_some(ToolStepResult {
                ok: true,
                summary: "done".into(),
                diff: path.map(|path| FileDiff {
                    path: path.into(),
                    old: None,
                    new: "new".into(),
                    old_unavailable: false,
                    backup_path: None,
                }),
            }),
            awaiting_permission: pending,
            diff: None,
            note: None,
        }
    }
    #[test]
    fn summaries_group_all_model_turns_deduplicate_steps_and_ignore_pending_permissions() {
        let mut summaries = TurnSummaries::default();
        for item in [
            message(1, Role::User, 0),
            message(2, Role::Assistant, 700),
            step(
                "one",
                false,
                Some(ToolTiming::start(1000).sample(2200, true)),
                Some("a"),
            ),
            step(
                "one",
                false,
                Some(ToolTiming::start(1000).sample(2200, true)),
                Some("a"),
            ),
            step("pending", true, None, Some("unwritten")),
            message(3, Role::Assistant, 800),
            message(4, Role::User, 0),
            step("two", false, None, None),
        ] {
            summaries.observe(&item);
        }
        let summaries = summaries.finish();
        assert_eq!(
            summaries[&1].label(summaries[&1].files.len()),
            "1 tool · 1 file changed · 2.7s"
        );
        assert_eq!(summaries[&4].label(0), "1 tool · 0 files changed");
        assert!(summaries[&4].timing_incomplete());
    }
    #[test]
    fn unknown_or_live_timing_is_a_lower_bound_not_a_fabricated_total() {
        let mut summaries = TurnSummaries::default();
        for item in [
            message(1, Role::User, 0),
            message(2, Role::Assistant, 0),
            step(
                "one",
                false,
                Some(ToolTiming::start(1000).sample(4200, false)),
                None,
            ),
        ] {
            summaries.observe(&item);
        }
        let summaries = summaries.finish();
        assert_eq!(summaries[&1].label(0), "1 tool · 0 files changed · ≥3.2s");
    }
}
