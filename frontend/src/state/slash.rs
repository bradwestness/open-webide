use leptos::prelude::*;
use openwebide_core::{FileDiff, SessionTelemetry, SlashCommand};

use super::{chat::ChatState, git::GitState, workspace::WorkspaceState};

/// A slash command's state-derived intent. Browser code executes actions that
/// need network or terminal access.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SlashAction {
    Compact,
    Goal(Option<String>),
    Context,
    Notify(String),
    SelectModel(String),
    DefaultModel,
    Clear,
    GitDiff {
        project_id: Option<i64>,
        path: Option<String>,
    },
    OpenPendingDiff(FileDiff),
    RunTests {
        project_id: Option<i64>,
        filter: String,
    },
    Commit {
        project_id: Option<i64>,
        message: String,
    },
    Checkout {
        project_id: Option<i64>,
        branch: String,
    },
    CreateBranch {
        project_id: Option<i64>,
        branch: String,
    },
    ListBranches {
        project_id: Option<i64>,
    },
    Sync {
        project_id: Option<i64>,
    },
    Stop,
}

/// Resolve a parsed slash command from current feature state without running
/// browser, network, terminal, or UI side effects.
pub fn dispatch(
    cmd: SlashCommand,
    chat: &ChatState,
    git: &GitState,
    workspace: &WorkspaceState,
) -> SlashAction {
    let project_id = workspace
        .active_project
        .get_untracked()
        .or_else(|| git.active_project.get_untracked());

    match cmd {
        SlashCommand::Help(query) => {
            let commands = openwebide_core::tui::slash_search(query.as_deref().unwrap_or_default());
            let help = commands
                .iter()
                .map(|info| {
                    format!(
                        "* `{} {}` — {}",
                        info.command, info.arguments, info.description
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            SlashAction::Notify(format!(
                "**Commands**\n\n{}{}",
                if help.is_empty() {
                    "No matching commands."
                } else {
                    &help
                },
                if query.is_none() { KEY_HELP } else { "" }
            ))
        }
        SlashCommand::Compact => SlashAction::Compact,
        SlashCommand::Goal(args) => SlashAction::Goal(args),
        SlashCommand::Model(Some(target)) if target.eq_ignore_ascii_case("default") => {
            SlashAction::DefaultModel
        }
        SlashCommand::Model(Some(target)) => chat.models.with_untracked(|models| {
            match models
                .iter()
                .find(|model| model.name.eq_ignore_ascii_case(&target))
            {
                Some(model) => SlashAction::SelectModel(model.name.clone()),
                None => {
                    SlashAction::Notify(format!("Model `{target}` not found in available models."))
                }
            }
        }),
        SlashCommand::Model(None) => {
            let names = chat.models.with_untracked(|models| {
                models
                    .iter()
                    .map(|model| {
                        format!(
                            "* `{}`{}",
                            model.name,
                            if chat
                                .selected_model
                                .get_untracked()
                                .as_deref()
                                .unwrap_or(&chat.session_telemetry.get_untracked().model)
                                == model.name
                            {
                                " (active)"
                            } else {
                                ""
                            }
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            });
            let current = chat
                .session_telemetry
                .with_untracked(|telemetry| telemetry.model.clone());
            SlashAction::Notify(format!(
                "Current model: `{current}`\n\nAvailable models:\n{names}\n\nUse `/model <name>` to switch or `/model default` to reset."
            ))
        }
        SlashCommand::Clear => SlashAction::Clear,
        SlashCommand::Diff(path) => workspace.pending_edits.with_untracked(|pending| {
            if pending.is_empty() {
                SlashAction::GitDiff { project_id, path }
            } else if let Some(path) = path {
                pending
                    .get(&path)
                    .cloned()
                    .map(SlashAction::OpenPendingDiff)
                    .unwrap_or_else(|| {
                        SlashAction::Notify(format!("No pending diff found for `{path}`."))
                    })
            } else {
                let list = pending
                    .keys()
                    .map(|path| format!("* `{path}`"))
                    .collect::<Vec<_>>()
                    .join("\n");
                SlashAction::Notify(format!(
                    "Pending file edits ({}):\n{list}\n\nUse `/diff <path>` to open in editor.",
                    pending.len()
                ))
            }
        }),
        SlashCommand::Test(filter) => SlashAction::RunTests {
            project_id,
            filter: filter.unwrap_or_default(),
        },
        SlashCommand::Context => SlashAction::Context,
        SlashCommand::Tokens => SlashAction::Notify(
            chat.session_telemetry
                .with_untracked(SessionTelemetry::tokens_report),
        ),
        SlashCommand::Stop => SlashAction::Stop,
        SlashCommand::Commit(message) => {
            let Some(message) = message.filter(|message| !message.trim().is_empty()) else {
                return SlashAction::Notify(
                    "Please provide a commit message: `/commit <message>`".into(),
                );
            };
            SlashAction::Commit {
                project_id,
                message,
            }
        }
        SlashCommand::Checkout(branch) => {
            let Some(branch) = branch.filter(|branch| !branch.trim().is_empty()) else {
                return SlashAction::Notify(
                    "Please specify a branch to checkout: `/checkout <branch>`".into(),
                );
            };
            SlashAction::Checkout { project_id, branch }
        }
        SlashCommand::Branch(branch) => {
            let branch = branch.and_then(|branch| {
                let branch = branch.trim().to_string();
                (!branch.is_empty()).then_some(branch)
            });
            match branch {
                Some(branch) => SlashAction::CreateBranch { project_id, branch },
                None => SlashAction::ListBranches { project_id },
            }
        }
        SlashCommand::Sync => SlashAction::Sync { project_id },
    }
}

const KEY_HELP: &str = "\n\n\
                        **Keybindings:**\n\
                        * `Ctrl+Shift+P` / `Cmd+Shift+P` — Open command palette\n\
                        * `Ctrl+/` / `Cmd+/` — Show keyboard shortcuts\n\
                        * `Cmd+L` / `Ctrl+L` — Capture active editor file & selection into context pill\n\
                        * `Ctrl+Backtick` / `Cmd+Backtick` — Toggle bottom terminal dock\n\
                        * `Ctrl+K` / `Cmd+K` — Cycle focus between chat, editor, file explorer, and terminal\n\
                        * `Up` / `Down` — Readline prompt history navigation\n\
                        * `Shift+Tab` — Cycle approval modes (Manual, Auto-accept edits, Auto, YOLO)\n\
                        * `Alt+Y` / `Alt+N` / `Alt+A` — Approve / deny / auto-accept file edits for this session\n\
                        * `Ctrl+C` / `Cmd+C` — Cancel streaming when no composer text is selected\n\
                        * `Esc` — Detach context when the draft is empty, otherwise cancel streaming";

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use leptos::prelude::Owner;
    use openwebide_core::{FileDiff, ModelInfo, SlashCommand};

    use super::*;

    #[test]
    fn tokens_command_preserves_full_accounting_text() {
        Owner::new().with(|| {
            let chat = ChatState::new();
            chat.session_telemetry.set(SessionTelemetry {
                last_call: None,
                context: None,
            model: "café".into(),
                context_tokens: 12345,
                context_limit: 65536,
                total_prompt_tokens: 12345,
                total_completion_tokens: 42,
                current_speed_tps: Some(12.345),
                context_estimated: true,
                context_limit_estimated: true,
                totals_estimated: true,
                speed_estimated: true,
                tool_calls_count: 7,
            });
            assert_eq!(dispatch(SlashCommand::Tokens, &chat, &GitState::new(), &WorkspaceState::new()),
                SlashAction::Notify("```text\n┌─ Session Token Accounting ────────────────────────────────────┐\n│ Model:             café                                       │\n│ Input Tokens:      ~12345                                     │\n│ Output Tokens:     ~42                                        │\n│ Context:           ~12345 / ~65536 (18.8%) [==········] │\n│ Current Speed:     ~12.3 t/s                                  │\n│ Tool Calls:        7                                          │\n└───────────────────────────────────────────────────────────────┘\n```".into()));
        });
    }

    #[test]
    fn model_command_selects_a_case_insensitive_match() {
        Owner::new().with(|| {
            let chat = ChatState::new();
            chat.models.set(vec![ModelInfo { name: "X".into() }]);

            assert_eq!(
                dispatch(
                    SlashCommand::parse("/model x").unwrap(),
                    &chat,
                    &GitState::new(),
                    &WorkspaceState::new(),
                ),
                SlashAction::SelectModel("X".into())
            );
        });
    }

    #[test]
    fn test_command_dispatches_the_filter_and_project() {
        Owner::new().with(|| {
            let workspace = WorkspaceState::new();
            workspace.active_project.set(Some(12));
            assert_eq!(
                dispatch(
                    SlashCommand::parse("/test parser").unwrap(),
                    &ChatState::new(),
                    &GitState::new(),
                    &workspace,
                ),
                SlashAction::RunTests {
                    project_id: Some(12),
                    filter: "parser".into()
                },
            );
        });
    }

    #[test]
    fn help_command_returns_the_help_notice() {
        Owner::new().with(|| {
            let action = dispatch(
                SlashCommand::parse("/help").unwrap(),
                &ChatState::new(),
                &GitState::new(),
                &WorkspaceState::new(),
            );

            let SlashAction::Notify(help) = action else {
                panic!("Expected help text");
            };
            assert!(help.contains("/compact"));
            assert!(help.contains("/goal"));
            assert!(help.contains("Keybindings"));
        });
    }

    #[test]
    fn diff_command_opens_the_matching_pending_edit() {
        Owner::new().with(|| {
            let chat = ChatState::new();
            let git = GitState::new();
            let workspace = WorkspaceState::new();
            workspace.pending_edits.set(HashMap::from([(
                "p".into(),
                FileDiff {
                    path: "p".into(),
                    old: Some("old".into()),
                    new: "new".into(),
                    old_unavailable: false,
                    backup_path: None,
                },
            )]));

            assert_eq!(
                dispatch(
                    SlashCommand::parse("/diff p").unwrap(),
                    &chat,
                    &git,
                    &workspace,
                ),
                SlashAction::OpenPendingDiff(FileDiff {
                    path: "p".into(),
                    old: Some("old".into()),
                    new: "new".into(),
                    old_unavailable: false,
                    backup_path: None,
                })
            );
        });
    }

    #[test]
    fn every_slash_command_has_a_pure_action() {
        Owner::new().with(|| {
            let chat = ChatState::new();
            let git = GitState::new();
            let workspace = WorkspaceState::new();
            let cases = [
                (SlashCommand::Clear, SlashAction::Clear),
                (
                    SlashCommand::Model(Some("missing".into())),
                    SlashAction::Notify(
                        "Model `missing` not found in available models.".into(),
                    ),
                ),
                (
                    SlashCommand::Model(None),
                    SlashAction::Notify(
                        "Current model: `default`\n\nAvailable models:\n\n\nUse `/model <name>` to switch or `/model default` to reset."
                            .into(),
                    ),
                ),
                (
                    SlashCommand::Diff(Some("src/no.rs".into())),
                    SlashAction::GitDiff {
                        project_id: None,
                        path: Some("src/no.rs".into()),
                    },
                ),
                (
                    SlashCommand::Test(Some("unit".into())),
                    SlashAction::RunTests {
                        project_id: None,
                        filter: "unit".into(),
                    },
                ),
                (
                    SlashCommand::Tokens,
                    SlashAction::Notify(SessionTelemetry::default().tokens_report()),
                ),
                (SlashCommand::Context, SlashAction::Context),
                (SlashCommand::Stop, SlashAction::Stop),
                (
                    SlashCommand::Commit(Some("message".into())),
                    SlashAction::Commit {
                        project_id: None,
                        message: "message".into(),
                    },
                ),
                (
                    SlashCommand::Commit(None),
                    SlashAction::Notify(
                        "Please provide a commit message: `/commit <message>`".into(),
                    ),
                ),
                (
                    SlashCommand::Checkout(Some("main".into())),
                    SlashAction::Checkout {
                        project_id: None,
                        branch: "main".into(),
                    },
                ),
                (
                    SlashCommand::Checkout(None),
                    SlashAction::Notify(
                        "Please specify a branch to checkout: `/checkout <branch>`".into(),
                    ),
                ),
                (
                    SlashCommand::Branch(Some("feature".into())),
                    SlashAction::CreateBranch {
                        project_id: None,
                        branch: "feature".into(),
                    },
                ),
                (
                    SlashCommand::Branch(None),
                    SlashAction::ListBranches { project_id: None },
                ),
                (SlashCommand::Sync, SlashAction::Sync { project_id: None }),
            ];

            for (cmd, expected) in cases {
                assert_eq!(dispatch(cmd, &chat, &git, &workspace), expected);
            }
        });
    }

    #[test]
    fn git_actions_use_the_active_project() {
        Owner::new().with(|| {
            let active_project = RwSignal::new(Some(12));
            let git = GitState::with_active_project(active_project);
            let workspace = WorkspaceState::with_active_project(active_project);

            assert_eq!(
                dispatch(SlashCommand::Sync, &ChatState::new(), &git, &workspace),
                SlashAction::Sync {
                    project_id: Some(12)
                }
            );
        });
    }
}
