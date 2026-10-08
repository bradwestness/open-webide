//! Shared session workflows; HTTP and browser downloads remain thin primitives.
use crate::{
    backend::Api,
    state::{
        auth::AuthState, chat::ChatState, projects::ProjectsState, sessions::SessionsState,
        ui::UiState,
    },
};
use leptos::{prelude::*, task::spawn_local};
use openwebide_core::{SessionExport, SessionPreferences, SessionSearch};
use std::rc::Rc;
use wasm_bindgen::JsCast;

pub trait SessionDownload {
    fn save(&self, export: &SessionExport) -> Result<(), String>;
}
pub struct BrowserSessionDownload;
impl SessionDownload for BrowserSessionDownload {
    fn save(&self, export: &SessionExport) -> Result<(), String> {
        let window = web_sys::window().ok_or("Browser window unavailable")?;
        let document = window.document().ok_or("Document unavailable")?;
        let parts = js_sys::Array::new();
        parts.push(&wasm_bindgen::JsValue::from_str(&export.markdown));
        let options = web_sys::BlobPropertyBag::new();
        options.set_type("text/markdown;charset=utf-8");
        let blob = web_sys::Blob::new_with_str_sequence_and_options(&parts, &options)
            .map_err(|error| format!("{error:?}"))?;
        let url = web_sys::Url::create_object_url_with_blob(&blob)
            .map_err(|error| format!("{error:?}"))?;
        let result = (|| {
            let anchor = document
                .create_element("a")
                .map_err(|error| format!("{error:?}"))?
                .dyn_into::<web_sys::HtmlAnchorElement>()
                .map_err(|_| "Download link unavailable")?;
            anchor.set_href(&url);
            anchor.set_download(&export.filename);
            document
                .body()
                .ok_or("Document body unavailable")?
                .append_child(&anchor)
                .map_err(|error| format!("{error:?}"))?;
            anchor.click();
            anchor.remove();
            Ok(())
        })();
        // Let the browser begin consuming the object URL before revoking it.
        spawn_local(async move {
            crate::util::sleep_ms(1000).await;
            let _ = web_sys::Url::revoke_object_url(&url);
        });
        result
    }
}
#[derive(Clone, Copy)]
pub struct SessionActions {
    pub preferences: Callback<(i64, SessionPreferences)>,
    pub export: Callback<i64>,
}
impl SessionActions {
    pub fn from_context() -> Self {
        if let Some(actions) = use_context::<Self>() {
            return actions;
        }
        let actions = Self::new(
            expect_context::<Api>(),
            expect_context::<AuthState>(),
            expect_context::<ChatState>(),
            expect_context::<ProjectsState>(),
            expect_context::<SessionsState>(),
            expect_context::<UiState>(),
            Rc::new(BrowserSessionDownload),
        );
        provide_context(actions);
        actions
    }
    pub fn new(
        api: Api,
        auth: AuthState,
        chat: ChatState,
        projects: ProjectsState,
        state: SessionsState,
        ui: UiState,
        download: Rc<dyn SessionDownload>,
    ) -> Self {
        let assistance_queue =
            StoredValue::new_local(super::assistance::AssistanceQueue::from_context());
        let revision = StoredValue::new(0u64);
        let title_attempts = StoredValue::new(HashSet::<(u64, i64, usize)>::new());
        let title_busy = RwSignal::new(false);
        Effect::new(move |_| {
            auth.generation.track();
            title_attempts.update_value(HashSet::clear);
            title_busy.set(false);
            revision.update_value(|revision| *revision += 1);
            state.query.set(String::new());
            state.archived.set(false);
            state.matches.set(None);
            state.search_explanations.set(Default::default());
            state.rewritten_query.set(None);
            state.search_error.set(None);
            state.searching.set(false);
            state.busy.update(HashSet::clear);
            state.exporting.update(HashSet::clear);
        });
        Effect::new(move |_| {
            let epoch = auth.generation.get();
            let project = projects.active_project.get();
            let archived = state.archived.get();
            let query = state.query.get();
            chat.sessions.track();
            revision.update_value(|revision| *revision += 1);
            let request = revision.get_value();
            state.search_explanations.set(Default::default());
            state.rewritten_query.set(None);
            state.search_error.set(None);
            state.matches.set(None);
            state.searching.set(false);
            if query.trim().is_empty() {
                return;
            }
            state.searching.set(true);
            spawn_local(async move {
                crate::util::sleep_ms(200).await;
                let current = move || {
                    auth.generation.try_get_untracked() == Some(epoch)
                        && projects.active_project.try_get_untracked() == Some(project)
                        && revision.try_get_value() == Some(request)
                };
                if !current() {
                    return;
                }
                let search = SessionSearch {
                    project_id: project,
                    query,
                    archived,
                };
                let result = api.with_value(Clone::clone).search_sessions(&search).await;
                if !current() {
                    return;
                }
                state.searching.set(false);
                match result {
                    Ok(sessions) => state
                        .matches
                        .set(Some(sessions.iter().map(|session| session.id).collect())),
                    Err(error) => {
                        state.search_error.set(Some(error));
                        return;
                    }
                }
                let Some(queue) = assistance_queue.try_with_value(Clone::clone) else {
                    return;
                };
                let _permit = queue.permit().await;
                if !current() || chat.streaming.get_untracked() {
                    return;
                }
                if let Ok(suggestions) = api
                    .with_value(Clone::clone)
                    .session_search_suggestions(&search)
                    .await
                {
                    if !current() || chat.streaming.get_untracked() {
                        return;
                    }
                    state.matches.set(Some(
                        suggestions
                            .sessions
                            .iter()
                            .map(|session| session.id)
                            .collect(),
                    ));
                    state.search_explanations.set(suggestions.explanations);
                    state.rewritten_query.set(suggestions.rewritten_query);
                }
            });
        });
        Effect::new(move |_| {
            let epoch = auth.generation.get();
            let sessions = chat.sessions.get();
            let active = chat.active_session.get();
            let streaming = chat.streaming.get();
            let count = chat.messages.handles.get().len();
            if streaming || title_busy.get() {
                return;
            }
            for pending in sessions.into_iter().filter(|session| session.auto_title) {
                let history = if active == Some(pending.id) { count } else { 0 };
                let key = (epoch, pending.id, history);
                if title_attempts.with_value(|attempts| attempts.contains(&key)) {
                    continue;
                }
                title_attempts.update_value(|attempts| {
                    attempts.insert(key);
                });
                title_busy.set(true);
                spawn_local(async move {
                    let Some(queue) = assistance_queue.try_with_value(Clone::clone) else {
                        return;
                    };
                    let _permit = queue.permit().await;
                    if auth.generation.try_get_untracked() != Some(epoch) {
                        return;
                    }
                    if chat.streaming.try_get_untracked() != Some(false) {
                        title_busy.set(false);
                        title_attempts.update_value(|attempts| {
                            attempts.remove(&key);
                        });
                        return;
                    }
                    let result = api.with_value(Clone::clone).session_title(pending.id).await;
                    if auth.generation.try_get_untracked() != Some(epoch) {
                        return;
                    }
                    title_busy.set(false);
                    if let Ok(Some(saved)) = result {
                        chat.sessions.update(|sessions| {
                            if let Some(session) =
                                sessions.iter_mut().find(|session| session.id == pending.id)
                                && session.auto_title
                                && session.title_revision == pending.title_revision
                            {
                                session.name = saved.name;
                                session.auto_title = saved.auto_title;
                                session.title_revision = saved.title_revision;
                            }
                        });
                    }
                });
                break;
            }
        });
        let preferences = Callback::new(move |(id, preferences): (i64, SessionPreferences)| {
            if state.busy.with_untracked(|busy| busy.contains(&id)) {
                return;
            }
            let epoch = auth.generation.get_untracked();
            let project = projects.active_project.get_untracked();
            if !chat.sessions.with_untracked(|sessions| {
                sessions
                    .iter()
                    .any(|session| session.id == id && session.project_id == project)
            }) {
                return;
            }
            state.busy.update(|busy| {
                busy.insert(id);
            });
            spawn_local(async move {
                let result = api
                    .with_value(Clone::clone)
                    .session_preferences(id, &preferences)
                    .await;
                if auth.generation.try_get_untracked() != Some(epoch) {
                    return;
                }
                state.busy.update(|busy| {
                    busy.remove(&id);
                });
                match result {
                    Ok(saved) => chat.sessions.update(|sessions| {
                        if let Some(session) = sessions.iter_mut().find(|session| session.id == id)
                        {
                            session.pinned = saved.pinned;
                            session.archived = saved.archived;
                        }
                    }),
                    Err(error) => ui.notify(format!("Could not update session: {error}")),
                }
            });
        });
        let download = StoredValue::new_local(download);
        let export = Callback::new(move |id: i64| {
            if state.exporting.with_untracked(|busy| busy.contains(&id)) {
                return;
            }
            let epoch = auth.generation.get_untracked();
            let project = projects.active_project.get_untracked();
            if !chat.sessions.with_untracked(|sessions| {
                sessions
                    .iter()
                    .any(|session| session.id == id && session.project_id == project)
            }) {
                return;
            }
            state.exporting.update(|busy| {
                busy.insert(id);
            });
            spawn_local(async move {
                let result = api.with_value(Clone::clone).export_session(id).await;
                if auth.generation.try_get_untracked() != Some(epoch) {
                    return;
                }
                state.exporting.update(|busy| {
                    busy.remove(&id);
                });
                if projects.active_project.try_get_untracked() != Some(project) {
                    return;
                }
                let result =
                    result.and_then(|export| download.with_value(|host| host.save(&export)));
                if let Err(error) = result {
                    ui.notify(format!("Could not export session: {error}"));
                }
            });
        });
        Self {
            preferences,
            export,
        }
    }
}
use std::collections::HashSet;
