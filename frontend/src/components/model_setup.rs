use crate::{
    backend::Api,
    state::{auth::AuthState, settings::SettingsState},
};
use leptos::prelude::*;
use leptos::task::spawn_local;
use openwebide_core::{
    ModelDefaults, ModelProfile, ModelSelection, ModelSettings, ServerSettingsUpdate,
};

#[derive(Clone, Debug)]
struct ModelOption {
    selection: ModelSelection,
    label: String,
}
fn selection_value(selection: &Option<ModelSelection>) -> String {
    selection
        .as_ref()
        .and_then(|selection| serde_json::to_string(selection).ok())
        .unwrap_or_default()
}

#[component]
fn ModelChoice(
    label: &'static str,
    value: RwSignal<Option<ModelSelection>>,
    choices: ReadSignal<Vec<ModelOption>>,
    empty: &'static str,
) -> impl IntoView {
    let node = NodeRef::<leptos::html::Select>::new();
    Effect::new(move |_| {
        let value = selection_value(&value.get());
        choices.track();
        if let Some(node) = node.get() {
            node.set_value(&value);
        }
    });
    view! {
        <label class="setting-row"><span class="setting-label">{label}</span>
            <select class="form-input" node_ref=node on:change=move |event| value.set(serde_json::from_str(&event_target_value(&event)).ok())>
                <option value="">{empty}</option>
                <For each=move || choices.get() key=|item| (item.selection.server_id, item.selection.model.clone()) children=move |item| {
                    let encoded = selection_value(&Some(item.selection));
                    view! { <option value=encoded>{item.label}</option> }
                } />
            </select>
        </label>
    }
}

#[component]
fn TextSetting(
    label: &'static str,
    value: RwSignal<String>,
    #[prop(default = "text")] input_type: &'static str,
    #[prop(default = "")] placeholder: &'static str,
) -> impl IntoView {
    view! {
        <label class="setting-row"><span class="setting-label">{label}</span>
            <input class="form-input" type=input_type placeholder=placeholder prop:value=move || value.get()
                on:input=move |event| value.set(event_target_value(&event)) />
        </label>
    }
}

#[component]
pub fn ModelSetupDialog() -> impl IntoView {
    let settings = expect_context::<SettingsState>();
    let server = settings.model_setup_server.get_untracked();
    view! {
        <super::modal::Modal title=Signal::derive(|| "Model configuration".to_string()) on_close=Callback::new(move |()| settings.show_model_setup.set(false))>
            <div class="modal-body">
                <button class="btn" on:click=move |_| {
                    settings.begin_model_setup(server);
                }>"Run setup wizard"</button>
                <ModelSetupPanel server=server />
            </div>
        </super::modal::Modal>
    }
}

