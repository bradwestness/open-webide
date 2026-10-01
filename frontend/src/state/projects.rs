use std::collections::HashSet;

use leptos::prelude::*;
use openwebide_core::{Project, WorkspaceMode};

/// Project lists, open tabs, and the currently selected project.
#[derive(Clone, Copy)]
pub struct ProjectsState {
    pub projects: RwSignal<Vec<Project>>,
    pub open_tab_ids: RwSignal<Vec<i64>>,
    pub active_project: RwSignal<Option<i64>>,
    pub local_mode: Memo<bool>,
    pub needs_grant: RwSignal<HashSet<i64>>,
    pub projects_loaded: RwSignal<bool>,
    #[cfg(target_arch = "wasm32")]
    pub local_handles: RwSignal<std::collections::HashMap<i64, web_sys::FileSystemDirectoryHandle>>,
}

impl ProjectsState {
    pub fn new() -> Self {
        Self::with_active_project(RwSignal::new(None))
    }

    pub fn with_active_project(active_project: RwSignal<Option<i64>>) -> Self {
        let projects = RwSignal::new(Vec::<Project>::new());
        let local_mode = Memo::new(move |_| {
            active_project.get().is_some_and(|active_id| {
                projects
                    .get()
                    .iter()
                    .any(|project| project.id == active_id && project.mode == WorkspaceMode::Local)
            })
        });
        Self {
            projects,
            open_tab_ids: RwSignal::new(Vec::new()),
            active_project,
            local_mode,
            needs_grant: RwSignal::new(HashSet::new()),
            projects_loaded: RwSignal::new(false),
            #[cfg(target_arch = "wasm32")]
            local_handles: RwSignal::new(std::collections::HashMap::new()),
        }
    }

    pub fn project(&self, id: i64) -> Option<Project> {
        self.projects
            .with_untracked(|projects| projects.iter().find(|project| project.id == id).cloned())
    }

    pub fn open_tabs(&self) -> Vec<Project> {
        let projects = self.projects.get();
        self.open_tab_ids.with(|ids| {
            ids.iter()
                .filter_map(|id| projects.iter().find(|project| project.id == *id).cloned())
                .collect()
        })
    }

    pub fn add_project(&self, project: Project) -> bool {
        let mut added = false;
        self.projects.update(|projects| {
            if !projects.iter().any(|existing| existing.id == project.id) {
                projects.push(project);
                added = true;
            }
        });
        added
    }

    /// Add an existing project to the open tabs, preserving tab order.
    pub fn open_tab(&self, id: i64) -> bool {
        if self.project(id).is_none() {
            return false;
        }

        let mut opened = false;
        self.open_tab_ids.update(|ids| {
            if !ids.contains(&id) {
                ids.push(id);
                opened = true;
            }
        });
        opened
    }

    pub fn select_project(&self, id: i64) {
        self.active_project.set(Some(id));
    }

    /// Close a tab and select the tab now occupying its position, or the last
    /// remaining tab when the closed tab was at the end.
    pub fn close_tab(&self, id: i64) -> Option<i64> {
        let active = self.active_project.get_untracked();
        let mut ids = self.open_tab_ids.get_untracked();
        let Some(index) = ids.iter().position(|tab_id| *tab_id == id) else {
            return active;
        };

        ids.remove(index);
        self.open_tab_ids.set(ids.clone());

        if active == Some(id) {
            let next = ids
                .get(index)
                .or_else(|| index.checked_sub(1).and_then(|previous| ids.get(previous)))
                .copied();
            self.active_project.set(next);
            next
        } else {
            active
        }
    }

    pub fn reset(&self) {
        self.projects.set(Vec::new());
        self.open_tab_ids.set(Vec::new());
        self.active_project.set(None);
        self.needs_grant.set(HashSet::new());
        self.projects_loaded.set(false);
        #[cfg(target_arch = "wasm32")]
        self.local_handles.set(std::collections::HashMap::new());
    }
}

impl Default for ProjectsState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use openwebide_core::WorkspaceMode;

    fn project(id: i64) -> Project {
        Project {
            id,
            name: format!("project-{id}"),
            mode: WorkspaceMode::Remote,
            path: None,
            user_id: None,
            created_at: id,
        }
    }

    #[test]
    fn closing_an_active_tab_selects_the_neighbor() {
        Owner::new().with(|| {
            let projects = ProjectsState::new();
            projects
                .projects
                .set(vec![project(1), project(2), project(3)]);
            projects.open_tab_ids.set(vec![1, 2, 3]);
            projects.active_project.set(Some(2));

            assert_eq!(projects.close_tab(2), Some(3));
            assert_eq!(projects.open_tab_ids.get_untracked(), vec![1, 3]);
            assert_eq!(projects.active_project.get_untracked(), Some(3));

            assert_eq!(projects.close_tab(3), Some(1));
            assert_eq!(projects.open_tab_ids.get_untracked(), vec![1]);
            assert_eq!(projects.active_project.get_untracked(), Some(1));
        });
    }
}
