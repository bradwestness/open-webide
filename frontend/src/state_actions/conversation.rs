//! Editing and branching share the same persisted conversation snapshot.
use crate::{
    backend::Api,
    conversation::ConversationItem,
    state::{
        auth::AuthState,
        chat::ChatState,
        projects::ProjectsState,
        ui::{ConfirmRequest, UiState},
    },
};
use leptos::{prelude::*, task::spawn_local};
use openwebide_core::{ForkedSession, PromptContent, Role};

#[derive(Clone, Copy)]
pub struct ConversationActions {
    pub edit: Callback<i64>,
    pub fork: Callback<i64>,
    pub cancel_edit: Callback<()>,
}

/// Install the atomically copied prefix before starting or displaying the branch.
pub fn install_branch(chat: ChatState, source: i64, branch: &ForkedSession) {
    let model = chat
        .session_model
        .with_untracked(|models| models.get(&source).cloned().flatten())
        .or_else(|| chat.selected_model.get_untracked());
    let mode = chat
        .approval_mode
        .with_untracked(|modes| modes.get(&source).copied().unwrap_or_default());
    chat.sessions
        .update(|sessions| sessions.push(branch.session.clone()));
    chat.session_model.update(|models| {
        models.insert(branch.session.id, model);
    });
    chat.set_approval_mode(branch.session.id, mode);
    chat.history_gen.update_value(|generation| *generation += 1);
    chat.session_telemetry
        .update(|telemetry| telemetry.restore_from_conversation(&branch.history));
    chat.messages
        .install_history(super::chat::history_items(branch.history.clone()));
    chat.interrupted_run.set(None);
    chat.active_run.set(None);
    chat.current_run_anchor.set(None);
    chat.loading_history.set(None);
    chat.prompt_edit.set(None);
    chat.skip_history_load.set_value(Some(branch.session.id));
    chat.active_session.set(Some(branch.session.id));
}

pub fn actions(
    api: Api,
    chat: ChatState,
    projects: ProjectsState,
    ui: UiState,
) -> ConversationActions {
    let auth = expect_context::<AuthState>();
    let begin = Callback::new(move |(message_id, fork): (i64, bool)| {
        if (chat.streaming.get_untracked()
            || chat.compacting.get_untracked()
            || chat.goal_busy.get_untracked())
            || chat.branching.get_untracked()
            || chat.rewinding.get_untracked()
            || chat.queue_busy.get_untracked()
            || chat.reading_images.get_untracked()
            || chat.loading_history.get_untracked().is_some()
        {
            return;
        }
        let Some(source) = chat.active_session.get_untracked() else {
            return;
        };
        let Some(prompt) = chat.messages.message(message_id).and_then(|row| {
            row.item.with_untracked(|item| match item {
                ConversationItem::Message(message)
                    if message.role == Role::User && message.session_id == source =>
                {
                    Some(message.content.clone())
                }
                _ => None,
            })
        }) else {
            return;
        };
        let account = auth.generation.get_untracked();
        let project = projects.active_project.get_untracked();
        let history = chat.history_gen.get_value();
        let send = chat.send_generation.get_value();
        let current = move || {
            auth.generation.try_get_untracked() == Some(account)
                && projects.active_project.try_get_untracked() == Some(project)
                && chat.active_session.try_get_untracked() == Some(Some(source))
                && chat.history_gen.try_get_value() == Some(history)
                && chat.send_generation.try_get_value() == Some(send)
        };
        let apply = Callback::new(move |()| {
            if !current()
                || (chat.streaming.get_untracked()
                    || chat.compacting.get_untracked()
                    || chat.goal_busy.get_untracked())
                || chat.branching.get_untracked()
                || chat.rewinding.get_untracked()
            {
                return;
            }
            chat.queue_running.update(|sessions| {
                sessions.remove(&source);
            });
            chat.queue_edit.set(None);
            if !fork {
                let prompt = PromptContent::decode(&prompt);
                chat.prompt_edit.set(Some((source, message_id)));
                chat.draft.set(
                    openwebide_core::tui::extract_editor_context_prelude(&prompt.text)
                        .1
                        .into(),
                );
                chat.prompt_images.set(prompt.images);
                chat.active_editor_context.set(None);
                return;
            }
            chat.branching.set(true);
            let draft = chat.draft.get_untracked();
            let images = chat.prompt_images.get_untracked();
            spawn_local(async move {
                let result = api
                    .with_value(Clone::clone)
                    .fork_session(source, message_id)
                    .await;
                if !current() {
                    return;
                }
                match result {
                    Ok(branch) => {
                        let replace_draft = chat.draft.get_untracked() == draft
                            && chat.prompt_images.get_untracked() == images;
                        let latest_draft = chat.draft.get_untracked();
                        let latest_images = chat.prompt_images.get_untracked();
                        chat.branch_draft_context.set_value(Some((
                            account,
                            project,
                            branch.session.id,
                        )));
                        install_branch(chat, source, &branch);
                        let installed = chat.history_gen.get_value();
                        // Let composer context effects discard images belonging to
                        // the old session before restoring this branch's draft.
                        crate::util::sleep_ms(0).await;
                        if auth.generation.try_get_untracked() != Some(account)
                            || projects.active_project.try_get_untracked() != Some(project)
                            || chat.active_session.try_get_untracked()
                                != Some(Some(branch.session.id))
                            || chat.history_gen.try_get_value() != Some(installed)
                            || chat.send_generation.try_get_value() != Some(send)
                        {
                            return;
                        }
                        if replace_draft && chat.draft.get_untracked() == draft {
                            chat.active_editor_context.set(None);
                            let prompt = PromptContent::decode(&branch.prompt);
                            chat.draft.set(
                                openwebide_core::tui::extract_editor_context_prelude(&prompt.text)
                                    .1
                                    .into(),
                            );
                            chat.prompt_images.set(prompt.images);
                        } else if chat.draft.get_untracked() == latest_draft
                            && chat.prompt_images.with_untracked(Vec::is_empty)
                        {
                            chat.prompt_images.set(latest_images);
                        }
                        chat.branch_draft_context.set_value(None);
                        chat.branching.set(false);
                        ui.notify("Conversation branched. Project files are unchanged.");
                    }
                    Err(error) => {
                        chat.branching.set(false);
                        chat.error
                            .set(Some(format!("Could not branch this conversation: {error}")));
                    }
                }
            });
        });
        if !chat.draft.with_untracked(String::is_empty)
            || !chat.prompt_images.with_untracked(Vec::is_empty)
        {
            ui.set_confirm(ConfirmRequest {
                title: if fork {
                    "Fork this prompt?"
                } else {
                    "Edit this prompt?"
                }
                .into(),
                message: "Replace the current draft with this earlier prompt?".into(),
                confirm_label: if fork { "Fork" } else { "Edit" }.into(),
                action: apply,
            });
        } else {
            apply.run(());
        }
    });
    ConversationActions {
        edit: Callback::new(move |id| begin.run((id, false))),
        fork: Callback::new(move |id| begin.run((id, true))),
        cancel_edit: Callback::new(move |()| chat.prompt_edit.set(None)),
    }
}
