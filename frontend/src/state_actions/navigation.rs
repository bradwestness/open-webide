//! Shared project/session navigation for menus, omnibar and notifications.
use super::layout::LayoutActions;
use crate::state::{chat::ChatState, layout::Panel, projects::ProjectsState, ui::UiState};
use leptos::prelude::*;

#[derive(Clone, Copy)]
pub struct NavigationActions {
    pub open_project: Callback<i64>,
    pub open_session: Callback<i64>,
}
impl NavigationActions {
    pub fn new(
        projects: ProjectsState,
        chat: ChatState,
        ui: UiState,
        layout: LayoutActions,
        open_project: Callback<i64>,
        select_chat: Callback<()>,
    ) -> Self {
        let open_project = Callback::new(move |id| {
            if projects.project(id).is_some() {
                open_project.run(id);
            } else {
                ui.notify("This project is no longer available");
            }
        });
        let open_session = Callback::new(move |id| {
            let session = chat.sessions.with_untracked(|sessions| {
                sessions.iter().find(|session| session.id == id).cloned()
            });
            let Some(session) = session else {
                ui.notify("This session is no longer available");
                return;
            };
            if let Some(project) = session.project_id {
                if projects.project(project).is_none() {
                    ui.notify("This session's project is no longer available");
                    return;
                }
                open_project.run(project);
            } else {
                select_chat.run(());
            }
            layout.show.run(Panel::Chat);
            chat.active_session.set(Some(id));
        });
        Self {
            open_project,
            open_session,
        }
    }
}
