use crate::{
    project_git::ProjectGit,
    state::{auth::AuthState, git::GitState, projects::ProjectsState},
};
use leptos::{prelude::*, task::spawn_local};
use openwebide_core::git::*;

#[derive(Clone, Copy)]
pub struct GitHistoryActions {
    pub commits: RwSignal<Vec<GitHistoryCommit>>,
    pub refs: RwSignal<Vec<GitHistoryRef>>,
    pub search: RwSignal<String>,
    pub displayed: RwSignal<GitHistoryRequest>,
    pub reference: RwSignal<Option<String>>,
    pub loading: RwSignal<bool>,
    pub error: RwSignal<Option<String>>,
    pub has_more: RwSignal<bool>,
    pub selected: RwSignal<Option<GitHistoryCommit>>,
    pub diff: RwSignal<Option<GitCommitDiff>>,
    pub files: RwSignal<Vec<GitCommitFile>>,
    pub diff_loading: RwSignal<bool>,
    pub diff_error: RwSignal<Option<String>>,
    pub parent: RwSignal<Option<String>>,
    pub path: RwSignal<Option<String>>,
    pub load: Callback<bool>,
    pub select: Callback<(GitHistoryCommit, Option<String>, Option<String>)>,
}
impl GitHistoryActions {
    pub fn new() -> Self {
        Self::for_path(None)
    }
    pub fn for_path(file: Option<String>) -> Self {
        let file = StoredValue::new(file);
        let file_history = file.get_value().is_some();
        let facade = expect_context::<ProjectGit>();
        let projects = expect_context::<ProjectsState>();
        let auth = expect_context::<AuthState>();
        let git = expect_context::<GitState>();
        let commits = RwSignal::new(Vec::<GitHistoryCommit>::new());
        let refs = RwSignal::new(Vec::<GitHistoryRef>::new());
        let search = RwSignal::new(String::new());
        let reference = RwSignal::new(file.get_value().map(|_| "HEAD".to_owned()));
        let loading = RwSignal::new(false);
        let error = RwSignal::new(None::<String>);
        let has_more = RwSignal::new(false);
        let selected = RwSignal::new(None::<GitHistoryCommit>);
        let diff = RwSignal::new(None::<GitCommitDiff>);
        let files = RwSignal::new(Vec::<GitCommitFile>::new());
        let diff_loading = RwSignal::new(false);
        let diff_error = RwSignal::new(None::<String>);
        let parent = RwSignal::new(None::<String>);
        let path = RwSignal::new(None::<String>);
        let applied = StoredValue::new(GitHistoryRequest::default());
        let displayed = RwSignal::new(GitHistoryRequest::default());
        let version = RwSignal::new(0_u64);
        let selection_version = RwSignal::new(0_u64);
        let load = Callback::new(move |append: bool| {
            let project = projects.active_project.get_untracked();
            if project.is_none() {
                return;
            }
            if append
                && (loading.get_untracked()
                    || search.get_untracked() != applied.get_value().search
                    || reference.get_untracked() != applied.get_value().reference)
            {
                return;
            }
            version.update(|value| *value += 1);
            let request_version = version.get_untracked();
            let generation = auth.generation.get_untracked();
            let host = facade.revision();
            let request = if append {
                let mut request = applied.get_value();
                request.offset = commits.with_untracked(Vec::len);
                request
            } else {
                GitHistoryRequest {
                    offset: 0,
                    search: search.get_untracked(),
                    reference: reference.get_untracked(),
                    path: file.get_value(),
                }
            };
            loading.set(true);
            error.set(None);
            diff_error.set(None);
            let selected_before = selected.get_untracked();
            let parent_before = parent.get_untracked();
            let path_before = path.get_untracked();
            let current = move || {
                projects.active_project.try_get_untracked() == Some(project)
                    && auth.generation.try_get_untracked() == Some(generation)
                    && version.try_get_untracked() == Some(request_version)
                    && facade.revision() == host
            };
            spawn_local(async move {
                if !current() {
                    return;
                }
                let result = read_query(async {
                    let page = facade.repository(project).await?.history(&request).await?;
                    if !current() {
                        return Err("History query superseded".into());
                    }
                    let detail = if append {
                        None
                    } else {
                        let chosen = selected_before.as_ref().and_then(|selected| {
                            page.commits
                                .iter()
                                .find(|commit| commit.hash == selected.hash)
                                .or_else(|| page.commits.first())
                        });
                        if let Some(commit) = chosen {
                            let surviving = selected_before
                                .as_ref()
                                .is_some_and(|old| old.hash == commit.hash);
                            Some(
                                prepare_detail(
                                    facade,
                                    project,
                                    commit.clone(),
                                    if surviving { parent_before } else { None },
                                    if surviving { path_before } else { None },
                                    file_history,
                                    None,
                                )
                                .await?,
                            )
                        } else {
                            None
                        }
                    };
                    Ok::<_, String>((page, detail))
                })
                .await;
                if projects.active_project.try_get_untracked() != Some(project)
                    || auth.generation.try_get_untracked() != Some(generation)
                    || facade.revision() != host
                    || version.try_get_untracked() != Some(request_version)
                {
                    return;
                }
                loading.set(false);
                match result {
                    Ok((page, detail)) => batch(|| {
                        if refs.get_untracked() != page.refs {
                            refs.set(page.refs);
                        }
                        has_more.set(page.has_more);
                        if append {
                            commits.update(|commits| commits.extend(page.commits));
                        } else {
                            displayed.set(request.clone());
                            applied.set_value(request);
                            if commits.get_untracked() != page.commits {
                                commits.set(page.commits);
                            }
                            if let Some((commit, comparison, selected_path, changed_files, value)) =
                                detail
                            {
                                selection_version.update(|version| *version += 1);
                                if selected.get_untracked().as_ref() != Some(&commit) {
                                    selected.set(Some(commit));
                                }
                                parent.set(comparison);
                                path.set(selected_path);
                                if files.get_untracked() != changed_files {
                                    files.set(changed_files);
                                }
                                if diff.get_untracked().as_ref() != Some(&value) {
                                    diff.set(Some(value));
                                }
                                diff_error.set(None);
                                diff_loading.set(false);
                            } else {
                                selected.set(None);
                                parent.set(None);
                                path.set(None);
                                files.set(Vec::new());
                                diff.set(None);
                            }
                        }
                    }),
                    Err(message) => error.set(Some(message)),
                }
            });
        });
        let settings = expect_context::<crate::state::settings::SettingsState>();
        let head = Memo::new(move |_| {
            git.status.with(|status| {
                status
                    .as_ref()
                    .map(|status| (status.branch.clone(), status.commit_hash.clone()))
            })
        });
        let boundary = StoredValue::new(None);
        Effect::new(move |_| {
            let identity = (
                projects.active_project.get(),
                auth.generation.get(),
                settings.bridge_url.get(),
                facade.revision(),
            );
            let changed_scope = boundary.get_value().as_ref() != Some(&identity);
            if changed_scope {
                boundary.set_value(Some(identity));
                search.set(String::new());
                reference.set(file.get_value().map(|_| "HEAD".to_owned()));
            }
            settings.bridge_url.track();
            projects.local_handles.track();
            head.track();
            git.history_revision.track();
            projects.active_project.track();
            auth.generation.track();
            git.branch_revision.track();
            version.update(|value| *value += 1);
            selection_version.update(|value| *value += 1);
            if changed_scope {
                commits.set(Vec::new());
                refs.set(Vec::new());
                selected.set(None);
                diff.set(None);
                files.set(Vec::new());
                parent.set(None);
                path.set(None);
            }
            loading.set(false);
            diff_loading.set(false);
            load.run(false);
        });
        let search_revision = RwSignal::new(0_u64);
        let previous_search = StoredValue::new(String::new());
        Effect::new(move |_| {
            let query = search.get();
            if previous_search.get_value() == query {
                return;
            }
            previous_search.set_value(query.clone());
            search_revision.update(|revision| *revision += 1);
            version.update(|revision| *revision += 1);
            has_more.set(false);
            loading.set(false);
            let revision = search_revision.get_untracked();
            let query_version = version.get_untracked();
            let project = projects.active_project.get_untracked();
            let generation = auth.generation.get_untracked();
            let host = facade.revision();
            spawn_local(async move {
                crate::util::sleep_ms(300).await;
                if version.try_get_untracked() == Some(query_version)
                    && search_revision.try_get_untracked() == Some(revision)
                    && search.try_get_untracked() == Some(query)
                    && projects.active_project.try_get_untracked() == Some(project)
                    && auth.generation.try_get_untracked() == Some(generation)
                    && facade.revision() == host
                {
                    load.run(false);
                }
            });
        });
        let select = Callback::new(
            move |(commit, requested_parent, requested_path): (
                GitHistoryCommit,
                Option<String>,
                Option<String>,
            )| {
                let requested_parent = requested_parent.or_else(|| commit.parents.first().cloned());
                let same_comparison = selected.with_untracked(|selected| {
                    selected
                        .as_ref()
                        .is_some_and(|selected| selected.hash == commit.hash)
                }) && parent.get_untracked() == requested_parent;
                let requested_path = requested_path.or_else(|| commit.history_path.clone());
                version.update(|version| *version += 1);
                loading.set(false);
                diff_error.set(None);
                diff_loading.set(true);
                selection_version.update(|value| *value += 1);
                let revision = selection_version.get_untracked();
                let project = projects.active_project.get_untracked();
                let generation = auth.generation.get_untracked();
                let host = facade.revision();
                let cached_files = same_comparison.then(|| files.get_untracked());
                spawn_local(async move {
                    if projects.active_project.try_get_untracked() != Some(project)
                        || auth.generation.try_get_untracked() != Some(generation)
                        || selection_version.try_get_untracked() != Some(revision)
                        || facade.revision() != host
                    {
                        return;
                    }
                    let result = read_query(prepare_detail(
                        facade,
                        project,
                        commit,
                        requested_parent,
                        requested_path,
                        file_history,
                        cached_files,
                    ))
                    .await;
                    if projects.active_project.try_get_untracked() != Some(project)
                        || auth.generation.try_get_untracked() != Some(generation)
                        || facade.revision() != host
                        || selection_version.try_get_untracked() != Some(revision)
                    {
                        return;
                    }
                    diff_loading.set(false);
                    match result {
                        Ok((commit, comparison, selected_path, changed_files, value)) => {
                            batch(|| {
                                if selected.get_untracked().as_ref() != Some(&commit) {
                                    selected.set(Some(commit));
                                }
                                parent.set(comparison);
                                path.set(selected_path);
                                if files.get_untracked() != changed_files {
                                    files.set(changed_files);
                                }
                                if diff.get_untracked().as_ref() != Some(&value) {
                                    diff.set(Some(value));
                                }
                            });
                        }
                        Err(message) => diff_error.set(Some(message)),
                    }
                });
            },
        );
        Self {
            commits,
            refs,
            search,
            displayed,
            reference,
            loading,
            error,
            has_more,
            selected,
            diff,
            files,
            diff_loading,
            diff_error,
            parent,
            path,
            load,
            select,
        }
    }
}

