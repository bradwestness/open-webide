//! Shared session-run facade. UI actions describe intent; host adapters supply runtime primitives.
use leptos::prelude::*;
use openwebide_core::{EditorContext, Project, RunEvent, WorkspaceMode};
use web_sys::{AbortController, FileSystemDirectoryHandle};

use crate::{
    backend::Api,
    bridge::BridgeConn,
    project_host::ProjectHost,
    state::{
        auth::AuthState,
        chat::{ChatState, InterruptedRun},
        projects::ProjectsState,
        settings::SettingsState,
    },
    state_actions::runs::RunActions,
};

#[derive(Clone, Copy)]
pub struct ProjectRuns {
    api: Api,
    chat: ChatState,
    projects: ProjectsState,
    settings: SettingsState,
    host: ProjectHost,
    runs: RunActions,
    bridge: RwSignal<Option<BridgeConn>, LocalStorage>,
    auth: AuthState,
}

pub struct RunInput {
    pub session: i64,
    pub content: String,
    pub model: Option<String>,
    pub editor: Option<EditorContext>,
    pub controller: AbortController,
    pub resume: Option<InterruptedRun>,
    pub queued_prompt: Option<openwebide_core::QueuedPromptKey>,
}

enum RunHost {
    Browser {
        project: Project,
        handle: FileSystemDirectoryHandle,
    },
    Server,
}

impl ProjectRuns {
    pub(crate) fn new(
        api: Api,
        chat: ChatState,
        projects: ProjectsState,
        settings: SettingsState,
        host: ProjectHost,
        runs: RunActions,
        bridge: RwSignal<Option<BridgeConn>, LocalStorage>,
    ) -> Self {
        Self {
            api,
            chat,
            projects,
            settings,
            host,
            runs,
            bridge,
            auth: expect_context(),
        }
    }

    // Adapter/capability selection is the only place that inspects workspace mode.
    pub fn browser_owned(self, project: Option<i64>) -> bool {
        project
            .and_then(|id| self.projects.project(id))
            .is_some_and(|project| project.mode == WorkspaceMode::Local)
    }
    fn select(self, project: Option<i64>) -> Result<RunHost, String> {
        let Some(project) = project else {
            return Ok(RunHost::Server);
        };
        let project = self
            .projects
            .project(project)
            .ok_or("Project is no longer available")?;
        match project.mode {
            WorkspaceMode::Remote => Ok(RunHost::Server),
            WorkspaceMode::Local => {
                let handle = self
                    .projects
                    .local_handles
                    .with_untracked(|handles| handles.get(&project.id).cloned())
                    .ok_or("local directory handle not available; re-open the folder")?;
                Ok(RunHost::Browser { project, handle })
            }
        }
    }

    pub fn restore(self, session: i64) {
        let project = self.chat.sessions.with_untracked(|sessions| {
            sessions
                .iter()
                .find(|entry| entry.id == session)
                .and_then(|entry| entry.project_id)
        });
        let resumable = self.browser_owned(project);
        self.chat.detect_interrupted_run(resumable);
        if resumable
            && !self.chat.streaming.get_untracked()
            && self.chat.active_run.get_untracked().is_none()
            && let Some(anchor) =
                self.chat
                    .messages
                    .snapshot()
                    .iter()
                    .rev()
                    .find_map(|item| match item {
                        crate::conversation::ConversationItem::Message(message)
                            if message.role == openwebide_core::Role::User =>
                        {
                            Some(message.id)
                        }
                        _ => None,
                    })
        {
            self.chat.cancel_run_prompts(anchor);
        }
    }
    pub fn can_resume(self, session_project: Option<i64>) -> bool {
        session_project == self.projects.active_project.get_untracked()
            && self.browser_owned(session_project)
            && self.chat.active_run.get_untracked().is_none()
    }
    pub fn stop(self) {
        if let Some((_, run_id, _)) = self.chat.active_run.get_untracked() {
            self.runs
                .control(openwebide_core::BridgeClientMessage::RunCancel { run_id });
            return;
        }
        self.chat
            .local_cancel_flag
            .with_value(|flag| flag.store(true, std::sync::atomic::Ordering::Relaxed));
        if !self.chat.streaming_is_local.get_untracked()
            && let Some(session) = self.chat.streaming_session.get_untracked()
        {
            leptos::task::spawn_local(async move {
                let _ = self
                    .api
                    .with_value(Clone::clone)
                    .cancel_session(session)
                    .await;
            });
        }
        self.chat.mark_stopped();
        self.chat.abort.with_untracked(|abort| {
            if let Some(controller) = abort {
                controller.abort();
            }
        });
    }

