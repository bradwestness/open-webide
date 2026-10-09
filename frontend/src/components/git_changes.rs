use super::ui::{Icon, IconButton, IconName, PanelToolbar};
use leptos::prelude::*;
use openwebide_core::{
    FileEntry,
    git::{GitPathAction, GitStashAction, GitStashRequest},
};

#[component]
pub fn GitChanges(
    on_open: Callback<String>,
    #[prop(optional)] actions: Option<crate::state_actions::git_changes::GitChangesActions>,
) -> impl IntoView {
    let git = expect_context::<crate::state::git::GitState>();
    let actions = actions.unwrap_or_default();
    let files = use_context::<crate::state_actions::file_tree::FileTreeActions>();
    let busy = Signal::derive(move || {
        files.is_none_or(crate::state_actions::file_tree::FileTreeActions::disabled)
            || git.status_error.get().is_some()
    });
    view! {
        <div class="git-changes">
            <super::ui::FeedbackOverlay message=actions.error.read_only() on_dismiss=Callback::new(|()|()) />
            {[true,false].into_iter().map(move |staged| {
                let expanded=RwSignal::new(true);
                let paths=Memo::new(move |_|git.path_changes.with(|changes|changes.as_ref().map_or_else(||if staged {Vec::new()}else{git.status.with(|status|status.as_ref().map_or_else(Vec::new,|status|status.files.keys().cloned().collect()))},|changes|if staged {changes.staged.iter().cloned().collect()} else {changes.unstaged.union(&changes.untracked).cloned().collect()})));
                view! {<section class="git-change-section" class:staged=staged>
                    <PanelToolbar class="git-change-heading">
                        <IconButton label=if staged {"Toggle staged changes"} else {"Toggle unstaged changes"} on_click=Callback::new(move |_|expanded.update(|value|*value = !*value))><Icon name=Signal::derive(move ||if expanded.get(){IconName::ChevronDown}else{IconName::ChevronRight}) /></IconButton>
                        <span>{if staged {"Staged"} else {"Unstaged"}}{move ||format!(" ({})",paths.with(Vec::len))}</span>
                        <IconButton label=if staged {"Unstage all changes"} else {"Stage all changes"} disabled=Signal::derive(move ||busy.get() || paths.with(Vec::is_empty)) on_click=Callback::new(move |_|if let Some(files)=files {files.git_action("",if staged {GitPathAction::UnstageAll}else{GitPathAction::StageAll});})><Icon name=if staged {IconName::ArrowDownToLine}else{IconName::ArrowUpFromLine} /></IconButton>
                    </PanelToolbar>
                    <div class="git-files" role="tree" aria-label=if staged {"Staged changes"} else {"Unstaged changes"} hidden=move ||!expanded.get()>
                        <For each=move ||paths.get() key=Clone::clone children=move |path|{
                            let action_path=path.clone();
                            view!{<div class="git-change-file">
                                <super::file_tree::FileTreeEntry entry=FileEntry {name:path.clone(),path,is_dir:false,size:0} depth=0 on_toggle=Callback::new(|_:String|()) on_open=on_open changes_only=true />
                                <IconButton label=if staged {"Unstage file"}else{"Stage file"} disabled=busy on_click=Callback::new(move |_|if let Some(files)=files {files.git_action(&action_path,if staged {GitPathAction::Unstage}else{GitPathAction::Stage});})><Icon name=if staged {IconName::ArrowDownToLine}else{IconName::ArrowUpFromLine} /></IconButton>
                            </div>}
                        } />
                        <Show when=move ||paths.with(Vec::is_empty)><p class="form-hint">{if staged {"No staged changes"}else{"No unstaged changes"}}</p></Show>
                    </div>
                </section>}
            }).collect_view()}
            <section class="git-stashes">
                <PanelToolbar class="git-change-heading"><span>"Stashes"</span>
                    <super::dropdown::ActionMenu aria_label="Stash actions">
                        <button class="ui-dropdown-item recent-item btn" role="menuitem" disabled=move ||busy.get() ||git.status.with(|status|status.as_ref().is_none_or(|status|status.is_clean)) on:click=move |_|if let Some(files)=files {files.stash(GitStashRequest {action:GitStashAction::Save,hash:None,message:None});}><Icon name=IconName::Archive /><span>"Stash all changes"</span></button>
                        <button class="ui-dropdown-item recent-item btn" role="menuitem" on:click=move |_|actions.refresh.run(())><Icon name=IconName::RefreshCw /><span>"Refresh stashes"</span></button>
                    </super::dropdown::ActionMenu>
                </PanelToolbar>
                <For each=move ||actions.stashes.get() key=|stash|stash.hash.clone() children=move |stash|{
                    let apply=StoredValue::new(stash.hash.clone());let drop=StoredValue::new(stash.hash.clone());
                    view!{<div class="git-stash-row" data-context-menu=""><span title=stash.hash>{stash.reference}{": "}{stash.subject}</span>
                        <super::dropdown::ActionMenu aria_label="Saved stash actions">
                            <button class="ui-dropdown-item recent-item btn" role="menuitem" disabled=busy on:click=move |_|if let Some(files)=files {files.stash(GitStashRequest {action:GitStashAction::Apply,hash:Some(apply.get_value()),message:None});}><Icon name=IconName::ArchiveRestore /><span>"Apply stash"</span></button>
                            <button class="ui-dropdown-item recent-item btn" role="menuitem" disabled=busy on:click=move |_|if let Some(files)=files {files.stash(GitStashRequest {action:GitStashAction::Drop,hash:Some(drop.get_value()),message:None});}><Icon name=IconName::Trash2 /><span>"Drop stash…"</span></button>
                        </super::dropdown::ActionMenu>
                    </div>}
                } />
                <Show when=move ||actions.stashes.with(Vec::is_empty)><p class="form-hint">"No saved stashes"</p></Show>
            </section>
        </div>
    }
}
