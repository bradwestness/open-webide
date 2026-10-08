use super::ui::{
    DialogActions, DialogBody, DialogSize, FormField, FormNotice, FormSection, NoticeTone,
};
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
    let preset = RwSignal::new(openwebide_core::ServerPreset::Auto);
    let options = RwSignal::new(Ok::<_, String>(
        openwebide_core::ServerSettingsUpdate::default(),
    ));
    let pending = RwSignal::new(None::<openwebide_core::ModelProbe>);
    let models = RwSignal::new(Vec::<DiscoveredModel>::new());
    let drafts = RwSignal::new(std::collections::BTreeMap::<
        String,
        Result<openwebide_core::ModelSettings, String>,
    >::new());
    let busy = RwSignal::new(false);
    let inflight = RwSignal::new(std::collections::BTreeMap::<String, bool>::new());
    let error = RwSignal::new(None::<String>);
    let generation = RwSignal::new(0_u64);
    let has_key = RwSignal::new(false);
    let preset_edited = RwSignal::new(false);
    if let Some(id) = server_id.get_untracked() {
        let epoch = auth.generation.get_untracked();
        spawn_local(async move {
            if let Ok(settings) = api.with_value(Clone::clone).server_settings(id).await
                && auth.generation.get_untracked() == epoch
            {
                has_key.try_set(settings.has_api_key);
                if preset_edited.try_get_untracked() == Some(false) {
                    preset.try_set(settings.preset);
                }
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
        let mut transport = match options.get_untracked() {
            Ok(value) => value,
            Err(message) => {
                error.set(Some(message));
                return;
            }
        };
        transport.api_key = (!token.get_untracked().is_empty()).then(|| token.get_untracked());
        transport.preset = Some(preset.get_untracked());
        let probe = openwebide_core::ModelProbe {
            server_id: server_id.get_untracked(),
            kind: kind.get_untracked(),
            base_url: url.get_untracked(),
            transport,
            model: None,
        };
        busy.set(true);
        error.set(None);
        spawn_local(async move {
            let backend = api.with_value(Clone::clone);
            let result = model_setup::preview(&*backend, &probe, current).await;
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
                    pending.set(Some(probe));
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
            let Some(probe) = pending.get_untracked() else {
                busy.set(false);
                return;
            };
            let backend = api.with_value(Clone::clone);
            let result = model_setup::save_setup(&*backend, &probe, &profiles, current).await;
            if !current() {
                return;
            }
            busy.set(false);
            match result {
                Ok((server, setup)) => {
                    state.connections.update(|servers| {
                        if let Some(saved) = servers.iter_mut().find(|saved| saved.id == server.id)
                        {
                            *saved = server;
                        } else {
                            servers.push(server);
                        }
                    });
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
        <super::modal::Modal title=Signal::derive(|| "Model setup".into()) on_close=on_close size=DialogSize::Wide>
            <DialogBody class="model-setup">
                <ol class="ui-steps" aria-label="Model setup steps">
                    {[(1, "Provider"), (2, "Server"), (3, "Models")].into_iter().map(|(number, label)| view! {
                        <li class:is-current=move || step.get() == number aria-current=move || (step.get() == number).then_some("step")>
                            <span class="ui-step-number">{number}</span>{label}
                        </li>
                    }).collect_view()}
                </ol>
                <Show when=move || step.get() == 1>
                    <FormSection title="Provider">
                    <FormField label="Provider type">
                        <super::dropdown::DropdownSelect label="Provider type" value=Signal::derive(move || kind.get().as_str().to_string()) options=Signal::derive(|| vec![super::dropdown::SelectOption::new("ollama", "Ollama"), super::dropdown::SelectOption::new("llamacpp", "OpenAI-compatible (llama.cpp, LM Studio, vLLM…)")]) on_change=Callback::new(move |value: String| { if let Some(value) = ProviderKind::parse(&value) { kind.set(value); } }) />
                    </FormField>
                    <FormField label="Server preset"><super::dropdown::DropdownSelect label="Server preset" value=Signal::derive(move || serde_json::to_string(&preset.get()).unwrap_or_default().trim_matches('"').to_string()) options=Signal::derive(|| [("auto", "Detect automatically"), ("ollama", "Ollama"), ("llama_cpp", "llama.cpp"), ("lm_studio", "LM Studio"), ("vllm", "vLLM"), ("lite_llm", "LiteLLM"), ("open_router", "OpenRouter"), ("sg_lang", "SGLang"), ("kobold_cpp", "KoboldCpp")].into_iter().map(|(value, label)| super::dropdown::SelectOption::new(value, label)).collect::<Vec<_>>()) on_change=Callback::new(move |selection: String| {
                        if let Ok(value) = serde_json::from_str::<openwebide_core::ServerPreset>(&format!("\"{selection}\"")) { preset_edited.set(true); preset.set(value); if value != openwebide_core::ServerPreset::Auto { kind.set(value.kind()); url.set(value.base_url().into()); } }
                    }) /></FormField>


                    </FormSection>
                </Show>
                <Show when=move || step.get() == 2>
                    <FormSection title="Server connection">
                    <FormField label="Server URL"><input class="form-input" type="url" disabled=move || busy.get() prop:value=move || url.get() on:input=move |event| url.set(event_target_value(&event)) /></FormField>
                    <FormField label="Auth token (optional)"><input class="form-input" type="password" disabled=move || busy.get() autocomplete="new-password" prop:value=move || token.get() on:input=move |event| token.set(event_target_value(&event)) /></FormField>
                    <p class="form-hint">{move || if has_key.get() { "A token is saved. Leave this blank to keep it." } else { "Leave the token blank for servers without authentication." }}</p>
                    <super::model_setup::ServerOptions disabled=Signal::derive(move || busy.get()) id=server_id.get_untracked() on_edit=Callback::new(move |value| options.set(value)) />


                    </FormSection>
                </Show>
                <Show when=move || step.get() == 3>
                    <FormSection title="Models and settings">
                    <p class="form-hint">"Review and customize detected settings. Existing overrides are preserved. Detect settings refreshes a model’s values. Changes are saved together when you choose Save."</p>
                    <For each=move || models.get() key=|model| (model.profile.selection.server_id, model.profile.selection.model.clone()) children=move |model| {
                        let selection = model.profile.selection.clone();
                        let model_name = selection.model.clone();
                        let busy_model = model_name.clone();
                        let chat_capable = model.detection.as_ref().is_none_or(openwebide_core::ModelDetection::chat_capable);
                        let details = model.detection.as_ref().map(openwebide_core::ModelDetection::details).unwrap_or_default();
                        view! { <details open=true class="model-settings-editor"><summary>{selection.model.clone()}</summary>
                            {if chat_capable {
                                view! { <super::model_setup::ModelSettingsEditor selection=selection initial_error=model.error on_busy=Callback::new(move |busy| inflight.update(|values| { values.insert(busy_model.clone(), busy); })) initial=Some(model.profile.settings) discovered=model.detection auto_detect=false probe=pending.get_untracked() on_edit=Callback::new(move |settings| drafts.update(|values| { values.insert(model_name.clone(), settings); })) /> }.into_any()
                            } else {
                                view! { <p class="form-hint">"Embedding model — unavailable for chat."</p><p class="form-hint">{details}</p> }.into_any()
                            }}
                        </details> }

                    } />


                    </FormSection>
                </Show>

                <Show when=move || error.get().is_some()><FormNotice tone=NoticeTone::Error>{move || error.get()}</FormNotice></Show>
            </DialogBody>
            <DialogActions>
                <Show when=move || step.get() != 3><button class="btn" disabled=move || busy.get() on:click=move |_| on_close.run(())>"Cancel"</button></Show>
                <Show when=move || step.get() == 1><button class="btn send" on:click=move |_| { if url.get_untracked().is_empty() { url.set(if kind.get_untracked() == ProviderKind::Ollama { "http://localhost:11434" } else { "http://localhost:8080" }.into()); } step.set(2); }>"Next: server"</button></Show>
                <Show when=move || step.get() == 2><button class="btn" disabled=move || busy.get() on:click=move |_| step.set(1)>"Back"</button><button class="btn send" disabled=move || busy.get() on:click=discover>{move || if busy.get() { "Discovering models and settings…" } else { "Discover models" }}</button></Show>
                <Show when=move || step.get() == 3><button class="btn" disabled=move || busy.get() on:click=move |_| on_close.run(())>"Cancel"</button><button class="btn send" disabled=move || busy.get() || inflight.with(|values| values.values().any(|busy| *busy)) on:click=apply>"Save"</button></Show>
            </DialogActions>
        </super::modal::Modal>
    }
}
