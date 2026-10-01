use leptos::prelude::*;
use openwebide_core::{SessionTelemetry, User};

use super::{
    chat::ChatState, git::GitState, projects::ProjectsState, settings::SettingsState, ui::UiState,
    workspace::WorkspaceState,
};

/// Authentication state shared by the gate and app shell.
#[derive(Clone, Copy)]
pub struct AuthState {
    pub user: RwSignal<Option<User>>,
    pub checked: RwSignal<bool>,
    pub username: Memo<Option<String>>,
    #[cfg(target_arch = "wasm32")]
    pub bridge: StoredValue<Option<crate::bridge::BridgeConn>, LocalStorage>,
}

impl AuthState {
    pub fn new() -> Self {
        let user = RwSignal::new(Option::<User>::None);
        let checked = RwSignal::new(false);
        let username =
            Memo::new(move |_| user.with(|user| user.as_ref().map(|user| user.username.clone())));

        Self {
            user,
            checked,
            username,
            #[cfg(target_arch = "wasm32")]
            bridge: StoredValue::new_local(None),
        }
    }

    pub fn set_user(&self, user: User) {
        self.user.set(Some(user));
    }

    pub fn mark_checked(&self) {
        self.checked.set(true);
    }

    /// Clear the signed-in account while keeping the completed auth check.
    pub fn logout(&self) {
        self.user.set(None);
    }

    /// Clear user-scoped frontend state after logout or session expiry.
    /// Returns object URLs for the browser shell to revoke.
    pub fn reset_user_state(
        &self,
        projects: ProjectsState,
        workspace: WorkspaceState,
        git: GitState,
        chat: ChatState,
        settings: SettingsState,
        ui: UiState,
    ) -> Vec<String> {
        #[cfg(target_arch = "wasm32")]
        self.bridge.update_value(|connection| {
            if let Some(connection) = connection.take() {
                connection.close();
            }
        });
        projects.projects_loaded.set(false);
        #[cfg(target_arch = "wasm32")]
        if let Some(controller) = chat.abort.get() {
            controller.abort();
        }
        chat.local_cancel_flag
            .with_value(|flag| flag.store(true, std::sync::atomic::Ordering::Relaxed));
        chat.streaming.set(false);
        chat.streaming_session.set(None);

        self.logout();
        chat.active_session.set(None);
        projects.reset();
        chat.sessions.set(Vec::new());
        chat.messages.set(Vec::new());
        let mut object_urls = workspace
            .snapshots
            .get_untracked()
            .into_values()
            .filter_map(|snapshot| snapshot.media_url)
            .collect::<Vec<_>>();
        object_urls.extend(workspace.media_url.get_untracked());
        workspace.reset();
        git.reset();

        chat.approval_mode.set(Default::default());
        chat.current_run_anchor.set(None);
        chat.session_telemetry.set(SessionTelemetry::default());
        settings.connections.set(Vec::new());
        settings.system_prompts.set(Vec::new());
        chat.models.set(Vec::new());
        chat.session_model.set(Default::default());
        chat.selected_model.set(None);
        settings.default_connection.set(None);
        settings.default_prompt.set(None);
        chat.show_terminal.set(false);
        chat.active_editor_context.set(None);
        ui.clear_toast();

        object_urls
    }
}

