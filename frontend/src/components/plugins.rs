use super::{
    dropdown::{ActionMenu, DropdownSelect, SelectOption},
    ui::{
        Button, ButtonVariant, DisclosurePanel, FormField, FormSection, Icon, IconName,
        InlineActions, PanelSearchRow, TextInput,
    },
};
use crate::{
    project_plugins::{CatalogSelection, ProjectPluginActions},
    state::{plugins::PluginsState, projects::ProjectsState},
};
use leptos::prelude::*;
use openwebide_core::plugins::{
    PluginInstallation,
    marketplace::{CachedMarketplace, CatalogPlugin, MarketplaceSource},
};

#[component]
pub fn Plugins() -> impl IntoView {
    let state = expect_context::<PluginsState>();
    let actions = expect_context::<ProjectPluginActions>();
    let projects = expect_context::<ProjectsState>();
    let ui = expect_context::<crate::state::ui::UiState>();
    let settings = expect_context::<crate::state::settings::SettingsState>();
    let attempted = RwSignal::new(false);
    let auth = expect_context::<crate::state::auth::AuthState>();
    Effect::new(move |_| {
        auth.generation.track();
        attempted.set(false);
    });
    Effect::new(move |_| {
        if state.loaded.get()
            && !state.busy.get()
            && !attempted.get()
            && state
                .marketplaces
                .with(|m| m.catalogs.is_empty() && !m.sources.is_empty())
        {
            attempted.set(true);
            actions.refresh_catalogs.run(());
        }
    });
    view! {
        <div class="ui-section-content">
            <PanelSearchRow unpadded=true>
                <input type="search" class="form-input panel-search-input" aria-label="Search plugins" placeholder="Search plugins…" maxlength="128"
                    prop:value=move ||state.search.get() on:input=move |event|state.search.set(event_target_value(&event))/>
                <ActionMenu aria_label="Plugin actions" icon=IconName::Menu>
                    <button type="button" role="menuitem" class="ui-dropdown-item recent-item" disabled=move ||state.busy.get() on:click=move |_|actions.refresh.run(())><Icon name=IconName::RefreshCw/><span>"Refresh installations"</span></button>
                    <button type="button" role="menuitem" class="ui-dropdown-item recent-item" disabled=move ||state.busy.get() on:click=move |_|actions.refresh_catalogs.run(())><Icon name=IconName::RefreshCw/><span>"Refresh marketplaces"</span></button>
                    <button type="button" role="menuitem" class="ui-dropdown-item recent-item" on:click=move |_| {
                        ui.plugins_open.set(false);
                        settings.requested_tab.set(5);
                        settings.show_settings.set(true);
                    }><Icon name=IconName::Settings/><span>"Manage marketplace sources"</span></button>
                </ActionMenu>
            </PanelSearchRow>
            <p class="form-hint">"Browse packages, install them on a host, and enable their skills for this project."</p>
            <Show when=move ||state.busy.get()><p class="form-hint" role="status">"Working…"</p></Show>
            <Show when=move ||state.error.get().is_some()><p class="error" role="alert">{move ||state.error.get().unwrap_or_default()}</p></Show>
            <For each=move ||state.failures.get() key=|f|(f.source.repository.clone(),f.source.reference.clone(),f.source.path.clone()) children=move |failure|view!{<p class="error" role="alert">{format!("{}: {} Previously cached releases remain available.",failure.source.repository,failure.message)}</p>}/>
            <Show when=move ||projects.active_project.get().is_none()><p class="form-hint">"Open a project to install or enable a plugin on its host."</p></Show>
            <For each=move ||state.marketplaces.get().catalogs key=|c|(c.source.repository.clone(),c.source.reference.clone(),c.source.path.clone(),c.commit.clone()) children=move |catalog| {
                let name=catalog.catalog.name.clone();let plugins=catalog.catalog.plugins.clone();let cache=StoredValue::new(catalog);
                view!{<FormSection title="Marketplace"><p class="form-hint">{name}</p>
                    <For each=move ||{let query=state.search.get().to_lowercase();plugins.iter().filter(|p|format!("{} {} {} {}",p.publisher,p.name,p.display_name,p.description).to_lowercase().contains(&query)).cloned().collect::<Vec<_>>()} key=|p|(p.publisher.clone(),p.name.clone()) children=move |plugin|view!{<CatalogPackage catalog=cache.get_value() plugin=plugin/>}/>
                </FormSection>}
            }/>
            <Show when=move ||state.loaded.get()&&!state.busy.get()&&state.marketplaces.with(|m|m.catalogs.is_empty())><p class="form-hint">"No cached catalogs. Refresh marketplaces to fetch the configured repositories."</p></Show>
            <FormSection title="Installed packages" description="Enabling a package adds managed skills to this project. Version changes take effect here when you apply them.">
                <For each=move ||state.installations.get() key=|e|(e.prepared.source.repository.clone(),e.prepared.source.path.clone(),e.revision) children=move |entry|view!{<InstalledPackage entry=entry/>}/>
            </FormSection>
            <DisclosurePanel summary=||"Install a pinned package manually"><div class="ui-section-content">
                <FormField label="Repository URL"><TextInput label="Plugin repository URL" value=state.repository.read_only() on_change=Callback::new(move |v|state.repository.set(v)) maxlength=2048 disabled=state.busy.read_only()/></FormField>
                <FormField label="Commit"><TextInput label="Plugin commit" value=state.commit.read_only() on_change=Callback::new(move |v|state.commit.set(v)) maxlength=64 disabled=state.busy.read_only()/></FormField>
                <FormField label="Package directory"><TextInput label="Plugin package directory" value=state.path.read_only() on_change=Callback::new(move |v|state.path.set(v)) maxlength=512 disabled=state.busy.read_only()/></FormField>
                <p class="form-hint">"Use a full commit ID and the directory containing plugin.json. Use . for the repository root."</p>
                <InlineActions><Button disabled=Signal::derive(move ||state.busy.get()||projects.active_project.get().is_none()) on_click=Callback::new(move |_|actions.install.run(()))><Icon name=IconName::Plus/>"Install on project host"</Button></InlineActions>
            </div></DisclosurePanel>
        </div>
    }
}
/// Search and package lifecycle controls opened from the status bar.
#[component]
pub fn PluginsDialog() -> impl IntoView {
    let ui = expect_context::<crate::state::ui::UiState>();
    view! {
        <super::modal::Modal title=Signal::derive(|| "Plugins".to_string())
            on_close=Callback::new(move |()| ui.plugins_open.set(false))
            size=super::ui::DialogSize::Wide>
            <super::ui::DialogBody><Plugins/></super::ui::DialogBody>
        </super::modal::Modal>
    }
}

