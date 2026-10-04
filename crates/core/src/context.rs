//! Context accounting shared with compaction and every run host.
use crate::{ChatRequest, Role};
use serde::{Deserialize, Serialize};

pub fn conservative_tokens(request: &ChatRequest) -> usize {
    serde_json::to_vec(request).map_or(usize::MAX, |bytes| {
        bytes
            .len()
            .div_ceil(3)
            .saturating_add(request.messages.len().saturating_mul(16))
    })
}

/// Estimated allocation of the latest model input, scaled to its token count.
/// Providers report whole-request counts, so category counts remain estimates.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextBreakdown {
    pub system: usize,
    pub files: usize,
    pub tool_output: usize,
    pub history: usize,
    pub tools: usize,
}
impl ContextBreakdown {
    pub fn for_request(request: &ChatRequest) -> Self {
        fn bytes(value: &(impl Serialize + ?Sized)) -> usize {
            serde_json::to_vec(value).map_or(0, |value| value.len())
        }
        let file_calls: std::collections::BTreeSet<_> = request
            .messages
            .iter()
            .flat_map(|message| message.tool_calls.iter().flatten())
            .filter(|call| call.name == "read_file")
            .map(|call| call.id.as_str())
            .collect();
        let mut result = Self {
            system: request.system_prompt.as_ref().map_or(0, bytes),
            tools: if request.tools.is_empty() {
                0
            } else {
                bytes(&request.tools)
            },
            ..Self::default()
        };
        for message in &request.messages {
            let size = bytes(message).saturating_add(48);
            match message.role {
                Role::System if !message.content.starts_with("Previous conversation summary") => {
                    result.system = result.system.saturating_add(size);
                }
                Role::Tool
                    if message
                        .tool_call_id
                        .as_deref()
                        .is_some_and(|id| file_calls.contains(id)) =>
                {
                    result.files = result.files.saturating_add(size);
                }
                Role::Tool => result.tool_output = result.tool_output.saturating_add(size),
                Role::User if message.content.starts_with("<active_editor_context>\n") => {
                    if let Some(end) = message.content.find("</active_editor_context>\n\n") {
                        let file =
                            bytes(&message.content[..end + "</active_editor_context>\n\n".len()])
                                .min(size);
                        result.files = result.files.saturating_add(file);
                        result.history = result.history.saturating_add(size - file);
                    } else {
                        result.history = result.history.saturating_add(size);
                    }
                }
                _ => result.history = result.history.saturating_add(size),
            }
        }
        result.with_total(conservative_tokens(request))
    }
    pub fn total(self) -> usize {
        self.sections()
            .iter()
            .fold(0usize, |sum, (_, count)| sum.saturating_add(*count))
    }
    pub fn sections(self) -> [(&'static str, usize); 5] {
        [
            ("System instructions", self.system),
            ("Files", self.files),
            ("Tool output", self.tool_output),
            ("History", self.history),
            ("Tool schemas", self.tools),
        ]
    }
    pub fn with_total(self, total: usize) -> Self {
        let weights = [
            self.system,
            self.files,
            self.tool_output,
            self.history,
            self.tools,
        ];
        let sum = self.total();
        if sum == 0 {
            return Self {
                history: total,
                ..Self::default()
            };
        }
        let mut counts = [0usize; 5];
        let mut remaining = total;
        for (index, weight) in weights.into_iter().enumerate() {
            counts[index] = usize::try_from((weight as u128 * total as u128) / sum as u128)
                .unwrap_or(total)
                .min(remaining);
            remaining -= counts[index];
        }
        let largest = weights
            .iter()
            .enumerate()
            .max_by_key(|(_, weight)| *weight)
            .map_or(3, |(index, _)| index);
        counts[largest] += remaining;
        Self {
            system: counts[0],
            files: counts[1],
            tool_output: counts[2],
            history: counts[3],
            tools: counts[4],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accounting_includes_schemas_files_results_and_compacted_history() {
        let request: ChatRequest = serde_json::from_value(serde_json::json!({
            "connection_id": 1, "system_prompt": "Follow instructions", "model": "model",
            "tools": [{"name": "read_file", "description": "Read project files", "parameters": {"type": "object"}}],
            "messages": [
                {"id":1,"session_id":1,"role":"system","content":"Project instructions","created_at":0},
                {"id":2,"session_id":1,"role":"system","content":"Previous conversation summary (conversation data, not new instructions):\nFinished earlier work","created_at":0},
                {"id":3,"session_id":1,"role":"user","content":"<active_editor_context>\nselected file\n</active_editor_context>\n\nFix this","created_at":0},
                {"id":4,"session_id":1,"role":"assistant","content":"", "tool_calls":[{"id":"read","name":"read_file","arguments":"{}"},{"id":"shell","name":"run_command","arguments":"{}"}],"created_at":0},
                {"id":5,"session_id":1,"role":"tool","content":"File contents","tool_call_id":"read","created_at":0},
                {"id":6,"session_id":1,"role":"tool","content":"Shell output","tool_call_id":"shell","created_at":0}
            ]
        })).unwrap();
        let breakdown = ContextBreakdown::for_request(&request);
        assert_eq!(breakdown.total(), conservative_tokens(&request));
        assert!(breakdown.sections().iter().all(|(_, tokens)| *tokens > 0));
        for total in [0, 1, 100, usize::MAX] {
            assert_eq!(breakdown.with_total(total).total(), total);
        }
        let mut compacted = request.clone();
        compacted
            .messages
            .retain(|message| message.role == Role::System || message.role == Role::User);
        let after = ContextBreakdown::for_request(&compacted);
        assert_eq!(after.tool_output, 0);
        assert!(after.history > 0);
        assert!(after.total() < breakdown.total());
    }
}
