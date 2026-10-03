//! Shared setup orchestration. Backend supplies server, model and persistence primitives.
use crate::backend::Backend;
use openwebide_core::{Connection, ModelDetection, ModelProfile, ModelSelection};

#[derive(Clone, Debug)]
pub struct DiscoveredModel {
    pub profile: ModelProfile,
    pub detection: Option<ModelDetection>,
    pub error: Option<String>,
}

pub async fn preview(
    backend: &dyn Backend,
    probe: &openwebide_core::ModelProbe,
    current: impl Fn() -> bool,
) -> Result<Vec<DiscoveredModel>, String> {
    if !current() {
        return Err("Setup was closed or the account changed.".into());
    }
    let server = backend.preview_server(probe).await?;
    if !current() {
        return Err("Setup was closed or the account changed.".into());
    }
    let setup = backend.model_setup().await?;
    let mut results = Vec::new();
    for model in server.models {
        if !current() {
            return Err("Setup was closed or the account changed.".into());
        }
        let selection = ModelSelection {
            server_id: probe.server_id.unwrap_or(0),
            model: model.name,
        };
        let mut model_probe = probe.clone();
        model_probe.model = Some(selection.model.clone());
        let detection = if let Some(detected) = server.detections.get(&selection.model) {
            Ok(detected.clone())
        } else if let Some(message) = server.detection_errors.get(&selection.model) {
            Err(message.clone())
        } else {
            backend.preview_model(&model_probe).await
        };
        let (detection, error) = match detection {
            Ok(value) => (Some(value), None),
            Err(error) => (None, Some(error)),
        };
        let saved = setup
            .profiles
            .iter()
            .find(|profile| profile.selection == selection)
            .map(|profile| profile.settings.clone())
            .unwrap_or_default();
        let settings = detection
            .as_ref()
            .map_or(saved.clone(), |detection| detection.defaults_for(&saved));
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
    results.sort_by(|left, right| {
        let rank = |model: &DiscoveredModel| {
            let detection = model.detection.as_ref();
            (
                detection.is_none_or(ModelDetection::chat_capable),
                detection.is_some_and(|d| d.loaded == Some(true)),
                model.profile.settings.tools == Some(true),
                detection.and_then(|d| d.size_bytes).unwrap_or(0),
            )
        };
        rank(right).cmp(&rank(left)).then_with(|| {
            left.profile
                .selection
                .model
                .cmp(&right.profile.selection.model)
        })
    });
    // Prefer a loaded small model for background work; otherwise keep the primary fallback.
    if let Some(fast) = results
        .iter()
        .enumerate()
        .filter(|(_, model)| {
            model.detection.as_ref().is_some_and(|detection| {
                detection.chat_capable()
                    && detection.loaded == Some(true)
                    && detection.size_bytes.is_some()
            })
        })
        .min_by_key(|(_, model)| {
            model
                .detection
                .as_ref()
                .and_then(|detection| detection.size_bytes)
        })
        .map(|(index, _)| index)
        && fast > 0
    {
        let fast = results.remove(fast);
        results.push(fast);
    }
    Ok(results)
}

pub async fn save_setup(
    backend: &dyn Backend,
    probe: &openwebide_core::ModelProbe,
    profiles: &[ModelProfile],
    current: impl Fn() -> bool,
) -> Result<(Connection, openwebide_core::ModelSetup), String> {
    if !current() {
        return Err("Setup was closed or the account changed.".into());
    }
    for profile in profiles {
        profile.settings.validate()?;
    }
    let saved = backend.save_model_setup(probe, profiles).await?;
    if !current() {
        return Err("Setup was closed or the account changed.".into());
    }
    Ok(saved)
}

/// Refresh known settings through the shared setup facade.
pub async fn refresh(
    backend: &dyn Backend,
    selection: &ModelSelection,
    probe: Option<openwebide_core::ModelProbe>,
) -> Result<ModelDetection, String> {
    if let Some(mut probe) = probe {
        probe.model = Some(selection.model.clone());
        backend.preview_model(&probe).await
    } else {
        backend
            .detect_model(selection.server_id, &selection.model)
            .await
    }
}

pub async fn test(
    backend: &dyn Backend,
    probe: &openwebide_core::ModelProbe,
) -> Result<openwebide_core::ModelTestResult, String> {
    backend.test_model(probe).await
}
