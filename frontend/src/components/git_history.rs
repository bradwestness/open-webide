use super::dropdown::{ActionMenu, DropdownSelect, SelectOption};
use super::ui::{Icon, IconButton, IconName, PanelSearchRow, PanelToolbar};
use leptos::prelude::*;
use openwebide_core::git::history_graph;

/// A single inline History tool panel retains its selection while collapsed.
#[component]
pub fn GitHistory(
    on_open: Callback<String>,
    on_select_branch: Callback<String>,
    on_new_branch: Callback<()>,
) -> impl IntoView {
    let actions = crate::state_actions::git_history::GitHistoryActions::new();
    view! { <GitHistoryContent actions=actions on_open=on_open on_select_branch=on_select_branch on_new_branch=on_new_branch /> }
}

#[component]
fn GitHistoryContent(
    actions: crate::state_actions::git_history::GitHistoryActions,
    #[prop(default = false)] file_history: bool,
    on_open: Callback<String>,
    on_select_branch: Callback<String>,
    on_new_branch: Callback<()>,
) -> impl IntoView {
    let crate::state_actions::git_history::GitHistoryActions {
        commits,
        refs,
        search,
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
        path: selected_path,
        load,
        select,
    } = actions;
    view! {
        <div class="git-history">
            <PanelToolbar class="git-history-toolbar">
                <PanelSearchRow class="git-history-search">
                    <super::ui::TextInput label="Search commit messages" placeholder="Search commit messages…" value=search.read_only() on_change=Callback::new(move |value| search.set(value)) maxlength=256 />
                    <IconButton label="Clear search" disabled=Signal::derive(move || search.with(String::is_empty)) on_click=Callback::new(move |_| search.set(String::new()))><Icon name=IconName::X /></IconButton>
                </PanelSearchRow>
                <IconButton label="Refresh history" disabled=loading.read_only() on_click=Callback::new(move |_| load.run(false))><Icon name=IconName::RefreshCw /></IconButton>
                <Show when=move ||!file_history><ActionMenu aria_label="History actions">
                    <button role="menuitem" class="ui-dropdown-item btn" disabled=move || !reference.with(|reference| reference.as_ref().is_some_and(|value| value.starts_with("refs/heads/") || value.starts_with("refs/remotes/"))) on:click=move |_| {if let Some(branch)=reference.get_untracked().and_then(|value|value.strip_prefix("refs/heads/").or_else(||value.strip_prefix("refs/remotes/")).map(str::to_owned)) {on_select_branch.run(branch);}}><Icon name=IconName::GitBranch /><span>"Checkout branch"</span></button>
                    <button role="menuitem" class="ui-dropdown-item btn" on:click=move |_| on_new_branch.run(())><Icon name=IconName::Plus /><span>"New branch"</span></button>
                </ActionMenu></Show>
            </PanelToolbar>
            <PanelToolbar class="git-history-reference">
                <Icon name=IconName::GitBranch />
                <DropdownSelect label="History branch" trigger_class="btn ghost" value=Signal::derive(move ||reference.get().unwrap_or_default()) options=Signal::derive(move || {
                    let mut options=vec![SelectOption::new("HEAD","Current branch")];if !file_history {options.insert(0,SelectOption::new("","All branches"));}
                    options.extend(refs.get().into_iter().map(|item| {let prefix=match item.kind.as_str(){"branch"=>"heads","remote"=>"remotes",_=>"tags"};SelectOption::new(format!("refs/{prefix}/{}",item.name),format!("{} · {}",item.name,item.kind))}));options
                }) on_change=Callback::new(move |value:String| {reference.set(if value.is_empty(){None}else{Some(value)});load.run(false);}) />
            </PanelToolbar>
            <div class="git-history-body">
                <div class="git-history-main">
                    <div class="git-history-list" role="group" aria-label="Commit history">
                        {move || {
                            let history = commits.get(); let graph = history_graph(&history);
                            history.into_iter().zip(graph).map(|(commit, row)| {
                                let chosen = commit.clone(); let hash = commit.hash.clone(); let pressed_hash = hash.clone();
                                let width = row.width.max(1) * 14 + 12;
                                let paths = row.edges.iter().map(|edge| {
                                    let path = if edge.through_commit { format!("M {} 14 C {} 24 {} 24 {} 32", edge.from*14+12, edge.from*14+12, edge.to*14+12, edge.to*14+12) }
                                        else { format!("M {} 0 L {} 14 C {} 24 {} 24 {} 32", edge.from*14+12, edge.from*14+12, edge.from*14+12, edge.to*14+12, edge.to*14+12) };
                                    (path, edge.color)
                                }).collect::<Vec<_>>();
                                view! { <button class="btn git-history-row" aria-pressed=move || selected.with(|value| value.as_ref().is_some_and(|value| value.hash == pressed_hash)) class:selected=move || selected.with(|value| value.as_ref().is_some_and(|value| value.hash == hash)) on:click=move |_| select.run((chosen.clone(), None, None))>
                                    <svg width=width height="100%" viewBox=format!("0 0 {width} 32") preserveAspectRatio="none" class="git-graph" aria-hidden="true" hidden=file_history>
                                        {paths.into_iter().map(|(path,color)| view!{<path d=path fill="none" stroke=format!("var(--git-lane-{})", color%6) stroke-width="2"/>}).collect_view()}
                                        {row.incoming.then(|| view!{<path d=format!("M {} 0 L {} 14",row.lane*14+12,row.lane*14+12) stroke=format!("var(--git-lane-{})",row.color%6) stroke-width="2"/>})}
                                        <circle cx=row.lane*14+12 cy="14" r="4" fill=format!("var(--git-lane-{})",row.color%6)/>
                                    </svg>
                                    <span class="git-commit-subject"><span class="git-commit-title">{commit.subject}</span>{(!commit.refs.is_empty()).then(|| view!{<span class="git-commit-refs">{commit.refs.join(" · ")}</span>})}</span>
                                    <span class="git-commit-author">{commit.author}</span><time>{commit.authored_at.chars().take(10).collect::<String>()}</time><code>{commit.hash.chars().take(8).collect::<String>()}</code>
                                </button> }
                            }).collect_view()
                        }}
                        <Show when=move || loading.get()><div class="git-history-empty" role="status">"Loading history…"</div></Show>
                        <Show when=move || !loading.get() && commits.with(Vec::is_empty) && error.get().is_none()><div class="git-history-empty">"No matching commits"</div></Show>
                        {move || error.get().map(|message| view!{<div class="git-history-error" role="alert">{message}<button class="btn" on:click=move |_| load.run(false)>"Retry"</button></div>})}
                        <Show when=move || has_more.get()><button class="btn git-history-more" disabled=move || loading.get() on:click=move |_| load.run(true)>"Load older commits"</button></Show>
                    </div>
                    <div class="git-commit-details">
                        {move || selected.get().map(|commit| {
                            let parent_commit=StoredValue::new(commit.clone());
                            view!{
                                <div class="git-inspector-heading"><strong>{commit.subject.clone()}</strong><code>{commit.hash.chars().take(8).collect::<String>()}</code></div>
                                <details class="git-commit-metadata"><summary>{format!("{} · {}",commit.author,commit.authored_at.chars().take(10).collect::<String>())}</summary>
                                    <p>{format!("Author: {} <{}> · {}",commit.author,commit.author_email,commit.authored_at)}</p>
                                    <p>{format!("Committer: {} <{}> · {}",commit.committer,commit.committer_email,commit.committed_at)}</p><code>{commit.hash}</code><pre class="git-commit-message">{commit.message}</pre>
                                </details>
                                <Show when={let count=parent_commit.with_value(|commit|commit.parents.len());move || count > 1}>
                                    <DropdownSelect label="Compare with parent" trigger_class="btn ghost" value=Signal::derive(move ||parent.get().unwrap_or_default()) options=Signal::derive({move ||parent_commit.with_value(|commit|commit.parents.iter().enumerate().map(|(index,hash)|SelectOption::new(hash,format!("Parent {} · {}",index+1,&hash[..8]))).collect())}) on_change=Callback::new(move |hash|select.run((parent_commit.get_value(),Some(hash),None))) />
                                </Show>
                            }
                        })}
                        <Show when=move || selected.get().is_none()><div class="git-history-empty">"Select a commit to inspect its changes"</div></Show>
                        <div class="git-commit-inspector">
                            <div class="git-commit-files" role="tree" aria-label="Changed files">{move || render_file_tree(files.get(), "", actions, on_open)}</div>
                            <div class="git-readonly-diff" role="region" aria-label="Read-only commit diff">
                                <div class="git-diff-filter"><span>{move || selected_path.get().unwrap_or_else(||"Changes".into())}</span></div>
                                <Show when=move ||diff_loading.get()><p class="form-hint" role="status">"Loading changes…"</p></Show>
                                {move ||diff_error.get().map(|message|view!{<p class="git-history-error" role="alert">{message}</p>})}
                                {move ||diff.get().map(|changes|view!{
                                    {changes.truncated.then(||view!{<p class="git-history-limit">"Preview limited to 5,000 lines or 512 KiB."</p>})}
                                    {changes.diff.is_empty().then(||view!{<p class="form-hint">"No text changes (the file may be binary or empty)."</p>})}
                                    {super::editor::render_commit_diff(&changes.diff,&selected_path.get().unwrap_or_default())}
                                })}
                            </div>
                        </div>
                    </div>
                </div>
            </div>
        </div>
    }
}

