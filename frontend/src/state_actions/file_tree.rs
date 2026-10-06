//! File-tree policy shared by toolbar, context menus and editor actions.
use crate::{
    project_git::ProjectGit,
    state::{
        auth::AuthState,
        chat::ChatState,
        layout::Panel,
        projects::ProjectsState,
        ui::{ConfirmRequest, PromptRequest, UiState},
        workspace::WorkspaceState,
    },
    workspace::Workspace,
};
use leptos::{prelude::*, task::spawn_local};
use openwebide_core::{
    FileEntry,
    git::{GitPathAction, GitPathRequest},
    vfs::VfsEntryKind,
    workspace_entries::{contains_path, entry_path, moved_path, parent},
};
use wasm_bindgen::JsCast;

#[derive(Clone, Debug)]
enum Mutation {
    Create(String, VfsEntryKind),
    Move(String, String),
    Delete(String),
    Ignore(String, bool),
    Git(GitPathRequest),
}
impl Mutation {
    fn path(&self) -> &str {
        match self {
            Self::Create(path, _)
            | Self::Move(path, _)
            | Self::Delete(path)
            | Self::Ignore(path, _) => path,
            Self::Git(request) => &request.path,
        }
    }
    fn discards(&self) -> bool {
        matches!(
            self,
            Self::Delete(_)
                | Self::Git(GitPathRequest {
                    action: GitPathAction::Revert,
                    ..
                })
        )
    }
}
#[derive(Clone, Copy)]
struct Scope {
    project: i64,
    epoch: u64,
    auth: u64,
}

