use std::sync::atomic::Ordering;

use crate::conversation::{ConversationItem, next_item_nonce};
use crate::state::{
    chat::{ChatEffect, ChatState},
    git::GitState,
    projects::ProjectsState,
    settings::SettingsState,
    slash::{SlashAction, dispatch as dispatch_slash},
    ui::{ConfirmRequest, PromptRequest, UiState},
    workspace::WorkspaceState,
};
use leptos::prelude::*;
use leptos::task::spawn_local;
use openwebide_agent::policy::ApprovalMode;
use openwebide_core::{
    ConversationEntry, GitCheckoutRequest, GitCommitRequest, WorkspaceMode,
    tui::{DEFAULT_CONTEXT_LIMIT, SlashCommand},
};
use web_sys::AbortController;

use crate::{
    backend::Api, bridge::BridgeCredentials, components::ToolStepResult, local_agent, local_fs,
};

pub struct ChatActionContext {
    pub api: Api,
    pub chat: ChatState,
    pub projects: ProjectsState,
    pub workspace: WorkspaceState,
    pub settings: SettingsState,
    pub ui: UiState,
    pub git: GitState,
    pub bridge_credentials: StoredValue<BridgeCredentials>,
    pub bridge: RwSignal<Option<crate::bridge::BridgeConn>, LocalStorage>,
    pub request_open: Callback<String>,
    pub refresh_git: Callback<()>,
    pub on_sync_click: Callback<()>,
}

/// App-owned actions for sending chat, handling approvals, choosing models,
/// and executing slash-command intents.
pub struct ChatActions {
    pub send: Callback<()>,
    pub stop: Callback<()>,
    pub permission: Callback<(String, bool)>,
    pub permission_always: Callback<String>,
    pub select_model: Callback<Option<String>>,
    pub slash_command: Callback<SlashCommand>,
    pub on_select_session: Callback<i64>,
    pub on_new_session: Callback<()>,
    pub on_rename_session: Callback<i64>,
    pub on_delete_session: Callback<i64>,
}

