//! Server configuration and user-scoped model defaults.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const DEFAULT_AUTO_COMPACT_THRESHOLD: u8 = 85;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelSelection {
    pub server_id: i64,
    pub model: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelDefaults {
    pub primary: Option<ModelSelection>,
    pub fast: Option<ModelSelection>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelSettings {
    pub context_limit: Option<usize>,
    pub max_output_tokens: Option<usize>,
    pub sampling: BTreeMap<String, serde_json::Value>,
    pub thinking: Option<bool>,
    pub tools: Option<bool>,
    pub fast: Option<ModelSelection>,
    /// None uses 85%; zero disables compaction.
    pub auto_compact_threshold: Option<u8>,
}
impl ModelSettings {
    pub fn validate(&self) -> Result<(), String> {
        if self.context_limit == Some(0) || self.max_output_tokens == Some(0) {
            return Err("Token limits must be positive whole numbers.".into());
        }
        if self
            .auto_compact_threshold
            .is_some_and(|threshold| threshold > 95)
        {
            return Err("Auto-compaction must be 1–95%, or 0 to disable it.".into());
        }
        if self
            .fast
            .as_ref()
            .is_some_and(|selection| selection.model.trim().is_empty())
        {
            return Err("Choose a model for the fast-model server.".into());
        }
        for (name, value) in &self.sampling {
            let valid = match name.as_str() {
                "temperature" => value.as_f64().is_some_and(|n| (0.0..=2.0).contains(&n)),
                "top_p" | "min_p" => value.as_f64().is_some_and(|n| (0.0..=1.0).contains(&n)),
                "repeat_penalty" => value.as_f64().is_some_and(|n| n > 0.0 && n <= 10.0),
                "top_k" => value.as_u64().is_some(),
                "seed" => value.as_i64().is_some(),
                _ => false,
            };
            if !valid {
                return Err(format!("Invalid sampling setting: {name}."));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelProfile {
    pub selection: ModelSelection,
    pub settings: ModelSettings,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelSetup {
    pub defaults: ModelDefaults,
    pub profiles: Vec<ModelProfile>,
}
impl ModelSetup {
    pub fn resolve(&self, server_id: i64, model: &str) -> ModelSettings {
        let mut settings = self
            .profiles
            .iter()
            .find(|profile| {
                profile.selection.server_id == server_id && profile.selection.model == model
            })
            .map(|profile| profile.settings.clone())
            .unwrap_or_default();
        settings.fast = self.defaults.fast.clone().or_else(|| {
            (!model.is_empty()).then(|| ModelSelection {
                server_id,
                model: model.into(),
            })
        });
        if settings.auto_compact_threshold.is_none() {
            settings.auto_compact_threshold = Some(DEFAULT_AUTO_COMPACT_THRESHOLD);
        }
        settings
    }
}

/// Persisted server secrets. Never serialize this into a browser response.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerTransport {
    pub api_key: Option<String>,
    pub headers: BTreeMap<String, String>,
    pub timeout_seconds: u32,
    pub keep_alive: Option<String>,
}
impl Default for ServerTransport {
    fn default() -> Self {
        Self {
            api_key: None,
            headers: BTreeMap::new(),
            timeout_seconds: 300,
            keep_alive: None,
        }
    }
}
impl std::fmt::Debug for ServerTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerTransport")
            .field("has_api_key", &self.api_key.is_some())
            .field("timeout_seconds", &self.timeout_seconds)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerSettings {
    pub has_api_key: bool,
    /// Only non-secret header names are returned; values are never read back.
    pub header_names: Vec<String>,
    pub timeout_seconds: u32,
    pub keep_alive: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerSettingsUpdate {
    /// None preserves the stored key; clear_api_key explicitly removes it.
    pub api_key: Option<String>,
    pub clear_api_key: bool,
    pub headers: Option<BTreeMap<String, String>>,
    pub timeout_seconds: Option<u32>,
    pub keep_alive: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelRuntime {
    pub connection: crate::Connection,
    pub settings: ModelSettings,
    pub transport: ServerTransport,
}

impl ModelRuntime {
    /// Apply the resolved model and capability policy at every completion entry point.
    pub fn apply_to(&self, request: &mut crate::ChatRequest) {
        request.model = self.connection.model.clone();
        let output_budget = request.model_settings.max_output_tokens;
        request.model_settings = self.settings.clone();
        if let Some(budget) = output_budget {
            request.model_settings.max_output_tokens =
                Some(budget.min(self.settings.max_output_tokens.unwrap_or(usize::MAX)));
        }
        if self.settings.tools == Some(false) {
            request.tools.clear();
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelDetection {
    pub context_limit: Option<usize>,
    pub source: String,
    pub capabilities: Vec<String>,
    pub size_bytes: Option<u64>,
    pub quantization: Option<String>,
    pub loaded: Option<bool>,
}
impl ModelDetection {
    /// Detected defaults fill gaps; explicit server/model overrides always win.
    pub fn defaults_for(&self, current: &ModelSettings) -> ModelSettings {
        let mut settings = current.clone();
        settings.context_limit = settings.context_limit.or(self.context_limit);
        if !self.capabilities.is_empty() {
            settings.tools = settings.tools.or(Some(
                self.capabilities
                    .iter()
                    .any(|capability| capability == "tools"),
            ));
            settings.thinking = settings.thinking.or(Some(
                self.capabilities
                    .iter()
                    .any(|capability| capability == "thinking"),
            ));
        }
        settings
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerDiscovery {
    pub base_url: String,
    pub kind: crate::ProviderKind,
    pub models: Vec<crate::ModelInfo>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fast_model_falls_back_to_primary_and_threshold_defaults_to_85() {
        let setup = ModelSetup::default();
        let resolved = setup.resolve(7, "main");
        assert_eq!(
            resolved.fast,
            Some(ModelSelection {
                server_id: 7,
                model: "main".into()
            })
        );
        assert_eq!(resolved.auto_compact_threshold, Some(85));
        assert!(setup.resolve(7, "").fast.is_none());
    }

    #[test]
    fn profile_overrides_user_defaults_without_leaking_to_other_models() {
        let global_fast = ModelSelection {
            server_id: 2,
            model: "small".into(),
        };
        let override_fast = ModelSelection {
            server_id: 3,
            model: "tiny".into(),
        };
        let setup = ModelSetup {
            defaults: ModelDefaults {
                fast: Some(global_fast.clone()),
                ..Default::default()
            },
            profiles: vec![ModelProfile {
                selection: ModelSelection {
                    server_id: 1,
                    model: "main".into(),
                },
                settings: ModelSettings {
                    fast: Some(override_fast.clone()),
                    auto_compact_threshold: Some(0),
                    ..Default::default()
                },
            }],
        };
        assert_eq!(setup.resolve(1, "main").fast, Some(global_fast.clone()));
        assert_eq!(setup.resolve(1, "main").auto_compact_threshold, Some(0));
        assert_eq!(setup.resolve(1, "other").fast, Some(global_fast));
        assert_eq!(setup.resolve(1, "other").auto_compact_threshold, Some(85));
    }
    #[test]
    fn completion_entry_preserves_reserved_output_below_server_limit() {
        let runtime = ModelRuntime {
            connection: crate::Connection {
                id: 1,
                name: String::new(),
                kind: crate::ProviderKind::Ollama,
                base_url: "http://model.test".into(),
                model: Some("main".into()),
                enabled: true,
                context_limit: Some(4096),
                tool_stream_unsupported: false,
                tool_stream_revision: 0,
            },
            settings: ModelSettings {
                max_output_tokens: Some(2048),
                ..Default::default()
            },
            transport: Default::default(),
        };
        for (budget, expected) in [(512, 512), (8192, 2048)] {
            let mut request = crate::ChatRequest {
                connection_id: 1,
                model: None,
                system_prompt: None,
                messages: vec![],
                tools: vec![],
                model_settings: ModelSettings {
                    max_output_tokens: Some(budget),
                    ..Default::default()
                },
            };
            runtime.apply_to(&mut request);
            assert_eq!(request.model_settings.max_output_tokens, Some(expected));
        }
    }
    #[test]
    fn discovery_fills_unknown_settings_without_replacing_overrides() {
        let detected = ModelDetection {
            context_limit: Some(32768),
            capabilities: vec!["tools".into()],
            ..Default::default()
        };
        let current = ModelSettings {
            context_limit: Some(8192),
            tools: Some(false),
            auto_compact_threshold: Some(0),
            ..Default::default()
        };
        let merged = detected.defaults_for(&current);
        assert_eq!(merged.context_limit, Some(8192));
        assert_eq!(merged.tools, Some(false));
        assert_eq!(merged.auto_compact_threshold, Some(0));
        assert_eq!(merged.thinking, Some(false));
        assert_eq!(
            detected
                .defaults_for(&ModelSettings::default())
                .context_limit,
            Some(32768)
        );
        assert_eq!(ModelDetection::default().defaults_for(&current), current);
    }
}
