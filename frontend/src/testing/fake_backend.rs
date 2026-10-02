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
    pub tool_sources: RefCell<BTreeMap<(i64, String), bool>>,
    pub completion_error: RefCell<Option<String>>,
    pub persisted_edits: RefCell<BTreeMap<(i64, String), PersistedEdit>>,
    pub resolution_error: RefCell<Option<String>>,
    pub sessions: RefCell<Vec<ChatSession>>,
    pub messages: RefCell<BTreeMap<i64, Vec<ConversationEntry>>>,
    pub projects: RefCell<Vec<Project>>,
    pub project_delete_error: RefCell<Option<String>>,
    pub browse_entries: RefCell<BTreeMap<String, Vec<FileEntry>>>,
    pub files: RefCell<BTreeMap<(i64, String), String>>,
    pub directories: RefCell<BTreeSet<(i64, String)>>,
    pub models: RefCell<Vec<ModelInfo>>,
    pub connections: RefCell<Vec<Connection>>,
    pub system_prompts: RefCell<Vec<SystemPrompt>>,
    pub settings: RefCell<BTreeMap<String, String>>,
    pub settings_load_error: RefCell<Option<String>>,
    pub settings_load_results: RefCell<VecDeque<SettingsLoad>>,
    pub history_save_results:
        RefCell<VecDeque<futures::channel::oneshot::Receiver<Result<(), String>>>>,
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
            let prefix = if path.is_empty() {
                String::new()
            } else {
                format!("{}/", path.trim_end_matches('/'))
            };
            let files = self.files.borrow();
            let directories = self.directories.borrow();
            let mut entries = BTreeMap::new();
            for (project, child) in files.keys().chain(directories.iter()) {
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
            self.calls
                .borrow_mut()
                .push(Call::DeleteFile { path: path.into() });
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
        _project_id: Option<i64>,
    ) -> LocalBoxFuture<'a, Result<GitRepoStatus, String>> {
        Box::pin(async move {
            self.calls.borrow_mut().push(Call::Request {
                method: "git_status",
            });
            Err("git_status has no scripted response".into())
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
            Ok(String::new())
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
            Ok(item.clone())
        })
    }
    fn delete_session<'a>(&'a self, id: i64) -> LocalBoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            self.calls.borrow_mut().push(Call::Request {
                method: "delete_session",
            });
            self.sessions.borrow_mut().retain(|item| item.id != id);
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
            self.tool_sources
                .borrow_mut()
                .entry((session_id, tool_call_id.into()))
                .or_insert(false);
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
            if let Some(error) = self.resolution_error.borrow().as_ref() {
                return Err(error.clone());
            }
            if request.decision == EditDecision::Pending || request.revision <= 0 {
                return Err("invalid edit resolution".into());
            }
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
            Ok(edit.clone())
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
    fn send_message<'a>(
        &'a self,
        session_id: i64,
        content: &'a str,
        model: Option<&'a str>,
        _editor_context: Option<&'a EditorContext>,
        signal: Option<&'a AbortSignal>,
        mut on_event: Box<dyn FnMut(RunEvent) + 'a>,
    ) -> LocalBoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
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