impl Default for GitHistoryActions {
    fn default() -> Self {
        Self::new()
    }
}
pub(crate) async fn read_query<T>(
    query: impl std::future::Future<Output = Result<T, String>>,
) -> Result<T, String> {
    match futures::future::select(Box::pin(query), Box::pin(crate::util::sleep_ms(40000))).await {
        futures::future::Either::Left((result, _)) => result,
        futures::future::Either::Right(_) => {
            Err("Git query timed out. Check the execution host and retry.".into())
        }
    }
}

async fn prepare_detail(
    facade: ProjectGit,
    project: Option<i64>,
    commit: GitHistoryCommit,
    comparison: Option<String>,
    requested_path: Option<String>,
    file_history: bool,
    retained_files: Option<Vec<GitCommitFile>>,
) -> Result<
    (
        GitHistoryCommit,
        Option<String>,
        Option<String>,
        Vec<GitCommitFile>,
        GitCommitDiff,
    ),
    String,
> {
    let comparison = comparison.or_else(|| commit.parents.first().cloned());
    let mut path = requested_path.or_else(|| commit.history_path.clone());
    let repo = facade.repository(project).await?;
    let mut value = repo
        .commit_diff(&GitCommitDiffRequest {
            hash: commit.hash.clone(),
            parent: comparison.clone(),
            path: path.clone(),
        })
        .await?;
    let changed_files = retained_files.unwrap_or_else(|| {
        value
            .files
            .iter()
            .filter(|file| {
                !file_history
                    || Some(&file.path) == path.as_ref()
                    || file.previous_path.as_ref() == path.as_ref()
            })
            .cloned()
            .collect()
    });
    if path.is_none()
        && let Some(first) = value.files.first()
    {
        path = Some(first.path.clone());
        value = repo
            .commit_diff(&GitCommitDiffRequest {
                hash: commit.hash.clone(),
                parent: comparison.clone(),
                path: path.clone(),
            })
            .await?;
    }
    Ok((commit, comparison, path, changed_files, value))
}