impl Default for AuthState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use openwebide_agent::policy::ApprovalMode;
    use openwebide_core::{
        ChatSession, Connection, FileDiff, FileEntry, GitRepoStatus, Project, ProviderKind,
        SearchHit, SystemPrompt, UserRole, WorkspaceMode,
    };

    use crate::state::{projects::ProjectsState, settings::Theme, workspace::WorkspaceSnapshot};

    #[test]
    fn logout_clears_the_user_and_derived_username_but_keeps_checked_state() {
        Owner::new().with(|| {
            let auth = AuthState::new();
            auth.set_user(User {
                id: 1,
                username: "alice".into(),
                role: UserRole::User,
                created_at: 0,
            });
            auth.mark_checked();

            assert_eq!(auth.username.get_untracked().as_deref(), Some("alice"));

            auth.logout();

            assert!(auth.user.get_untracked().is_none());
            assert!(auth.username.get_untracked().is_none());
            assert!(auth.checked.get_untracked());
        });
    }

    #[test]
    fn reset_user_state_clears_user_scoped_stores_and_returns_media_urls() {
        Owner::new().with(|| {
            let auth = AuthState::new();
            auth.set_user(User {
                id: 1,
                username: "alice".into(),
                role: UserRole::User,
                created_at: 0,
            });
            auth.mark_checked();

            let projects = ProjectsState::new();
            let workspace = WorkspaceState::with_active_project(projects.active_project);
            let chat = ChatState::with_active_session(workspace.active_session);
            let git = GitState::with_active_project(projects.active_project);
            let settings = SettingsState::new(Theme::Light, "https://bridge.test".into());
            let ui = UiState::new();

            projects.projects_loaded.set(true);
            projects.active_project.set(Some(1));
            projects.needs_grant.set([1].into_iter().collect());
            projects.projects.set(vec![Project {
                id: 1,
                name: "project".into(),
                mode: WorkspaceMode::Remote,
                path: None,
                user_id: Some(1),
                created_at: 0,
            }]);
            projects.open_tab_ids.set(vec![1]);
            workspace.entries.set(
                [(
                    String::new(),
                    vec![FileEntry {
                        name: "main.rs".into(),
                        path: "main.rs".into(),
                        is_dir: false,
                        size: 12,
                    }],
                )]
                .into_iter()
                .collect(),
            );
            workspace
                .expanded
                .set(["src".to_string()].into_iter().collect());
            workspace.open_file.set(Some("main.rs".into()));
            workspace.content.set("unsaved edit".into());
            workspace.dirty.set(true);
            workspace.search.set(Some(vec![SearchHit {
                path: "main.rs".into(),
                line: 1,
                text: "match".into(),
            }]));
            workspace.pending_edits.set(
                [(
                    "main.rs".into(),
                    FileDiff {
                        path: "main.rs".into(),
                        old: Some("old".into()),
                        new: "new".into(),
                        old_unavailable: false,
                        backup_path: None,
                    },
                )]
                .into_iter()
                .collect(),
            );
            workspace.active_session.set(Some(2));
            workspace.media_url.set(Some("blob:active".into()));
            workspace.snapshots.set(
                [(
                    1,
                    WorkspaceSnapshot {
                        media_url: Some("blob:saved".into()),
                        ..WorkspaceSnapshot::default()
                    },
                )]
                .into_iter()
                .collect(),
            );
            git.status.set(Some(GitRepoStatus::default()));
            chat.sessions.set(vec![ChatSession {
                id: 2,
                name: "session".into(),
                connection_id: Some(3),
                system_prompt_id: None,
                project_id: Some(1),
                user_id: Some(1),
                created_at: 0,
            }]);
            chat.notify("notice");
            chat.streaming.set(true);
            chat.streaming_session.set(Some(2));
            chat.local_cancel_flag
                .with_value(|flag| flag.store(false, std::sync::atomic::Ordering::Relaxed));
            chat.models.set(vec![openwebide_core::ModelInfo {
                name: "model".into(),
            }]);
            chat.selected_model.set(Some("model".into()));
            chat.session_model.update(|models| {
                models.insert(2, Some("model".into()));
            });
            chat.approval_mode
                .set([(2, ApprovalMode::AlwaysForSession)].into_iter().collect());
            chat.current_run_anchor.set(Some(4));
            chat.session_telemetry.update(|telemetry| {
                telemetry.tool_calls_count = 2;
            });
            chat.show_terminal.set(true);
            chat.active_editor_context
                .set(Some(openwebide_core::EditorContext {
                    file_path: "main.rs".into(),
                    cursor_line: 1,
                    cursor_col: 1,
                    selection: None,
                }));
            settings.connections.set(vec![Connection {
                id: 3,
                name: "connection".into(),
                kind: ProviderKind::Ollama,
                base_url: "http://localhost:11434".into(),
                model: None,
                enabled: true,
                context_limit: None,
            }]);
            settings.system_prompts.set(vec![SystemPrompt {
                id: 4,
                name: "prompt".into(),
                content: "prompt".into(),
            }]);
            settings.default_connection.set(Some(3));
            settings.default_prompt.set(Some(4));
            ui.notify("session expired");

            let urls = auth.reset_user_state(projects, workspace, git, chat, settings, ui);

            assert!(!projects.projects_loaded.get_untracked());
            assert!(projects.projects.get_untracked().is_empty());
            assert!(projects.open_tab_ids.get_untracked().is_empty());
            assert!(projects.needs_grant.get_untracked().is_empty());
            assert!(projects.active_project.get_untracked().is_none());
            assert!(workspace.active_session.get_untracked().is_none());
            assert!(workspace.entries.get_untracked().is_empty());
            assert!(workspace.expanded.get_untracked().is_empty());
            assert!(workspace.open_file.get_untracked().is_none());
            assert!(workspace.content.get_untracked().is_empty());
            assert!(!workspace.dirty.get_untracked());
            assert!(workspace.search.get_untracked().is_none());
            assert!(workspace.pending_edits.get_untracked().is_empty());
            assert!(workspace.pending_diff.get_untracked().is_none());
            assert!(workspace.snapshots.get_untracked().is_empty());
            assert!(workspace.media_url.get_untracked().is_none());
            assert!(git.status.get_untracked().is_none());
            assert!(chat.sessions.get_untracked().is_empty());
            assert!(chat.messages.get_untracked().is_empty());
            assert!(!chat.streaming.get_untracked());
            assert!(chat.streaming_session.get_untracked().is_none());
            assert!(
                chat.local_cancel_flag
                    .with_value(|flag| flag.load(std::sync::atomic::Ordering::Relaxed))
            );
            assert!(chat.models.get_untracked().is_empty());
            assert!(chat.selected_model.get_untracked().is_none());
            assert!(chat.session_model.get_untracked().is_empty());
            assert!(chat.approval_mode.get_untracked().is_empty());
            assert!(chat.current_run_anchor.get_untracked().is_none());
            assert_eq!(
                chat.session_telemetry.get_untracked(),
                SessionTelemetry::default()
            );
            assert!(!chat.show_terminal.get_untracked());
            assert!(chat.active_editor_context.get_untracked().is_none());
            assert!(settings.connections.get_untracked().is_empty());
            assert!(settings.system_prompts.get_untracked().is_empty());
            assert!(settings.default_connection.get_untracked().is_none());
            assert!(settings.default_prompt.get_untracked().is_none());
            assert!(ui.toast.get_untracked().is_none());
            assert!(auth.user.get_untracked().is_none());
            assert!(auth.checked.get_untracked());
            assert_eq!(urls, vec!["blob:saved", "blob:active"]);
        });
    }
}