#[component]
pub fn ModelSetupPanel(
    #[prop(default = false)] defaults_only: bool,
    #[prop(optional_no_strip)] server: Option<i64>,
) -> impl IntoView {
    let api = expect_context::<Api>();
    let auth = expect_context::<AuthState>();
    let settings = expect_context::<SettingsState>();
    let choices = RwSignal::new(Vec::<ModelOption>::new());
    let primary = RwSignal::new(settings.model_setup.get_untracked().defaults.primary);
    let fast = RwSignal::new(settings.model_setup.get_untracked().defaults.fast);
    let target = RwSignal::new(Option::<ModelSelection>::None);
    let busy = RwSignal::new(false);
    let loading = RwSignal::new(false);
    let error = RwSignal::new(Option::<String>::None);
    let reload = RwSignal::new(0_u64);
    let generation = StoredValue::new(0_u64);
    Effect::new(move |_| {
        reload.track();
        let connections = settings
            .connections
            .get()
            .into_iter()
            .filter(|connection| server.is_none_or(|id| id == connection.id))
            .collect::<Vec<_>>();
        let epoch = auth.generation.get();
        generation.update_value(|value| *value += 1);
        let request = generation.get_value();
        loading.set(true);
        spawn_local(async move {
            let backend = api.with_value(Clone::clone);
            let results =
                futures::future::join_all(connections.iter().filter(|server| server.enabled).map(
                    |server| {
                        let backend = &backend;
                        async move {
                            let models = backend.list_models(server.id).await;
                            (server, models)
                        }
                    },
                ))
                .await;
            let setup = backend.model_setup().await;
            if generation.try_get_value() != Some(request)
                || auth.generation.get_untracked() != epoch
            {
                return;
            }
            if let Ok(setup) = setup {
                settings.model_setup.set(setup);
            }
            let mut items = Vec::new();
            let mut failures = Vec::new();
            for (server, result) in results {
                match result {
                    Ok(models) => {
                        for model in models {
                            items.push(ModelOption {
                                label: model.name.clone(),
                                selection: ModelSelection {
                                    server_id: server.id,
                                    model: model.name,
                                },
                            });
                        }
                    }
                    Err(message) => failures.push(format!("{}: {message}", server.name)),
                }
            }
            let mut names = std::collections::BTreeMap::new();
            for item in &items {
                *names.entry(item.label.clone()).or_insert(0_usize) += 1;
            }
            for item in &mut items {
                if names[&item.label] > 1
                    && let Some(server) = connections
                        .iter()
                        .find(|server| server.id == item.selection.server_id)
                {
                    item.label = format!("{} @ {}", item.label, server.base_url);
                }
            }
            if !failures.is_empty() {
                error.set(Some(failures.join("; ")));
            }
            if !defaults_only && target.get_untracked().is_none() {
                target.set(items.first().map(|item| item.selection.clone()));
            }
            choices.set(items);
            loading.set(false);
        });
    });
    let project_host = expect_context::<crate::project_host::ProjectHost>();
    let candidates = RwSignal::new(Vec::<openwebide_core::ServerDiscovery>::new());
    let discovering = RwSignal::new(false);
    let project = expect_context::<crate::state::projects::ProjectsState>().active_project;
    let discover = move |_| {
        let epoch = auth.generation.get_untracked();
        let revision = project_host.revision();
        let project_id = project.get_untracked();
        discovering.set(true);
        error.set(None);
        spawn_local(async move {
            let result = project_host.discover_servers().await;
            if auth.generation.try_get_untracked() != Some(epoch)
                || discovering.try_get_untracked().is_none()
            {
                return;
            }
            discovering.set(false);
            if project_host.revision() != revision
                || project.try_get_untracked() != Some(project_id)
            {
                return;
            }
            match result {
                Ok(found) => candidates.set(found),
                Err(message) => error.set(Some(message)),
            }
        });
    };
    let save = move |_| {
        let defaults = ModelDefaults {
            primary: primary.get_untracked(),
            fast: fast.get_untracked(),
        };
        let epoch = auth.generation.get_untracked();
        busy.set(true);
        error.set(None);
        spawn_local(async move {
            let result = api
                .with_value(Clone::clone)
                .save_model_defaults(&defaults)
                .await;
            if auth.generation.get_untracked() != epoch || busy.try_get_untracked().is_none() {
                return;
            }
            busy.set(false);
            match result {
                Ok(setup) => {
                    if let Some(primary) = &setup.defaults.primary {
                        settings.default_connection.set(Some(primary.server_id));
                    }
                    settings.model_setup.set(setup);
                }
                Err(message) => error.set(Some(message)),
            }
        });
    };
    view! {
        <div class="model-setup">
            <Show when=move || !defaults_only>
                <p class="form-hint">"Configuration is shared by everyone using this server and model."</p>
            <button class="btn" disabled=move || discovering.get() on:click=discover>{move || if discovering.get() { "Looking for local model servers…" } else { "Discover local servers" }}</button>
            <p class="form-hint">"Checks standard model-server ports on the project’s execution host. Add other machines by URL in Servers."</p>
            <For each=move || candidates.get() key=|server| server.base_url.clone() children=move |candidate| {
                let label = format!("Set up {} @ {}", candidate.kind.display_name(), candidate.base_url);
                let url = candidate.base_url.clone();
                view! { <button class="btn" on:click=move |_| {
                    let existing = settings.connections.with_untracked(|servers| servers.iter().find(|server| server.base_url.trim_end_matches('/') == url.trim_end_matches('/')).map(|server| server.id));
                    settings.begin_model_setup(existing);
                    settings.conn_kind.set(candidate.kind);
                    settings.conn_base_url.set(url.clone());
                }>{label}</button> }
            } />
            <Show when=move || loading.get()><p class="form-hint">"Loading models from saved servers…"</p></Show>
            </Show>
            <Show when=move || defaults_only>
            <ModelChoice label="Default model" value=primary choices=choices.read_only() empty="Use default server’s model" />
            <ModelChoice label="Fast model" value=fast choices=choices.read_only() empty="Use the primary model" />
            <p class="form-hint">"Used for Auto approvals and context compaction. If unset, the primary model does that work."</p>
            <div class="form-actions">
                <button class="btn send" disabled=move || busy.get() on:click=save>"Save model defaults"</button>
            </div>
            </Show>
            <button class="btn" disabled=move || loading.get() on:click=move |_| reload.update(|value| *value += 1)>"Refresh models"</button>
            <Show when=move || error.get().is_some()><p class="form-error" role="alert">{move || error.get()}</p></Show>
            <Show when=move || !defaults_only>
            <ModelChoice label="Model settings" value=target choices=choices.read_only() empty="Choose a model to configure" />
            <For each=move || { target.get().into_iter().collect::<Vec<_>>() } key=|selection| (selection.server_id, selection.model.clone()) children=move |selection| view! {
                <ModelSettingsEditor selection=selection />
            } />
            <For each=move || { target.get().map(|selection| selection.server_id).into_iter().collect::<Vec<_>>() } key=|id| *id children=move |id| view! { <ServerOptions id=id /> } />
            </Show>
        </div>
    }
}