impl ChatActions {
    pub fn new(context: ChatActionContext) -> Self {
        let ChatActionContext {
            api,
            chat,
            projects,
            workspace,
            settings,
            ui,
            git,
            bridge_credentials,
            bridge,
            request_open,
            refresh_git,
            on_sync_click,
        } = context;
        let run_controls = StoredValue::new_local(super::runs::RunControls::default());
        let stop = {
            let local_cancel = chat.local_cancel_flag.get_value();
            Callback::new(move |_| {
                if let Some((_, run_id, _)) = chat.active_run.get_untracked() {
                    run_controls.update_value(|controls| {
                        controls.send(
                            bridge.get_untracked().as_ref(),
                            openwebide_core::BridgeClientMessage::RunCancel { run_id },
                        );
                    });
                    return;
                }
                local_cancel.store(true, Ordering::Relaxed);
                // Ask the server to stop the run; the browser abort below can't do it.
                if let Some(session_id) = chat.streaming_session.get()
                    && !chat.streaming_is_local.get_untracked()
                {
                    spawn_local(async move {
                        let _ = api
                            .with_value(Clone::clone)
                            .cancel_session(session_id)
                            .await;
                    });
                }
                // The abort tears down the stream before the server's Cancelled
                // event can arrive, so mark the stop here.
                chat.mark_stopped();
                chat.abort.with(|abort| {
                    if let Some(controller) = abort.as_ref() {
                        controller.abort();
                    }
                });
            })
        };

        let permission = {
            let local_permissions = chat.local_permissions.get_value();
            Callback::new(move |(tool_call_id, approved): (String, bool)| {
                if let Some((_, run_id, _)) = chat.active_run.get_untracked() {
                    let mut sent = false;
                    run_controls.update_value(|controls| {
                        sent = controls.send(
                            bridge.get_untracked().as_ref(),
                            openwebide_core::BridgeClientMessage::RunPermission {
                                run_id,
                                tool_call_id: tool_call_id.clone(),
                                approved,
                            },
                        );
                    });
                    if !sent {
                        return;
                    }
                }
                if let Ok(mut map) = local_permissions.lock() {
                    map.insert(tool_call_id.clone(), approved);
                }
                // Clear the prompt immediately; the ToolCall (approved) or
                // ToolResult (denied) event that follows confirms it.
                chat.messages.update(|items| {
                    if let Some(awaiting) = items.iter_mut().find_map(|item| match item {
                        crate::conversation::ConversationItem::ToolStep {
                            id,
                            awaiting_permission,
                            ..
                        } if *id == tool_call_id => Some(awaiting_permission),
                        _ => None,
                    }) {
                        *awaiting = false;
                    }
                });
                if chat.active_run.get_untracked().is_some() {
                    return;
                }
                if let Some(session_id) = chat.streaming_session.get() {
                    spawn_local(async move {
                        let _ = api
                            .with_value(Clone::clone)
                            .set_permission(session_id, &tool_call_id, approved)
                            .await;
                    });
                }
            })
        };

        let permission_always = {
            Callback::new(move |tool_call_id: String| {
                if let Some(session_id) = chat.streaming_session.get_untracked() {
                    chat.set_approval_mode(session_id, ApprovalMode::AlwaysForSession);
                } else {
                    return;
                }
                permission.run((tool_call_id, true));
            })
        };

        let apply_stream_event = Callback::new(
            move |(session_id, event): (i64, openwebide_core::RunEvent)| {
                let run_project_id = chat
                    .sessions
                    .get_untracked()
                    .into_iter()
                    .find(|session| session.id == session_id)
                    .and_then(|session| session.project_id);
                for effect in chat.apply_event_for_session(session_id, event) {
                    match effect {
                        ChatEffect::ApprovePermission { id } => {
                            permission.run((id, true));
                        }
                        ChatEffect::ToolDiff(diff) => {
                            if let Some(project_id) = run_project_id {
                                workspace.merge_pending(project_id, diff.clone());
                            }
                            if chat.active_session.get_untracked() != Some(session_id) {
                                continue;
                            }
                            if projects.active_project.get_untracked() == run_project_id {
                                if !workspace.dirty.get_untracked() {
                                    request_open.run(diff.path.clone());
                                } else {
                                    chat.notify(format!(
                                        "Agent edited `{}`; review it with `/diff {}`.",
                                        diff.path, diff.path
                                    ));
                                }
                            }
                        }
                    }
                }
            },
        );
        let runs =
            super::runs::RunActions::new(bridge, chat, api, apply_stream_event, run_controls);
        runs.install_reconnect();

        let send = {
            let local_cancel = chat.local_cancel_flag.get_value();
            let local_permissions = chat.local_permissions.get_value();
            Callback::new(move |_| {
                let content = chat.draft.with(|draft| draft.trim().to_string());
                if content.is_empty() || chat.streaming.get() {
                    return;
                }
                local_cancel.store(false, Ordering::Relaxed);
                if let Some(project_id) = projects.active_project.get_untracked()
                    && let Some(project) = projects
                        .projects
                        .get_untracked()
                        .into_iter()
                        .find(|project| project.id == project_id)
                {
                    chat.streaming_is_local
                        .set(project.mode == WorkspaceMode::Local);
                }
                if let Ok(mut map) = local_permissions.lock() {
                    map.clear();
                }
                // Clear the draft and mark streaming synchronously so a second
                // send can't fire while the session is (possibly) created.
                chat.draft.set(String::new());
                chat.streaming.set(true);
                chat.error.set(None);
                chat.notice.set(None);
                let local_cancel = local_cancel.clone();
                let local_permissions = local_permissions.clone();
                spawn_local(async move {
                    // Use the active session, or create one named from the first
                    // prompt when a fresh chat has no session selected.
                    let session_id = match chat.active_session.get() {
                        Some(id) => id,
                        None => {
                            let Some(project_id) = projects.active_project.get() else {
                                chat.streaming.set(false);
                                return;
                            };
                            let name = derive_session_name(&content);
                            let connection_id = settings.default_connection.get().or_else(|| {
                                settings
                                    .connections
                                    .get()
                                    .into_iter()
                                    .find(|connection| connection.enabled)
                                    .map(|connection| connection.id)
                            });
                            let prompt_id = settings.default_prompt.get();
                            match api
                                .with_value(Clone::clone)
                                .create_session(&name, connection_id, prompt_id, Some(project_id))
                                .await
                            {
                                Ok(session) => {
                                    chat.sessions
                                        .update(|sessions| sessions.push(session.clone()));
                                    chat.skip_history_load.set_value(Some(session.id));
                                    chat.active_session.set(Some(session.id));
                                    if let Some(model) = chat.selected_model.get() {
                                        chat.session_model.update(|models| {
                                            let _ = models.insert(session.id, Some(model));
                                        });
                                    }
                                    session.id
                                }
                                Err(error) => {
                                    chat.error.set(Some(error));
                                    chat.streaming.set(false);
                                    return;
                                }
                            }
                        }
                    };
                    let model = chat
                        .session_model
                        .get()
                        .get(&session_id)
                        .cloned()
                        .flatten()
                        .or_else(|| chat.selected_model.get());
                    let Ok(controller) = AbortController::new() else {
                        chat.streaming.set(false);
                        return;
                    };
                    chat.abort.set(Some(controller.clone()));
                    chat.streaming_session.set(Some(session_id));

                    // Resolve local mode and its browser-owned directory handle.
                    let active_project = projects.active_project.get().and_then(|project_id| {
                        projects
                            .projects
                            .get()
                            .into_iter()
                            .find(|project| project.id == project_id)
                    });
                    let is_local = active_project
                        .as_ref()
                        .is_some_and(|project| project.mode == WorkspaceMode::Local);
                    let local_handle = active_project
                        .as_ref()
                        .and_then(|project| projects.local_handles.get().get(&project.id).cloned());

                    let on_event = move |event| apply_stream_event.run((session_id, event));

                    let editor_context = chat.active_editor_context.get();
                    chat.active_editor_context.set(None);

                    if is_local {
                        if let Some(handle) = local_handle {
                            let session = chat
                                .sessions
                                .get()
                                .into_iter()
                                .find(|session| session.id == session_id);
                            let connection_id = session
                                .as_ref()
                                .and_then(|session| session.connection_id)
                                .or_else(|| {
                                    settings.default_connection.get().or_else(|| {
                                        settings
                                            .connections
                                            .get()
                                            .into_iter()
                                            .find(|connection| connection.enabled)
                                            .map(|connection| connection.id)
                                    })
                                });
                            let Some(connection_id) = connection_id else {
                                chat.error.set(Some(
                                    "session has no connection; configure one in settings first"
                                        .into(),
                                ));
                                chat.streaming.set(false);
                                chat.abort.set(None);
                                chat.streaming_session.set(None);
                                return;
                            };
                            let system_prompt_id = session
                                .as_ref()
                                .and_then(|session| session.system_prompt_id)
                                .or_else(|| settings.default_prompt.get());
                            let system_prompt = system_prompt_id.and_then(|prompt_id| {
                                settings
                                    .system_prompts
                                    .get()
                                    .into_iter()
                                    .find(|prompt| prompt.id == prompt_id)
                                    .map(|prompt| prompt.content)
                            });
                            let vfs = local_fs::BrowserFsaVfs::new(handle);
                            let result = local_agent::run_local_agent(
                                api,
                                session_id,
                                content,
                                model,
                                editor_context,
                                connection_id,
                                system_prompt,
                                vfs,
                                local_cancel,
                                local_permissions,
                                on_event,
                                crate::bridge::BridgeConfig::new(&settings.bridge_url.get()),
                                bridge_credentials.with_value(Clone::clone),
                                bridge.get_untracked(),
                            )
                            .await;
                            if let Err(error) = result
                                && !controller.signal().aborted()
                            {
                                chat.error.set(Some(error));
                            }
                        } else {
                            chat.error.set(Some(
                                "local directory handle not available; re-open the folder".into(),
                            ));
                        }
                    } else if !runs
                        .start(openwebide_core::BridgeClientMessage::RunStart {
                            run_id: format!(
                                "run-{}",
                                js_sys::Math::random().to_string().trim_start_matches("0.")
                            ),
                            session_id,
                            content: content.clone(),
                            model: model.clone(),
                            editor_context: editor_context.clone(),
                        })
                        .await
                    {
                        let result = api
                            .with_value(Clone::clone)
                            .send_message(
                                session_id,
                                &content,
                                model.as_deref(),
                                editor_context.as_ref(),
                                Some(&controller.signal()),
                                Box::new(on_event),
                            )
                            .await;
                        if let Err(error) = result
                            && !controller.signal().aborted()
                        {
                            chat.error.set(Some(error));
                        }
                    }

                    if chat
                        .abort
                        .get_untracked()
                        .as_ref()
                        .is_none_or(|active| active == &controller)
                    {
                        chat.streaming.set(false);
                        chat.abort.set(None);
                        chat.streaming_session.set(None);
                        chat.current_run_anchor.set(None);
                    }
                });
            })
        };

        let select_model = Callback::new(move |model: Option<String>| {
            chat.selected_model.set(model.clone());
            if let Some(session_id) = chat.active_session.get() {
                chat.session_model.update(|models| {
                    let _ = models.insert(session_id, model);
                });
            }
        });

        let on_select_session =
            Callback::new(move |session_id: i64| chat.active_session.set(Some(session_id)));
        let on_new_session = Callback::new(move |_| chat.active_session.set(None));

        let on_rename_session = Callback::new(move |session_id: i64| {
            let current = chat
                .sessions
                .with(|sessions| {
                    sessions
                        .iter()
                        .find(|session| session.id == session_id)
                        .map(|session| session.name.clone())
                })
                .unwrap_or_default();
            ui.set_prompt(PromptRequest {
                title: "Rename session".to_string(),
                value: current,
                placeholder: "Session name".to_string(),
                submit_label: "Rename".to_string(),
                on_submit: Callback::new(move |name: String| {
                    spawn_local(async move {
                        match api
                            .with_value(Clone::clone)
                            .rename_session(session_id, &name)
                            .await
                        {
                            Ok(updated) => chat.sessions.update(|sessions| {
                                if let Some(session) =
                                    sessions.iter_mut().find(|session| session.id == session_id)
                                {
                                    *session = updated;
                                }
                            }),
                            Err(error) => ui.notify(error),
                        }
                    });
                }),
            });
        });

        let on_delete_session = Callback::new(move |session_id: i64| {
            ui.set_confirm(ConfirmRequest {
                title: "Delete session".to_string(),
                message: "Delete this session and its messages?".to_string(),
                confirm_label: "Delete".to_string(),
                action: Callback::new(move |_| {
                    spawn_local(async move {
                        if let Err(error) = api
                            .with_value(Clone::clone)
                            .delete_session(session_id)
                            .await
                        {
                            ui.notify(error);
                            return;
                        }
                        chat.sessions
                            .update(|sessions| sessions.retain(|session| session.id != session_id));
                        chat.approval_mode.update(|modes| {
                            modes.remove(&session_id);
                        });
                        if chat.active_session.get() == Some(session_id) {
                            chat.active_session.set(None);
                        }
                    });
                }),
            });
        });

        let slash_command = {
            Callback::new(move |command: SlashCommand| {
                match dispatch_slash(command, &chat, &git, &workspace) {
                    SlashAction::Notify(text) => chat.notify(text),
                    SlashAction::SelectModel(name) => {
                        select_model.run(Some(name.clone()));
                        chat.notify(format!("Switched model to `{name}`."));
                    }
                    SlashAction::Clear => chat.messages.set(Vec::new()),
                    SlashAction::GitDiff { project_id, path } => {
                        spawn_local(async move {
                            match api
                                .with_value(Clone::clone)
                                .git_diff(project_id, path.as_deref())
                                .await
                            {
                                Ok(diff) if !diff.trim().is_empty() => chat.notify(format!(
                                    "**Git Repository Diff:**\n```diff\n{diff}\n```"
                                )),
                                Ok(_) => {
                                    chat.notify("Working tree is clean (no uncommitted diffs).")
                                }
                                Err(error) => chat.notify(format!("Git diff failed: {error}")),
                            }
                        });
                    }
                    SlashAction::OpenPendingDiff(diff) => {
                        request_open.run(diff.path.clone());
                        chat.notify(format!(
                            "Opened pending diff for `{}` in editor.",
                            diff.path
                        ));
                    }
                    SlashAction::RunTests { filter, .. } => {
                        chat.notify(format!(
                            "Dispatched test run: `cargo test {filter}` via execution bridge.\nCheck terminal dock below for full stream."
                        ));
                        chat.show_terminal.set(true);
                    }
                    SlashAction::Commit {
                        project_id,
                        message,
                    } => {
                        spawn_local(async move {
                            let request = GitCommitRequest {
                                message: message.clone(),
                                paths: None,
                                include_untracked: false,
                            };
                            match api
                                .with_value(Clone::clone)
                                .git_commit(project_id, &request)
                                .await
                            {
                                Ok(result) => {
                                    refresh_git.run(());
                                    chat.notify(format!(
                                        "Committed `{}`: {}\nSigned: {}",
                                        result.commit_hash, result.summary, result.is_signed
                                    ));
                                }
                                Err(error) => {
                                    chat.notify(format!("Git commit failed: {error}"));
                                }
                            }
                        });
                    }
                    SlashAction::Checkout { project_id, branch } => {
                        spawn_local(async move {
                            let request = GitCheckoutRequest {
                                branch: branch.clone(),
                                create_if_missing: false,
                            };
                            match api
                                .with_value(Clone::clone)
                                .git_checkout(project_id, &request)
                                .await
                            {
                                Ok(result) => {
                                    refresh_git.run(());
                                    chat.notify(format!(
                                        "Checked out branch `{}` (previous: `{}`).",
                                        result.branch,
                                        result.previous_branch.as_deref().unwrap_or("none")
                                    ));
                                }
                                Err(error) => {
                                    chat.notify(format!("Git checkout failed: {error}"));
                                }
                            }
                        });
                    }
                    SlashAction::CreateBranch { project_id, branch } => {
                        spawn_local(async move {
                            let request = GitCheckoutRequest {
                                branch: branch.clone(),
                                create_if_missing: true,
                            };
                            match api
                                .with_value(Clone::clone)
                                .git_checkout(project_id, &request)
                                .await
                            {
                                Ok(result) => {
                                    refresh_git.run(());
                                    chat.notify(format!(
                                        "Created and checked out branch `{}`.",
                                        result.branch
                                    ));
                                }
                                Err(error) => {
                                    chat.notify(format!("Git branch failed: {error}"));
                                }
                            }
                        });
                    }
                    SlashAction::ListBranches { project_id } => {
                        spawn_local(async move {
                            match api.with_value(Clone::clone).git_branches(project_id).await {
                                Ok(branches) => {
                                    let branch_list = if branches.is_empty() {
                                        "No branches found.".to_string()
                                    } else {
                                        branches
                                            .into_iter()
                                            .map(|branch| {
                                                if branch.is_current {
                                                    format!("* **{}** (current)", branch.name)
                                                } else {
                                                    format!("  {}", branch.name)
                                                }
                                            })
                                            .collect::<Vec<_>>()
                                            .join("\n")
                                    };
                                    chat.notify(format!(
                                        "**Repository Branches:**\n\n{branch_list}"
                                    ));
                                }
                                Err(error) => {
                                    chat.notify(format!("Failed to list branches: {error}"));
                                }
                            }
                        });
                    }
                    SlashAction::Sync { .. } => on_sync_click.run(()),
                    SlashAction::Stop => stop.run(()),
                }
            })
        };

        install_effects(api, chat, settings, runs);

        Self {
            send,
            stop,
            permission,
            permission_always,
            select_model,
            slash_command,
            on_select_session,
            on_new_session,
            on_rename_session,
            on_delete_session,
        }
    }
}

