//! Shared setup orchestration. Backend supplies server, model and persistence primitives.
use crate::backend::Backend;
use openwebide_core::{
    Connection, ModelDetection, ModelProfile, ModelSelection, ProviderKind, ServerSettingsUpdate,
};

#[derive(Clone, Debug)]
pub struct DiscoveredModel {
    pub profile: ModelProfile,
    pub detection: Option<ModelDetection>,
    pub error: Option<String>,
}

pub async fn connect(
    backend: &dyn Backend,
    existing: Option<Connection>,
    kind: ProviderKind,
    url: &str,
    token: &str,
    current: impl Fn() -> bool,
    registered: impl Fn(Connection),
) -> Result<Connection, String> {
    if !current() {
        return Err("Setup was closed or the account changed.".into());
    }
    let url = url.trim().trim_end_matches('/');
    if url.is_empty() {
        return Err("Enter a server URL.".into());
    }
    let mut server = if let Some(mut server) = existing {
        server.kind = kind;
        server.base_url = url.into();
        backend.update_connection(&server).await?
    } else {
        let name = format!("{} @ {url}", kind.display_name());
        backend
            .create_connection(&name, kind, url, None, None)
            .await?
    };
    if !current() {
        return Err("Setup was closed or the account changed.".into());
    }
    registered(server.clone());
    if !token.is_empty() {
        backend
            .save_server_settings(
                server.id,
                &ServerSettingsUpdate {
                    api_key: Some(token.into()),
                    ..Default::default()
                },
            )
            .await?;
    }
    // Model selection happens only after authenticated discovery.
    server.model = server.model.filter(|model| !model.is_empty());
    Ok(server)
}

pub async fn discover(
    backend: &dyn Backend,
    id: i64,
    current: impl Fn() -> bool,
) -> Result<Vec<DiscoveredModel>, String> {
    if !current() {
        return Err("Setup was closed or the account changed.".into());
    }
    let models = backend.list_models(id).await?;
    if !current() {
        return Err("Setup was closed or the account changed.".into());
    }
    let setup = backend.model_setup().await?;
    let mut results = Vec::new();
    for model in models {
        if !current() {
            return Err("Setup was closed or the account changed.".into());
        }
        let selection = ModelSelection {
            server_id: id,
            model: model.name,
        };
        let saved = setup
            .profiles
            .iter()
            .find(|profile| profile.selection == selection)
            .map(|profile| profile.settings.clone())
            .unwrap_or_default();
        let detection = backend.detect_model(id, &selection.model).await;
        let (detection, error) = match detection {
            Ok(value) => (Some(value), None),
            Err(error) => (None, Some(error)),
        };
        let settings = detection
            .as_ref()
            .map_or(saved.clone(), |detected| detected.defaults_for(&saved));
        results.push(DiscoveredModel {
            profile: ModelProfile {
                selection,
                settings,
            },
            detection,
            error,
        });
    }
    if !current() {
        return Err("Setup was closed or the account changed.".into());
    }
    Ok(results)
}

pub async fn apply_review(
    backend: &dyn Backend,
    profiles: &[ModelProfile],
    current: impl Fn() -> bool,
) -> Result<openwebide_core::ModelSetup, String> {
    if !current() {
        return Err("Setup was closed or the account changed.".into());
    }
    for profile in profiles {
        profile.settings.validate()?;
    }
    let mut setup = backend.model_setup().await?;
    for profile in profiles {
        if !current() {
            return Err("Setup was closed or the account changed.".into());
        }
        setup = backend.save_model_profile(profile).await?;
    }
    if !current() {
        return Err("Setup was closed or the account changed.".into());
    }
    if setup.defaults.primary.is_none()
        && let Some(profile) = profiles.first()
    {
        let mut defaults = setup.defaults.clone();
        defaults.primary = Some(profile.selection.clone());
        setup = backend.save_model_defaults(&defaults).await?;
    }
    Ok(setup)
}