    pub fn permission(self, tool_call_id: String, approved: bool) {
        if let Some((_, run_id, _)) = self.chat.active_run.get_untracked() {
            if !self
                .runs
                .control(openwebide_core::BridgeClientMessage::RunPermission {
                    run_id,
                    tool_call_id: tool_call_id.clone(),
                    approved,
                })
            {
                return;
            }
        } else if self.chat.streaming_is_local.get_untracked() {
            self.chat.local_permissions.with_value(|decisions| {
                if let Ok(mut decisions) = decisions.lock() {
                    decisions.insert(tool_call_id.clone(), approved);
                }
            });
        } else if let Some(session) = self.chat.streaming_session.get_untracked() {
            let id = tool_call_id.clone();
            leptos::task::spawn_local(async move {
                let _ = self
                    .api
                    .with_value(Clone::clone)
                    .set_permission(session, &id, approved)
                    .await;
            });
        }
        self.chat.messages.clear_prompt(&tool_call_id, true);
    }

    pub async fn run(
        self,
        input: RunInput,
        on_event: impl FnMut(RunEvent) + 'static,
    ) -> Result<(), String> {
        let session = self.chat.sessions.with_untracked(|sessions| {
            sessions
                .iter()
                .find(|session| session.id == input.session)
                .cloned()
        });
        if session.is_none() {
            return Err("Session is no longer available".into());
        }
        let project_id = session.as_ref().and_then(|session| session.project_id);
        let host = self.select(project_id)?;
        self.chat
            .streaming_is_local
            .set(matches!(host, RunHost::Browser { .. }));
        let generation = self.auth.generation.get_untracked();
        let controller = input.controller.clone();
        let current = move || {
            self.auth.generation.try_get_untracked() == Some(generation)
                && !controller.signal().aborted()
                && project_id.is_none_or(|id| {
                    self.projects.projects.try_with_untracked(|projects| {
                        projects.iter().any(|project| project.id == id)
                    }) == Some(true)
                })
                && self.chat.sessions.try_with_untracked(|sessions| {
                    sessions.iter().any(|session| {
                        session.id == input.session && session.project_id == project_id
                    })
                }) == Some(true)
        };
        // Model/default policy is shared before host dispatch.
        let connection = session
            .as_ref()
            .and_then(|session| session.connection_id)
            .or_else(|| self.settings.default_connection.get_untracked())
            .or_else(|| {
                self.settings.connections.with_untracked(|connections| {
                    connections
                        .iter()
                        .find(|connection| connection.enabled)
                        .map(|connection| connection.id)
                })
            })
            .ok_or("session has no connection; configure one in settings first")?;
        let prompt_id = session
            .as_ref()
            .and_then(|session| session.system_prompt_id)
            .or_else(|| self.settings.default_prompt.get_untracked());
        let system_prompt = prompt_id.and_then(|id| {
            self.settings.system_prompts.with_untracked(|prompts| {
                prompts
                    .iter()
                    .find(|prompt| prompt.id == id)
                    .map(|prompt| prompt.content.clone())
            })
        });
        if let Some(resume) = input.resume {
            self.chat.current_run_anchor.set(Some(resume.anchor_id));
        }
        match host {
            RunHost::Browser { project, handle } => {
                crate::local_agent::run_local_agent(
                    self.api,
                    input.session,
                    input.content,
                    input.model,
                    input.editor,
                    connection,
                    system_prompt,
                    handle,
                    project,
                    self.chat,
                    self.chat.local_cancel_flag.get_value(),
                    self.chat.local_permissions.get_value(),
                    on_event,
                    self.host,
                    self.bridge.get_untracked(),
                    input.resume,
                    input.queued_prompt,
                    current,
                )
                .await
            }
            RunHost::Server => {
                if input.resume.is_some() {
                    return Err("This run is recovered by its execution host.".into());
                }
                let started = self
                    .runs
                    .start(openwebide_core::BridgeClientMessage::RunStart {
                        run_id: format!(
                            "run-{}",
                            js_sys::Math::random().to_string().trim_start_matches("0.")
                        ),
                        session_id: input.session,
                        content: input.content.clone(),
                        model: input.model.clone(),
                        editor_context: input.editor.clone(),
                        queued_prompt: input.queued_prompt,
                    })
                    .await;
                if started || !current() {
                    return Ok(());
                }
                self.api
                    .with_value(Clone::clone)
                    .send_message(
                        input.session,
                        &input.content,
                        input.model.as_deref(),
                        input.editor.as_ref(),
                        input.queued_prompt,
                        Some(&input.controller.signal()),
                        Box::new(on_event),
                    )
                    .await
            }
        }
    }
}