#[derive(Clone, Copy)]
pub struct FileTreeActions {
    pub busy: RwSignal<bool>,
    pub epoch: Memo<u64>,
    pub send_prompt: RwSignal<Option<Callback<String>>>,
    projects: ProjectsState,
    workspace: WorkspaceState,
    auth: AuthState,
    ui: UiState,
    chat: Option<ChatState>,
    git: Option<ProjectGit>,
    layout: Option<super::layout::LayoutActions>,
    workspace_for: Callback<i64, Option<Workspace>>,
    load_dir: Callback<(i64, String)>,
    open: Callback<String>,
    refresh_git: Callback<()>,
}
impl FileTreeActions {
    pub fn new(
        projects: ProjectsState,
        workspace: WorkspaceState,
        ui: UiState,
        workspace_for: Callback<i64, Option<Workspace>>,
        load_dir: Callback<(i64, String)>,
        open: Callback<String>,
        refresh_git: Callback<()>,
    ) -> Self {
        let auth = expect_context::<AuthState>();
        let git = use_context::<ProjectGit>();
        let epoch = super::workspace::project_epoch(projects, auth);
        let busy = RwSignal::new(false);
        let previous = StoredValue::new(epoch.get_untracked());
        Effect::new(move |_| {
            let next = epoch.get();
            if previous.get_value() != next {
                busy.set(false);
            }
            previous.set_value(next);
        });
        Self {
            busy,
            epoch,
            send_prompt: RwSignal::new(None),
            projects,
            workspace,
            auth,
            ui,
            git,
            chat: use_context::<ChatState>(),
            layout: use_context::<super::layout::LayoutActions>(),
            workspace_for,
            load_dir,
            open,
            refresh_git,
        }
    }
    fn scope(self) -> Option<Scope> {
        Some(Scope {
            project: self.projects.active_project.get_untracked()?,
            epoch: self.epoch.get_untracked(),
            auth: self.auth.generation.get_untracked(),
        })
    }
    fn current(self, scope: Scope) -> bool {
        self.projects.active_project.try_get_untracked() == Some(Some(scope.project))
            && self.auth.generation.try_get_untracked() == Some(scope.auth)
            && self.epoch.try_get_untracked() == Some(scope.epoch)
    }
    pub fn disabled(self) -> bool {
        self.busy.get()
            || self
                .chat
                .is_some_and(|chat| chat.streaming.get() || chat.rewinding.get())
    }
    fn guard(self, scope: Scope, mutation: &Mutation) -> Result<(), String> {
        if !self.current(scope) {
            return Err("Project changed; reopen the menu".into());
        }
        if self.disabled() {
            return Err("Wait for the current operation to finish".into());
        }
        self.guard_editor(mutation)
    }
    fn guard_editor(self, mutation: &Mutation) -> Result<(), String> {
        entry_path(mutation.path())?;
        if self.workspace.is_resolving()
            || self.workspace.pending_edits.with_untracked(|edits| {
                edits.keys().any(|path| {
                    contains_path(mutation.path(), path)
                        || (matches!(mutation, Mutation::Ignore(..)) && path == ".gitignore")
                })
            })
        {
            return Err("Accept or reject pending edits before changing this path".into());
        }
        let affected = self.workspace.open_file.with_untracked(|path| {
            path.as_ref()
                .is_some_and(|path| contains_path(mutation.path(), path))
        });
        let ignore_dirty = matches!(mutation, Mutation::Ignore(..))
            && self.workspace.open_file.get_untracked().as_deref() == Some(".gitignore");
        let retained_dirty = self.workspace.editor_buffers.with_untracked(|buffers| {
            buffers.iter().any(|((project, path), buffer)| {
                Some(*project) == self.workspace.active_project.get_untracked()
                    && buffer.dirty
                    && (contains_path(mutation.path(), path)
                        || (matches!(mutation, Mutation::Ignore(..)) && path == ".gitignore"))
            })
        });
        if ((self.workspace.dirty.get_untracked() && (affected || ignore_dirty)) || retained_dirty)
            && !mutation.discards()
        {
            return Err("Save or discard the editor's unsaved changes first".into());
        }
        Ok(())
    }
    fn dispatch(self, scope: Scope, mutation: Mutation) {
        if !self.current(scope) {
            return;
        }
        if let Err(error) = self.guard(scope, &mutation) {
            self.ui.notify(error);
            return;
        }
        self.busy.set(true);
        spawn_local(async move {
            let result = self.execute(scope, &mutation).await;
            if !self.current(scope) {
                return;
            }
            self.busy.set(false);
            match result {
                Ok(()) => {
                    // Clear old directory keys before reloading expanded folders.
                    self.workspace.entries.set(Default::default());
                    self.workspace.search.set(None);
                    let mut dirs = self.workspace.expanded.get_untracked();
                    dirs.insert(String::new());
                    for dir in dirs {
                        self.load_dir.run((scope.project, dir));
                    }
                    self.refresh_git.run(());
                }
                Err(error) => {
                    self.ui.notify(error);
                    self.load_dir.run((scope.project, String::new()));
                    self.refresh_git.run(());
                }
            }
        });
    }
    async fn execute(self, scope: Scope, mutation: &Mutation) -> Result<(), String> {
        if !self.current(scope) {
            return Err("Project changed".into());
        }
        let files = self
            .workspace_for
            .run(scope.project)
            .ok_or("Grant access to the project folder first")?;
        let current = || self.current(scope);
        match mutation {
            Mutation::Create(path, kind) => {
                openwebide_core::workspace_entries::create_entry(&files, path, *kind, current)
                    .await?;
                if current() && *kind == VfsEntryKind::File {
                    self.open.run(path.clone());
                }
            }
            Mutation::Move(from, to) => {
                files.move_entry(from, to, current).await?;
                if current() {
                    self.workspace.expanded.update(|dirs| {
                        *dirs = dirs
                            .iter()
                            .map(|path| moved_path(path, from, to).unwrap_or_else(|| path.clone()))
                            .collect();
                    });
                    let moved = self
                        .workspace
                        .open_file
                        .get_untracked()
                        .and_then(|path| moved_path(&path, from, to));
                    self.close_affected(from);
                    if let Some(path) = moved {
                        self.open.run(path);
                    }
                }
            }
            Mutation::Delete(path) => {
                openwebide_core::workspace_entries::delete_entry(&files, path, current).await?;
                if current() {
                    self.close_affected(path);
                }
            }
            Mutation::Ignore(path, is_dir) => {
                files.ignore(path, *is_dir, current).await?;
                if current()
                    && self.workspace.open_file.get_untracked().as_deref() == Some(".gitignore")
                {
                    self.close_affected(".gitignore");
                    self.open.run(".gitignore".into());
                }
            }
            Mutation::Git(request) => {
                let repo = self
                    .git
                    .ok_or("Git execution unavailable")?
                    .repository(Some(scope.project))
                    .await?;
                if !current() {
                    return Err("Project changed".into());
                }
                let changes = repo.path_changes().await?;
                if !current() {
                    return Err("Project changed".into());
                }
                let paths = changes.action_paths(&request.path)?;
                for path in &paths {
                    self.guard_editor(&Mutation::Git(GitPathRequest {
                        path: path.clone(),
                        action: request.action,
                    }))?;
                }
                repo.path_action(request).await?;
                if current() && request.action == GitPathAction::Revert {
                    for path in &paths {
                        self.invalidate_buffers(path);
                    }
                }
                if current()
                    && request.action == GitPathAction::Revert
                    && let Some(open) = self.workspace.open_file.get_untracked()
                    && paths.iter().any(|path| contains_path(path, &open))
                {
                    self.workspace.dirty.set(false);
                    // A revert may remove a staged addition or rename destination.
                    let exists = openwebide_core::rewind::RewindFiles::read(&files, &open)
                        .await?
                        .is_some();
                    if current() {
                        if exists {
                            self.close_affected(&open);
                            self.open.run(open);
                        } else {
                            self.close_affected(&open);
                        }
                    }
                }
            }
        }
        Ok(())
    }
    fn invalidate_buffers(self, path: &str) {
        let project = self.workspace.active_project.get_untracked();
        self.workspace.editor_tabs.update(|tabs| {
            if let Some(paths) = project.and_then(|project| tabs.get_mut(&project)) {
                paths.retain(|file| !contains_path(path, file));
            }
        });
        self.workspace.editor_recovery_overwrites.update(|permits| {
            permits.retain(|(id, file), _| Some(*id) != project || !contains_path(path, file));
        });
        self.workspace.editor_recovery_checks.update(|checks| {
            checks.retain(|(id, file), _| Some(*id) != project || !contains_path(path, file));
        });
        self.workspace.editor_recovered.update(|files| {
            files.retain(|(id, file)| Some(*id) != project || !contains_path(path, file));
        });
        self.workspace.editor_buffers.update(|buffers| {
            buffers.retain(|(id, file), _| Some(*id) != project || !contains_path(path, file));
        });
        self.workspace.editor_documents.update(|documents| {
            documents.retain(|(id, file), _| Some(*id) != project || !contains_path(path, file));
        });
        self.workspace.editor_scroll.update(|positions| {
            positions.retain(|(id, file), _| Some(*id) != project || !contains_path(path, file));
        });
    }
    fn close_affected(self, path: &str) {
        self.invalidate_buffers(path);
        self.workspace
            .expanded
            .update(|dirs| dirs.retain(|dir| !contains_path(path, dir)));
        if self
            .workspace
            .open_file
            .with_untracked(|open| open.as_ref().is_some_and(|open| contains_path(path, open)))
        {
            if let Some(url) = self.workspace.media_url.get_untracked() {
                let _ = web_sys::Url::revoke_object_url(&url);
            }
            self.workspace.media_url.set(None);
            self.workspace.open_file.set(None);
            self.workspace.content.set(String::new());
            self.workspace.dirty.set(false);
        }
    }
    pub fn create(self, directory: &str, kind: VfsEntryKind) {
        let Some(scope) = self.scope() else {
            return;
        };
        if self.disabled() {
            return;
        }
        let value = if directory.is_empty() {
            String::new()
        } else {
            format!("{directory}/")
        };
        self.ui.set_prompt(PromptRequest {
            title: if kind.is_dir() {
                "New folder"
            } else {
                "New file"
            }
            .into(),
            value,
            placeholder: "Path relative to project".into(),
            submit_label: "Create".into(),
            on_submit: Callback::new(move |path: String| {
                self.dispatch(scope, Mutation::Create(path, kind));
            }),
        });
    }
    pub fn move_entry(self, entry: &FileEntry, rename: bool) {
        let Some(scope) = self.scope() else {
            return;
        };
        let source = entry.path.clone();
        if let Err(error) = self.guard(scope, &Mutation::Move(source.clone(), source.clone())) {
            self.ui.notify(error);
            return;
        }
        let value = if rename {
            entry.name.clone()
        } else {
            source.clone()
        };
        self.ui.set_prompt(PromptRequest {
            title: if rename { "Rename" } else { "Move" }.into(),
            value,
            placeholder: if rename {
                "New name"
            } else {
                "Destination path relative to project"
            }
            .into(),
            submit_label: if rename { "Rename" } else { "Move" }.into(),
            on_submit: Callback::new(move |value: String| {
                if rename && (value.is_empty() || value.contains(['/', '\\'])) {
                    self.ui.notify("Enter a single file or folder name");
                    return;
                }
                let to = if rename && !parent(&source).is_empty() {
                    format!("{}/{value}", parent(&source))
                } else {
                    value
                };
                self.dispatch(scope, Mutation::Move(source.clone(), to));
            }),
        });
    }
    pub fn delete(self, entry: &FileEntry) {
        self.confirm(Mutation::Delete(entry.path.clone()), "Delete", format!(
            "Delete `{}`{}? Unsaved editor changes for this path will be discarded. This cannot be undone.",
            entry.path, if entry.is_dir {" and everything inside it"} else {""}));
    }
    pub fn git_action(self, path: &str, action: GitPathAction) {
        let mutation = Mutation::Git(GitPathRequest {
            path: path.into(),
            action,
        });
        if action == GitPathAction::Revert {
            self.confirm(mutation, "Revert", format!("Discard staged, working-tree and unsaved editor changes for `{path}` (including the original paths of renamed files) and restore HEAD? Untracked files are preserved."));
        } else if let Some(scope) = self.scope() {
            self.dispatch(scope, mutation);
        }
    }
    fn confirm(self, mutation: Mutation, title: &str, message: String) {
        let Some(scope) = self.scope() else {
            return;
        };
        if let Err(error) = self.guard(scope, &mutation) {
            self.ui.notify(error);
            return;
        }
        self.ui.set_confirm(ConfirmRequest {
            title: title.into(),
            message,
            confirm_label: title.into(),
            action: Callback::new(move |()| self.dispatch(scope, mutation.clone())),
        });
    }
    pub fn ignore(self, entry: &FileEntry) {
        if let Some(scope) = self.scope() {
            self.dispatch(scope, Mutation::Ignore(entry.path.clone(), entry.is_dir));
        }
    }
    pub fn copy_path(self, path: &str) {
        let Some(scope) = self.scope() else {
            return;
        };
        let path = path.to_string();
        spawn_local(async move {
            if let Err(error) = crate::clipboard::copy_text(&path).await
                && self.current(scope)
            {
                self.ui.notify(error);
            }
        });
    }
    pub async fn path_changes(self) -> Result<openwebide_core::git::GitPathChanges, String> {
        let scope = self.scope().ok_or("Open a project first")?;
        let repository = self
            .git
            .ok_or("Git execution unavailable")?
            .repository(Some(scope.project))
            .await?;
        if !self.current(scope) {
            return Err("Project changed".into());
        }
        let result = repository.path_changes().await;
        if !self.current(scope) {
            return Err("Project changed".into());
        }
        result
    }
    pub fn chat(self, entry: &FileEntry, task: &str, diff: bool) {
        let Some(scope) = self.scope() else {
            return;
        };
        let Some(chat) = self.chat else {
            return;
        };
        let path = &entry.path;
        let kind = if diff {
            openwebide_core::prompt::MentionKind::Diff
        } else if entry.is_dir {
            openwebide_core::prompt::MentionKind::Folder
        } else {
            openwebide_core::prompt::MentionKind::File
        };
        let prompt = format!(
            "{task} {}",
            openwebide_core::prompt::mention_token(kind, path)
        );
        if self.disabled()
            || chat.connection_changing.get_untracked()
            || chat.creating_session.get_untracked()
        {
            self.ui
                .notify("Wait for the current chat operation to finish");
            return;
        }
        let Some(send) = self.send_prompt.get_untracked() else {
            self.ui.notify("Chat actions unavailable");
            return;
        };
        send.run(prompt);
        if let Some(layout) = self.layout {
            layout.show.run(Panel::Chat);
        }
        leptos::leptos_dom::helpers::request_animation_frame(move || {
            if !self.current(scope) {
                return;
            }
            if let Ok(Some(input)) = document().query_selector(".composer-input")
                && let Some(input) = input.dyn_ref::<web_sys::HtmlElement>()
            {
                let _ = input.focus();
            }
        });
    }
}
