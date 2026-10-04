use crate::backend::Backend;
use futures::future::LocalBoxFuture;
use leptos::prelude::RwSignal;
use openwebide_core::{
    ChatCompletion, ChatMessage, ChatRequest, ChatSession, Connection, ConversationEntry,
    EditDecision, EditorContext, FileDiff, FileEntry, GitBranchInfo, GitCheckoutRequest,
    GitCheckoutResult, GitCommitRequest, GitCommitResult, GitRepoStatus, GitSyncRequest,
    GitSyncResult, Health, ModelInfo, PersistedEdit, Project, ProviderKind, ResolveEditRequest,
    Role, RunEvent, SearchHit, SystemPrompt, TurnTelemetry, User, WebSearchResult, WorkspaceMode,
    vfs::SearchOptions,
};
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet, VecDeque},
};
use web_sys::AbortSignal;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Call {
    SetSetting {
        key: String,
        value: String,
    },
    SetPermission {
        session: i64,
        id: String,
        approved: bool,
    },
    WriteFile {
        path: String,
        content: String,
    },
    CopyFile {
        from: String,
        to: String,
    },
    DeleteFile {
        path: String,
    },
    SendMessage {
        session: i64,
        content: String,
        model: Option<String>,
    },
    CancelSession {
        session: i64,
    },
    Request {
        method: &'static str,
    },
}

pub type Deferred<T> = futures::channel::oneshot::Receiver<Result<T, String>>;

type SettingsLoad = futures::channel::oneshot::Receiver<Result<BTreeMap<String, String>, String>>;

#[derive(Default)]
pub struct FakeBackend {
    pub session_search_results: RefCell<VecDeque<Deferred<Vec<ChatSession>>>>,
    pub session_export_results: RefCell<VecDeque<Deferred<openwebide_core::SessionExport>>>,
    pub title_results: RefCell<VecDeque<Deferred<Option<ChatSession>>>>,
    pub todo_updates: RefCell<BTreeMap<i64, Vec<openwebide_core::TodoUpdate>>>,
    pub todo_load_results: RefCell<VecDeque<Deferred<Option<openwebide_core::TodoUpdate>>>>,
    pub todo_errors: RefCell<VecDeque<String>>,

    pub fork_results: RefCell<VecDeque<Deferred<openwebide_core::ForkedSession>>>,
    pub fork_errors: RefCell<VecDeque<String>>,
    pub queued_prompts: RefCell<BTreeMap<i64, Vec<openwebide_core::QueuedPrompt>>>,
    pub queue_load_results: RefCell<VecDeque<Deferred<Vec<openwebide_core::QueuedPrompt>>>>,
    pub queue_errors: RefCell<VecDeque<String>>,
    pub queue_next_id: std::cell::Cell<i64>,
    pub run_changes: RefCell<BTreeMap<i64, Vec<openwebide_core::RunChange>>>,
    pub reviews: RefCell<BTreeMap<i64, openwebide_core::ReviewPlan>>,
    pub review_results: RefCell<VecDeque<Deferred<openwebide_core::ReviewPlan>>>,
    pub review_history: RefCell<
        Vec<(
            i64,
            openwebide_core::ReviewRequest,
            openwebide_core::RunChange,
        )>,
    >,
    pub rewinds: RefCell<BTreeMap<i64, openwebide_core::RewindPlan>>,
    pub git_diffs: RefCell<VecDeque<Result<String, String>>>,
    pub git_statuses: RefCell<VecDeque<Deferred<GitRepoStatus>>>,
    pub git_status_requests: RefCell<Vec<Option<i64>>>,
    pub model_setup: RefCell<openwebide_core::ModelSetup>,
    pub detections: RefCell<BTreeMap<(i64, String), openwebide_core::ModelDetection>>,
    pub test_results: RefCell<VecDeque<Result<openwebide_core::ModelTestResult, String>>>,
    pub server_settings: RefCell<BTreeMap<i64, openwebide_core::ServerSettings>>,
    pub endpoint_latency_ms: RefCell<i32>,
    pub project_results: RefCell<VecDeque<Deferred<Vec<Project>>>>,
    pub connection_results: RefCell<VecDeque<Deferred<Vec<Connection>>>>,
    pub session_results: RefCell<VecDeque<Deferred<Vec<ChatSession>>>>,
    pub prompt_results: RefCell<VecDeque<Deferred<Vec<SystemPrompt>>>>,
    pub model_results: RefCell<VecDeque<Deferred<Vec<ModelInfo>>>>,
    pub context_results: RefCell<VecDeque<Deferred<Option<usize>>>>,
    pub search_results: RefCell<VecDeque<Deferred<Vec<SearchHit>>>>,
    pub search_requests: RefCell<Vec<(i64, String, SearchOptions)>>,
    pub model_requests: RefCell<Vec<i64>>,
    pub context_requests: RefCell<Vec<(i64, Option<String>)>>,
    pub tool_timings: RefCell<BTreeMap<(i64, String), openwebide_core::ToolTiming>>,
    pub tool_sources: RefCell<BTreeMap<(i64, String), bool>>,
    pub message_save_results: RefCell<VecDeque<Result<(), String>>>,
    pub step_save_error: RefCell<Option<String>>,
    pub completion_error: RefCell<Option<String>>,
    pub persisted_edits: RefCell<BTreeMap<(i64, String), PersistedEdit>>,
    pub resolution_error: RefCell<Option<String>>,
    pub file_list_results: RefCell<VecDeque<Deferred<Vec<FileEntry>>>>,
    pub pending_results: RefCell<VecDeque<Deferred<Vec<PersistedEdit>>>>,
    pub resolution_results: RefCell<VecDeque<Deferred<()>>>,
    pub resolution_response_results: RefCell<VecDeque<Deferred<()>>>,
    pub resolution_requests: RefCell<Vec<(i64, ResolveEditRequest)>>,
    pub file_error: RefCell<Option<String>>,
    pub sessions: RefCell<Vec<ChatSession>>,
    pub messages: RefCell<BTreeMap<i64, Vec<ConversationEntry>>>,
    pub projects: RefCell<Vec<Project>>,
    pub project_delete_error: RefCell<Option<String>>,
    pub browse_entries: RefCell<BTreeMap<String, Vec<FileEntry>>>,
    pub files: RefCell<BTreeMap<(i64, String), String>>,
    pub binary_files: RefCell<BTreeMap<(i64, String), Vec<u8>>>,
    pub directories: RefCell<BTreeSet<(i64, String)>>,
    pub models: RefCell<Vec<ModelInfo>>,
    pub connections: RefCell<Vec<Connection>>,
    pub system_prompts: RefCell<Vec<SystemPrompt>>,
    pub panel_save_results:
        RefCell<VecDeque<futures::channel::oneshot::Receiver<Result<(), String>>>>,
    pub settings: RefCell<BTreeMap<String, String>>,
    pub settings_load_error: RefCell<Option<String>>,
    pub settings_load_results: RefCell<VecDeque<SettingsLoad>>,
    pub history_save_results:
        RefCell<VecDeque<futures::channel::oneshot::Receiver<Result<(), String>>>>,
    pub background_completion: RefCell<Option<Result<ChatCompletion, String>>>,
    pub background_requests: RefCell<Vec<ChatRequest>>,
    pub scripted_completions: RefCell<VecDeque<ChatCompletion>>,
    pub completion_requests: RefCell<Vec<ChatRequest>>,
    pub scripted_events: RefCell<VecDeque<Vec<RunEvent>>>,
    pub calls: RefCell<Vec<Call>>,
    pub session_expired: RwSignal<bool>,
}

impl FakeBackend {
    fn read(&self, project: i64, path: &str) -> Result<String, String> {
        self.files
            .borrow()
            .get(&(project, path.to_string()))
            .cloned()
            .ok_or_else(|| format!("file not found: {path}"))
    }
}