fn install_effects(
    api: Api,
    chat: ChatState,
    settings: SettingsState,
    runs: super::runs::RunActions,
) {
    let active_session = chat.active_session;
    let sessions = chat.sessions;
    let connections = settings.connections;
    let default_connection = settings.default_connection;
    let selected_model = chat.selected_model;
    let session_telemetry = chat.session_telemetry;

    let ctx_request_gen = StoredValue::new(0u64);
    Effect::new(move |_| {
        let this_gen = {
            ctx_request_gen.update_value(|generation| *generation += 1);
            ctx_request_gen.get_value()
        };
        let session_id = active_session.get();
        let connection_id = session_id
            .and_then(|id| {
                sessions
                    .get()
                    .into_iter()
                    .find(|session| session.id == id)
                    .and_then(|session| session.connection_id)
            })
            .or_else(|| {
                default_connection.get().or_else(|| {
                    connections
                        .get()
                        .into_iter()
                        .find(|connection| connection.enabled)
                        .map(|connection| connection.id)
                })
            });
        let connection = connection_id.and_then(|id| {
            connections
                .get()
                .into_iter()
                .find(|connection| connection.id == id)
        });
        let connection_model = connection
            .as_ref()
            .and_then(|connection| connection.model.clone());
        let connection_name = connection
            .as_ref()
            .map(|connection| connection.name.clone())
            .unwrap_or_else(|| "unknown".to_string());

        let effective_model = selected_model.get().or(connection_model);
        let display_model = effective_model
            .clone()
            .unwrap_or_else(|| format!("Default ({connection_name})"));
        session_telemetry.update(|telemetry| telemetry.model = display_model);

        let Some(connection_id) = connection_id else {
            session_telemetry.update(|telemetry| {
                telemetry.context_limit = DEFAULT_CONTEXT_LIMIT;
                telemetry.context_limit_estimated = true;
            });
            return;
        };
        spawn_local(async move {
            let result = api
                .with_value(Clone::clone)
                .model_context(connection_id, effective_model.as_deref())
                .await;
            if ctx_request_gen.get_value() != this_gen {
                return;
            }
            let resolved = result.ok().flatten();
            session_telemetry.update(|telemetry| {
                telemetry.context_limit = resolved.unwrap_or(DEFAULT_CONTEXT_LIMIT);
                telemetry.context_limit_estimated = resolved.is_none();
            });
        });
    });

    let history_gen = chat.history_gen;
    let skip_history_load = chat.skip_history_load;
    Effect::new(move |_| {
        let session_id = active_session.get();
        if let Some(id) = session_id
            && skip_history_load.get_value() == Some(id)
        {
            skip_history_load.set_value(None);
            return;
        }
        let this_gen = {
            history_gen.update_value(|generation| *generation += 1);
            history_gen.get_value()
        };
        spawn_local(async move {
            match session_id {
                Some(id) => {
                    let result = api.with_value(Clone::clone).list_messages(id).await;
                    if history_gen.get_value() != this_gen
                        || active_session.get_untracked() != Some(id)
                    {
                        return;
                    }
                    match result {
                        Ok(entries) => {
                            session_telemetry
                                .update(|telemetry| telemetry.restore_from_conversation(&entries));
                            chat.messages.set(history_items(entries));
                            runs.history.set(Some((id, this_gen)));
                        }
                        Err(error) => {
                            session_telemetry
                                .update(|telemetry| telemetry.restore_from_conversation(&[]));
                            chat.error.set(Some(error));
                        }
                    }
                }
                None => {
                    session_telemetry.update(|telemetry| telemetry.restore_from_conversation(&[]));
                    chat.messages.set(Vec::new());
                }
            }
        });
    });

    Effect::new(move |_| {
        let session_id = active_session.get();
        let connection_id = session_id
            .and_then(|id| {
                sessions
                    .get()
                    .into_iter()
                    .find(|session| session.id == id)
                    .and_then(|session| session.connection_id)
            })
            .or_else(|| {
                default_connection.get().or_else(|| {
                    connections
                        .get()
                        .into_iter()
                        .find(|connection| connection.enabled)
                        .map(|connection| connection.id)
                })
            });
        spawn_local(async move {
            let models = match connection_id {
                Some(id) => api
                    .with_value(Clone::clone)
                    .list_models(id)
                    .await
                    .unwrap_or_default(),
                None => Vec::new(),
            };
            if active_session.get() == session_id {
                chat.models.set(models);
            }
        });
    });

    Effect::new(move |_| {
        if let Some(session_id) = active_session.get() {
            let model = chat.session_model.get().get(&session_id).cloned().flatten();
            chat.selected_model.set(model);
        }
    });
}

fn derive_session_name(prompt: &str) -> String {
    let line = prompt
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("")
        .trim();
    let mut name: String = line.chars().take(40).collect();
    if line.chars().count() > 40 {
        name.push('…');
    }
    name
}

pub(super) fn history_items(entries: Vec<ConversationEntry>) -> Vec<ConversationItem> {
    entries
        .into_iter()
        .map(|entry| match entry {
            ConversationEntry::Message(message) => ConversationItem::Message(message),
            ConversationEntry::ToolStep(step) => ConversationItem::ToolStep {
                key: next_item_nonce(),
                id: step.tool_call_id,
                name: step.name,
                summary: step.summary,
                result: step.ok.map(|ok| ToolStepResult {
                    ok,
                    summary: step.result_summary.clone().unwrap_or_default(),
                    diff: step.diff.clone(),
                }),
                awaiting_permission: false,
            },
        })
        .collect()
}
