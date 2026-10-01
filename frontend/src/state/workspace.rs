use std::collections::{HashMap, HashSet};

use leptos::prelude::*;
use openwebide_core::{FileDiff, FileEntry, SearchHit};

/// Workspace data preserved while a project is not the active tab.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WorkspaceSnapshot {
    pub entries: HashMap<String, Vec<FileEntry>>,
    pub expanded: HashSet<String>,
    pub open_file: Option<String>,
    pub content: String,
    pub dirty: bool,
    pub search: Option<Vec<SearchHit>>,
    pub active_session: Option<i64>,
    pub pending_edits: HashMap<String, FileDiff>,
    pub media_url: Option<String>,
}

/// Active editor and file-browser state, with a snapshot for each project.
#[derive(Clone, Copy)]
pub struct WorkspaceState {
    pub active_project: RwSignal<Option<i64>>,
    pub entries: RwSignal<HashMap<String, Vec<FileEntry>>>,
    pub expanded: RwSignal<HashSet<String>>,
    pub open_file: RwSignal<Option<String>>,
    pub content: RwSignal<String>,
    pub dirty: RwSignal<bool>,
    pub search: RwSignal<Option<Vec<SearchHit>>>,
    pub active_session: RwSignal<Option<i64>>,
    pub pending_edits: RwSignal<HashMap<String, FileDiff>>,
    pub pending_diff: Memo<Option<FileDiff>>,
    pub media_url: RwSignal<Option<String>>,
    pub snapshots: RwSignal<HashMap<i64, WorkspaceSnapshot>>,
}

impl WorkspaceState {
    pub fn new() -> Self {
        Self::with_active_project(RwSignal::new(None))
    }

    pub fn with_active_project(active_project: RwSignal<Option<i64>>) -> Self {
        let open_file = RwSignal::new(None);
        let pending_edits = RwSignal::new(HashMap::new());
        let pending_diff = Memo::new(move |_| {
            let open_file = open_file.get()?;
            pending_edits.with(|pending| pending.get(&open_file).cloned())
        });
        Self {
            active_project,
            entries: RwSignal::new(HashMap::new()),
            expanded: RwSignal::new(HashSet::new()),
            open_file,
            content: RwSignal::new(String::new()),
            dirty: RwSignal::new(false),
            search: RwSignal::new(None),
            active_session: RwSignal::new(None),
            pending_edits,
            pending_diff,
            media_url: RwSignal::new(None),
            snapshots: RwSignal::new(HashMap::new()),
        }
    }

    pub fn active_snapshot(&self) -> WorkspaceSnapshot {
        WorkspaceSnapshot {
            entries: self.entries.get_untracked(),
            expanded: self.expanded.get_untracked(),
            open_file: self.open_file.get_untracked(),
            content: self.content.get_untracked(),
            dirty: self.dirty.get_untracked(),
            search: self.search.get_untracked(),
            active_session: self.active_session.get_untracked(),
            pending_edits: self.pending_edits.get_untracked(),
            media_url: self.media_url.get_untracked(),
        }
    }

    pub fn save_active(&self, project_id: i64) {
        let snapshot = self.active_snapshot();
        self.snapshots.update(|snapshots| {
            let _ = snapshots.insert(project_id, snapshot);
        });
    }

    pub fn restore_project(&self, project_id: i64) {
        let snapshot = self
            .snapshots
            .with_untracked(|snapshots| snapshots.get(&project_id).cloned())
            .unwrap_or_default();
        self.active_project.set(Some(project_id));
        self.apply_snapshot(snapshot);
    }

    /// Save the current workspace, then restore the target project's snapshot.
    pub fn switch_project(&self, current: Option<i64>, target: i64) {
        if current == Some(target) {
            return;
        }
        if let Some(current) = current {
            self.save_active(current);
        }
        self.restore_project(target);
    }

    pub fn clear_active(&self) {
        self.active_project.set(None);
        self.apply_snapshot(WorkspaceSnapshot::default());
    }

    pub fn reset(&self) {
        self.clear_active();
        self.snapshots.set(HashMap::new());
    }

    /// Merge a new agent edit into the active workspace or its saved snapshot.
    pub fn merge_pending(&self, project_id: i64, diff: FileDiff) {
        if self.active_project.get_untracked() == Some(project_id) {
            self.pending_edits
                .update(|pending| crate::pending::merge_pending(pending, diff));
        } else {
            self.snapshots.update(|snapshots| {
                let snapshot = snapshots.entry(project_id).or_default();
                crate::pending::merge_pending(&mut snapshot.pending_edits, diff);
            });
        }
    }

    fn apply_snapshot(&self, snapshot: WorkspaceSnapshot) {
        self.entries.set(snapshot.entries);
        self.expanded.set(snapshot.expanded);
        self.open_file.set(snapshot.open_file);
        self.content.set(snapshot.content);
        self.dirty.set(snapshot.dirty);
        self.search.set(snapshot.search);
        self.active_session.set(snapshot.active_session);
        self.pending_edits.set(snapshot.pending_edits);
        self.media_url.set(snapshot.media_url);
    }
}

impl Default for WorkspaceState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn diff(old: &str, new: &str) -> FileDiff {
        FileDiff {
            path: "main.rs".into(),
            old: Some(old.into()),
            new: new.into(),
            old_unavailable: false,
            backup_path: None,
        }
    }

    #[test]
    fn switching_projects_saves_and_restores_workspace() {
        Owner::new().with(|| {
            let active_project = RwSignal::new(Some(1));
            let workspace = WorkspaceState::with_active_project(active_project);
            workspace.open_file.set(Some("one.rs".into()));
            workspace.content.set("project one".into());
            workspace.dirty.set(true);

            workspace.switch_project(Some(1), 2);
            assert_eq!(active_project.get_untracked(), Some(2));
            assert!(workspace.open_file.get_untracked().is_none());
            workspace.open_file.set(Some("two.rs".into()));
            workspace.content.set("project two".into());

            workspace.switch_project(Some(2), 1);
            assert_eq!(active_project.get_untracked(), Some(1));
            assert_eq!(
                workspace.open_file.get_untracked().as_deref(),
                Some("one.rs")
            );
            assert_eq!(workspace.content.get_untracked(), "project one");
            assert!(workspace.dirty.get_untracked());
        });
    }

    #[test]
    fn merge_pending_preserves_the_first_old_value() {
        Owner::new().with(|| {
            let active_project = RwSignal::new(Some(1));
            let workspace = WorkspaceState::with_active_project(active_project);

            workspace.merge_pending(1, diff("v0", "v1"));
            workspace.merge_pending(1, diff("v1", "v2"));

            let pending = workspace.pending_edits.get_untracked();
            let merged = &pending["main.rs"];
            assert_eq!(merged.old.as_deref(), Some("v0"));
            assert_eq!(merged.new, "v2");
        });
    }
}
