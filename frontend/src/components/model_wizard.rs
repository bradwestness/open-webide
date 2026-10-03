use crate::{
    backend::Api,
    model_setup::{self, DiscoveredModel},
    state::{auth::AuthState, settings::SettingsState},
};
use leptos::{prelude::*, task::spawn_local};
use openwebide_core::ProviderKind;

#[component]
pub fn ModelSetupWizard(on_close: Callback<()>) -> impl IntoView {
    let api = expect_context::<Api>();
    let auth = expect_context::<AuthState>();
    let state = expect_context::<SettingsState>();
    let step = RwSignal::new(1_u8);
    let kind = RwSignal::new(state.conn_kind.get_untracked());
    let url = RwSignal::new(state.conn_base_url.get_untracked());
    let token = RwSignal::new(String::new());
    let server_id = RwSignal::new(state.conn_edit_id.get_untracked());
    let models = RwSignal::new(Vec::<DiscoveredModel>::new());
    let drafts = RwSignal::new(std::collections::BTreeMap::<
        String,
        Result<openwebide_core::ModelSettings, String>,
    >::new());
    let busy = RwSignal::new(false);
    let error = RwSignal::new(None::<String>);
    let generation = RwSignal::new(0_u64);
    let has_key = RwSignal::new(false);
    if let Some(id) = server_id.get_untracked() {
        let epoch = auth.generation.get_untracked();
        spawn_local(async move {
            if let Ok(settings) = api.with_value(Clone::clone).server_settings(id).await
                && auth.generation.get_untracked() == epoch
            {
                has_key.try_set(settings.has_api_key);
            }
        });
    }
    let discover = move |_| {
        let epoch = auth.generation.get_untracked();
        generation.update(|value| *value += 1);
        let request = generation.get_untracked();
        let current = move || {
            auth.generation.try_get_untracked() == Some(epoch)
                && generation.try_get_untracked() == Some(request)
        };
        let existing = state.connections.with_untracked(|servers| {
            servers
                .iter()
                .find(|server| Some(server.id) == server_id.get_untracked())
                .cloned()
        });
        let kind = kind.get_untracked();
        let url = url.get_untracked();
        let secret = token.get_untracked();
        busy.set(true);
        error.set(None);
        spawn_local(async move {
            let backend = api.with_value(Clone::clone);
            let registered = move |server: openwebide_core::Connection| {
                if !current() {
                    return;
                }
                server_id.set(Some(server.id));
                state.conn_edit_id.set(Some(server.id));
                state.connections.update(|servers| {
                    if let Some(existing) =
                        servers.iter_mut().find(|existing| existing.id == server.id)
                    {
                        *existing = server;
                    } else {
                        servers.push(server);
                    }
                });
            };
            let result = match model_setup::connect(
                &*backend, existing, kind, &url, &secret, current, registered,
            )
            .await
            {
                Ok(server) => {
                    if current() {
                        token.set(String::new());
                        if !secret.is_empty() {
                            has_key.set(true);
                        }
                    }
                    model_setup::discover(&*backend, server.id, current).await
                }
                Err(error) => Err(error),
            };
            if !current() {
                return;
            }
            busy.set(false);
            match result {
                Ok(found) if found.is_empty() => error.set(Some(
                    "No models found. Install or load a chat model on this server, then retry."
                        .into(),
                )),
                Ok(found) => {
                    models.set(found);
                    step.set(3);
                }
                Err(message) => error.set(Some(message)),
            }
        });
    };
    let apply = move |_| {
        let epoch = auth.generation.get_untracked();
        let request = generation.get_untracked();
        let current = move || {
            auth.generation.try_get_untracked() == Some(epoch)
                && generation.try_get_untracked() == Some(request)
        };
        let found = models.get_untracked();
        let reviewed = drafts.get_untracked();
        let mut profiles = Vec::new();
        for model in found {
            if model.detection.as_ref().is_some_and(|detection| {
                detection
                    .capabilities
                    .iter()
                    .any(|capability| capability == "embedding")
                    && !detection
                        .capabilities
                        .iter()
                        .any(|capability| capability == "completion")
            }) {
                continue;
            }
            let mut profile = model.profile;
            match reviewed.get(&profile.selection.model) {
                Some(Ok(settings)) => profile.settings = settings.clone(),
                Some(Err(message)) => {
                    error.set(Some(message.clone()));
                    return;
                }
                None => {}
            }
            profiles.push(profile);
        }
        if profiles.is_empty() {
            error.set(Some("No chat models are available to configure.".into()));
            return;
        }
        busy.set(true);
        error.set(None);
        spawn_local(async move {
            let result =
                model_setup::apply_review(&*api.with_value(Clone::clone), &profiles, current).await;
            if !current() {
                return;
            }
            busy.set(false);
            match result {
                Ok(setup) => {
                    state.default_connection.set(
                        setup
                            .defaults
                            .primary
                            .as_ref()
                            .map(|primary| primary.server_id),
                    );
                    state.model_setup.set(setup);
                    on_close.run(());
                }
                Err(message) => error.set(Some(message)),
            }
        });
    };
    view! {
        <super::modal::Modal title=Signal::derive(|| "Model setup".into()) on_close=on_close>
            <div class="modal-body model-setup">
                <p class="form-hint">{move || format!("Step {} of 3 · {}", step.get(), match step.get() { 1 => "Provider", 2 => "Server", _ => "Discover and customize" })}</p>
                <Show when=move || step.get() == 1>
                    <label class="setting-row"><span class="setting-label">"Provider type"</span>
                        <select class="form-input" prop:value=move || kind.get().as_str() on:change=move |event| { if let Some(value) = ProviderKind::parse(&event_target_value(&event)) { kind.set(value); } }>
                            <option value="ollama">"Ollama"</option><option value="llamacpp">"OpenAI-compatible (llama.cpp, LM Studio, vLLM…)"</option>
                        </select>
                    </label>
                    <button class="btn send" on:click=move |_| { if url.get_untracked().is_empty() { url.set(if kind.get_untracked() == ProviderKind::Ollama { "http://localhost:11434" } else { "http://localhost:8080" }.into()); } step.set(2); }>"Next: server"</button>
                </Show>
                <Show when=move || step.get() == 2>
                    <label class="setting-row"><span class="setting-label">"Server URL"</span><input class="form-input" type="url" prop:value=move || url.get() on:input=move |event| url.set(event_target_value(&event)) /></label>
                    <label class="setting-row"><span class="setting-label">"Auth token (optional)"</span><input class="form-input" type="password" autocomplete="new-password" prop:value=move || token.get() on:input=move |event| token.set(event_target_value(&event)) /></label>
                    <p class="form-hint">{move || if has_key.get() { "A token is saved. Leave this blank to keep it." } else { "Leave the token blank for servers without authentication." }}</p>
                    <div class="form-actions"><button class="btn" disabled=move || busy.get() on:click=move |_| step.set(1)>"Back"</button><button class="btn send" disabled=move || busy.get() on:click=discover>{move || if busy.get() { "Discovering models and settings…" } else { "Discover models" }}</button></div>
                </Show>
                <Show when=move || step.get() == 3>
                    <p class="form-hint">"Review and customize detected settings. Existing overrides are preserved. Save the models you want to configure; you can run setup again at any time."</p>
                    <For each=move || models.get() key=|model| (model.profile.selection.server_id, model.profile.selection.model.clone()) children=move |model| {
                        let selection = model.profile.selection.clone();
                        let model_name = selection.model.clone();
                        view! { <details open=true class="model-settings-editor"><summary>{selection.model.clone()}</summary>
                            {model.error.map(|error| view! { <p class="form-error">{error}</p> })}
                            <super::model_setup::ModelSettingsEditor selection=selection initial=Some(model.profile.settings) discovered=model.detection auto_detect=false on_edit=Callback::new(move |settings| drafts.update(|values| { values.insert(model_name.clone(), settings); })) />
                        </details> }
                    } />
                    <div class="form-actions"><button class="btn" disabled=move || busy.get() on:click=move |_| { models.set(vec![]); drafts.set(Default::default()); step.set(2); }>"Re-run discovery"</button><button class="btn send" disabled=move || busy.get() on:click=apply>"Apply settings"</button><button class="btn" disabled=move || busy.get() on:click=move |_| on_close.run(())>"Done"</button></div>
                </Show>
                <Show when=move || error.get().is_some()><p class="form-error" role="alert">{move || error.get()}</p></Show>
            </div>
        </super::modal::Modal>
    }
}
