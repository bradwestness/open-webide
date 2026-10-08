//! Background assistance contracts shared across UI and hosts.
/// Keep records usable when optional generation is unavailable.
pub fn fallback_name(content: &str) -> String {
    content
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(80)
        .collect()
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssistanceKind {
    SessionName,
    TaskName,
    MemoryName,
    Recap,
    Activity,
    Completion,
    NextActions,
    Context,
    Commit,
    Branch,
    PullRequest,
    Search,
}

impl AssistanceKind {
    pub fn instruction(self) -> &'static str {
        match self {
            Self::SessionName => "Write a descriptive conversation title, at most 80 characters.",
            Self::TaskName => "Name this task from its prompt, at most 80 characters.",
            Self::MemoryName => "Name this saved memory from its contents, at most 80 characters.",
            Self::Recap => {
                "Summarize where the user left off in at most two sentences. Include completed work, outstanding work and blockers only when evidenced. Do not infer commits, pushes or passing tests without evidence."
            }
            Self::Activity => {
                "Describe the current work in one short present-tense sentence, at most 120 characters."
            }
            Self::Completion => {
                "Summarize the actual result in one short sentence, at most 240 characters. Include failures or blockers when present. Do not invent success."
            }
            Self::NextActions => {
                "Suggest at most two concrete next prompts supported by the conversation. Return one prompt per line, without numbering. Return NONE if there is no useful next action."
            }
            Self::Context => {
                "Select at most three relevant context candidates from the supplied list. Skip candidates already referenced in the draft. Return only their exact identifiers, one per line. Never invent identifiers. Return NONE if none is relevant."
            }
            Self::Commit => {
                "Draft a concise Git commit message from the supplied diff and instructions. Describe only changes present in the diff. Honor supplied repository commit conventions. No attribution trailers."
            }
            Self::Branch => {
                "Suggest one short kebab-case Git branch name for this change, honoring supplied ticket and repository conventions. Return only the branch name."
            }
            Self::PullRequest => {
                "Draft a concise pull request description from the supplied diff and instructions. Describe only changes present in the diff. Honor supplied repository conventions. No attribution or session links."
            }
            Self::Search => {
                "Rewrite the user's search into a few useful search terms for matching previous conversations. Preserve distinctive identifiers and words. Return only the query, at most 160 characters."
            }
        }
    }
    pub fn limit(self) -> usize {
        match self {
            Self::SessionName | Self::TaskName | Self::MemoryName | Self::Branch => 80,
            Self::Activity => 120,
            Self::Search => 160,
            Self::Completion => 240,
            Self::Recap | Self::NextActions | Self::Context => 800,
            Self::Commit | Self::PullRequest => 2000,
        }
    }
    pub fn multiline(self) -> bool {
        matches!(
            self,
            Self::NextActions | Self::Context | Self::Commit | Self::PullRequest
        )
    }
    pub fn normalize(self, text: &str) -> Option<String> {
        let text = crate::strip_reasoning(text)
            .trim()
            .trim_matches(['"', '\''])
            .trim();
        if text.is_empty()
            || text.chars().count() > self.limit()
            || text
                .chars()
                .any(|c| c.is_control() && !(self.multiline() && c == '\n'))
        {
            return None;
        }
        if matches!(self, Self::Branch)
            && (!text
                .chars()
                .all(|c| c.is_ascii_alphabetic() || c.is_ascii_digit() || c == '-')
                || text.starts_with('-')
                || text.ends_with('-'))
        {
            return None;
        }
        Some(text.to_owned())
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct AssistanceRequest {
    pub kind: AssistanceKind,
    pub connection_id: i64,
    pub session_id: Option<i64>,
    pub project_id: Option<i64>,
    pub input: String,
}
impl AssistanceRequest {
    pub fn validate(&self) -> Result<(), String> {
        if self.input.trim().is_empty() || self.input.len() > 48 * 1024 {
            return Err("Background input must contain 1–49152 bytes".into());
        }
        Ok(())
    }
}

/// A bounded model operation used by host adapters for optional work.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct BackgroundCompletion {
    pub request: crate::ChatRequest,
    pub timeout_seconds: u32,
}

/// A truthful excerpt when optional completion generation is unavailable.
pub fn completion_excerpt(content: &str) -> String {
    crate::strip_reasoning(content)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .filter(|character| !character.is_control())
        .take(240)
        .collect()
}
