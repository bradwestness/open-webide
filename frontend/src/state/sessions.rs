//! Session-list state, independent of workspace transport.
use super::chat::ChatState;
use leptos::prelude::*;
use openwebide_core::ChatSession;
use std::collections::{BTreeSet, HashSet};

#[derive(Clone, Copy)]
pub struct SessionsState {
    pub query: RwSignal<String>,
    pub archived: RwSignal<bool>,
    pub matches: RwSignal<Option<BTreeSet<i64>>>,
    pub searching: RwSignal<bool>,
    pub search_error: RwSignal<Option<String>>,
    pub busy: RwSignal<HashSet<i64>>,
    pub exporting: RwSignal<HashSet<i64>>,
    pub visible: Memo<Vec<ChatSession>>,
}
impl SessionsState {
    pub fn new(chat: ChatState, project: RwSignal<Option<i64>>) -> Self {
        let query = RwSignal::new(String::new());
        let archived = RwSignal::new(false);
        let matches = RwSignal::new(None::<BTreeSet<i64>>);
        let visible = Memo::new(move |_| {
            let query = query.get();
            let archived = archived.get();
            let project = project.get();
            let matches = matches.get();
            let mut sessions: Vec<_> = chat.sessions.with(|sessions| {
                sessions
                    .iter()
                    .filter(|session| {
                        session.project_id == project
                            && session.archived == archived
                            && (query.trim().is_empty()
                                || matches
                                    .as_ref()
                                    .is_some_and(|ids| ids.contains(&session.id)))
                    })
                    .cloned()
                    .collect()
            });
            sessions.sort_by_key(|session| {
                (
                    std::cmp::Reverse(session.pinned),
                    std::cmp::Reverse(session.id),
                )
            });
            sessions
        });
        Self {
            query,
            archived,
            matches,
            visible,
            searching: RwSignal::new(false),
            search_error: RwSignal::new(None),
            busy: RwSignal::new(HashSet::new()),
            exporting: RwSignal::new(HashSet::new()),
        }
    }
}
