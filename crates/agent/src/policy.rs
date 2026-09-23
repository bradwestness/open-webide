//! Tool approval policy.
//!
//! Tools are evaluated against an allow-list of known safe, read-only tools.
//! This is an allow-list on purpose: any new or unclassified tool is gated
//! by default until someone decides otherwise (default-deny policy).

use openwebide_core::ToolCall;

/// Tools that are safe to run automatically without explicit user approval.
///
/// Every other tool (destructive operations, external network calls like
/// `fetch_web_page`, shell execution, git mutations) requires user approval.
pub const AUTO_APPROVED: &[&str] = &[
    "read_file",
    "list_dir",
    "search",
    "grep_search",
    "git_status",
    "git_diff",
    "search_web",
];

/// Returns `true` if the tool call requires explicit user approval before execution.
///
/// Uses an allow-list ([`AUTO_APPROVED`]); any tool not explicitly listed
/// returns `true`.
pub fn requires_approval(call: &ToolCall) -> bool {
    !AUTO_APPROVED.contains(&call.name.as_str())
}

pub const NEVER_ALWAYS_APPROVED: &[&str] = &["run_command"];

pub fn always_approvable(name: &str) -> bool {
    !NEVER_ALWAYS_APPROVED.contains(&name)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ApprovalMode {
    #[default]
    Default,
    AlwaysForSession,
}

impl ApprovalMode {
    pub fn auto_approves(self, tool: &str) -> bool {
        match self {
            ApprovalMode::Default => false,
            ApprovalMode::AlwaysForSession => always_approvable(tool),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vfs_tools;

    #[test]
    fn every_builtin_tool_is_classified() {
        const GATED: &[&str] = &[
            "write_file",
            "run_command",
            "git_commit",
            "git_branch",
            "fetch_web_page",
        ];

        let tools = vfs_tools();
        for tool in &tools {
            let name = tool.name.as_str();
            let is_auto = AUTO_APPROVED.contains(&name);
            let is_gated = GATED.contains(&name);
            assert!(
                is_auto ^ is_gated,
                "Tool '{name}' must be either in AUTO_APPROVED or GATED (is_auto: {is_auto}, is_gated: {is_gated})"
            );
        }
    }

    #[test]
    fn unknown_tool_requires_approval() {
        let unknown = ToolCall {
            id: "call-1".into(),
            name: "unknown_tool".into(),
            arguments: "{}".into(),
        };
        assert!(requires_approval(&unknown));

        for name in AUTO_APPROVED {
            let call = ToolCall {
                id: "call-auto".into(),
                name: (*name).into(),
                arguments: "{}".into(),
            };
            assert!(
                !requires_approval(&call),
                "expected {name} to be auto-approved"
            );
        }

        let gated_tools = [
            "write_file",
            "run_command",
            "git_commit",
            "git_branch",
            "fetch_web_page",
        ];
        for name in gated_tools {
            let call = ToolCall {
                id: "call-gated".into(),
                name: name.into(),
                arguments: "{}".into(),
            };
            assert!(
                requires_approval(&call),
                "expected {name} to require approval"
            );
        }
    }

    #[test]
    fn test_always_approvable() {
        assert!(always_approvable("write_file"));
        assert!(!always_approvable("run_command"));
    }

    #[test]
    fn test_auto_approves() {
        assert!(!ApprovalMode::Default.auto_approves("write_file"));
        assert!(!ApprovalMode::Default.auto_approves("run_command"));
        assert!(ApprovalMode::AlwaysForSession.auto_approves("write_file"));
        assert!(!ApprovalMode::AlwaysForSession.auto_approves("run_command"));
    }
}
