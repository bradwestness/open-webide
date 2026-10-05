//! Shared command catalog, search and capability rules for the palette and keys.
use crate::state::layout::Panel;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Palette,
    Shortcuts,
    NewSession,
    OpenLocal,
    OpenRemote,
    Settings,
    ModelSetup,
    TogglePanel(Panel),
    ToggleTerminal,
    FocusChat,
    CycleFocus,
    CaptureEditor,
    Context,
    Stop,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CommandContext {
    pub project: bool,
    pub editor: bool,
    pub running: bool,
}
impl Command {
    pub const fn unavailable(self, context: CommandContext) -> Option<&'static str> {
        match self {
            Self::TogglePanel(panel) if panel.requires_project() && !context.project => {
                Some("Open a project first")
            }
            Self::ToggleTerminal if !context.project => Some("Open a project first"),
            Self::CaptureEditor if !context.project || !context.editor => {
                Some("Open a file in the editor first")
            }
            Self::Stop if !context.running => Some("No run is active"),
            _ => None,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CommandDefinition {
    pub id: &'static str,
    pub command: Command,
    pub label: &'static str,
    pub keywords: &'static str,
    pub shortcut: &'static str,
}
macro_rules! command {
    ($id:literal, $command:expr, $label:literal, $keywords:literal, $shortcut:literal) => {
        CommandDefinition {
            id: $id,
            command: $command,
            label: $label,
            keywords: $keywords,
            shortcut: $shortcut,
        }
    };
}
pub const COMMANDS: &[CommandDefinition] = &[
    command!(
        "new-session",
        Command::NewSession,
        "New chat session",
        "conversation agent",
        ""
    ),
    command!(
        "open-local",
        Command::OpenLocal,
        "Open local project",
        "folder browser files",
        ""
    ),
    command!(
        "open-remote",
        Command::OpenRemote,
        "Open remote project",
        "folder server files",
        ""
    ),
    command!(
        "settings",
        Command::Settings,
        "Open settings",
        "preferences theme configuration",
        ""
    ),
    command!(
        "models",
        Command::ModelSetup,
        "Model setup",
        "provider server discovery configuration",
        ""
    ),
    command!(
        "sessions",
        Command::TogglePanel(Panel::Sessions),
        "Toggle Sessions panel",
        "sidebar collapse expand",
        ""
    ),
    command!(
        "files",
        Command::TogglePanel(Panel::Files),
        "Toggle Files panel",
        "tree explorer collapse expand",
        ""
    ),
    command!(
        "editor",
        Command::TogglePanel(Panel::Editor),
        "Toggle Editor panel",
        "code collapse expand",
        ""
    ),
    command!(
        "chat",
        Command::TogglePanel(Panel::Chat),
        "Toggle Chat panel",
        "conversation collapse expand",
        ""
    ),
    command!(
        "git",
        Command::TogglePanel(Panel::Git),
        "Toggle Git changes",
        "diff status changes",
        ""
    ),
    command!(
        "search",
        Command::TogglePanel(Panel::Search),
        "Toggle Search",
        "find files contents",
        ""
    ),
    command!(
        "terminal",
        Command::ToggleTerminal,
        "Toggle terminal",
        "shell console dock",
        "Ctrl/⌘+`"
    ),
    command!(
        "focus-chat",
        Command::FocusChat,
        "Focus chat composer",
        "prompt input",
        ""
    ),
    command!(
        "focus-next",
        Command::CycleFocus,
        "Focus next panel",
        "cycle chat editor files terminal",
        "Ctrl/⌘+K"
    ),
    command!(
        "capture-editor",
        Command::CaptureEditor,
        "Attach editor context",
        "selection file prompt",
        "Ctrl/⌘+L"
    ),
    command!(
        "context",
        Command::Context,
        "Show context usage",
        "tokens model input breakdown",
        "/context"
    ),
    command!(
        "stop",
        Command::Stop,
        "Stop active run",
        "cancel interrupt agent",
        "/stop"
    ),
    command!(
        "shortcuts",
        Command::Shortcuts,
        "Keyboard shortcuts",
        "help keybindings keys",
        "Ctrl/⌘+/"
    ),
    command!(
        "palette",
        Command::Palette,
        "Command palette",
        "actions search",
        "Ctrl/⌘+Shift+P"
    ),
];
pub fn search(query: &str) -> Vec<&'static CommandDefinition> {
    let words: Vec<_> = query.split_whitespace().map(str::to_lowercase).collect();
    COMMANDS
        .iter()
        .filter(|command| {
            let text = format!(
                "{} {} {}",
                command.label, command.keywords, command.shortcut
            )
            .to_lowercase();
            words.iter().all(|word| text.contains(word))
        })
        .collect()
}
/// Composer-only bindings remain owned by the composer, documented here.
pub const COMPOSER_SHORTCUTS: &[(&str, &str)] = &[
    (
        "Enter / Shift+Enter",
        "Send or queue a prompt / insert a new line",
    ),
    (
        "↑ / ↓",
        "Prompt history at the start of the composer; navigate mention suggestions",
    ),
    ("Shift+Tab", "Cycle approval mode"),
    ("Alt+Y / Alt+N", "Approve / deny the current tool request"),
    ("Alt+A", "Auto-accept file edits for this session"),
    ("Ctrl/⌘+C", "Stop the run when no composer text is selected"),
    (
        "Escape",
        "Close suggestions, detach empty-draft context, or stop the run",
    ),
];
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn catalog_is_unique_searchable_and_capability_based() {
        let ids: std::collections::BTreeSet<_> = COMMANDS.iter().map(|c| c.id).collect();
        assert_eq!(ids.len(), COMMANDS.len());
        assert_eq!(
            search("EDITOR attach")
                .iter()
                .map(|c| c.command)
                .collect::<Vec<_>>(),
            [Command::CaptureEditor]
        );
        assert!(search("nonexistent").is_empty());
        assert_eq!(search("  ").len(), COMMANDS.len());
        let no_project = CommandContext::default();
        for command in [
            Command::ToggleTerminal,
            Command::TogglePanel(Panel::Files),
            Command::TogglePanel(Panel::Editor),
            Command::CaptureEditor,
        ] {
            assert!(command.unavailable(no_project).is_some());
            assert!(
                command
                    .unavailable(CommandContext {
                        project: true,
                        editor: true,
                        running: true
                    })
                    .is_none()
            );
        }
        assert!(Command::Stop.unavailable(no_project).is_some());
        for command in [
            Command::Context,
            Command::NewSession,
            Command::ModelSetup,
            Command::FocusChat,
        ] {
            assert!(command.unavailable(no_project).is_none());
        }
    }
}