#[component]
pub(crate) fn ModelSettingsEditor(
    selection: ModelSelection,
    #[prop(optional_no_strip)] initial: Option<ModelSettings>,
    #[prop(optional_no_strip)] discovered: Option<openwebide_core::ModelDetection>,
    #[prop(optional)] on_edit: Option<Callback<Result<ModelSettings, String>>>,
    #[prop(default = true)] auto_detect: bool,
) -> impl IntoView {
    let api = expect_context::<Api>();
    let auth = expect_context::<AuthState>();
    let state = expect_context::<SettingsState>();
    let saved = initial.unwrap_or_else(|| {
        state.model_setup.with_untracked(|setup| {
            setup
                .profiles
                .iter()
                .find(|profile| profile.selection == selection)
                .map(|profile| profile.settings.clone())
                .unwrap_or_default()
        })
    });
    let context = RwSignal::new(
        saved
            .context_limit
            .map(|value| value.to_string())
            .unwrap_or_default(),
    );
    let output = RwSignal::new(
        saved
            .max_output_tokens
            .map(|value| value.to_string())
            .unwrap_or_default(),
    );
    let threshold = RwSignal::new(
        saved
            .auto_compact_threshold
            .map(|value| value.to_string())
            .unwrap_or_default(),
    );
    let thinking = RwSignal::new(saved.thinking);
    let tools = RwSignal::new(saved.tools);
    let sampling = [
        "temperature",
        "top_p",
        "top_k",
        "min_p",
        "repeat_penalty",
        "seed",
    ]
    .map(|name| {
        (
            name,
            RwSignal::new(
                saved
                    .sampling
                    .get(name)
                    .map(ToString::to_string)
                    .unwrap_or_default(),
            ),
        )
    });
    let error = RwSignal::new(Option::<String>::None);
    let busy = RwSignal::new(false);
    let has_discovery = discovered.is_some();
    let detection = RwSignal::new(discovered);
    let detecting = RwSignal::new(false);
    let selection_for_detection = selection.clone();
    let detect = Callback::new(move |()| {
        let selection = selection_for_detection.clone();
        let epoch = auth.generation.get_untracked();
        detecting.set(true);
        error.set(None);
        spawn_local(async move {
            let result = api
                .with_value(Clone::clone)
                .detect_model(selection.server_id, &selection.model)
                .await;
            if auth.generation.get_untracked() != epoch || detecting.try_get_untracked().is_none() {
                return;
            }
            detecting.set(false);
            match result {
                Ok(result) => detection.set(Some(result)),
                Err(message) => error.set(Some(message)),
            }
        });
    });
    if auto_detect && !has_discovery {
        detect.run(());
    }
    let read_settings = std::rc::Rc::new(move || -> Result<ModelSettings, String> {
        let number = |signal: RwSignal<String>| -> Result<Option<usize>, String> {
            let value = signal.get_untracked();
            if value.trim().is_empty() {
                Ok(None)
            } else {
                value
                    .parse::<usize>()
                    .map(Some)
                    .map_err(|_| "Token limits must be positive whole numbers.".into())
            }
        };
        let mut settings = ModelSettings {
            context_limit: number(context)?,
            max_output_tokens: number(output)?,
            thinking: thinking.get_untracked(),
            tools: tools.get_untracked(),
            auto_compact_threshold: number(threshold)?
                .map(u8::try_from)
                .transpose()
                .map_err(|_| "Invalid auto-compaction percentage.".to_string())?,
            ..saved.clone()
        };
        for (name, value) in sampling {
            let value = value.get_untracked();
            if !value.trim().is_empty() {
                settings.sampling.insert(
                    name.into(),
                    serde_json::from_str(&value)
                        .map_err(|_| format!("Enter a number for {name}."))?,
                );
            }
        }
        settings.validate()?;
        Ok(settings)
    });
    if let Some(on_edit) = on_edit {
        let read_settings = read_settings.clone();
        Effect::new(move |_| {
            context.track();
            output.track();
            threshold.track();
            thinking.track();
            tools.track();
            for (_, value) in sampling {
                value.track();
            }
            on_edit.run(read_settings());
        });
    }
    let save = move |_| {
        let result = read_settings();
        let profile = match result {
            Ok(settings) => ModelProfile {
                selection: selection.clone(),
                settings,
            },
            Err(message) => {
                error.set(Some(message));
                return;
            }
        };
        let epoch = auth.generation.get_untracked();
        busy.set(true);
        error.set(None);
        spawn_local(async move {
            let result = api
                .with_value(Clone::clone)
                .save_model_profile(&profile)
                .await;
            if auth.generation.get_untracked() != epoch || busy.try_get_untracked().is_none() {
                return;
            }
            busy.set(false);
            match result {
                Ok(setup) => state.model_setup.set(setup),
                Err(message) => error.set(Some(message)),
            }
        });
    };
    view! {
        <div class="model-settings-editor">
            <div class="form-actions"><button class="btn" disabled=move || detecting.get() on:click=move |_| detect.run(())>{move || if detecting.get() { "Detecting context and capabilities…" } else { "Detect model" }}</button></div>
            <Show when=move || detection.get().is_some()>
                <p class="form-hint">{move || detection.get().map(|result| format!("{} · context {} · {}", result.source, result.context_limit.map(|limit| limit.to_string()).unwrap_or_else(|| "unknown".into()), if result.capabilities.is_empty() { "capabilities unknown".into() } else { result.capabilities.join(", ") }))}</p>
                <button class="btn" on:click=move |_| { if let Some(result) = detection.get_untracked() && let Some(limit) = result.context_limit { context.set(limit.to_string()); } }>"Use detected context"</button>
            </Show>

            <TextSetting label="Context tokens" value=context input_type="number" placeholder="Detect from server" />
            <TextSetting label="Max output tokens" value=output input_type="number" placeholder="Model default" />
            {sampling.into_iter().map(|(name, value)| view! { <TextSetting label=name value=value placeholder="Model default" /> }).collect::<Vec<_>>()}
            <BooleanSetting label="Thinking" value=thinking />
            <BooleanSetting label="Tool calling" value=tools />
            <TextSetting label="Auto-compact (%)" value=threshold input_type="number" placeholder="85 (default)" />
            <p class="form-hint">"Blank fields use the server default. Auto-compact defaults to 85%; 0 disables it. Runs compact before model requests when the context limit is known; original history is retained."</p>
            <button class="btn send" disabled=move || busy.get() on:click=save>"Save model settings"</button>
            <Show when=move || error.get().is_some()><p class="form-error" role="alert">{move || error.get()}</p></Show>
        </div>
    }
}

