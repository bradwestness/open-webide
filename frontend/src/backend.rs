use crate::api::BackendApi;
use futures::future::LocalBoxFuture;
use leptos::prelude::{LocalStorage, RwSignal, StoredValue};
use openwebide_core::{
    ChatCompletion, ChatMessage, ChatRequest, ChatSession, Connection, ConversationEntry,
    EditorContext, FileDiff, FileEntry, GitBranchInfo, GitCheckoutRequest, GitCheckoutResult,
    GitCommitRequest, GitCommitResult, GitRepoStatus, GitSyncRequest, GitSyncResult, Health,
    ModelInfo, PersistedEdit, Project, ProviderKind, ResolveEditRequest, Role, RunEvent, SearchHit,
    SystemPrompt, TurnTelemetry, User, WebSearchResult, WorkspaceMode, vfs::SearchOptions,
};
use std::rc::Rc;
use web_sys::AbortSignal;

pub type Api = StoredValue<Rc<dyn Backend>, LocalStorage>;

pub trait Backend {
    fn session_expired(&self) -> RwSignal<bool>;
    fn register<'a>(
        &'a self,
        username: &'a str,
        password: &'a str,
    ) -> LocalBoxFuture<'a, Result<User, String>>;
    fn login<'a>(
        &'a self,
        username: &'a str,
        password: &'a str,
    ) -> LocalBoxFuture<'a, Result<User, String>>;
    fn me<'a>(&'a self) -> LocalBoxFuture<'a, Result<User, String>>;
    fn logout<'a>(&'a self) -> LocalBoxFuture<'a, Result<(), String>>;
    fn bridge_token<'a>(&'a self) -> LocalBoxFuture<'a, Result<(String, i64), String>>;
    fn health<'a>(&'a self) -> LocalBoxFuture<'a, Result<Health, String>>;
    fn preview_server<'a>(
        &'a self,
        probe: &'a openwebide_core::ModelProbe,
    ) -> LocalBoxFuture<'a, Result<openwebide_core::ServerDiscovery, String>>;
    fn save_model_setup<'a>(
        &'a self,
        probe: &'a openwebide_core::ModelProbe,
        profiles: &'a [openwebide_core::ModelProfile],
    ) -> LocalBoxFuture<'a, Result<(Connection, openwebide_core::ModelSetup), String>>;
    fn test_model<'a>(
        &'a self,
        probe: &'a openwebide_core::ModelProbe,
    ) -> LocalBoxFuture<'a, Result<openwebide_core::ModelTestResult, String>>;
    fn preview_model<'a>(
        &'a self,
        probe: &'a openwebide_core::ModelProbe,
    ) -> LocalBoxFuture<'a, Result<openwebide_core::ModelDetection, String>>;
    fn detect_model<'a>(
        &'a self,
        id: i64,
        model: &'a str,
    ) -> LocalBoxFuture<'a, Result<openwebide_core::ModelDetection, String>>;
    fn inspect_server<'a>(
        &'a self,
        base_url: &'a str,
        kind: Option<ProviderKind>,
    ) -> LocalBoxFuture<'a, Result<openwebide_core::ServerDiscovery, String>>;
    fn discover_servers<'a>(
        &'a self,
    ) -> LocalBoxFuture<'a, Result<Vec<openwebide_core::ServerDiscovery>, String>>;
    fn model_runtime<'a>(
        &'a self,
        id: i64,
        model: Option<&'a str>,
    ) -> LocalBoxFuture<'a, Result<openwebide_core::ModelRuntime, String>>;
    fn model_setup<'a>(&'a self)
    -> LocalBoxFuture<'a, Result<openwebide_core::ModelSetup, String>>;
    fn save_model_defaults<'a>(
        &'a self,
        defaults: &'a openwebide_core::ModelDefaults,
    ) -> LocalBoxFuture<'a, Result<openwebide_core::ModelSetup, String>>;
    fn save_model_profile<'a>(
        &'a self,
        profile: &'a openwebide_core::ModelProfile,
    ) -> LocalBoxFuture<'a, Result<openwebide_core::ModelSetup, String>>;
    fn server_settings<'a>(
        &'a self,
        id: i64,
    ) -> LocalBoxFuture<'a, Result<openwebide_core::ServerSettings, String>>;
    fn save_server_settings<'a>(
        &'a self,
        id: i64,
        update: &'a openwebide_core::ServerSettingsUpdate,
    ) -> LocalBoxFuture<'a, Result<openwebide_core::ServerSettings, String>>;
    fn list_connections<'a>(&'a self) -> LocalBoxFuture<'a, Result<Vec<Connection>, String>>;
    fn create_connection<'a>(
        &'a self,
        name: &'a str,
        kind: ProviderKind,
        base_url: &'a str,
        model: Option<&'a str>,
        context_limit: Option<usize>,
    ) -> LocalBoxFuture<'a, Result<Connection, String>>;
    fn update_connection<'a>(
        &'a self,
        connection: &'a Connection,
    ) -> LocalBoxFuture<'a, Result<Connection, String>>;
    fn delete_connection<'a>(&'a self, id: i64) -> LocalBoxFuture<'a, Result<(), String>>;
    fn list_sessions<'a>(&'a self) -> LocalBoxFuture<'a, Result<Vec<ChatSession>, String>>;
    fn list_models<'a>(
        &'a self,
        connection_id: i64,
    ) -> LocalBoxFuture<'a, Result<Vec<ModelInfo>, String>>;
    fn list_system_prompts<'a>(&'a self) -> LocalBoxFuture<'a, Result<Vec<SystemPrompt>, String>>;
    fn create_system_prompt<'a>(
        &'a self,
        name: &'a str,
        content: &'a str,
    ) -> LocalBoxFuture<'a, Result<SystemPrompt, String>>;
    fn update_system_prompt<'a>(
        &'a self,
        id: i64,
        name: &'a str,
        content: &'a str,
    ) -> LocalBoxFuture<'a, Result<SystemPrompt, String>>;
    fn delete_system_prompt<'a>(&'a self, id: i64) -> LocalBoxFuture<'a, Result<(), String>>;
    fn get_settings<'a>(
        &'a self,
    ) -> LocalBoxFuture<'a, Result<std::collections::BTreeMap<String, String>, String>>;
    fn set_setting<'a>(
        &'a self,
        key: &'a str,
        value: &'a str,
    ) -> LocalBoxFuture<'a, Result<(), String>>;
    fn startup_context(
        &self,
        project: i64,
        tools: bool,
    ) -> LocalBoxFuture<'_, Result<String, String>>;
    fn create_session<'a>(
        &'a self,
        name: &'a str,
        connection_id: Option<i64>,
        system_prompt_id: Option<i64>,
        project_id: Option<i64>,
    ) -> LocalBoxFuture<'a, Result<ChatSession, String>>;
    fn list_projects<'a>(&'a self) -> LocalBoxFuture<'a, Result<Vec<Project>, String>>;
    fn create_project<'a>(
        &'a self,
        name: &'a str,
        mode: WorkspaceMode,
        path: Option<String>,
    ) -> LocalBoxFuture<'a, Result<Project, String>>;
    fn rename_project<'a>(
        &'a self,
        id: i64,
        name: &'a str,
    ) -> LocalBoxFuture<'a, Result<Project, String>>;
    fn delete_project<'a>(&'a self, id: i64) -> LocalBoxFuture<'a, Result<(), String>>;
    fn list_files<'a>(
        &'a self,
        project_id: i64,
        path: &'a str,
    ) -> LocalBoxFuture<'a, Result<Vec<FileEntry>, String>>;
    fn read_file_object_url<'a>(
        &'a self,
        project_id: i64,
        path: &'a str,
    ) -> LocalBoxFuture<'a, Result<String, String>>;
    fn read_file<'a>(
        &'a self,
        project_id: i64,
        path: &'a str,
    ) -> LocalBoxFuture<'a, Result<String, String>>;
    fn canonical_file_path<'a>(
        &'a self,
        _project: i64,
        path: &'a str,
    ) -> LocalBoxFuture<'a, Result<String, String>> {
        Box::pin(
            async move { openwebide_core::vfs::workspace_path(path).map_err(|e| e.to_string()) },
        )
    }
    fn read_file_bytes<'a>(
        &'a self,
        project: i64,
        path: &'a str,
    ) -> LocalBoxFuture<'a, Result<Vec<u8>, String>> {
        Box::pin(async move { self.read_file(project, path).await.map(String::into_bytes) })
    }
    fn read_file_lossy<'a>(
        &'a self,
        project_id: i64,
        path: &'a str,
    ) -> LocalBoxFuture<'a, Result<String, String>>;
    fn write_file<'a>(
        &'a self,
        project_id: i64,
        path: &'a str,
        content: &'a str,
    ) -> LocalBoxFuture<'a, Result<(), String>>;
    fn write_file_bytes<'a>(
        &'a self,
        project: i64,
        path: &'a str,
        bytes: &'a [u8],
    ) -> LocalBoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            let text = std::str::from_utf8(bytes).map_err(|e| e.to_string())?;
            self.write_file(project, path, text).await
        })
    }
    fn copy_file<'a>(
        &'a self,
        project_id: i64,
        from: &'a str,
        to: &'a str,
    ) -> LocalBoxFuture<'a, Result<(), String>>;
    fn create_file<'a>(
        &'a self,
        project_id: i64,
        path: &'a str,
        is_dir: bool,
    ) -> LocalBoxFuture<'a, Result<(), String>>;
    fn delete_file<'a>(
        &'a self,
        project_id: i64,
        path: &'a str,
    ) -> LocalBoxFuture<'a, Result<(), String>>;
    fn browse<'a>(&'a self, path: &'a str) -> LocalBoxFuture<'a, Result<Vec<FileEntry>, String>>;
    fn search_content<'a>(
        &'a self,
        project_id: i64,
        query: &'a str,
        path: &'a str,
        opts: SearchOptions,
    ) -> LocalBoxFuture<'a, Result<Vec<SearchHit>, String>>;
    fn git_status<'a>(
        &'a self,
        project_id: Option<i64>,
    ) -> LocalBoxFuture<'a, Result<GitRepoStatus, String>>;
    fn git_diff<'a>(
        &'a self,
        project_id: Option<i64>,
        path: Option<&'a str>,
    ) -> LocalBoxFuture<'a, Result<String, String>>;
    fn git_file_head<'a>(
        &'a self,
        project_id: Option<i64>,
        path: &'a str,
    ) -> LocalBoxFuture<'a, Result<String, String>>;
    fn git_branches<'a>(
        &'a self,
        project_id: Option<i64>,
    ) -> LocalBoxFuture<'a, Result<Vec<GitBranchInfo>, String>>;
    fn git_commit<'a>(
        &'a self,
        project_id: Option<i64>,
        req: &'a GitCommitRequest,
    ) -> LocalBoxFuture<'a, Result<GitCommitResult, String>>;
    fn git_checkout<'a>(
        &'a self,
        project_id: Option<i64>,
        req: &'a GitCheckoutRequest,
    ) -> LocalBoxFuture<'a, Result<GitCheckoutResult, String>>;
    fn git_sync<'a>(
        &'a self,
        project_id: Option<i64>,
        req: &'a GitSyncRequest,
    ) -> LocalBoxFuture<'a, Result<GitSyncResult, String>>;
    fn set_session_connection(
        &self,
        id: i64,
        connection_id: i64,
    ) -> LocalBoxFuture<'_, Result<ChatSession, String>>;
    fn rename_session<'a>(
        &'a self,
        id: i64,
        name: &'a str,
    ) -> LocalBoxFuture<'a, Result<ChatSession, String>>;
    fn delete_session<'a>(&'a self, id: i64) -> LocalBoxFuture<'a, Result<(), String>>;
    fn cancel_session<'a>(&'a self, session_id: i64) -> LocalBoxFuture<'a, Result<(), String>>;
    fn set_permission<'a>(
        &'a self,
        session_id: i64,
        tool_call_id: &'a str,
        approved: bool,
    ) -> LocalBoxFuture<'a, Result<(), String>>;
    fn list_messages<'a>(
        &'a self,
        session_id: i64,
    ) -> LocalBoxFuture<'a, Result<Vec<ConversationEntry>, String>>;
    fn prepare_rewind(
        &self,
        session: i64,
        message: i64,
    ) -> LocalBoxFuture<'_, Result<openwebide_core::RewindPlan, String>>;
    fn complete_rewind(
        &self,
        session: i64,
        message: i64,
    ) -> LocalBoxFuture<'_, Result<Vec<ConversationEntry>, String>>;
    fn model_complete<'a>(
        &'a self,
        request: &'a ChatRequest,
    ) -> LocalBoxFuture<'a, Result<ChatCompletion, String>> {
        self.chat_tools(request)
    }
    fn model_tokens<'a>(
        &'a self,
        _request: &'a ChatRequest,
    ) -> LocalBoxFuture<'a, Result<Option<usize>, String>> {
        Box::pin(async { Ok(None) })
    }
    fn approval_check<'a>(
        &'a self,
        session: i64,
        check: &'a openwebide_core::ApprovalCheck,
    ) -> LocalBoxFuture<'a, Result<openwebide_core::ApprovalDecision, String>>;
    fn chat_tools<'a>(
        &'a self,
        request: &'a ChatRequest,
    ) -> LocalBoxFuture<'a, Result<ChatCompletion, String>>;
    fn persist_message<'a>(
        &'a self,
        session_id: i64,
        role: Role,
        content: &'a str,
        usage: Option<&'a TurnTelemetry>,
        tool_calls: Option<&'a [openwebide_core::ToolCall]>,
    ) -> LocalBoxFuture<'a, Result<ChatMessage, String>>;
    fn model_context<'a>(
        &'a self,
        connection_id: i64,
        model: Option<&'a str>,
    ) -> LocalBoxFuture<'a, Result<Option<usize>, String>>;
    #[allow(clippy::too_many_arguments)]
    fn upsert_tool_step<'a>(
        &'a self,
        session_id: i64,
        anchor_message_id: i64,
        tool_call_id: &'a str,
        name: &'a str,
        summary: &'a str,
        diff: Option<&'a FileDiff>,
    ) -> LocalBoxFuture<'a, Result<(), String>>;
    fn save_project_checkpoint<'a>(
        &'a self,
        _session: i64,
        _id: &'a str,
        _checkpoint: &'a openwebide_core::rewind::ProjectCheckpoint,
    ) -> LocalBoxFuture<'a, Result<(), String>> {
        Box::pin(async { Ok(()) })
    }
    fn complete_tool_step<'a>(
        &'a self,
        session_id: i64,
        tool_call_id: &'a str,
        ok: bool,
        result_summary: &'a str,
        diff: Option<&'a FileDiff>,
    ) -> LocalBoxFuture<'a, Result<(), String>>;
    fn list_pending_edits(
        &self,
        project_id: i64,
    ) -> LocalBoxFuture<'_, Result<Vec<PersistedEdit>, String>>;
    fn resolve_pending_edit<'a>(
        &'a self,
        project_id: i64,
        request: &'a ResolveEditRequest,
    ) -> LocalBoxFuture<'a, Result<PersistedEdit, String>>;
    fn web_search<'a>(
        &'a self,
        query: &'a str,
        limit: usize,
    ) -> LocalBoxFuture<'a, Result<Vec<WebSearchResult>, String>>;
    fn fetch_web_page<'a>(
        &'a self,
        target_url: &'a str,
    ) -> LocalBoxFuture<'a, Result<String, String>>;
    fn send_message<'a>(
        &'a self,
        session_id: i64,
        content: &'a str,
        model: Option<&'a str>,
        editor_context: Option<&'a EditorContext>,
        signal: Option<&'a AbortSignal>,
        on_event: Box<dyn FnMut(RunEvent) + 'a>,
    ) -> LocalBoxFuture<'a, Result<(), String>>;
}

