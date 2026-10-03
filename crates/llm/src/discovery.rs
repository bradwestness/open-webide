//! Read-only server probes. Detection never overwrites user settings.
use crate::{HttpClient, LlmProvider, ProviderError, registry::Provider, url_for};
use openwebide_core::{Connection, ModelDetection, ProviderKind};
use serde_json::{Value, json};

fn positive(value: Option<&Value>) -> Option<usize> {
    value
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .filter(|value| *value > 0)
}
fn root(base: &str) -> &str {
    base.trim_end_matches('/')
        .strip_suffix("/v1")
        .unwrap_or(base.trim_end_matches('/'))
}

pub async fn detect<C: HttpClient + 'static>(
    connection: &Connection,
    model: &str,
    http: C,
) -> Result<ModelDetection, ProviderError> {
    let mut result = ModelDetection::default();
    let base = root(&connection.base_url);
    if connection.kind == ProviderKind::Ollama {
        let show = http
            .post_json(&url_for(base, "/api/show"), &json!({"model": model}))
            .await?;
        result.capabilities = show
            .get("capabilities")
            .and_then(Value::as_array)
            .map(|caps| {
                caps.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        result.quantization = show
            .pointer("/details/quantization_level")
            .and_then(Value::as_str)
            .map(str::to_string);
        // Only an explicit runtime num_ctx or a running instance tells us the actual window.
        if let Some(parameters) = show.get("parameters").and_then(Value::as_str) {
            result.context_limit = parameters.lines().find_map(|line| {
                let mut parts = line.split_whitespace();
                if parts.next() == Some("num_ctx") {
                    parts
                        .next()?
                        .parse::<usize>()
                        .ok()
                        .filter(|value| *value > 0)
                } else {
                    None
                }
            });
        }
        result.source = "Ollama /api/show".into();
        if let Ok(running) = http.get_json(&url_for(base, "/api/ps")).await {
            let entry = running
                .get("models")
                .and_then(Value::as_array)
                .and_then(|models| {
                    models.iter().find(|entry| {
                        entry
                            .get("name")
                            .or_else(|| entry.get("model"))
                            .and_then(Value::as_str)
                            == Some(model)
                    })
                });
            result.loaded = Some(entry.is_some());
            if let Some(entry) = entry {
                result.context_limit =
                    positive(entry.get("context_length")).or(result.context_limit);
                result.size_bytes = entry.get("size").and_then(Value::as_u64);
                if result.context_limit.is_some() {
                    result.source = "Ollama /api/ps runtime context".into();
                }
            }
        }
        return Ok(result);
    }
    // llama.cpp reports the allocated context; model-training limits are not substitutes.
    if let Ok(props) = http.get_json(&url_for(base, "/props")).await {
        result.context_limit = positive(props.pointer("/default_generation_settings/n_ctx"))
            .or_else(|| positive(props.get("n_ctx")));
        if let Some(caps) = props.get("chat_template_caps") {
            for (key, capability) in [
                ("supports_tool_calls", "tools"),
                ("supports_thinking", "thinking"),
            ] {
                if caps.get(key).and_then(Value::as_bool) == Some(true) {
                    result.capabilities.push(capability.into());
                }
            }
        }
        if result.context_limit.is_some() {
            result.source = "llama.cpp /props runtime context".into();
            return Ok(result);
        }
    }
    if let Ok(models) = http.get_json(&url_for(base, "/v1/models")).await {
        let entry = models
            .get("data")
            .and_then(Value::as_array)
            .and_then(|models| {
                models
                    .iter()
                    .find(|entry| entry.get("id").and_then(Value::as_str) == Some(model))
            });
        if let Some(entry) = entry {
            result.context_limit = positive(entry.get("max_model_len"));
            if result.context_limit.is_some() {
                result.source = "OpenAI-compatible /v1/models".into();
                return Ok(result);
            }
        }
    }
    if let Ok(models) = http.get_json(&url_for(base, "/api/v1/models")).await
        && let Some(entry) = models
            .get("models")
            .and_then(Value::as_array)
            .and_then(|models| {
                models
                    .iter()
                    .find(|entry| entry.get("key").and_then(Value::as_str) == Some(model))
            })
    {
        let loaded = entry
            .get("loaded_instances")
            .and_then(Value::as_array)
            .and_then(|instances| instances.first());
        result.loaded = Some(loaded.is_some());
        result.context_limit =
            loaded.and_then(|instance| positive(instance.pointer("/config/context_length")));
        result.size_bytes = entry.get("size_bytes").and_then(Value::as_u64);
        result.quantization = entry
            .pointer("/quantization/name")
            .and_then(Value::as_str)
            .map(str::to_string);
        for (key, capability) in [("trained_for_tool_use", "tools"), ("vision", "vision")] {
            if entry
                .pointer(&format!("/capabilities/{key}"))
                .and_then(Value::as_bool)
                == Some(true)
            {
                result.capabilities.push(capability.into());
            }
        }
        if result.context_limit.is_some() {
            result.source = "LM Studio /api/v1/models loaded context".into();
            return Ok(result);
        }
    }
    if let Ok(info) = http.get_json(&url_for(base, "/get_model_info")).await {
        result.context_limit = positive(info.get("context_length"));
        if result.context_limit.is_some() {
            result.source = "SGLang /get_model_info".into();
            return Ok(result);
        }
    }
    if let Ok(info) = http
        .get_json(&url_for(base, "/api/extra/true_max_context_length"))
        .await
    {
        result.context_limit = positive(info.get("value"));
        if result.context_limit.is_some() {
            result.source = "KoboldCpp runtime context".into();
            return Ok(result);
        }
    }
    result.source =
        "No runtime context detected; use the configured value or estimated 4,096 tokens".into();
    Ok(result)
}

pub async fn inspect<C: HttpClient + Clone + 'static>(
    base_url: &str,
    kind: Option<ProviderKind>,
    http: C,
) -> Result<openwebide_core::ServerDiscovery, ProviderError> {
    let kind = match kind {
        Some(kind) => kind,
        None => {
            if http
                .get_json(&url_for(root(base_url), "/api/version"))
                .await
                .ok()
                .is_some_and(|value| value.get("version").and_then(Value::as_str).is_some())
            {
                ProviderKind::Ollama
            } else {
                ProviderKind::LlamaCpp
            }
        }
    };
    let connection = Connection {
        id: 0,
        name: base_url.into(),
        kind,
        base_url: base_url.into(),
        model: None,
        enabled: true,
        context_limit: None,
        tool_stream_unsupported: false,
        tool_stream_revision: 0,
    };
    let models = Provider::for_connection(&connection, http)
        .list_models()
        .await?;
    Ok(openwebide_core::ServerDiscovery {
        base_url: base_url.into(),
        kind,
        models,
    })
}

/// Scan the execution host using the same probes on Spin and native bridges.
pub async fn discover<C: HttpClient + Clone + 'static>(
    http: C,
) -> Vec<openwebide_core::ServerDiscovery> {
    futures::future::join_all(
        [11434, 8080, 1234, 8000, 5001, 5000, 1337]
            .into_iter()
            .map(|port| {
                let http = http.clone();
                async move { inspect(&format!("http://127.0.0.1:{port}"), None, http).await }
            }),
    )
    .await
    .into_iter()
    .filter_map(Result::ok)
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Clone)]
    struct Host;
    impl HttpClient for Host {
        async fn get_json(&self, url: &str) -> Result<Value, ProviderError> {
            match url {
                "http://127.0.0.1:11434/api/version" => Ok(json!({"version":"test"})),
                "http://127.0.0.1:11434/api/tags" => Ok(json!({"models":[{"name":"local-model"}]})),
                "http://127.0.0.1:8080/v1/models" => Ok(json!({"data":[{"id":"host-model"}]})),
                _ => Err(ProviderError::Http("unavailable".into())),
            }
        }
        async fn post_json(&self, _url: &str, _body: &Value) -> Result<Value, ProviderError> {
            Err(ProviderError::NotImplemented("unused".into()))
        }
        fn post_stream(
            &self,
            _url: &str,
            _body: &Value,
        ) -> std::pin::Pin<
            Box<dyn futures::Stream<Item = Result<bytes::Bytes, ProviderError>> + Send>,
        > {
            Box::pin(futures::stream::empty())
        }
    }
    #[test]
    fn host_scan_reuses_provider_probes_and_ignores_unavailable_ports() {
        futures::executor::block_on(async {
            let servers = discover(Host).await;
            assert_eq!(servers.len(), 2);
            assert_eq!(servers[0].kind, ProviderKind::Ollama);
            assert_eq!(servers[0].models[0].name, "local-model");
            assert_eq!(servers[1].kind, ProviderKind::LlamaCpp);
            assert_eq!(servers[1].models[0].name, "host-model");
        });
    }
}