/// Marketplace configuration stays separate from discovery and package lifecycle.
#[component]
pub fn PluginMarketplaceSources() -> impl IntoView {
    let state = expect_context::<PluginsState>();
    let actions = expect_context::<ProjectPluginActions>();
    let ui = expect_context::<crate::state::ui::UiState>();
    let settings = expect_context::<crate::state::settings::SettingsState>();
    view! {
        <FormSection title="Plugin marketplaces" description="Configure the public Git repositories used to discover plugins.">
            <Show when=move ||state.busy.get()><p class="form-hint" role="status">"Working…"</p></Show>
            <Show when=move ||state.error.get().is_some()><p class="error" role="alert">{move ||state.error.get().unwrap_or_default()}</p></Show>
            <div class="ui-section-content">
                <p class="form-hint">"Each catalog lists packages in its own public Git repository. Leave the reference empty to follow its default branch."</p>
                <For each=move ||state.marketplaces.get().sources key=|s|(s.repository.clone(),s.reference.clone(),s.path.clone()) children=move |source| {
                    let official=source==MarketplaceSource::official();let remove=StoredValue::new(source.clone());
                    view!{<div class="ui-section-content"><p class="form-hint">{format!("{} · {} · {}",source.repository,if source.reference.is_empty(){"default branch"}else{&source.reference},source.path)}</p><Show when=move ||official><p class="form-hint">"Official marketplace · always available"</p></Show><Show when=move ||!official><InlineActions><Button disabled=state.busy.read_only() on_click=Callback::new(move |_|{let mut sources=state.marketplaces.get_untracked().sources;sources.retain(|s|s!=&remove.get_value());actions.save_sources.run(sources);})>"Remove marketplace"</Button></InlineActions></Show></div>}
                }/>
                <FormField label="Repository URL"><TextInput label="Marketplace repository URL" value=state.marketplace_repository.read_only() on_change=Callback::new(move |v|state.marketplace_repository.set(v)) maxlength=2048 disabled=state.busy.read_only()/></FormField>
                <FormField label="Reference"><TextInput label="Marketplace reference" value=state.marketplace_reference.read_only() on_change=Callback::new(move |v|state.marketplace_reference.set(v)) maxlength=256 disabled=state.busy.read_only()/></FormField>
                <FormField label="Catalog path"><TextInput label="Marketplace catalog path" value=state.marketplace_path.read_only() on_change=Callback::new(move |v|state.marketplace_path.set(v)) maxlength=512 disabled=state.busy.read_only()/></FormField>
                <InlineActions><Button disabled=state.busy.read_only() on_click=Callback::new(move |_|{let mut sources=state.marketplaces.get_untracked().sources;sources.push(MarketplaceSource{repository:state.marketplace_repository.get_untracked().trim().into(),reference:state.marketplace_reference.get_untracked().trim().into(),path:state.marketplace_path.get_untracked().trim().into()});actions.save_sources.run(sources);})><Icon name=IconName::Plus/>"Add marketplace"</Button></InlineActions>
            </div>

            <InlineActions><Button variant=ButtonVariant::Ghost on_click=Callback::new(move |_| {
                settings.show_settings.set(false);
                ui.plugins_open.set(true);
            })>"Browse plugins"</Button></InlineActions>
        </FormSection>
    }
}

