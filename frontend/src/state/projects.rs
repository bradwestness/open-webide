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
    pub recent_projects: Memo<Vec<Project>>,
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
                projects.with(|projects| {
                    projects.iter().any(|project| {
                        project.id == active_id && project.mode == WorkspaceMode::Local
                    })
                })
            })
        });
        let open_tab_ids = RwSignal::new(Vec::new());
        let recent_projects = Memo::new(move |_| {
            projects.with(|projects| open_tab_ids.with(|ids| recent_projects_for(projects, ids)))
        });
        Self {
            projects,
            open_tab_ids,
            recent_projects,
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
        self.projects.with(|projects| {
            self.open_tab_ids.with(|ids| {
                ids.iter()
                    .filter_map(|id| projects.iter().find(|project| project.id == *id).cloned())
                    .collect()
            })
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

fn recent_projects_for(projects: &[Project], open_ids: &[i64]) -> Vec<Project> {
    let open: Vec<_> = open_ids
        .iter()
        .filter_map(|id| projects.iter().find(|project| project.id == *id))
        .collect();
    let open_keys: HashSet<_> = open
        .iter()
        .map(|project| project_workspace_key(project))
        .collect();
    let mut seen_keys = HashSet::new();
    projects
        .iter()
        .rev()
        .filter(|project| {
            let key = project_workspace_key(project);
            !open.iter().any(|tab| tab.id == project.id)
                && !open_keys.contains(&key)
                && seen_keys.insert(key)
        })
        .cloned()
        .collect()
}

fn project_workspace_key(p: &Project) -> String {
    let mode_str = p.mode.as_str();
    if let Some(path) = &p.path {
        format!("{}:{}", mode_str, path.trim_matches('/'))
    } else {
        format!("{}:name:{}", mode_str, p.name.trim())
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
    fn recent_projects_preserve_reverse_order_and_workspace_identity() {
        Owner::new().with(|| {
            let state = ProjectsState::new();
            let mut saved: Vec<_> = (1..=8).map(project).collect();
            saved[0].path = Some("/same/".into());
            saved[1].path = Some("same".into());
            saved[2].name = " café ".into();
            saved[3].name = "café".into();
            saved[4].path = Some("/same/".into());
            saved[4].mode = WorkspaceMode::Local;
            saved[5].path = Some("same".into());
            saved[5].mode = WorkspaceMode::Local;
            saved[6].path = Some("/open/".into());
            saved[7].path = Some("open".into());
            state.projects.set(saved);
            state.open_tab_ids.set(vec![1, 7, 999]);
            assert_eq!(
                state
                    .recent_projects
                    .get()
                    .iter()
                    .map(|project| project.id)
                    .collect::<Vec<_>>(),
                vec![6, 4]
            );
            state.open_tab_ids.set(vec![1, 7, 6, 4]);
            assert!(state.recent_projects.with(Vec::is_empty));
            state.open_tab_ids.set(Vec::new());
            assert_eq!(
                state
                    .recent_projects
                    .get()
                    .iter()
                    .map(|project| project.id)
                    .collect::<Vec<_>>(),
                vec![8, 6, 4, 2]
            );
            assert!(state.open_tabs().is_empty());
            state.open_tab_ids.set(vec![4, 1, 999]);
            assert_eq!(
                state
                    .open_tabs()
                    .iter()
                    .map(|project| project.id)
                    .collect::<Vec<_>>(),
                vec![4, 1]
            );
        });
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