fn render_file_tree(
    files: Vec<openwebide_core::git::GitCommitFile>,
    prefix: &str,
    actions: crate::state_actions::git_history::GitHistoryActions,
    on_open: Callback<String>,
) -> AnyView {
    let mut folders =
        std::collections::BTreeMap::<String, Vec<openwebide_core::git::GitCommitFile>>::new();
    let mut leaves = Vec::new();
    for file in files {
        let relative = file.path.strip_prefix(prefix).unwrap_or(&file.path);
        if let Some((folder, _)) = relative.split_once('/') {
            folders.entry(folder.to_owned()).or_default().push(file);
        } else {
            leaves.push(file);
        }
    }
    let directories=folders.into_iter().map(|(name,files)|{let next=format!("{prefix}{name}/");view!{<details class="git-history-folder" open><summary><span class="git-folder-chevron"><Icon name=IconName::ChevronDown /></span><Icon name=IconName::Folder />{name}</summary><div role="group">{render_file_tree(files,&next,actions,on_open)}</div></details>}}).collect_view();
    let entries=leaves.into_iter().map(|file|{let click=file.path.clone();let active=click.clone();let open=click.clone();let chosen=click.clone();let label=file.path.strip_prefix(prefix).unwrap_or(&file.path).to_owned();view!{<div class="git-commit-file" role="treeitem" aria-selected=move ||(actions.path.get().as_ref()==Some(&active)).to_string()>
        <button class="btn ghost" title=file.path class:selected=move ||actions.path.get().as_ref()==Some(&chosen) on:click={let path=click.clone();move |_|{if let Some(commit)=actions.selected.get_untracked(){actions.select.run((commit,actions.parent.get_untracked(),Some(path.clone())));}}}><Icon name=IconName::File /><code>{file.status}</code><span>{label}</span></button>
        <IconButton label="Open current file in editor" on_click=Callback::new(move |_|on_open.run(open.clone()))><Icon name=IconName::ExternalLink /></IconButton>
    </div>}}).collect_view();
    view! { {directories}{entries} }.into_any()
}

