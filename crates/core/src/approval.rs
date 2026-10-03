//! Session tool-approval choices and transport-neutral requests.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalMode {
    #[default]
    Default,
    AutoAcceptEdits,
    Auto,
    Yolo,
    /// Compatibility with the earlier per-session Always shortcut.
    AlwaysForSession,
}
impl ApprovalMode {
    pub const CHOICES: [Self; 4] = [Self::Default, Self::AutoAcceptEdits, Self::Auto, Self::Yolo];
    pub fn label(self) -> &'static str {
        match self {
            Self::Default => "Default",
            Self::AutoAcceptEdits => "Auto-accept edits",
            Self::Auto => "Auto",
            Self::Yolo => "YOLO",
            Self::AlwaysForSession => "Always",
        }
    }
    pub fn next(self) -> Self {
        match self {
            Self::Default | Self::AlwaysForSession => Self::AutoAcceptEdits,
            Self::AutoAcceptEdits => Self::Auto,
            Self::Auto => Self::Yolo,
            Self::Yolo => Self::Default,
        }
    }
    pub fn auto_approves(self, tool: &str) -> bool {
        match self {
            Self::Default | Self::Auto => false,
            Self::AutoAcceptEdits => tool == "write_file",
            Self::Yolo => true,
            Self::AlwaysForSession => tool != "run_command",
        }
    }
    pub fn setting_key(session: i64) -> String {
        format!("approval_mode_{session}")
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ApprovalCheck {
    pub connection_id: i64,
    pub model: Option<String>,
    pub call: crate::ToolCall,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ApprovalDecision {
    pub approved: bool,
}
