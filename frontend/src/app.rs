use crate::state::auth::AuthState;
use crate::state::chat::ChatState;
use crate::state::git::GitState;
use crate::state::layout::{ActiveResizer, LayoutState};
use crate::state::projects::ProjectsState;
use crate::state::settings::SettingsState;
use crate::state::ui::UiState;
use crate::state::workspace::WorkspaceState;
use leptos::prelude::*;

use crate::components::{
    AuthGate, ChatPane, ConfirmDialog, Editor, FileBrowser, FileTree, PanelResizer, PromptDialog,
    Settings, Sidebar, StatusBar, TabBar, TerminalPane, TopBar,
};
use crate::state_actions::{
    auth::{AuthActionContext, AuthActions},
    chat::{ChatActionContext, ChatActions},
    git::{GitActionContext, GitActions},
    lifecycle::{ProjectEffectContext, install_keyboard_shortcuts, install_project_effects},
    projects::{ProjectsActionContext, ProjectsActions, build_projects_actions},
    settings::{SettingsActionContext, SettingsActions, build_settings_actions},
    workspace::WorkspaceActions,
};
use crate::{api::HealthState, backend::Api};

#[component]
pub fn App() -> impl IntoView {
    let api: Api =
        StoredValue::new_local(std::rc::Rc::new(crate::api::BackendApi::from_location()));
    provide_context(api);

    let ui = UiState::new();
    provide_context(ui);
    let auth = AuthState::new();
    provide_context(auth);
    let settings = SettingsState::new(
        crate::state_actions::settings::read_theme_from_storage(),
        crate::bridge::default_bridge_url(),
    );
    provide_context(settings);
    let layout = LayoutState::new();
    provide_context(layout);
    let projects_state = ProjectsState::new();
    let active_project = projects_state.active_project;
    let workspace_state = WorkspaceState::with_active_project(active_project);
    let git_state = GitState::with_active_project(active_project);
    let active_session = workspace_state.active_session;
    let chat_state = ChatState::with_active_session_and_toast(active_session, ui.toast);
    provide_context(projects_state);
    provide_context(workspace_state);
    provide_context(git_state);
    provide_context(chat_state);

    // -- auth --------------------------------------------------------------
    // The signed-in account, once the cached token has been verified (or the
    // user has logged in). `None` while the gate is showing.
    let current_user = auth.user;
    // True once the initial token check has finished (regardless of outcome).
    let auth_checked = auth.checked;
    // The signed-in user's name, for the top bar.

    // -- global state ------------------------------------------------------
    let health = RwSignal::new(Option::<HealthState>::None);
    let bridge_url = settings.bridge_url;
    let bridge_connection = RwSignal::new_local(Option::<crate::bridge::BridgeConn>::None);
    let bridge_credentials = StoredValue::new(crate::bridge::BridgeCredentials::new(api));
    Effect::new(move |_| {
        let user = auth.user.get().map(|user| user.id);
        let url = bridge_url.get();
        auth.bridge.update_value(|connection| {
            if let Some(connection) = connection.take() {
                connection.close();
            }
            if user.is_some() {
                *connection = Some(crate::bridge::BridgeConn::new(
                    crate::bridge::BridgeConfig::new(&url),
                    bridge_credentials.get_value(),
                ));
            }
            bridge_connection.set(connection.clone());
        });
    });
    on_cleanup(move || {
        auth.bridge.with_value(|connection| {
            if let Some(connection) = connection {
                connection.close();
            }
        })
    });
    let active_resizer = layout.active_resizer;
    let error = ui.toast;

    // -- active project's workspace state ----------------------------------
    let ws_read_only = RwSignal::new(false);
    // "Include ignored folders" search toggle: per query, not persisted,
    // resets to off on reload.
    let include_ignored_search = RwSignal::new(false);

    // -- bottom dock terminal & TUI telemetry/context ----------------------
    let show_terminal = chat_state.show_terminal;
    let on_toggle_terminal = move || show_terminal.update(|v| *v = !*v);

    install_keyboard_shortcuts(workspace_state, chat_state);

    let show_settings = settings.show_settings;

    // -- confirmation dialogs (Phase 10) -----------------------------------
    // A pending confirmation request; the ConfirmDialog renders it and runs
    // its action on confirm. Destructive actions set this instead of using
    // window.confirm.
    // -- remote file browser (Phase 10) ------------------------------------
    // Opens the host-folder browser so a remote project's folder can be picked
    // by navigating the mount tree. The chosen path is relative to the mount
    // root, which is exactly what NewProject.path wants.
    let show_browser = RwSignal::new(false);
    let on_open_remote = Callback::new(move |_| {
        show_browser.set(true);
    });
    let on_close_browser = Callback::new(move |_| {
        show_browser.set(false);
    });

    let refresh_git = GitActions::refresh(api, projects_state, git_state);

    let auth_actions = AuthActions::new(AuthActionContext {
        api,
        auth,
        projects: projects_state,
        workspace: workspace_state,
        git: git_state,
        chat: chat_state,
        settings,
        ui,
    });
    let on_logout = auth_actions.on_logout;

    let workspace_actions = WorkspaceActions::new(
        api,
        projects_state,
        workspace_state,
        ui,
        ws_read_only,
        refresh_git,
    );
    let WorkspaceActions {
        workspace_for,
        on_grant_access,
        ensure_root,
        on_open_lossy,
        request_open,
        on_toggle,
        on_save,
        on_accept,
        on_reject,
        on_new_file,
        on_new_dir,
        on_search,
        on_clear_search,
        ..
    } = workspace_actions;

    let git_actions = GitActions::new(GitActionContext {
        api,
        projects: projects_state,
        workspace: workspace_state,
        git: git_state,
        chat: chat_state,
        ui,
        workspace_for,
        refresh: refresh_git,
    });
    let on_branch_click = git_actions.on_branch_click;
    let on_sync_click = git_actions.on_sync_click;
    let on_load_git_diff = git_actions.on_load_diff;
    let on_discard_git_diff = git_actions.on_discard_diff;

    let project_actions = build_projects_actions(ProjectsActionContext {
        api,
        projects: projects_state,
        workspace: workspace_state,
        git: git_state,
        chat: chat_state,
        ui,
        ensure_root,
        refresh_git,
    });
    let ProjectsActions {
        select_project,
        close_project,
        on_open_project,
        on_open_local,
        on_browser_select,
        on_delete_project,
    } = project_actions;

    let settings_actions = build_settings_actions(SettingsActionContext { api, settings, ui });
    let SettingsActions {
        on_new_prompt,
        on_edit_prompt,
        on_cancel_prompt,
        on_save_prompt,
        on_delete_prompt,
        on_new_connection,
        on_edit_connection,
        on_cancel_connection,
        on_save_connection,
        on_delete_connection,
        on_open_settings,
        on_set_theme,
        on_set_default_connection,
        on_set_default_prompt,
        on_set_bridge_url,
    } = settings_actions;

    let chat_actions = ChatActions::new(ChatActionContext {
        api,
        chat: chat_state,
        projects: projects_state,
        workspace: workspace_state,
        settings,
        ui,
        git: git_state,
        bridge_credentials,
        bridge: bridge_connection,
        request_open,
        refresh_git,
        on_sync_click,
    });
    let on_send = chat_actions.send;
    let on_stop = chat_actions.stop;
    let on_permission = chat_actions.permission;
    let on_permission_always = chat_actions.permission_always;
    let on_select_model = chat_actions.select_model;
    let on_select_session = chat_actions.on_select_session;
    let on_new_session = chat_actions.on_new_session;
    let on_rename_session = chat_actions.on_rename_session;
    let on_delete_session = chat_actions.on_delete_session;
    let on_slash_command = chat_actions.slash_command;

    install_project_effects(ProjectEffectContext {
        api,
        health,
        auth,
        settings,
        projects: projects_state,
        chat: chat_state,
        layout,
        select_project,
    });

    view! {
        <>
        <Show
            when=move || auth_checked.get() && current_user.get().is_some()
            fallback=move || {
                view! {
                    <Show
                        when=move || auth_checked.get()
                        fallback=move || {
                            view! {
                                <div class="auth-gate">
                                    <div class="auth-loading">"Loading…"</div>
                                </div>
                            }
                        }
                    >
                        <AuthGate />
                    </Show>
                }
            }
        >
            <div class="app">
                <TopBar
                    health=health.read_only()
                    on_open_settings=on_open_settings
                    on_logout=on_logout
                />
                <TabBar
                    on_select=select_project
                    on_close=close_project
                    on_open_local=on_open_local
                    on_open_remote=on_open_remote
                    on_open_project=on_open_project
                    on_delete_project=on_delete_project
                />
                <div class=move || if active_resizer.get() != ActiveResizer::None { "app-body is-resizing" } else { "app-body" }>
                <Sidebar
                    on_new_connection=on_new_connection
                    on_edit_connection=on_edit_connection
                    on_save_connection=on_save_connection
                    on_cancel_connection=on_cancel_connection
                    on_delete_connection=on_delete_connection
                    on_select_session=on_select_session
                    on_new_session=on_new_session
                    on_rename_session=on_rename_session
                    on_delete_session=on_delete_session
                    on_new_prompt=on_new_prompt
                    on_edit_prompt=on_edit_prompt
                    on_save_prompt=on_save_prompt
                    on_cancel_prompt=on_cancel_prompt
                    on_delete_prompt=on_delete_prompt
                />
                <PanelResizer kind=ActiveResizer::Sidebar />
                <FileTree
                    on_toggle=on_toggle
                    on_open=request_open
                    on_new_file=on_new_file
                    on_new_dir=on_new_dir
                    on_search=on_search
                    on_clear_search=on_clear_search
                    include_ignored=include_ignored_search.read_only()
                    on_toggle_include_ignored=Callback::new(move |_| include_ignored_search.update(|v| *v = !*v))
                    on_grant_access=on_grant_access
                />
                <PanelResizer kind=ActiveResizer::Tree />
                <div class="center-pane">
                    <Editor
                        read_only=ws_read_only.read_only().into()
                        on_open_lossy=on_open_lossy
                        on_load_git_diff=on_load_git_diff
                        on_discard_git_diff=on_discard_git_diff
                        on_save=on_save
                        on_accept=on_accept
                        on_reject=on_reject
                    />
                    <Show when=move || show_terminal.get() fallback=|| ()>
                        {move || bridge_connection.get().map(|bridge| view! {
                            <TerminalPane bridge=bridge on_close=move || show_terminal.set(false) />
                        })}
                    </Show>
                </div>
                <PanelResizer kind=ActiveResizer::Chat />
                <ChatPane
                    on_select_model=on_select_model
                    on_send=on_send
                    on_stop=on_stop
                    on_permission=on_permission
                    on_permission_always=on_permission_always
                    on_slash_command=on_slash_command
                />
            </div>
            <StatusBar
                health=health.read_only()
                on_toggle_terminal=on_toggle_terminal
                on_branch_click=on_branch_click
                on_sync_click=on_sync_click
            />
            <Show when=move || show_settings.get() fallback=|| ()>
                <Settings
                    on_set_theme=on_set_theme
                    on_set_default_connection=on_set_default_connection
                    on_set_default_prompt=on_set_default_prompt
                    on_set_bridge_url=on_set_bridge_url
                />
            </Show>
            <ConfirmDialog />
            <PromptDialog />
            <Show when=move || show_browser.get() fallback=|| ()>
                <FileBrowser
                    on_close=on_close_browser
                    on_select=on_browser_select
                />
            </Show>
        </div>
        </Show>
        <Show when=move || chat_state.notice.get().is_some()>
            <div class="toast toast-info" role="status">
                <span class="toast-message">{move || chat_state.notice.get().unwrap_or_default()}</span>
                <button class="icon-btn toast-close" title="Dismiss" on:click=move |_| chat_state.notice.set(None)>"×"</button>
            </div>
        </Show>
        <Show when=move || error.get().is_some() fallback=|| ()>
            <div class="toast" role="alert">
                <span class="toast-message">{move || error.get().unwrap_or_default()}</span>
                <button class="icon-btn toast-close" title="Dismiss" on:click=move |_| error.set(None)>
                    "✕"
                </button>
            </div>
        </Show>
        </>
    }
}
