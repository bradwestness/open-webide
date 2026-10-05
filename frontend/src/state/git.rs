use std::collections::HashMap;

use leptos::prelude::*;
use openwebide_core::{FileDiff, GitBranchInfo, GitCheckoutResult, GitRepoStatus};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeadContent {
    pub project_id: Option<i64>,
    pub path: String,
    pub content: Result<String, String>,
}

/// Current Git status and HEAD content, with statuses retained per project.
#[derive(Clone, Copy)]
pub struct GitState {
    pub active_project: RwSignal<Option<i64>>,
    pub status: RwSignal<Option<GitRepoStatus>>,
    pub head_content: RwSignal<Option<HeadContent>>,
    pub branches: RwSignal<Vec<GitBranchInfo>>,
    pub branches_loading: RwSignal<bool>,
    pub branches_error: RwSignal<Option<String>>,
    pub branch_busy: RwSignal<bool>,
    pub branch_revision: RwSignal<u64>,
    statuses_by_project: RwSignal<HashMap<i64, Option<GitRepoStatus>>>,
}

impl GitState {
    pub fn new() -> Self {
        Self::with_active_project(RwSignal::new(None))
    }

    pub fn with_active_project(active_project: RwSignal<Option<i64>>) -> Self {
        Self {
            active_project,
            status: RwSignal::new(None),
            head_content: RwSignal::new(None),
            branches: RwSignal::new(Vec::new()),
            branches_loading: RwSignal::new(false),
            branches_error: RwSignal::new(None),
            branch_busy: RwSignal::new(false),
            branch_revision: RwSignal::new(0),
            statuses_by_project: RwSignal::new(HashMap::new()),
        }
    }

    pub fn set_status(&self, status: GitRepoStatus) {
        self.status.set(Some(status));
    }

    pub fn file_status(&self, path: &str) -> Option<openwebide_core::GitFileStatus> {
        self.status
            .with_untracked(|status| status.as_ref()?.files.get(path).copied())
    }

    pub fn is_clean(&self) -> Option<bool> {
        self.status
            .with_untracked(|status| status.as_ref().map(|status| status.is_clean))
    }

    pub fn switch_project(&self, current: Option<i64>, target: i64) {
        if current == Some(target) {
            return;
        }
        if let Some(current) = current {
            let status = self.status.get_untracked();
            self.statuses_by_project.update(|statuses| {
                let _ = statuses.insert(current, status);
            });
        }
        let status = self
            .statuses_by_project
            .with_untracked(|statuses| statuses.get(&target).cloned().flatten());
        self.reset_branches();
        self.status.set(status);
        self.head_content.set(None);
        self.active_project.set(Some(target));
    }

    pub fn reset_branches(&self) {
        self.branch_revision.update(|revision| *revision += 1);
        self.branches.set(Vec::new());
        self.branches_loading.set(false);
        self.branches_error.set(None);
        self.branch_busy.set(false);
    }

    pub fn reset_head_content(&self) {
        self.head_content.set(None);
    }

    pub fn save_active(&self, project_id: i64) {
        let status = self.status.get_untracked();
        self.statuses_by_project.update(|statuses| {
            statuses.insert(project_id, status);
        });
    }

    pub fn clear_active(&self) {
        self.reset_branches();
        self.status.set(None);
        self.head_content.set(None);
    }

    pub fn forget_project(&self, project_id: i64) {
        self.statuses_by_project.update(|statuses| {
            statuses.remove(&project_id);
        });
    }

    pub fn reset(&self) {
        self.clear_active();
        self.active_project.set(None);
        self.statuses_by_project.set(HashMap::new());
    }

    pub fn checkout_notice(result: &GitCheckoutResult) -> String {
        format!(
            "Switched to branch `{}` (previous: `{}`).",
            result.branch,
            result.previous_branch.as_deref().unwrap_or("none")
        )
    }

    pub fn head_diff(
        &self,
        project_id: Option<i64>,
        open_file: Option<String>,
        content: String,
    ) -> Option<FileDiff> {
        let open_file = open_file?;
        let head = self.head_content.get_untracked()?;
        if head.project_id != project_id || head.path != open_file || head.content.is_err() {
            return None;
        }
        let old = head.content.ok();
        if old.is_none() && content.is_empty() {
            return None;
        }
        Some(FileDiff {
            path: open_file,
            old,
            new: content,
            old_unavailable: false,
            backup_path: None,
        })
    }

    pub fn can_revert(&self, project_id: Option<i64>, open_file: Option<&str>) -> bool {
        let Some(open_file) = open_file else {
            return false;
        };
        self.head_content.with_untracked(|head| {
            head.as_ref().is_some_and(|head| {
                head.project_id == project_id && head.path == open_file && head.content.is_ok()
            })
        })
    }
}

impl Default for GitState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_switch_restores_git_status_and_clears_head_content() {
        Owner::new().with(|| {
            let active_project = RwSignal::new(Some(1));
            let git = GitState::with_active_project(active_project);
            git.set_status(GitRepoStatus {
                branch: "feature".into(),
                ..GitRepoStatus::default()
            });
            git.head_content.set(Some(HeadContent {
                project_id: Some(1),
                path: "main.rs".into(),
                content: Ok("old".into()),
            }));

            git.switch_project(Some(1), 2);
            assert!(git.status.get_untracked().is_none());
            assert!(git.head_content.get_untracked().is_none());

            git.switch_project(Some(2), 1);
            assert_eq!(git.status.get_untracked().unwrap().branch, "feature");
        });
    }

    #[test]
    fn binary_head_hides_revert_and_has_no_text_diff() {
        Owner::new().with(|| {
            let git = GitState::new();
            git.head_content.set(Some(HeadContent {
                project_id: Some(1),
                path: "image.png".into(),
                content: Err("binary file".into()),
            }));
            assert!(!git.can_revert(Some(1), Some("image.png")));
            assert!(
                git.head_diff(Some(1), Some("image.png".into()), "working content".into())
                    .is_none()
            );
        });
    }

    #[test]
    fn checkout_notice_keeps_the_existing_message() {
        let result = GitCheckoutResult {
            branch: "feature/new".into(),
            previous_branch: Some("main".into()),
            switched: true,
        };

        assert_eq!(
            GitState::checkout_notice(&result),
            "Switched to branch `feature/new` (previous: `main`)."
        );
    }

    #[test]
    fn forgetting_a_project_drops_its_cached_status() {
        Owner::new().with(|| {
            let active_project = RwSignal::new(Some(1));
            let git = GitState::with_active_project(active_project);
            git.set_status(GitRepoStatus {
                branch: "feature".into(),
                ..GitRepoStatus::default()
            });

            git.switch_project(Some(1), 2);
            git.forget_project(1);
            git.switch_project(Some(2), 1);

            assert!(git.status.get_untracked().is_none());
        });
    }
}