impl Backend for FakeBackend {
    fn session_expired(&self) -> RwSignal<bool> {
        self.session_expired
    }
    fn register<'a>(
        &'a self,
        _username: &'a str,
        _password: &'a str,
    ) -> LocalBoxFuture<'a, Result<User, String>> {
        Box::pin(async move {
            self.calls
                .borrow_mut()
                .push(Call::Request { method: "register" });
            Err("register has no scripted response".into())
        })
    }
    fn login<'a>(
        &'a self,
        _username: &'a str,
        _password: &'a str,
    ) -> LocalBoxFuture<'a, Result<User, String>> {
        Box::pin(async move {
            self.calls
                .borrow_mut()
                .push(Call::Request { method: "login" });
            Err("login has no scripted response".into())
        })
    }
    fn me<'a>(&'a self) -> LocalBoxFuture<'a, Result<User, String>> {
        Box::pin(async move {
            self.calls.borrow_mut().push(Call::Request { method: "me" });
            Err("me has no scripted response".into())
        })
    }
    fn logout<'a>(&'a self) -> LocalBoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            self.calls
                .borrow_mut()
                .push(Call::Request { method: "logout" });
            Ok(())
        })
    }
    fn bridge_token<'a>(&'a self) -> LocalBoxFuture<'a, Result<(String, i64), String>> {
        Box::pin(async move {
            self.calls.borrow_mut().push(Call::Request {
                method: "bridge_token",
            });
            Err("bridge_token has no scripted response".into())
        })
    }
    fn health<'a>(&'a self) -> LocalBoxFuture<'a, Result<Health, String>> {
        Box::pin(async move {
            self.calls
                .borrow_mut()
                .push(Call::Request { method: "health" });
            Ok(Health {
                status: "ok".into(),
                version: "test".into(),
            })
        })
    }
    fn preview_server<'a>(
        &'a self,
        probe: &'a openwebide_core::ModelProbe,
    ) -> LocalBoxFuture<'a, Result<openwebide_core::ServerDiscovery, String>> {
        Box::pin(async move {
            Ok(openwebide_core::ServerDiscovery {
                base_url: probe.base_url.clone(),
                kind: probe.kind,
                models: self.list_models(probe.server_id.unwrap_or(0)).await?,
                detections: Default::default(),
                detection_errors: Default::default(),
            })
        })
    }
    fn save_model_setup<'a>(
        &'a self,
        probe: &'a openwebide_core::ModelProbe,
        profiles: &'a [openwebide_core::ModelProfile],
    ) -> LocalBoxFuture<'a, Result<(Connection, openwebide_core::ModelSetup), String>> {
        Box::pin(async move {
            for profile in profiles {
                profile.settings.validate()?;
            }
            let server = if let Some(id) = probe.server_id {
                let mut server = self
                    .connections
                    .borrow()
                    .iter()
                    .find(|server| server.id == id)
                    .cloned()
                    .ok_or("Server not found")?;
                server.kind = probe.kind;
                server.base_url.clone_from(&probe.base_url);
                self.update_connection(&server).await?
            } else {
                self.create_connection("Server", probe.kind, &probe.base_url, None, None)
                    .await?
            };
            self.save_server_settings(server.id, &probe.transport)
                .await?;
            for profile in profiles {
                let mut profile = profile.clone();
                profile.selection.server_id = server.id;
                self.save_model_profile(&profile).await?;
            }
            let defaults = openwebide_core::model_setup::review_defaults(
                &self.model_setup.borrow().defaults,
                server.id,
                profiles,
            );
            let setup = self.save_model_defaults(&defaults).await?;
            Ok((server, setup))
        })
    }
    fn test_model<'a>(
        &'a self,
        _probe: &'a openwebide_core::ModelProbe,
    ) -> LocalBoxFuture<'a, Result<openwebide_core::ModelTestResult, String>> {
        Box::pin(async {
            if let Some(result) = self.test_results.borrow_mut().pop_front() {
                return result;
            }
            Ok(openwebide_core::ModelTestResult {
                structured_tools: true,
                streamed_tools: true,
                first_token_ms: Some(25),
                tokens_per_second: Some(50.0),
                ..Default::default()
            })
        })
    }
    fn preview_model<'a>(
        &'a self,
        probe: &'a openwebide_core::ModelProbe,
    ) -> LocalBoxFuture<'a, Result<openwebide_core::ModelDetection, String>> {
        self.detect_model(
            probe.server_id.unwrap_or(0),
            probe.model.as_deref().unwrap_or_default(),
        )
    }
    fn detect_model<'a>(
        &'a self,
        id: i64,
        model: &'a str,
    ) -> LocalBoxFuture<'a, Result<openwebide_core::ModelDetection, String>> {
        Box::pin(async move {
            if let Some(detected) = self.detections.borrow().get(&(id, model.into())) {
                return Ok(detected.clone());
            }
            Ok(openwebide_core::ModelDetection {
                context_limit: Some(8192),
                source: "test server".into(),
                capabilities: vec!["tools".into()],
                ..Default::default()
            })
        })
    }
    fn inspect_server<'a>(
        &'a self,
        base_url: &'a str,
        kind: Option<ProviderKind>,
    ) -> LocalBoxFuture<'a, Result<openwebide_core::ServerDiscovery, String>> {
        Box::pin(async move {
            Ok(openwebide_core::ServerDiscovery {
                base_url: base_url.into(),
                kind: kind.unwrap_or(ProviderKind::Ollama),
                models: self.models.borrow().clone(),
                detections: Default::default(),
                detection_errors: Default::default(),
            })
        })
    }
    fn discover_servers<'a>(
        &'a self,
    ) -> LocalBoxFuture<'a, Result<Vec<openwebide_core::ServerDiscovery>, String>> {
        Box::pin(async { Ok(Vec::new()) })
    }
    fn model_runtime<'a>(
        &'a self,
        id: i64,
        model: Option<&'a str>,
    ) -> LocalBoxFuture<'a, Result<openwebide_core::ModelRuntime, String>> {
        Box::pin(async move {
            let mut connection = self
                .connections
                .borrow()
                .iter()
                .find(|connection| connection.id == id)
                .cloned()
                .ok_or("Server not found")?;
            let setup = self.model_setup.borrow();
            let model = model
                .map(str::to_string)
                .or_else(|| {
                    setup
                        .defaults
                        .primary
                        .as_ref()
                        .filter(|selection| selection.server_id == id)
                        .map(|selection| selection.model.clone())
                })
                .or_else(|| connection.model.clone())
                .unwrap_or_default();
            let settings = setup.resolve(id, &model);
            connection.model = Some(model);
            Ok(openwebide_core::ModelRuntime {
                connection,
                settings,
                transport: Default::default(),
            })
        })
    }
    fn model_setup<'a>(
        &'a self,
    ) -> LocalBoxFuture<'a, Result<openwebide_core::ModelSetup, String>> {
        Box::pin(async { Ok(self.model_setup.borrow().clone()) })
    }
    fn save_model_defaults<'a>(
        &'a self,
        defaults: &'a openwebide_core::ModelDefaults,
    ) -> LocalBoxFuture<'a, Result<openwebide_core::ModelSetup, String>> {
        Box::pin(async move {
            self.model_setup.borrow_mut().defaults = defaults.clone();
            Ok(self.model_setup.borrow().clone())
        })
    }
    fn save_model_profile<'a>(
        &'a self,
        profile: &'a openwebide_core::ModelProfile,
    ) -> LocalBoxFuture<'a, Result<openwebide_core::ModelSetup, String>> {
        Box::pin(async move {
            let mut setup = self.model_setup.borrow_mut();
            setup
                .profiles
                .retain(|item| item.selection != profile.selection);
            setup.profiles.push(profile.clone());
            Ok(setup.clone())
        })
    }
    fn server_settings<'a>(
        &'a self,
        id: i64,
    ) -> LocalBoxFuture<'a, Result<openwebide_core::ServerSettings, String>> {
        Box::pin(async move {
            Ok(self.server_settings.borrow().get(&id).cloned().unwrap_or(
                openwebide_core::ServerSettings {
                    timeout_seconds: 300,
                    ..Default::default()
                },
            ))
        })
    }
    fn save_server_settings<'a>(
        &'a self,
        id: i64,
        update: &'a openwebide_core::ServerSettingsUpdate,
    ) -> LocalBoxFuture<'a, Result<openwebide_core::ServerSettings, String>> {
        Box::pin(async move {
            let mut settings = self.server_settings.borrow_mut();
            let item = settings
                .entry(id)
                .or_insert_with(|| openwebide_core::ServerSettings {
                    timeout_seconds: 300,
                    ..Default::default()
                });
            if let Some(preset) = update.preset {
                item.preset = preset;
            }
            if update.clear_api_key {
                item.has_api_key = false;
            } else if update.api_key.is_some() {
                item.has_api_key = true;
            }
            if let Some(timeout) = update.timeout_seconds {
                item.timeout_seconds = timeout;
            }
            if let Some(keep_alive) = &update.keep_alive {
                item.keep_alive = (!keep_alive.trim().is_empty()).then(|| keep_alive.clone());
            }
            Ok(item.clone())
        })
    }
    fn list_connections<'a>(&'a self) -> LocalBoxFuture<'a, Result<Vec<Connection>, String>> {
        Box::pin(async move {
            self.calls.borrow_mut().push(Call::Request {
                method: "list_connections",
            });
            let pending = self.connection_results.borrow_mut().pop_front();
            if let Some(pending) = pending {
                return pending
                    .await
                    .unwrap_or_else(|_| Err("response dropped".into()));
            }
            let latency = *self.endpoint_latency_ms.borrow();
            if latency > 0 {
                crate::util::sleep_ms(latency).await;
            }
            Ok(self.connections.borrow().clone())
        })
    }
    fn create_connection<'a>(
        &'a self,
        name: &'a str,
        kind: ProviderKind,
        base_url: &'a str,
        model: Option<&'a str>,
        context_limit: Option<usize>,
    ) -> LocalBoxFuture<'a, Result<Connection, String>> {
        Box::pin(async move {
            self.calls.borrow_mut().push(Call::Request {
                method: "create_connection",
            });
            let connection = Connection {
                id: self
                    .connections
                    .borrow()
                    .iter()
                    .map(|c| c.id)
                    .max()
                    .unwrap_or(0)
                    + 1,
                name: name.into(),
                kind,
                base_url: base_url.into(),
                model: model.map(str::to_string),
                enabled: true,
                context_limit,
                tool_stream_unsupported: false,
                tool_stream_revision: 0,
            };
            self.connections.borrow_mut().push(connection.clone());
            Ok(connection)
        })
    }
    fn update_connection<'a>(
        &'a self,
        connection: &'a Connection,
    ) -> LocalBoxFuture<'a, Result<Connection, String>> {
        Box::pin(async move {
            self.calls.borrow_mut().push(Call::Request {
                method: "update_connection",
            });
            let mut connections = self.connections.borrow_mut();
            let current = connections
                .iter_mut()
                .find(|c| c.id == connection.id)
                .ok_or("connection not found")?;
            *current = connection.clone();
            Ok(current.clone())
        })
    }
    fn delete_connection<'a>(&'a self, id: i64) -> LocalBoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            self.calls.borrow_mut().push(Call::Request {
                method: "delete_connection",
            });
            self.connections.borrow_mut().retain(|item| item.id != id);
            self.server_settings.borrow_mut().remove(&id);
            let mut setup = self.model_setup.borrow_mut();
            setup
                .profiles
                .retain(|profile| profile.selection.server_id != id);
            if setup
                .defaults
                .primary
                .as_ref()
                .is_some_and(|selection| selection.server_id == id)
            {
                setup.defaults.primary = None;
            }
            if setup
                .defaults
                .fast
                .as_ref()
                .is_some_and(|selection| selection.server_id == id)
            {
                setup.defaults.fast = None;
            }
            for session in self.sessions.borrow_mut().iter_mut() {
                if session.connection_id == Some(id) {
                    session.connection_id = None;
                }
            }
            Ok(())
        })
    }
    fn list_sessions<'a>(&'a self) -> LocalBoxFuture<'a, Result<Vec<ChatSession>, String>> {
        Box::pin(async move {
            self.calls.borrow_mut().push(Call::Request {
                method: "list_sessions",
            });
            let pending = self.session_results.borrow_mut().pop_front();
            if let Some(pending) = pending {
                return pending
                    .await
                    .unwrap_or_else(|_| Err("response dropped".into()));
            }
            let latency = *self.endpoint_latency_ms.borrow();
            if latency > 0 {
                crate::util::sleep_ms(latency).await;
            }
            Ok(self.sessions.borrow().clone())
        })
    }
    fn search_sessions<'a>(
        &'a self,
        search: &'a openwebide_core::SessionSearch,
    ) -> LocalBoxFuture<'a, Result<Vec<ChatSession>, String>> {
        Box::pin(async move {
            self.calls.borrow_mut().push(Call::Request {
                method: "search_sessions",
            });
            let pending = self.session_search_results.borrow_mut().pop_front();
            if let Some(pending) = pending {
                return pending
                    .await
                    .map_err(|_| "search response dropped".to_string())?;
            }
            search.validate()?;
            let query = search.query.trim().to_ascii_lowercase();
            let mut sessions:Vec<_>=self.sessions.borrow().iter().filter(|session| session.project_id==search.project_id && session.archived==search.archived
                && (query.is_empty() || session.name.to_ascii_lowercase().contains(&query) || self.messages.borrow().get(&session.id).is_some_and(|entries| entries.iter().any(|entry| matches!(entry,ConversationEntry::Message(message) if message.content.to_ascii_lowercase().contains(&query)))))).cloned().collect();
            sessions.sort_by_key(|session| {
                (
                    std::cmp::Reverse(session.pinned),
                    std::cmp::Reverse(session.id),
                )
            });
            Ok(sessions)
        })
    }
    fn session_preferences<'a>(
        &'a self,
        id: i64,
        preferences: &'a openwebide_core::SessionPreferences,
    ) -> LocalBoxFuture<'a, Result<ChatSession, String>> {
        Box::pin(async move {
            self.calls.borrow_mut().push(Call::Request {
                method: "session_preferences",
            });
            let mut sessions = self.sessions.borrow_mut();
            let session = sessions
                .iter_mut()
                .find(|session| session.id == id)
                .ok_or("not found")?;
            if let Some(value) = preferences.pinned {
                session.pinned = value;
            }
            if let Some(value) = preferences.archived {
                session.archived = value;
            }
            Ok(session.clone())
        })
    }
    fn session_title<'a>(
        &'a self,
        _id: i64,
    ) -> LocalBoxFuture<'a, Result<Option<ChatSession>, String>> {
        Box::pin(async move {
            self.calls.borrow_mut().push(Call::Request {
                method: "session_title",
            });
            let pending = self.title_results.borrow_mut().pop_front();
            match pending {
                Some(pending) => pending
                    .await
                    .map_err(|_| "title response dropped".to_string())?,
                None => Ok(None),
            }
        })
    }
    fn export_session<'a>(
        &'a self,
        id: i64,
    ) -> LocalBoxFuture<'a, Result<openwebide_core::SessionExport, String>> {
        Box::pin(async move {
            self.calls.borrow_mut().push(Call::Request {
                method: "export_session",
            });
            let pending = self.session_export_results.borrow_mut().pop_front();
            if let Some(pending) = pending {
                return pending
                    .await
                    .map_err(|_| "export response dropped".to_string())?;
            }
            let sessions = self.sessions.borrow();
            let session = sessions
                .iter()
                .find(|session| session.id == id)
                .ok_or("not found")?;
            let messages = self.messages.borrow();
            let entries = messages.get(&id).cloned().unwrap_or_default();
            Ok(openwebide_core::SessionExport {
                filename: openwebide_core::session_markdown_filename(session),
                markdown: openwebide_core::session_markdown(session, &entries),
            })
        })
    }
    fn list_models<'a>(
        &'a self,
        connection_id: i64,
    ) -> LocalBoxFuture<'a, Result<Vec<ModelInfo>, String>> {
        Box::pin(async move {
            self.calls.borrow_mut().push(Call::Request {
                method: "list_models",
            });
            self.model_requests.borrow_mut().push(connection_id);
            let pending = self.model_results.borrow_mut().pop_front();
            if let Some(pending) = pending {
                return pending
                    .await
                    .unwrap_or_else(|_| Err("response dropped".into()));
            }
            let latency = *self.endpoint_latency_ms.borrow();
            if latency > 0 {
                crate::util::sleep_ms(latency).await;
            }
            Ok(self.models.borrow().clone())
        })
    }
    fn list_system_prompts<'a>(&'a self) -> LocalBoxFuture<'a, Result<Vec<SystemPrompt>, String>> {
        Box::pin(async move {
            self.calls.borrow_mut().push(Call::Request {
                method: "list_system_prompts",
            });
            let pending = self.prompt_results.borrow_mut().pop_front();
            if let Some(pending) = pending {
                return pending
                    .await
                    .unwrap_or_else(|_| Err("response dropped".into()));
            }
            let latency = *self.endpoint_latency_ms.borrow();
            if latency > 0 {
                crate::util::sleep_ms(latency).await;
            }
            Ok(self.system_prompts.borrow().clone())
        })
    }
    fn create_system_prompt<'a>(
        &'a self,
        name: &'a str,
        content: &'a str,
    ) -> LocalBoxFuture<'a, Result<SystemPrompt, String>> {
        Box::pin(async move {
            self.calls.borrow_mut().push(Call::Request {
                method: "create_system_prompt",
            });
            let prompt = SystemPrompt {
                id: self
                    .system_prompts
                    .borrow()
                    .iter()
                    .map(|p| p.id)
                    .max()
                    .unwrap_or(0)
                    + 1,
                name: name.into(),
                content: content.into(),
            };
            self.system_prompts.borrow_mut().push(prompt.clone());
            Ok(prompt)
        })
    }
    fn update_system_prompt<'a>(
        &'a self,
        id: i64,
        name: &'a str,
        content: &'a str,
    ) -> LocalBoxFuture<'a, Result<SystemPrompt, String>> {
        Box::pin(async move {
            self.calls.borrow_mut().push(Call::Request {
                method: "update_system_prompt",
            });
            let mut prompts = self.system_prompts.borrow_mut();
            let prompt = prompts
                .iter_mut()
                .find(|p| p.id == id)
                .ok_or("prompt not found")?;
            prompt.name = name.into();
            prompt.content = content.into();
            Ok(prompt.clone())
        })
    }
    fn delete_system_prompt<'a>(&'a self, id: i64) -> LocalBoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            self.calls.borrow_mut().push(Call::Request {
                method: "delete_system_prompt",
            });
            self.system_prompts
                .borrow_mut()
                .retain(|item| item.id != id);
            Ok(())
        })
    }
    fn get_settings<'a>(
        &'a self,
    ) -> LocalBoxFuture<'a, Result<std::collections::BTreeMap<String, String>, String>> {
        Box::pin(async move {
            self.calls.borrow_mut().push(Call::Request {
                method: "get_settings",
            });
            let result = self.settings_load_results.borrow_mut().pop_front();
            if let Some(result) = result {
                return result.await.map_err(|error| error.to_string())?;
            }
            let latency = *self.endpoint_latency_ms.borrow();
            if latency > 0 {
                crate::util::sleep_ms(latency).await;
            }
            if let Some(error) = self.settings_load_error.borrow().clone() {
                return Err(error);
            }
            Ok(self.settings.borrow().clone())
        })
    }
    fn set_setting<'a>(
        &'a self,
        key: &'a str,
        value: &'a str,
    ) -> LocalBoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            self.calls.borrow_mut().push(Call::SetSetting {
                key: key.into(),
                value: value.into(),
            });
            if key == "panel_visibility" {
                let result = self.panel_save_results.borrow_mut().pop_front();
                if let Some(result) = result {
                    result.await.map_err(|error| error.to_string())??;
                }
            }
            if key == "prompt_history" {
                let result = self.history_save_results.borrow_mut().pop_front();
                if let Some(result) = result {
                    result.await.map_err(|error| error.to_string())??;
                }
            }
            self.settings.borrow_mut().insert(key.into(), value.into());
            Ok(())
        })
    }
    fn startup_context(
        &self,
        _project: i64,
        _tools: bool,
    ) -> LocalBoxFuture<'_, Result<String, String>> {
        Box::pin(async { Ok("Environment\nProject instructions ready".into()) })
    }
    fn create_session<'a>(
        &'a self,
        name: &'a str,
        connection_id: Option<i64>,
        system_prompt_id: Option<i64>,
        project_id: Option<i64>,
    ) -> LocalBoxFuture<'a, Result<ChatSession, String>> {
        Box::pin(async move {
            self.calls.borrow_mut().push(Call::Request {
                method: "create_session",
            });
            let session = ChatSession {
                pinned: false,
                archived: false,
                auto_title: true,
                title_revision: 0,
                id: self
                    .sessions
                    .borrow()
                    .iter()
                    .map(|s| s.id)
                    .max()
                    .unwrap_or(0)
                    + 1,
                name: name.into(),
                connection_id,
                system_prompt_id,
                project_id,
                user_id: None,
                created_at: 0,
            };
            self.settings.borrow_mut().insert(
                openwebide_core::ApprovalMode::setting_key(session.id),
                serde_json::to_string(&openwebide_core::ApprovalMode::NEW_SESSION).unwrap(),
            );
            self.sessions.borrow_mut().push(session.clone());
            Ok(session)
        })
    }
    fn list_projects<'a>(&'a self) -> LocalBoxFuture<'a, Result<Vec<Project>, String>> {
        Box::pin(async move {
            self.calls.borrow_mut().push(Call::Request {
                method: "list_projects",
            });
            let pending = self.project_results.borrow_mut().pop_front();
            if let Some(pending) = pending {
                return pending
                    .await
                    .unwrap_or_else(|_| Err("response dropped".into()));
            }
            let latency = *self.endpoint_latency_ms.borrow();
            if latency > 0 {
                crate::util::sleep_ms(latency).await;
            }
            Ok(self.projects.borrow().clone())
        })
    }
    fn create_project<'a>(
        &'a self,
        name: &'a str,
        mode: WorkspaceMode,
        path: Option<String>,
    ) -> LocalBoxFuture<'a, Result<Project, String>> {
        Box::pin(async move {
            self.calls.borrow_mut().push(Call::Request {
                method: "create_project",
            });
            let project = Project {
                id: self
                    .projects
                    .borrow()
                    .iter()
                    .map(|p| p.id)
                    .max()
                    .unwrap_or(0)
                    + 1,
                name: name.into(),
                mode,
                path,
                user_id: None,
                created_at: 0,
            };
            self.projects.borrow_mut().push(project.clone());
            Ok(project)
        })
    }
    fn rename_project<'a>(
        &'a self,
        id: i64,
        name: &'a str,
    ) -> LocalBoxFuture<'a, Result<Project, String>> {
        Box::pin(async move {
            self.calls.borrow_mut().push(Call::Request {
                method: "rename_project",
            });
            let mut items = self.projects.borrow_mut();
            let item = items
                .iter_mut()
                .find(|item| item.id == id)
                .ok_or("not found")?;
            item.name = name.into();
            Ok(item.clone())
        })
    }
    fn delete_project<'a>(&'a self, id: i64) -> LocalBoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            self.calls.borrow_mut().push(Call::Request {
                method: "delete_project",
            });
            if let Some(error) = self.project_delete_error.borrow().clone() {
                return Err(error);
            }
            self.projects.borrow_mut().retain(|item| item.id != id);
            let sessions = self
                .sessions
                .borrow()
                .iter()
                .filter(|session| session.project_id == Some(id))
                .map(|session| session.id)
                .collect::<Vec<_>>();
            for session in sessions {
                self.sessions.borrow_mut().retain(|item| item.id != session);
                self.messages.borrow_mut().remove(&session);
            }
            self.files
                .borrow_mut()
                .retain(|(project, _), _| *project != id);
            self.directories
                .borrow_mut()
                .retain(|(project, _)| *project != id);
            self.persisted_edits
                .borrow_mut()
                .retain(|(project, _), _| *project != id);
            Ok(())
        })
    }
    fn list_files<'a>(
        &'a self,
        project_id: i64,
        path: &'a str,
    ) -> LocalBoxFuture<'a, Result<Vec<FileEntry>, String>> {
        Box::pin(async move {
            self.calls.borrow_mut().push(Call::Request {
                method: "list_files",
            });
            let pending = self.file_list_results.borrow_mut().pop_front();
            if let Some(pending) = pending {
                return pending
                    .await
                    .unwrap_or_else(|_| Err("response dropped".into()));
            }
            let prefix = if path.is_empty() {
                String::new()
            } else {
                format!("{}/", path.trim_end_matches('/'))
            };
            let files = self.files.borrow();
            let directories = self.directories.borrow();
            let mut entries = BTreeMap::new();
            for (project, child) in files
                .keys()
                .chain(self.binary_files.borrow().keys())
                .chain(directories.iter())
            {
                if *project != project_id {
                    continue;
                }
                let Some(relative) = child.strip_prefix(&prefix) else {
                    continue;
                };
                if relative.is_empty() {
                    continue;
                }
                let name = relative.split('/').next().unwrap();
                let child_path = format!("{prefix}{name}");
                let is_dir = relative.contains('/')
                    || directories.contains(&(project_id, child_path.clone()));
                entries.insert(
                    name.to_string(),
                    FileEntry {
                        name: name.into(),
                        path: child_path,
                        is_dir,
                        size: 0,
                    },
                );
            }
            Ok(entries.into_values().collect())
        })
    }
    fn read_file_object_url<'a>(
        &'a self,
        project_id: i64,
        path: &'a str,
    ) -> LocalBoxFuture<'a, Result<String, String>> {
        Box::pin(async move {
            self.calls.borrow_mut().push(Call::Request {
                method: "read_file_object_url",
            });
            let content = self.read(project_id, path)?;
            let parts = js_sys::Array::new();
            parts.push(&wasm_bindgen::JsValue::from_str(&content));
            let blob =
                web_sys::Blob::new_with_str_sequence(&parts).map_err(|e| format!("{e:?}"))?;
            web_sys::Url::create_object_url_with_blob(&blob).map_err(|e| format!("{e:?}"))
        })
    }
    fn read_file<'a>(
        &'a self,
        project_id: i64,
        path: &'a str,
    ) -> LocalBoxFuture<'a, Result<String, String>> {
        Box::pin(async move {
            self.calls.borrow_mut().push(Call::Request {
                method: "read_file",
            });
            self.read(project_id, path)
        })
    }
    fn read_file_bytes<'a>(
        &'a self,
        project: i64,
        path: &'a str,
    ) -> LocalBoxFuture<'a, Result<Vec<u8>, String>> {
        Box::pin(async move {
            if let Some(bytes) = self.binary_files.borrow().get(&(project, path.into())) {
                return Ok(bytes.clone());
            }
            self.read(project, path).map(String::into_bytes)
        })
    }
    fn write_file_bytes<'a>(
        &'a self,
        project: i64,
        path: &'a str,
        bytes: &'a [u8],
    ) -> LocalBoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            if let Ok(text) = std::str::from_utf8(bytes) {
                self.binary_files
                    .borrow_mut()
                    .remove(&(project, path.into()));
                self.write_file(project, path, text).await
            } else {
                self.binary_files
                    .borrow_mut()
                    .insert((project, path.into()), bytes.to_vec());
                self.files.borrow_mut().remove(&(project, path.into()));
                Ok(())
            }
        })
    }
    fn read_file_lossy<'a>(
        &'a self,
        project_id: i64,
        path: &'a str,
    ) -> LocalBoxFuture<'a, Result<String, String>> {
        Box::pin(async move {
            self.calls.borrow_mut().push(Call::Request {
                method: "read_file_lossy",
            });
            self.read(project_id, path)
        })
    }
    fn write_file<'a>(
        &'a self,
        project_id: i64,
        path: &'a str,
        content: &'a str,
    ) -> LocalBoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            if let Some(error) = self.file_error.borrow().as_ref() {
                return Err(error.clone());
            }
            self.calls.borrow_mut().push(Call::WriteFile {
                path: path.into(),
                content: content.into(),
            });
            self.files
                .borrow_mut()
                .insert((project_id, path.into()), content.into());
            Ok(())
        })
    }
    fn copy_file<'a>(
        &'a self,
        project_id: i64,
        from: &'a str,
        to: &'a str,
    ) -> LocalBoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            if let Some(error) = self.file_error.borrow().as_ref() {
                return Err(error.clone());
            }
            self.calls.borrow_mut().push(Call::CopyFile {
                from: from.into(),
                to: to.into(),
            });
            let content = self.read(project_id, from)?;
            self.files
                .borrow_mut()
                .insert((project_id, to.into()), content);
            Ok(())
        })
    }
    fn create_file<'a>(
        &'a self,
        project_id: i64,
        path: &'a str,
        is_dir: bool,
    ) -> LocalBoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            self.calls.borrow_mut().push(Call::Request {
                method: "create_file",
            });
            let key = (project_id, path.to_string());
            if self.files.borrow().contains_key(&key)
                || (!is_dir && self.directories.borrow().contains(&key))
            {
                return Err(format!("file already exists: {path}"));
            }
            if is_dir {
                self.directories.borrow_mut().insert(key);
            } else {
                self.files.borrow_mut().insert(key, String::new());
            }
            Ok(())
        })
    }
    fn delete_file<'a>(
        &'a self,
        project_id: i64,
        path: &'a str,
    ) -> LocalBoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            if let Some(error) = self.file_error.borrow().as_ref() {
                return Err(error.clone());
            }
            self.calls
                .borrow_mut()
                .push(Call::DeleteFile { path: path.into() });
            self.binary_files
                .borrow_mut()
                .remove(&(project_id, path.into()));
            self.files.borrow_mut().remove(&(project_id, path.into()));
            Ok(())
        })
    }
    fn browse<'a>(&'a self, path: &'a str) -> LocalBoxFuture<'a, Result<Vec<FileEntry>, String>> {
        Box::pin(async move {
            self.calls
                .borrow_mut()
                .push(Call::Request { method: "browse" });
            Ok(self
                .browse_entries
                .borrow()
                .get(path)
                .cloned()
                .unwrap_or_default())
        })
    }
    fn search_content<'a>(
        &'a self,
        project_id: i64,
        query: &'a str,
        _path: &'a str,
        opts: SearchOptions,
    ) -> LocalBoxFuture<'a, Result<Vec<SearchHit>, String>> {
        Box::pin(async move {
            self.calls.borrow_mut().push(Call::Request {
                method: "search_content",
            });
            self.search_requests
                .borrow_mut()
                .push((project_id, query.into(), opts));
            let result = self.search_results.borrow_mut().pop_front();
            if let Some(result) = result {
                return result
                    .await
                    .map_err(|_| "search response dropped".to_string())?;
            }
            Ok(Vec::new())
        })
    }
    fn git_status<'a>(
        &'a self,
        project_id: Option<i64>,
    ) -> LocalBoxFuture<'a, Result<GitRepoStatus, String>> {
        Box::pin(async move {
            self.calls.borrow_mut().push(Call::Request {
                method: "git_status",
            });
            self.git_status_requests.borrow_mut().push(project_id);
            let result = self.git_statuses.borrow_mut().pop_front();
            match result {
                Some(result) => result
                    .await
                    .map_err(|_| "git status response dropped".to_string())?,
                None => Err("git_status has no scripted response".into()),
            }
        })
    }
    fn git_diff<'a>(
        &'a self,
        _project_id: Option<i64>,
        _path: Option<&'a str>,
    ) -> LocalBoxFuture<'a, Result<String, String>> {
        Box::pin(async move {
            self.calls
                .borrow_mut()
                .push(Call::Request { method: "git_diff" });
            self.git_diffs
                .borrow_mut()
                .pop_front()
                .unwrap_or_else(|| Ok(String::new()))
        })
    }
    fn git_file_head<'a>(
        &'a self,
        _project_id: Option<i64>,
        _path: &'a str,
    ) -> LocalBoxFuture<'a, Result<String, String>> {
        Box::pin(async move {
            self.calls.borrow_mut().push(Call::Request {
                method: "git_file_head",
            });
            Err("git_file_head has no scripted response".into())
        })
    }
    fn git_branches<'a>(
        &'a self,
        _project_id: Option<i64>,
    ) -> LocalBoxFuture<'a, Result<Vec<GitBranchInfo>, String>> {
        Box::pin(async move {
            self.calls.borrow_mut().push(Call::Request {
                method: "git_branches",
            });
            Ok(Vec::new())
        })
    }
    fn git_commit<'a>(
        &'a self,
        _project_id: Option<i64>,
        _req: &'a GitCommitRequest,
    ) -> LocalBoxFuture<'a, Result<GitCommitResult, String>> {
        Box::pin(async move {
            self.calls.borrow_mut().push(Call::Request {
                method: "git_commit",
            });
            Err("git_commit has no scripted response".into())
        })
    }
    fn git_checkout<'a>(
        &'a self,
        _project_id: Option<i64>,
        _req: &'a GitCheckoutRequest,
    ) -> LocalBoxFuture<'a, Result<GitCheckoutResult, String>> {
        Box::pin(async move {
            self.calls.borrow_mut().push(Call::Request {
                method: "git_checkout",
            });
            Err("git_checkout has no scripted response".into())
        })
    }
    fn git_sync<'a>(
        &'a self,
        _project_id: Option<i64>,
        _req: &'a GitSyncRequest,
    ) -> LocalBoxFuture<'a, Result<GitSyncResult, String>> {
        Box::pin(async move {
            self.calls
                .borrow_mut()
                .push(Call::Request { method: "git_sync" });
            Err("git_sync has no scripted response".into())
        })
    }
    fn set_session_connection(
        &self,
        id: i64,
        connection_id: i64,
    ) -> LocalBoxFuture<'_, Result<ChatSession, String>> {
        Box::pin(async move {
            let mut items = self.sessions.borrow_mut();
            let item = items
                .iter_mut()
                .find(|item| item.id == id)
                .ok_or("not found")?;
            item.connection_id = Some(connection_id);
            Ok(item.clone())
        })
    }
    fn rename_session<'a>(
        &'a self,
        id: i64,
        name: &'a str,
    ) -> LocalBoxFuture<'a, Result<ChatSession, String>> {
        Box::pin(async move {
            self.calls.borrow_mut().push(Call::Request {
                method: "rename_session",
            });
            let mut items = self.sessions.borrow_mut();
            let item = items
                .iter_mut()
                .find(|item| item.id == id)
                .ok_or("not found")?;
            item.name = name.into();
            item.auto_title = false;
            item.title_revision += 1;
            Ok(item.clone())
        })
    }
    fn delete_session<'a>(&'a self, id: i64) -> LocalBoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            self.calls.borrow_mut().push(Call::Request {
                method: "delete_session",
            });
            self.sessions.borrow_mut().retain(|item| item.id != id);
            self.messages.borrow_mut().remove(&id);
            self.queued_prompts.borrow_mut().remove(&id);
            self.todo_updates.borrow_mut().remove(&id);
            Ok(())
        })
    }
    fn cancel_session<'a>(&'a self, session_id: i64) -> LocalBoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            self.calls.borrow_mut().push(Call::CancelSession {
                session: session_id,
            });
            Ok(())
        })
    }
    fn set_permission<'a>(
        &'a self,
        session_id: i64,
        tool_call_id: &'a str,
        approved: bool,
    ) -> LocalBoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            self.calls.borrow_mut().push(Call::SetPermission {
                session: session_id,
                id: tool_call_id.into(),
                approved,
            });
            Ok(())
        })
    }
    fn list_messages<'a>(
        &'a self,
        session_id: i64,
    ) -> LocalBoxFuture<'a, Result<Vec<ConversationEntry>, String>> {
        Box::pin(async move {
            self.calls.borrow_mut().push(Call::Request {
                method: "list_messages",
            });
            Ok(self
                .messages
                .borrow()
                .get(&session_id)
                .cloned()
                .unwrap_or_default())
        })
    }
    fn list_run_changes(
        &self,
        project: i64,
    ) -> LocalBoxFuture<'_, Result<Vec<openwebide_core::RunChange>, String>> {
        Box::pin(async move {
            Ok(self
                .run_changes
                .borrow()
                .get(&project)
                .cloned()
                .unwrap_or_default())
        })
    }
    fn preview_run_review<'a>(
        &'a self,
        project: i64,
        request: &'a openwebide_core::ReviewRequest,
    ) -> LocalBoxFuture<'a, Result<openwebide_core::ReviewPlan, String>> {
        Box::pin(async move {
            let deferred = self.review_results.borrow_mut().pop_front();
            if let Some(deferred) = deferred {
                return deferred.await.map_err(|_| "review cancelled".to_string())?;
            }
            if let Some(plan) = self.reviews.borrow().get(&project) {
                return if plan.request == *request {
                    Ok(plan.clone())
                } else {
                    Err("Review in progress".into())
                };
            }
            self.run_changes
                .borrow()
                .get(&project)
                .and_then(|records| {
                    records.iter().find(|record| {
                        record.session_id == request.session_id
                            && record.message_id == request.message_id
                            && record.file.path == request.path
                    })
                })
                .ok_or("Review not found")?
                .prepare(request.clone())
        })
    }
    fn prepare_run_review<'a>(
        &'a self,
        project: i64,
        request: &'a openwebide_core::ReviewRequest,
    ) -> LocalBoxFuture<'a, Result<openwebide_core::ReviewPlan, String>> {
        Box::pin(async move {
            let plan = self.preview_run_review(project, request).await?;
            self.reviews.borrow_mut().insert(project, plan.clone());
            Ok(plan)
        })
    }
    fn complete_run_review<'a>(
        &'a self,
        project: i64,
        request: &'a openwebide_core::ReviewRequest,
    ) -> LocalBoxFuture<'a, Result<openwebide_core::RunChange, String>> {
        Box::pin(async move {
            if let Some(error) = self.resolution_error.borrow().clone() {
                return Err(error);
            }
            let plan = self.reviews.borrow().get(&project).cloned();
            let Some(plan) = plan else {
                return self
                    .review_history
                    .borrow()
                    .iter()
                    .find(|(id, previous, _)| *id == project && previous == request)
                    .map(|(_, _, record)| record.clone())
                    .ok_or("Review not prepared".into());
            };
            if plan.request != *request {
                return Err("Review changed".into());
            }
            let reviewed = plan.reviewed;
            let file = reviewed.pending_file()?;
            let mut edits = self.persisted_edits.borrow_mut();
            let revision = edits
                .get(&(project, request.path.clone()))
                .map_or(1, |edit| edit.revision + 1);
            edits.insert(
                (project, request.path.clone()),
                PersistedEdit {
                    project_id: project,
                    path: request.path.clone(),
                    revision,
                    decision: if reviewed.pending() > 0 {
                        EditDecision::Pending
                    } else {
                        request.decision
                    },
                    diff: file.preview(),
                    file: Some(file),
                },
            );
            let mut records = self.run_changes.borrow_mut();
            let record = records
                .get_mut(&project)
                .and_then(|records| {
                    records.iter_mut().find(|record| {
                        record.session_id == request.session_id
                            && record.message_id == request.message_id
                            && record.file.path == request.path
                    })
                })
                .ok_or("Review not found")?;
            *record = reviewed.clone();
            self.reviews.borrow_mut().remove(&project);
            self.review_history
                .borrow_mut()
                .push((project, request.clone(), reviewed.clone()));
            Ok(reviewed)
        })
    }
    fn prepare_rewind(
        &self,
        session: i64,
        message: i64,
    ) -> LocalBoxFuture<'_, Result<openwebide_core::RewindPlan, String>> {
        Box::pin(async move {
            if let Some(plan) = self.rewinds.borrow().get(&session).cloned() {
                return Ok(plan);
            }
            let entries = self.list_messages(session).await?;
            let plan = openwebide_core::RewindPlan::from_conversation(&entries, message)?;
            self.rewinds.borrow_mut().insert(session, plan.clone());
            Ok(plan)
        })
    }
    fn complete_rewind(
        &self,
        session: i64,
        message: i64,
    ) -> LocalBoxFuture<'_, Result<Vec<ConversationEntry>, String>> {
        Box::pin(async move {
            let plan = self
                .rewinds
                .borrow_mut()
                .remove(&session)
                .ok_or("rewind not prepared")?;
            if plan.message_id != message {
                return Err("checkpoint changed".into());
            }
            let mut all = self.messages.borrow_mut();
            let entries = all.entry(session).or_default();
            entries.retain(|entry| match entry {
                ConversationEntry::Message(m) => m.id < message,
                ConversationEntry::ToolStep(step) => step.anchor_message_id < message,
            });
            if let Some(project) = self
                .sessions
                .borrow()
                .iter()
                .find(|s| s.id == session)
                .and_then(|s| s.project_id)
            {
                let mut edits = self.persisted_edits.borrow_mut();
                for file in plan.files {
                    edits.remove(&(project, file.path));
                }
            }
            if let Some(updates) = self.todo_updates.borrow_mut().get_mut(&session) {
                updates.retain(|update| update.anchor_message_id < message);
            }
            Ok(entries.clone())
        })
    }
    fn approval_check<'a>(
        &'a self,
        session: i64,
        check: &'a openwebide_core::ApprovalCheck,
    ) -> LocalBoxFuture<'a, Result<openwebide_core::ApprovalDecision, String>> {
        Box::pin(async move {
            let mode = self
                .settings
                .borrow()
                .get(&openwebide_core::ApprovalMode::setting_key(session))
                .and_then(|value| serde_json::from_str::<openwebide_core::ApprovalMode>(value).ok())
                .unwrap_or_default();
            Ok(openwebide_core::ApprovalDecision {
                approved: mode.auto_approves(&check.call.name),
            })
        })
    }
    fn model_complete<'a>(
        &'a self,
        request: &'a ChatRequest,
    ) -> LocalBoxFuture<'a, Result<ChatCompletion, String>> {
        Box::pin(async move {
            self.background_requests.borrow_mut().push(request.clone());
            if let Some(result) = self.background_completion.borrow().clone() {
                result
            } else {
                self.chat_tools(request).await
            }
        })
    }
    fn chat_tools<'a>(
        &'a self,
        request: &'a ChatRequest,
    ) -> LocalBoxFuture<'a, Result<ChatCompletion, String>> {
        Box::pin(async move {
            self.calls.borrow_mut().push(Call::Request {
                method: "chat_tools",
            });
            self.completion_requests.borrow_mut().push(request.clone());
            self.scripted_completions
                .borrow_mut()
                .pop_front()
                .ok_or_else(|| "chat_tools has no scripted response".into())
        })
    }
    fn get_todo_plan(
        &self,
        session: i64,
    ) -> LocalBoxFuture<'_, Result<Option<openwebide_core::TodoUpdate>, String>> {
        Box::pin(async move {
            self.calls.borrow_mut().push(Call::Request {
                method: "get_todo_plan",
            });
            let pending = self.todo_load_results.borrow_mut().pop_front();
            if let Some(pending) = pending {
                return pending.await.map_err(|error| error.to_string())?;
            }
            Ok(self
                .todo_updates
                .borrow()
                .get(&session)
                .and_then(|updates| updates.last().cloned()))
        })
    }
    fn write_todo_plan<'a>(
        &'a self,
        session: i64,
        anchor: i64,
        plan: &'a openwebide_core::TodoPlan,
    ) -> LocalBoxFuture<'a, Result<openwebide_core::TodoUpdate, String>> {
        Box::pin(async move {
            self.calls.borrow_mut().push(Call::Request {
                method: "write_todo_plan",
            });
            plan.validate()?;
            if let Some(error) = self.todo_errors.borrow_mut().pop_front() {
                return Err(error);
            }
            let messages = self.messages.borrow();
            let latest = messages.get(&session).and_then(|entries| {
                entries.iter().rev().find_map(|entry| match entry {
                    ConversationEntry::Message(message) if message.role == Role::User => {
                        Some(message.id)
                    }
                    _ => None,
                })
            });
            if latest != Some(anchor) {
                return Err("Plan belongs to an earlier prompt".into());
            }
            let mut updates = self.todo_updates.borrow_mut();
            let updates = updates.entry(session).or_default();
            let update = openwebide_core::TodoUpdate {
                id: updates.last().map_or(1, |update| update.id + 1),
                session_id: session,
                anchor_message_id: anchor,
                created_at: 0,
                plan: plan.clone(),
            };
            updates.push(update.clone());
            Ok(update)
        })
    }
    fn fork_session<'a>(
        &'a self,
        source: i64,
        target: i64,
    ) -> LocalBoxFuture<'a, Result<openwebide_core::ForkedSession, String>> {
        Box::pin(async move {
            self.calls.borrow_mut().push(Call::Request {
                method: "fork_session",
            });
            let pending = self.fork_results.borrow_mut().pop_front();
            if let Some(pending) = pending {
                return pending.await.map_err(|error| error.to_string())?;
            }
            if let Some(error) = self.fork_errors.borrow_mut().pop_front() {
                return Err(error);
            }

            let source_session = self
                .sessions
                .borrow()
                .iter()
                .find(|session| session.id == source)
                .cloned()
                .ok_or("Session missing")?;
            let entries = self
                .messages
                .borrow()
                .get(&source)
                .cloned()
                .unwrap_or_default();
            let prompt = entries
                .iter()
                .find_map(|entry| match entry {
                    ConversationEntry::Message(message)
                        if message.id == target && message.role == Role::User =>
                    {
                        Some(message.content.clone())
                    }
                    _ => None,
                })
                .ok_or("Prompt missing")?;
            let session = self
                .create_session(
                    &format!("{} (branch)", source_session.name),
                    source_session.connection_id,
                    source_session.system_prompt_id,
                    source_session.project_id,
                )
                .await?;
            let mut ids = BTreeMap::new();
            let mut copied = Vec::new();
            for entry in entries.iter().filter(
                |entry| matches!(entry, ConversationEntry::Message(message) if message.id < target),
            ) {
                if let ConversationEntry::Message(message) = entry {
                    let copy = self
                        .persist_message(
                            session.id,
                            message.role,
                            &message.content,
                            message.usage.as_ref(),
                            message.tool_calls.as_deref(),
                        )
                        .await?;
                    ids.insert(message.id, copy.id);
                    copied.push(ConversationEntry::Message(copy));
                }
            }
            for entry in entries.iter().filter(|entry| matches!(entry, ConversationEntry::ToolStep(step) if step.anchor_message_id < target)) {
                if let ConversationEntry::ToolStep(step) = entry { let mut step = step.clone(); step.anchor_message_id = ids.get(&step.anchor_message_id).copied().ok_or("Missing tool anchor")?; copied.push(ConversationEntry::ToolStep(step)); }
            }
            self.messages
                .borrow_mut()
                .insert(session.id, copied.clone());
            let plans = self
                .todo_updates
                .borrow()
                .get(&source)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter(|update| update.anchor_message_id < target)
                .map(|mut update| {
                    update.session_id = session.id;
                    update.anchor_message_id = ids[&update.anchor_message_id];
                    update
                })
                .collect();
            self.todo_updates.borrow_mut().insert(session.id, plans);
            let mode = self
                .settings
                .borrow()
                .get(&openwebide_core::ApprovalMode::setting_key(source))
                .cloned();
            if let Some(mode) = mode {
                self.settings
                    .borrow_mut()
                    .insert(openwebide_core::ApprovalMode::setting_key(session.id), mode);
            }
            Ok(openwebide_core::ForkedSession {
                session,
                prompt,
                history: copied,
            })
        })
    }
    fn list_queued_prompts<'a>(
        &'a self,
        session: i64,
    ) -> LocalBoxFuture<'a, Result<Vec<openwebide_core::QueuedPrompt>, String>> {
        Box::pin(async move {
            let pending = self.queue_load_results.borrow_mut().pop_front();
            if let Some(pending) = pending {
                return pending.await.map_err(|error| error.to_string())?;
            }
            Ok(self
                .queued_prompts
                .borrow()
                .get(&session)
                .cloned()
                .unwrap_or_default())
        })
    }
    fn enqueue_prompt<'a>(
        &'a self,
        session: i64,
        content: &'a str,
        guidance: bool,
    ) -> LocalBoxFuture<'a, Result<openwebide_core::QueuedPrompt, String>> {
        Box::pin(async move {
            if let Some(error) = self.queue_errors.borrow_mut().pop_front() {
                return Err(error);
            }
            openwebide_core::chat_queue::validate_content(content)?;
            let id = self.queue_next_id.get() + 1;
            self.queue_next_id.set(id);
            let prompt = openwebide_core::QueuedPrompt {
                id,
                session_id: session,
                revision: 1,
                content: content.into(),
                created_at: 0,
                guidance,
            };
            self.queued_prompts
                .borrow_mut()
                .entry(session)
                .or_default()
                .push(prompt.clone());
            self.queued_prompts
                .borrow_mut()
                .get_mut(&session)
                .unwrap()
                .sort_by_key(|prompt| (!prompt.guidance, prompt.id));
            Ok(prompt)
        })
    }
    fn update_queued_prompt<'a>(
        &'a self,
        session: i64,
        key: openwebide_core::QueuedPromptKey,
        content: &'a str,
    ) -> LocalBoxFuture<'a, Result<openwebide_core::QueuedPrompt, String>> {
        Box::pin(async move {
            if let Some(error) = self.queue_errors.borrow_mut().pop_front() {
                return Err(error);
            }
            openwebide_core::chat_queue::validate_content(content)?;
            let mut all = self.queued_prompts.borrow_mut();
            let prompt = all
                .entry(session)
                .or_default()
                .iter_mut()
                .find(|entry| entry.key() == key)
                .ok_or("Queued prompt changed")?;
            prompt.revision += 1;
            prompt.content = content.into();
            Ok(prompt.clone())
        })
    }
    fn remove_queued_prompt<'a>(
        &'a self,
        session: i64,
        key: openwebide_core::QueuedPromptKey,
    ) -> LocalBoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            if let Some(error) = self.queue_errors.borrow_mut().pop_front() {
                return Err(error);
            }
            let mut all = self.queued_prompts.borrow_mut();
            let queue = all.entry(session).or_default();
            let index = queue
                .iter()
                .position(|entry| entry.key() == key)
                .ok_or("Queued prompt changed")?;
            queue.remove(index);
            Ok(())
        })
    }
    fn consume_queued_prompt<'a>(
        &'a self,
        session: i64,
        key: openwebide_core::QueuedPromptKey,
        content: &'a str,
    ) -> LocalBoxFuture<'a, Result<ChatMessage, String>> {
        Box::pin(async move {
            if let Some(error) = self.queue_errors.borrow_mut().pop_front() {
                return Err(error);
            }
            let first = self
                .queued_prompts
                .borrow()
                .get(&session)
                .and_then(|queue| queue.first())
                .cloned()
                .ok_or("Queued prompt missing")?;
            if first.key() != key || first.content != content {
                return Err("Queued prompt changed".into());
            }
            let message = self
                .persist_message(session, Role::User, content, None, None)
                .await?;
            self.queued_prompts
                .borrow_mut()
                .get_mut(&session)
                .unwrap()
                .remove(0);
            Ok(message)
        })
    }
    fn persist_message<'a>(
        &'a self,
        session_id: i64,
        role: Role,
        content: &'a str,
        usage: Option<&'a TurnTelemetry>,
        tool_calls: Option<&'a [openwebide_core::ToolCall]>,
    ) -> LocalBoxFuture<'a, Result<ChatMessage, String>> {
        Box::pin(async move {
            self.calls.borrow_mut().push(Call::Request {
                method: "persist_message",
            });
            if let Some(result) = self.message_save_results.borrow_mut().pop_front() {
                result?;
            }
            let message = ChatMessage {
                id: i64::try_from(self.messages.borrow().values().map(Vec::len).sum::<usize>())
                    .expect("test message count fits i64")
                    + 1,
                session_id,
                role,
                content: content.into(),
                created_at: 0,
                tool_calls: tool_calls.map(<[openwebide_core::ToolCall]>::to_vec),
                tool_call_id: None,
                usage: usage.cloned(),
            };
            self.messages
                .borrow_mut()
                .entry(session_id)
                .or_default()
                .push(ConversationEntry::Message(message.clone()));
            Ok(message)
        })
    }
    fn model_context<'a>(
        &'a self,
        connection_id: i64,
        model: Option<&'a str>,
    ) -> LocalBoxFuture<'a, Result<Option<usize>, String>> {
        Box::pin(async move {
            self.calls.borrow_mut().push(Call::Request {
                method: "model_context",
            });
            self.context_requests
                .borrow_mut()
                .push((connection_id, model.map(str::to_string)));
            let pending = self.context_results.borrow_mut().pop_front();
            if let Some(pending) = pending {
                return pending
                    .await
                    .unwrap_or_else(|_| Err("response dropped".into()));
            }
            let latency = *self.endpoint_latency_ms.borrow();
            if latency > 0 {
                crate::util::sleep_ms(latency).await;
            }
            Ok(self
                .connections
                .borrow()
                .iter()
                .find(|c| c.id == connection_id)
                .and_then(|c| c.context_limit))
        })
    }
    fn upsert_tool_step<'a>(
        &'a self,
        session_id: i64,
        _anchor_message_id: i64,
        tool_call_id: &'a str,
        _name: &'a str,
        _summary: &'a str,
        _diff: Option<&'a FileDiff>,
    ) -> LocalBoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            self.calls.borrow_mut().push(Call::Request {
                method: "upsert_tool_step",
            });
            if let Some(error) = self.step_save_error.borrow().as_ref() {
                return Err(error.clone());
            }
            self.tool_sources
                .borrow_mut()
                .entry((session_id, tool_call_id.into()))
                .or_insert(false);
            Ok(())
        })
    }
    fn save_tool_timing<'a>(
        &'a self,
        session: i64,
        id: &'a str,
        timing: &'a openwebide_core::ToolTiming,
    ) -> LocalBoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            if let Some(error) = self.step_save_error.borrow().as_ref() {
                return Err(error.clone());
            }
            let mut timings = self.tool_timings.borrow_mut();
            let key = (session, id.to_string());
            let timing = timings
                .get(&key)
                .map_or(Ok(*timing), |previous| previous.merge(*timing))
                .map_err(str::to_string)?;
            timings.insert(key, timing);
            if let Some(entries) = self.messages.borrow_mut().get_mut(&session) {
                for entry in entries {
                    if let ConversationEntry::ToolStep(step) = entry
                        && step.tool_call_id == id
                    {
                        step.timing = Some(timing);
                    }
                }
            }
            Ok(())
        })
    }
    fn complete_tool_step<'a>(
        &'a self,
        session_id: i64,
        tool_call_id: &'a str,
        ok: bool,
        _result_summary: &'a str,
        diff: Option<&'a FileDiff>,
    ) -> LocalBoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            self.calls.borrow_mut().push(Call::Request {
                method: "complete_tool_step",
            });
            if let Some(error) = self.completion_error.borrow().as_ref() {
                return Err(error.clone());
            }
            let session = self
                .sessions
                .borrow()
                .iter()
                .find(|session| session.id == session_id)
                .cloned()
                .ok_or("session not found")?;
            let mut sources = self.tool_sources.borrow_mut();
            let applied = sources
                .get_mut(&(session_id, tool_call_id.into()))
                .ok_or("tool step not found")?;
            if *applied {
                return Ok(());
            }
            if let (true, Some(project_id), Some(diff)) = (ok, session.project_id, diff) {
                let mut edits = self.persisted_edits.borrow_mut();
                let key = (project_id, diff.path.clone());
                let previous = edits.get(&key);
                let revision = previous.map_or(1, |edit| edit.revision + 1);
                let mut merged = diff.clone();
                let mut decision = EditDecision::Pending;
                if let Some(previous) =
                    previous.filter(|edit| edit.decision == EditDecision::Pending)
                {
                    merged = previous.diff.clone();
                    merged.new.clone_from(&diff.new);
                    if !merged.old_unavailable && merged.old.as_deref() == Some(merged.new.as_str())
                    {
                        decision = EditDecision::Accepted;
                    }
                }
                edits.insert(
                    key,
                    PersistedEdit {
                        file: None,
                        project_id,
                        path: diff.path.clone(),
                        revision,
                        decision,
                        diff: merged,
                    },
                );
            }
            *applied = true;
            Ok(())
        })
    }
    fn list_pending_edits(
        &self,
        project_id: i64,
    ) -> LocalBoxFuture<'_, Result<Vec<PersistedEdit>, String>> {
        Box::pin(async move {
            self.calls.borrow_mut().push(Call::Request {
                method: "list_pending_edits",
            });
            let pending = self.pending_results.borrow_mut().pop_front();
            if let Some(pending) = pending {
                return pending
                    .await
                    .map_err(|_| "pending edit request cancelled".to_string())?;
            }
            Ok(self
                .persisted_edits
                .borrow()
                .values()
                .filter(|edit| {
                    edit.project_id == project_id && edit.decision == EditDecision::Pending
                })
                .cloned()
                .collect())
        })
    }
    fn resolve_pending_edit<'a>(
        &'a self,
        project_id: i64,
        request: &'a ResolveEditRequest,
    ) -> LocalBoxFuture<'a, Result<PersistedEdit, String>> {
        Box::pin(async move {
            self.resolution_requests
                .borrow_mut()
                .push((project_id, request.clone()));
            let pending = self.resolution_results.borrow_mut().pop_front();
            if let Some(pending) = pending {
                pending
                    .await
                    .map_err(|_| "resolution cancelled".to_string())??;
            }
            if let Some(error) = self.resolution_error.borrow().as_ref() {
                return Err(error.clone());
            }
            if request.decision == EditDecision::Pending || request.revision <= 0 {
                return Err("invalid edit resolution".into());
            }
            let committed = {
                let mut edits = self.persisted_edits.borrow_mut();
                let edit = edits
                    .get_mut(&(project_id, request.path.clone()))
                    .ok_or("pending edit not found")?;
                if edit.revision != request.revision
                    || (edit.decision != EditDecision::Pending && edit.decision != request.decision)
                {
                    return Err("edit revision or decision changed".into());
                }
                edit.decision = request.decision;
                edit.clone()
            };
            let response = self.resolution_response_results.borrow_mut().pop_front();
            if let Some(response) = response {
                response
                    .await
                    .map_err(|_| "response cancelled".to_string())??;
            }
            Ok(committed)
        })
    }
    fn web_search<'a>(
        &'a self,
        _query: &'a str,
        _limit: usize,
    ) -> LocalBoxFuture<'a, Result<Vec<WebSearchResult>, String>> {
        Box::pin(async move {
            self.calls.borrow_mut().push(Call::Request {
                method: "web_search",
            });
            Err("web_search has no scripted response".into())
        })
    }
    fn fetch_web_page<'a>(
        &'a self,
        _target_url: &'a str,
    ) -> LocalBoxFuture<'a, Result<String, String>> {
        Box::pin(async move {
            self.calls.borrow_mut().push(Call::Request {
                method: "fetch_web_page",
            });
            Err("fetch_web_page has no scripted response".into())
        })
    }
    #[allow(
        clippy::too_many_arguments,
        reason = "Streaming transport carries prompt delivery metadata and callbacks"
    )]
    fn send_message<'a>(
        &'a self,
        session_id: i64,
        content: &'a str,
        model: Option<&'a str>,
        _editor_context: Option<&'a EditorContext>,
        queued_prompt: Option<openwebide_core::QueuedPromptKey>,
        signal: Option<&'a AbortSignal>,
        mut on_event: Box<dyn FnMut(RunEvent) + 'a>,
    ) -> LocalBoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            if let Some(key) = queued_prompt {
                let message = self.consume_queued_prompt(session_id, key, content).await?;
                on_event(RunEvent::Message { message });
            }

            self.calls.borrow_mut().push(Call::SendMessage {
                session: session_id,
                content: content.into(),
                model: model.map(str::to_string),
            });
            let events = self
                .scripted_events
                .borrow_mut()
                .pop_front()
                .unwrap_or_default();
            if queued_prompt.is_none() {
                if let Some(message) = events.iter().find_map(|event| match event {
                    RunEvent::Message { message } if message.role == Role::User => Some(message),
                    _ => None,
                }) {
                    self.messages
                        .borrow_mut()
                        .entry(session_id)
                        .or_default()
                        .push(ConversationEntry::Message(message.clone()));
                } else {
                    self.persist_message(session_id, Role::User, content, None, None)
                        .await?;
                }
            }
            let mut assistant = None;
            let mut deltas = String::new();
            let mut interrupted = false;
            for event in events {
                if signal.is_some_and(AbortSignal::aborted) {
                    return Err("aborted".into());
                }
                match &event {
                    RunEvent::Delta { content: delta } => deltas.push_str(delta),
                    RunEvent::Message { message } | RunEvent::Done { message }
                        if message.role == Role::Assistant =>
                    {
                        assistant = Some(message.clone());
                    }
                    RunEvent::Cancelled | RunEvent::Error { .. } => interrupted = true,
                    _ => {}
                }
                on_event(event);
            }
            if let Some(message) = assistant {
                self.messages
                    .borrow_mut()
                    .entry(session_id)
                    .or_default()
                    .push(ConversationEntry::Message(message));
            } else if !interrupted && !deltas.is_empty() {
                self.persist_message(session_id, Role::Assistant, &deltas, None, None)
                    .await?;
            }
            Ok(())
        })
    }
}