/// File history uses the same queries and inspector as repository History.
#[component]
pub fn FileHistory(
    path: String,
    on_open: Callback<String>,
    on_close: Callback<()>,
) -> impl IntoView {
    let actions =
        crate::state_actions::git_history::GitHistoryActions::for_path(Some(path.clone()));
    let current_path = StoredValue::new(path.clone());
    let open_current = Callback::new(move |_: String| on_open.run(current_path.get_value()));
    let title = Signal::derive(move || format!("History · {path}"));
    view! {<super::modal::Modal title=title on_close=on_close class="modal git-file-history-modal" size=super::ui::DialogSize::Wide>
        <div class="git-file-timeline" role="group" aria-label="File history timeline">
            {move ||actions.commits.get().into_iter().rev().map(|commit|{let chosen=commit.clone();let hash=commit.hash.clone();view!{<button class="btn ghost" title=format!("{} · {}",commit.authored_at,commit.subject) aria-label=format!("{} · {}",commit.authored_at,commit.subject) aria-pressed=move ||actions.selected.with(|selected|selected.as_ref().is_some_and(|selected|selected.hash==hash)) on:click=move |_|actions.select.run((chosen.clone(),None,None))><span class="git-timeline-dot"></span><time>{commit.authored_at.chars().take(10).collect::<String>()}</time></button>}}).collect_view()}
        </div>
        <GitHistoryContent actions=actions file_history=true on_open=open_current on_select_branch=Callback::new(|_|()) on_new_branch=Callback::new(|()|()) />
    </super::modal::Modal>}
}