#[component]
fn BooleanSetting(label: &'static str, value: RwSignal<Option<bool>>) -> impl IntoView {
    let node = NodeRef::<leptos::html::Select>::new();
    Effect::new(move |_| {
        let value = value
            .get()
            .map(|value| value.to_string())
            .unwrap_or_default();
        if let Some(node) = node.get() {
            node.set_value(&value);
        }
    });
    view! { <label class="setting-row"><span class="setting-label">{label}</span><select class="form-input" node_ref=node on:change=move |event| value.set(event_target_value(&event).parse().ok())>
        <option value="">"Model default"</option><option value="true">"On"</option><option value="false">"Off"</option>
    </select></label> }
}

#[component]
pub fn ServerOptions(id: i64) -> impl IntoView {
    let api = expect_context::<Api>();
    let auth = expect_context::<AuthState>();
    let key = RwSignal::new(String::new());
    let clear_key = RwSignal::new(false);
    let has_key = RwSignal::new(false);
    let timeout = RwSignal::new("300".to_string());
    let keep_alive = RwSignal::new(String::new());
    let headers = RwSignal::new(String::new());
    let clear_headers = RwSignal::new(false);
    let headers_node = NodeRef::<leptos::html::Textarea>::new();
    Effect::new(move |_| {
        let text = headers.get();
        if let Some(node) = headers_node.get() {
            node.set_value(&text);
        }
    });
    let error = RwSignal::new(Option::<String>::None);
    let busy = RwSignal::new(false);
    let epoch = auth.generation.get_untracked();
    spawn_local(async move {
        let result = api.with_value(Clone::clone).server_settings(id).await;
        if auth.generation.get_untracked() != epoch || has_key.try_get_untracked().is_none() {
            return;
        }
        match result {
            Ok(settings) => {
                has_key.set(settings.has_api_key);
                timeout.set(settings.timeout_seconds.to_string());
                keep_alive.set(settings.keep_alive.unwrap_or_default());
            }
            Err(message) => error.set(Some(message)),
        }
    });
    let save = move |_| {
        let Ok(timeout_seconds) = timeout.get_untracked().parse::<u32>() else {
            error.set(Some("Enter a timeout in seconds.".into()));
            return;
        };
        let mut extra_headers = std::collections::BTreeMap::new();
        for line in headers
            .get_untracked()
            .lines()
            .filter(|line| !line.trim().is_empty())
        {
            let Some((name, value)) = line.split_once(':') else {
                error.set(Some("Use one Name: Value header per line.".into()));
                return;
            };
            extra_headers.insert(name.trim().to_string(), value.trim().to_string());
        }
        let update = ServerSettingsUpdate {
            api_key: (!key.get_untracked().is_empty()).then(|| key.get_untracked()),
            clear_api_key: clear_key.get_untracked(),
            headers: (clear_headers.get_untracked() || !headers.get_untracked().trim().is_empty())
                .then_some(extra_headers),
            timeout_seconds: Some(timeout_seconds),
            keep_alive: Some(keep_alive.get_untracked()),
        };
        busy.set(true);
        error.set(None);
        spawn_local(async move {
            let result = api
                .with_value(Clone::clone)
                .save_server_settings(id, &update)
                .await;
            if auth.generation.get_untracked() != epoch || busy.try_get_untracked().is_none() {
                return;
            }
            busy.set(false);
            match result {
                Ok(settings) => {
                    has_key.set(settings.has_api_key);
                    key.set(String::new());
                    clear_key.set(false);
                    headers.set(String::new());
                }
                Err(message) => error.set(Some(message)),
            }
        });
    };
    view! { <details><summary>"Server authentication and options"</summary>
        <TextSetting label="API key" value=key input_type="password" placeholder="Leave blank to keep stored key" />
        <p class="form-hint">{move || if has_key.get() { "An API key is stored. Its value is never returned to the browser." } else { "No API key stored." }}</p>
        <label><input type="checkbox" prop:checked=move || clear_key.get() on:change=move |event| clear_key.set(event_target_checked(&event)) />"Clear stored API key"</label>
        <TextSetting label="Timeout (seconds)" value=timeout input_type="number" />
        <TextSetting label="Ollama keep alive" value=keep_alive placeholder="Server default (e.g. 5m)" />
        <label class="setting-row"><span class="setting-label">"Extra headers"</span><textarea class="form-input" node_ref=headers_node placeholder="One Name: Value per line" rows="3" on:input=move |event| headers.set(event_target_value(&event)) /></label>
        <label><input type="checkbox" prop:checked=move || clear_headers.get() on:change=move |event| clear_headers.set(event_target_checked(&event)) />"Clear stored extra headers"</label>
        <p class="form-hint">"Blank preserves stored headers. New headers replace them; stored values are never read back."</p>
        <button class="btn send" disabled=move || busy.get() on:click=save>"Save server options"</button>
        <Show when=move || error.get().is_some()><p class="form-error" role="alert">{move || error.get()}</p></Show>
    </details> }
}
