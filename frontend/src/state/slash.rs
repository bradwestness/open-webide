use leptos::prelude::*;
use openwebide_core::{FileDiff, SessionTelemetry, SlashCommand};

use super::{chat::ChatState, git::GitState, workspace::WorkspaceState};

/// A slash command's state-derived intent. Browser code executes actions that
/// need network or terminal access.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SlashAction {
    Notify(String),
    SelectModel(String),
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
        SlashCommand::Help => SlashAction::Notify(HELP_TEXT.into()),
        SlashCommand::Model(Some(target)) => {
            let models = chat.models.get_untracked();
            match models
                .iter()
                .find(|model| model.name.eq_ignore_ascii_case(&target))
            {
                Some(model) => SlashAction::SelectModel(model.name.clone()),
                None => {
                    SlashAction::Notify(format!("Model `{target}` not found in available models."))
                }
            }
        }
        SlashCommand::Model(None) => {
            let names = chat
                .models
                .get_untracked()
                .into_iter()
                .map(|model| format!("* `{}`", model.name))
                .collect::<Vec<_>>()
                .join("\n");
            let current = chat.session_telemetry.get_untracked().model;
            SlashAction::Notify(format!(
                "Current model: `{current}`\n\nAvailable models:\n{names}\n\nUse `/model <name>` to switch."
            ))
        }
        SlashCommand::Clear => SlashAction::Clear,
        SlashCommand::Diff(path) => {
            let pending = workspace.pending_edits.get_untracked();
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
        }
        SlashCommand::Test(filter) => SlashAction::RunTests {
            project_id,
            filter: filter.unwrap_or_default(),
        },
        SlashCommand::Tokens => {
            SlashAction::Notify(format_tokens(&chat.session_telemetry.get_untracked()))
        }
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

const HELP_TEXT: &str = "**Open WebIDE Terminal Execution & Slash Commands**\n\n\
                        **Commands:**\n\
                        * `/help` — Show this cheat sheet\n\
                        * `/model [name]` — Switch model or list available models\n\
                        * `/clear` — Clear the active chat stream\n\
                        * `/diff [path]` — View uncommitted Git diff or pending edits\n\
                        * `/commit <message>` — Stage and commit changes to host Git\n\
                        * `/checkout <branch>` — Switch active Git branch\n\
                        * `/branch [name]` — List branches, or create and switch to a new branch\n\
                        * `/sync` — Synchronize upstream commits (pull & push)\n\
                        * `/test [filter]` — run cargo test in the terminal\n\
                        * `/tokens` or `/context` — Show session token accounting\n\
                        * `/stop` — Abort active execution\n\n\
                        **Keybindings:**\n\
                        * `Cmd+L` / `Ctrl+L` — Capture active editor file & selection into context pill\n\
                        * `Ctrl+Backtick` / `Cmd+Backtick` — Toggle bottom terminal dock\n\
                        * `Ctrl+K` / `Cmd+K` — Cycle focus between chat, editor, file explorer, and terminal\n\
                        * `Up` / `Down` — Readline prompt history navigation\n\
                        * `Alt+Y` / `Alt+N` / `Alt+A` — Approve / deny / always for this session (always excludes shell commands)\n\
                        * `Ctrl+C` / `Cmd+C` — Cancel streaming when no composer text is selected\n\
                        * `Esc` — Detach context when the draft is empty, otherwise cancel streaming";

fn format_tokens(telemetry: &SessionTelemetry) -> String {
    let pct = telemetry.context_percent();
    let bar = telemetry.gauge_bar();
    let totals_approx = SessionTelemetry::approx(telemetry.totals_estimated);
    let context_approx = SessionTelemetry::approx(telemetry.context_estimated);
    let limit_approx = SessionTelemetry::approx(telemetry.context_limit_estimated);
    let speed_approx = SessionTelemetry::approx(telemetry.speed_estimated);
    let speed = telemetry
        .current_speed_tps
        .map(|speed| format!("{speed_approx}{speed:.1} t/s"))
        .unwrap_or_else(|| "-- t/s".into());
    let input_tokens = format!("{totals_approx}{}", telemetry.total_prompt_tokens);
    let output_tokens = format!("{totals_approx}{}", telemetry.total_completion_tokens);
    let context_tokens = format!("{context_approx}{}", telemetry.context_tokens);
    let context_limit = format!("{limit_approx}{}", telemetry.context_limit);

    format!(
        "```text\n\
         ┌─ Session Token Accounting ────────────────────────────────────┐\n\
         │ Model:             {:<42} │\n\
         │ Input Tokens:      {:<42} │\n\
         │ Output Tokens:     {:<42} │\n\
         │ Context:           {} / {} ({:.1}%) {} │\n\
         │ Current Speed:     {:<42} │\n\
         │ Tool Calls:        {:<42} │\n\
         └───────────────────────────────────────────────────────────────┘\n```",
        telemetry.model,
        input_tokens,
        output_tokens,
        context_tokens,
        context_limit,
        pct,
        bar,
        speed,
        telemetry.tool_calls_count,
    )
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use leptos::prelude::Owner;
    use openwebide_core::{FileDiff, ModelInfo, SlashCommand};

    use super::*;

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

            assert_eq!(action, SlashAction::Notify(HELP_TEXT.into()));
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
                        "Current model: `default`\n\nAvailable models:\n\n\nUse `/model <name>` to switch."
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
                    SlashAction::Notify(format_tokens(&SessionTelemetry::default())),
                ),
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