impl Backend for BackendApi {
    fn session_expired(&self) -> RwSignal<bool> {
        self.session_expired
    }
    fn register<'a>(
        &'a self,
        username: &'a str,
        password: &'a str,
    ) -> LocalBoxFuture<'a, Result<User, String>> {
        Box::pin(BackendApi::register(self, username, password))
    }
    fn login<'a>(
        &'a self,
        username: &'a str,
        password: &'a str,
    ) -> LocalBoxFuture<'a, Result<User, String>> {
        Box::pin(BackendApi::login(self, username, password))
    }
    fn me<'a>(&'a self) -> LocalBoxFuture<'a, Result<User, String>> {
        Box::pin(BackendApi::me(self))
    }
    fn logout<'a>(&'a self) -> LocalBoxFuture<'a, Result<(), String>> {
        Box::pin(BackendApi::logout(self))
    }
    fn bridge_token<'a>(&'a self) -> LocalBoxFuture<'a, Result<(String, i64), String>> {
        Box::pin(BackendApi::bridge_token(self))
    }
    fn health<'a>(&'a self) -> LocalBoxFuture<'a, Result<Health, String>> {
        Box::pin(BackendApi::health(self))
    }
    fn preview_server<'a>(
        &'a self,
        probe: &'a openwebide_core::ModelProbe,
    ) -> LocalBoxFuture<'a, Result<openwebide_core::ServerDiscovery, String>> {
        Box::pin(BackendApi::preview_server(self, probe))
    }
    fn save_model_setup<'a>(
        &'a self,
        probe: &'a openwebide_core::ModelProbe,
        profiles: &'a [openwebide_core::ModelProfile],
    ) -> LocalBoxFuture<'a, Result<(Connection, openwebide_core::ModelSetup), String>> {
        Box::pin(BackendApi::save_model_setup(self, probe, profiles))
    }
    fn test_model<'a>(
        &'a self,
        probe: &'a openwebide_core::ModelProbe,
    ) -> LocalBoxFuture<'a, Result<openwebide_core::ModelTestResult, String>> {
        Box::pin(BackendApi::test_model(self, probe))
    }
    fn preview_model<'a>(
        &'a self,
        probe: &'a openwebide_core::ModelProbe,
    ) -> LocalBoxFuture<'a, Result<openwebide_core::ModelDetection, String>> {
        Box::pin(BackendApi::preview_model(self, probe))
    }
    fn detect_model<'a>(
        &'a self,
        id: i64,
        model: &'a str,
    ) -> LocalBoxFuture<'a, Result<openwebide_core::ModelDetection, String>> {
        Box::pin(BackendApi::detect_model(self, id, model))
    }
    fn inspect_server<'a>(
        &'a self,
        base_url: &'a str,
        kind: Option<ProviderKind>,
    ) -> LocalBoxFuture<'a, Result<openwebide_core::ServerDiscovery, String>> {
        Box::pin(BackendApi::inspect_server(self, base_url, kind))
    }
    fn discover_servers<'a>(
        &'a self,
    ) -> LocalBoxFuture<'a, Result<Vec<openwebide_core::ServerDiscovery>, String>> {
        Box::pin(BackendApi::discover_servers(self))
    }
    fn model_runtime<'a>(
        &'a self,
        id: i64,
        model: Option<&'a str>,
    ) -> LocalBoxFuture<'a, Result<openwebide_core::ModelRuntime, String>> {
        Box::pin(BackendApi::model_runtime(self, id, model))
    }
    fn model_setup<'a>(
        &'a self,
    ) -> LocalBoxFuture<'a, Result<openwebide_core::ModelSetup, String>> {
        Box::pin(BackendApi::model_setup(self))
    }
    fn save_model_defaults<'a>(
        &'a self,
        defaults: &'a openwebide_core::ModelDefaults,
    ) -> LocalBoxFuture<'a, Result<openwebide_core::ModelSetup, String>> {
        Box::pin(BackendApi::save_model_defaults(self, defaults))
    }
    fn save_model_profile<'a>(
        &'a self,
        profile: &'a openwebide_core::ModelProfile,
    ) -> LocalBoxFuture<'a, Result<openwebide_core::ModelSetup, String>> {
        Box::pin(BackendApi::save_model_profile(self, profile))
    }
    fn server_settings<'a>(
        &'a self,
        id: i64,
    ) -> LocalBoxFuture<'a, Result<openwebide_core::ServerSettings, String>> {
        Box::pin(BackendApi::server_settings(self, id))
    }
    fn save_server_settings<'a>(
        &'a self,
        id: i64,
        update: &'a openwebide_core::ServerSettingsUpdate,
    ) -> LocalBoxFuture<'a, Result<openwebide_core::ServerSettings, String>> {
        Box::pin(BackendApi::save_server_settings(self, id, update))
    }
    fn list_connections<'a>(&'a self) -> LocalBoxFuture<'a, Result<Vec<Connection>, String>> {
        Box::pin(BackendApi::list_connections(self))
    }
    fn create_connection<'a>(
        &'a self,
        name: &'a str,
        kind: ProviderKind,
        base_url: &'a str,
        model: Option<&'a str>,
        context_limit: Option<usize>,
    ) -> LocalBoxFuture<'a, Result<Connection, String>> {
        Box::pin(BackendApi::create_connection(
            self,
            name,
            kind,
            base_url,
            model,
            context_limit,
        ))
    }
    fn update_connection<'a>(
        &'a self,
        connection: &'a Connection,
    ) -> LocalBoxFuture<'a, Result<Connection, String>> {
        Box::pin(BackendApi::update_connection(self, connection))
    }
    fn delete_connection<'a>(&'a self, id: i64) -> LocalBoxFuture<'a, Result<(), String>> {
        Box::pin(BackendApi::delete_connection(self, id))
    }
    fn list_sessions<'a>(&'a self) -> LocalBoxFuture<'a, Result<Vec<ChatSession>, String>> {
        Box::pin(BackendApi::list_sessions(self))
    }
    fn list_models<'a>(
        &'a self,
        connection_id: i64,
    ) -> LocalBoxFuture<'a, Result<Vec<ModelInfo>, String>> {
        Box::pin(BackendApi::list_models(self, connection_id))
    }
    fn list_system_prompts<'a>(&'a self) -> LocalBoxFuture<'a, Result<Vec<SystemPrompt>, String>> {
        Box::pin(BackendApi::list_system_prompts(self))
    }
    fn create_system_prompt<'a>(
        &'a self,
        name: &'a str,
        content: &'a str,
    ) -> LocalBoxFuture<'a, Result<SystemPrompt, String>> {
        Box::pin(BackendApi::create_system_prompt(self, name, content))
    }
    fn update_system_prompt<'a>(
        &'a self,
        id: i64,
        name: &'a str,
        content: &'a str,
    ) -> LocalBoxFuture<'a, Result<SystemPrompt, String>> {
        Box::pin(BackendApi::update_system_prompt(self, id, name, content))
    }
    fn delete_system_prompt<'a>(&'a self, id: i64) -> LocalBoxFuture<'a, Result<(), String>> {
        Box::pin(BackendApi::delete_system_prompt(self, id))
    }
    fn get_settings<'a>(
        &'a self,
    ) -> LocalBoxFuture<'a, Result<std::collections::BTreeMap<String, String>, String>> {
        Box::pin(BackendApi::get_settings(self))
    }
    fn set_setting<'a>(
        &'a self,
        key: &'a str,
        value: &'a str,
    ) -> LocalBoxFuture<'a, Result<(), String>> {
        Box::pin(BackendApi::set_setting(self, key, value))
    }
    fn startup_context(
        &self,
        project: i64,
        tools: bool,
    ) -> LocalBoxFuture<'_, Result<String, String>> {
        Box::pin(BackendApi::startup_context(self, project, tools))
    }
    fn create_session<'a>(
        &'a self,
        name: &'a str,
        connection_id: Option<i64>,
        system_prompt_id: Option<i64>,
        project_id: Option<i64>,
    ) -> LocalBoxFuture<'a, Result<ChatSession, String>> {
        Box::pin(BackendApi::create_session(
            self,
            name,
            connection_id,
            system_prompt_id,
            project_id,
        ))
    }
    fn list_projects<'a>(&'a self) -> LocalBoxFuture<'a, Result<Vec<Project>, String>> {
        Box::pin(BackendApi::list_projects(self))
    }
    fn create_project<'a>(
        &'a self,
        name: &'a str,
        mode: WorkspaceMode,
        path: Option<String>,
    ) -> LocalBoxFuture<'a, Result<Project, String>> {
        Box::pin(BackendApi::create_project(self, name, mode, path))
    }
    fn rename_project<'a>(
        &'a self,
        id: i64,
        name: &'a str,
    ) -> LocalBoxFuture<'a, Result<Project, String>> {
        Box::pin(BackendApi::rename_project(self, id, name))
    }
    fn delete_project<'a>(&'a self, id: i64) -> LocalBoxFuture<'a, Result<(), String>> {
        Box::pin(BackendApi::delete_project(self, id))
    }
    fn list_files<'a>(
        &'a self,
        project_id: i64,
        path: &'a str,
    ) -> LocalBoxFuture<'a, Result<Vec<FileEntry>, String>> {
        Box::pin(BackendApi::list_files(self, project_id, path))
    }
    fn read_file_object_url<'a>(
        &'a self,
        project_id: i64,
        path: &'a str,
    ) -> LocalBoxFuture<'a, Result<String, String>> {
        Box::pin(BackendApi::read_file_object_url(self, project_id, path))
    }
    fn read_file<'a>(
        &'a self,
        project_id: i64,
        path: &'a str,
    ) -> LocalBoxFuture<'a, Result<String, String>> {
        Box::pin(BackendApi::read_file(self, project_id, path))
    }
    fn canonical_file_path<'a>(
        &'a self,
        project: i64,
        path: &'a str,
    ) -> LocalBoxFuture<'a, Result<String, String>> {
        Box::pin(BackendApi::canonical_file_path(self, project, path))
    }
    fn read_file_bytes<'a>(
        &'a self,
        project: i64,
        path: &'a str,
    ) -> LocalBoxFuture<'a, Result<Vec<u8>, String>> {
        Box::pin(BackendApi::read_file_bytes(self, project, path))
    }
    fn read_file_lossy<'a>(
        &'a self,
        project_id: i64,
        path: &'a str,
    ) -> LocalBoxFuture<'a, Result<String, String>> {
        Box::pin(BackendApi::read_file_lossy(self, project_id, path))
    }
    fn write_file<'a>(
        &'a self,
        project_id: i64,
        path: &'a str,
        content: &'a str,
    ) -> LocalBoxFuture<'a, Result<(), String>> {
        Box::pin(BackendApi::write_file(self, project_id, path, content))
    }
    fn write_file_bytes<'a>(
        &'a self,
        project: i64,
        path: &'a str,
        bytes: &'a [u8],
    ) -> LocalBoxFuture<'a, Result<(), String>> {
        Box::pin(BackendApi::write_file_bytes(self, project, path, bytes))
    }
    fn copy_file<'a>(
        &'a self,
        project_id: i64,
        from: &'a str,
        to: &'a str,
    ) -> LocalBoxFuture<'a, Result<(), String>> {
        Box::pin(BackendApi::copy_file(self, project_id, from, to))
    }
    fn create_file<'a>(
        &'a self,
        project_id: i64,
        path: &'a str,
        is_dir: bool,
    ) -> LocalBoxFuture<'a, Result<(), String>> {
        Box::pin(BackendApi::create_file(self, project_id, path, is_dir))
    }
    fn delete_file<'a>(
        &'a self,
        project_id: i64,
        path: &'a str,
    ) -> LocalBoxFuture<'a, Result<(), String>> {
        Box::pin(BackendApi::delete_file(self, project_id, path))
    }
    fn browse<'a>(&'a self, path: &'a str) -> LocalBoxFuture<'a, Result<Vec<FileEntry>, String>> {
        Box::pin(BackendApi::browse(self, path))
    }
    fn search_content<'a>(
        &'a self,
        project_id: i64,
        query: &'a str,
        path: &'a str,
        opts: SearchOptions,
    ) -> LocalBoxFuture<'a, Result<Vec<SearchHit>, String>> {
        Box::pin(BackendApi::search_content(
            self, project_id, query, path, opts,
        ))
    }
    fn git_status<'a>(
        &'a self,
        project_id: Option<i64>,
    ) -> LocalBoxFuture<'a, Result<GitRepoStatus, String>> {
        Box::pin(BackendApi::git_status(self, project_id))
    }
    fn git_diff<'a>(
        &'a self,
        project_id: Option<i64>,
        path: Option<&'a str>,
    ) -> LocalBoxFuture<'a, Result<String, String>> {
        Box::pin(BackendApi::git_diff(self, project_id, path))
    }
    fn git_file_head<'a>(
        &'a self,
        project_id: Option<i64>,
        path: &'a str,
    ) -> LocalBoxFuture<'a, Result<String, String>> {
        Box::pin(BackendApi::git_file_head(self, project_id, path))
    }
    fn git_branches<'a>(
        &'a self,
        project_id: Option<i64>,
    ) -> LocalBoxFuture<'a, Result<Vec<GitBranchInfo>, String>> {
        Box::pin(BackendApi::git_branches(self, project_id))
    }
    fn git_commit<'a>(
        &'a self,
        project_id: Option<i64>,
        req: &'a GitCommitRequest,
    ) -> LocalBoxFuture<'a, Result<GitCommitResult, String>> {
        Box::pin(BackendApi::git_commit(self, project_id, req))
    }
    fn git_checkout<'a>(
        &'a self,
        project_id: Option<i64>,
        req: &'a GitCheckoutRequest,
    ) -> LocalBoxFuture<'a, Result<GitCheckoutResult, String>> {
        Box::pin(BackendApi::git_checkout(self, project_id, req))
    }
    fn git_sync<'a>(
        &'a self,
        project_id: Option<i64>,
        req: &'a GitSyncRequest,
    ) -> LocalBoxFuture<'a, Result<GitSyncResult, String>> {
        Box::pin(BackendApi::git_sync(self, project_id, req))
    }
    fn set_session_connection(
        &self,
        id: i64,
        connection_id: i64,
    ) -> LocalBoxFuture<'_, Result<ChatSession, String>> {
        Box::pin(BackendApi::set_session_connection(self, id, connection_id))
    }
    fn rename_session<'a>(
        &'a self,
        id: i64,
        name: &'a str,
    ) -> LocalBoxFuture<'a, Result<ChatSession, String>> {
        Box::pin(BackendApi::rename_session(self, id, name))
    }
    fn delete_session<'a>(&'a self, id: i64) -> LocalBoxFuture<'a, Result<(), String>> {
        Box::pin(BackendApi::delete_session(self, id))
    }
    fn cancel_session<'a>(&'a self, session_id: i64) -> LocalBoxFuture<'a, Result<(), String>> {
        Box::pin(BackendApi::cancel_session(self, session_id))
    }
    fn set_permission<'a>(
        &'a self,
        session_id: i64,
        tool_call_id: &'a str,
        approved: bool,
    ) -> LocalBoxFuture<'a, Result<(), String>> {
        Box::pin(BackendApi::set_permission(
            self,
            session_id,
            tool_call_id,
            approved,
        ))
    }
    fn list_messages<'a>(
        &'a self,
        session_id: i64,
    ) -> LocalBoxFuture<'a, Result<Vec<ConversationEntry>, String>> {
        Box::pin(BackendApi::list_messages(self, session_id))
    }
    fn model_complete<'a>(
        &'a self,
        request: &'a ChatRequest,
    ) -> LocalBoxFuture<'a, Result<ChatCompletion, String>> {
        Box::pin(BackendApi::model_complete(self, request))
    }
    fn model_tokens<'a>(
        &'a self,
        request: &'a ChatRequest,
    ) -> LocalBoxFuture<'a, Result<Option<usize>, String>> {
        Box::pin(BackendApi::model_tokens(self, request))
    }
    fn prepare_rewind(
        &self,
        session: i64,
        message: i64,
    ) -> LocalBoxFuture<'_, Result<openwebide_core::RewindPlan, String>> {
        Box::pin(BackendApi::prepare_rewind(self, session, message))
    }
    fn complete_rewind(
        &self,
        session: i64,
        message: i64,
    ) -> LocalBoxFuture<'_, Result<Vec<ConversationEntry>, String>> {
        Box::pin(BackendApi::complete_rewind(self, session, message))
    }
    fn approval_check<'a>(
        &'a self,
        session: i64,
        check: &'a openwebide_core::ApprovalCheck,
    ) -> LocalBoxFuture<'a, Result<openwebide_core::ApprovalDecision, String>> {
        Box::pin(BackendApi::approval_check(self, session, check))
    }
    fn chat_tools<'a>(
        &'a self,
        request: &'a ChatRequest,
    ) -> LocalBoxFuture<'a, Result<ChatCompletion, String>> {
        Box::pin(BackendApi::chat_tools(self, request))
    }
    fn persist_message<'a>(
        &'a self,
        session_id: i64,
        role: Role,
        content: &'a str,
        usage: Option<&'a TurnTelemetry>,
        tool_calls: Option<&'a [openwebide_core::ToolCall]>,
    ) -> LocalBoxFuture<'a, Result<ChatMessage, String>> {
        Box::pin(BackendApi::persist_message(
            self, session_id, role, content, usage, tool_calls,
        ))
    }
    fn model_context<'a>(
        &'a self,
        connection_id: i64,
        model: Option<&'a str>,
    ) -> LocalBoxFuture<'a, Result<Option<usize>, String>> {
        Box::pin(BackendApi::model_context(self, connection_id, model))
    }
    #[allow(clippy::too_many_arguments)]
    fn upsert_tool_step<'a>(
        &'a self,
        session_id: i64,
        anchor_message_id: i64,
        tool_call_id: &'a str,
        name: &'a str,
        summary: &'a str,
        diff: Option<&'a FileDiff>,
    ) -> LocalBoxFuture<'a, Result<(), String>> {
        Box::pin(BackendApi::upsert_tool_step(
            self,
            session_id,
            anchor_message_id,
            tool_call_id,
            name,
            summary,
            diff,
        ))
    }
    fn save_project_checkpoint<'a>(
        &'a self,
        session: i64,
        id: &'a str,
        checkpoint: &'a openwebide_core::rewind::ProjectCheckpoint,
    ) -> LocalBoxFuture<'a, Result<(), String>> {
        Box::pin(BackendApi::save_project_checkpoint(
            self, session, id, checkpoint,
        ))
    }
    fn complete_tool_step<'a>(
        &'a self,
        session_id: i64,
        tool_call_id: &'a str,
        ok: bool,
        result_summary: &'a str,
        diff: Option<&'a FileDiff>,
    ) -> LocalBoxFuture<'a, Result<(), String>> {
        Box::pin(BackendApi::complete_tool_step(
            self,
            session_id,
            tool_call_id,
            ok,
            result_summary,
            diff,
        ))
    }
    fn list_pending_edits(
        &self,
        project_id: i64,
    ) -> LocalBoxFuture<'_, Result<Vec<PersistedEdit>, String>> {
        Box::pin(BackendApi::list_pending_edits(self, project_id))
    }
    fn resolve_pending_edit<'a>(
        &'a self,
        project_id: i64,
        request: &'a ResolveEditRequest,
    ) -> LocalBoxFuture<'a, Result<PersistedEdit, String>> {
        Box::pin(BackendApi::resolve_pending_edit(self, project_id, request))
    }
    fn web_search<'a>(
        &'a self,
        query: &'a str,
        limit: usize,
    ) -> LocalBoxFuture<'a, Result<Vec<WebSearchResult>, String>> {
        Box::pin(BackendApi::web_search(self, query, limit))
    }
    fn fetch_web_page<'a>(
        &'a self,
        target_url: &'a str,
    ) -> LocalBoxFuture<'a, Result<String, String>> {
        Box::pin(BackendApi::fetch_web_page(self, target_url))
    }
    fn send_message<'a>(
        &'a self,
        session_id: i64,
        content: &'a str,
        model: Option<&'a str>,
        editor_context: Option<&'a EditorContext>,
        signal: Option<&'a AbortSignal>,
        on_event: Box<dyn FnMut(RunEvent) + 'a>,
    ) -> LocalBoxFuture<'a, Result<(), String>> {
        Box::pin(BackendApi::send_message(
            self,
            session_id,
            content,
            model,
            editor_context,
            signal,
            on_event,
        ))
    }
}