#[component]
fn CatalogPackage(catalog: CachedMarketplace, plugin: CatalogPlugin) -> impl IntoView {
    let state = expect_context::<PluginsState>();
    let actions = expect_context::<ProjectPluginActions>();
    let projects = expect_context::<ProjectsState>();
    let version = RwSignal::new(plugin.releases[0].version.clone());
    let options = plugin
        .releases
        .iter()
        .map(|r| SelectOption::new(&r.version, &r.version))
        .collect::<Vec<_>>();
    let publisher = plugin.publisher.clone();
    let name = plugin.name.clone();
    let source = catalog.source.clone();
    let inspect = StoredValue::new((catalog, plugin.clone()));
    view! {<DisclosurePanel summary=move ||plugin.display_name.clone()><div class="ui-section-content">
        <p class="form-hint">{plugin.description}</p><p class="form-hint">{format!("{}/{} · {}",plugin.publisher,plugin.name,source.repository)}</p>
        <DropdownSelect label="Plugin release" value=version.read_only() options=Signal::derive(move ||options.clone()) on_change=Callback::new(move |v|version.set(v)) disabled=state.busy.read_only()/>
        <p class="form-hint">{move ||inspect.with_value(|(catalog,plugin)|catalog.catalog.resolve(&catalog.source,&plugin.publisher,&plugin.name,&version.get()).map(|s|format!("{} · {}",s.path,s.commit)).unwrap_or_default())}</p>
        <InlineActions><Button disabled=Signal::derive(move ||state.busy.get()||projects.active_project.get().is_none()) on_click=Callback::new(move |_|actions.install_release.run(CatalogSelection{marketplace:source.clone(),publisher:publisher.clone(),name:name.clone(),version:version.get_untracked()}))><Icon name=IconName::Plus/>"Install selected release"</Button></InlineActions>
    </div></DisclosurePanel>}
}
#[component]
fn InstalledPackage(entry: PluginInstallation) -> impl IntoView {
    let state = expect_context::<PluginsState>();
    let actions = expect_context::<ProjectPluginActions>();
    let projects = expect_context::<ProjectsState>();
    let entry = StoredValue::new(entry);
    let confirming = RwSignal::new(false);
    let binding = Signal::derive(move || {
        state.project_plugins.with(|entries| {
            entry.with_value(|installed| {
                entries
                    .iter()
                    .find(|e| {
                        e.prepared.source.repository == installed.prepared.source.repository
                            && e.prepared.source.path == installed.prepared.source.path
                    })
                    .cloned()
            })
        })
    });
    view! {<DisclosurePanel summary=move ||entry.with_value(|e|format!("{} {}",e.prepared.manifest.display_name,e.prepared.manifest.version))><div class="ui-section-content">
        <p class="form-hint">{entry.with_value(|e|e.prepared.manifest.description.clone())}</p>
        <p class="form-hint">{entry.with_value(|e|format!("{} · {} · {} · {} host(s)",e.prepared.source.repository,e.prepared.source.path,e.prepared.source.commit,e.hosts.len()))}</p>
        <p class="form-hint">{entry.with_value(|e|format!("Skills: {}",e.prepared.manifest.contributions.skills.iter().map(|s|s.path.clone()).collect::<Vec<_>>().join(", ")))}</p>
        <p class="form-hint">{move ||binding.get().map_or_else(||"Not enabled in this project.".into(),|b|format!("{} in this project: {}",if b.enabled{"Enabled"}else{"Disabled"},b.prepared.manifest.version))}</p>
        <InlineActions>
            <Button disabled=Signal::derive(move ||state.busy.get()||projects.active_project.get().is_none()||binding.get().is_some_and(|b|b.enabled&&entry.with_value(|e|b.prepared.source==e.prepared.source))) on_click=Callback::new(move |_|actions.enable.run(entry.get_value()))>{move ||if binding.get().is_some_and(|b|b.enabled){"Apply installed version"}else{"Enable for project"}}</Button>
            <Button disabled=Signal::derive(move ||state.busy.get()||!binding.get().is_some_and(|b|b.enabled)) on_click=Callback::new(move |_|{if let Some(binding)=binding.get_untracked(){actions.disable.run(binding);}})>"Disable for project"</Button>
            <Button disabled=state.busy.read_only() on_click=Callback::new(move |_|confirming.set(true))>"Uninstall"</Button>
        </InlineActions>
        <Show when=move ||confirming.get()><p class="form-hint">"Uninstall removes this package’s managed skills from all your projects. Host caches remain available to running tasks."</p><InlineActions><Button disabled=state.busy.read_only() on_click=Callback::new(move |_|actions.remove.run(entry.get_value()))>"Confirm uninstall"</Button><Button on_click=Callback::new(move |_|confirming.set(false))>"Cancel"</Button></InlineActions></Show>
    </div></DisclosurePanel>}
}
