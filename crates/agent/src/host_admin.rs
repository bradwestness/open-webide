//! Project-less host tools. Plans persist before an explicitly approved apply call.
use crate::{BridgeClient, ToolOutcome, ToolPreview};
use openwebide_core::{
    ToolCall, ToolDefinition,
    host_admin::{HostPlan, HostRequest, HostResponse},
};
use serde::Deserialize;

pub const TOOL_NAMES: &[&str] = &[
    "host_inspect",
    "host_prepare",
    "host_apply",
    "host_operations",
];
pub fn is_host_tool(name: &str) -> bool {
    TOOL_NAMES.contains(&name)
}
pub fn definitions() -> Vec<ToolDefinition> {
    let command = serde_json::json!({"type":"object","properties":{"program":{"type":"string","description":"Exact program executable; shell scripts require explicit sh -c or powershell arguments"},"args":{"type":"array","items":{"type":"string"},"maxItems":128},"cwd":{"type":"string","description":"Absolute path on the administration host"},"elevated":{"type":"boolean","description":"Use sudo for this exact Unix command; interactive prompts are supported. Omit for user-space changes. Windows uses the SSH account privileges without UAC elevation"},"interactive":{"type":"boolean","description":"Allocate a terminal for user-only replies to sudo or other prompts (default true). Replies are never passed to the model."},"timeout_seconds":{"type":"integer","minimum":1,"maximum":3600}},"required":["program","cwd"],"additionalProperties":false});
    TOOL_NAMES.iter().map(|name| {
        let parameters = match *name {
            "host_prepare" => serde_json::json!({"type":"object","properties":{"plan":{"type":"object","properties":{"target":{"type":"string","description":"Exact target returned by host_inspect"},"title":{"type":"string","maxLength":256},"steps":{"type":"array","items":command,"minItems":1,"maxItems":16},"checks":{"type":"array","items":command,"maxItems":16},"expects_reboot":{"type":"boolean","description":"The final step intentionally reboots the host; provide post-reboot verification checks"}},"required":["target","title","steps"],"additionalProperties":false}},"required":["plan"],"additionalProperties":false}),
            "host_apply" => serde_json::json!({"type":"object","properties":{"id":{"type":"integer","minimum":1,"description":"Prepared operation id; the exact immutable stored plan is shown for explicit approval"}},"required":["id"],"additionalProperties":false}),
            _ => serde_json::json!({"type":"object","properties":{},"additionalProperties":false}),
        };
        ToolDefinition { name:(*name).into(), parameters, description:match *name {
            "host_inspect" => "Inspect the configured physical administration host, not the IDE container, browser, or model server. Returns OS, health, services, processes, containers, ports and storage with relationships. Available only in project-less chat.",
            "host_prepare" => "Prepare and persist an immutable host operation without executing commands. Inspect the host first, ask for missing paths/configuration choices, and include verification commands. User-space configuration needs no elevation. Host administration is independent of project folders.",
            "host_apply" => "Request explicit user approval of the stored host plan, then start a durable resident operation. Running means started, not completed. Use host_operations to check results. Closing chat or Stop does not stop host maintenance. No mutation is replayed after reconnect/reboot.",
            _ => "List this project-less session's retained host operation plans, commands, output and results. Report completion only when the operation state is succeeded; failed/interrupted operations require inspection before retry.",
        }.into() }
    }).collect()
}
pub fn request(call: &ToolCall) -> Result<HostRequest, String> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Empty {}
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Prepare {
        plan: HostPlan,
    }
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Apply {
        id: i64,
    }
    match call.name.as_str() {
        "host_inspect" | "host_operations" => {
            serde_json::from_str::<Empty>(&call.arguments).map_err(|error| error.to_string())?;
            Ok(if call.name == "host_inspect" {
                HostRequest::Inspect
            } else {
                HostRequest::Operations
            })
        }
        "host_prepare" => {
            let body: Prepare =
                serde_json::from_str(&call.arguments).map_err(|error| error.to_string())?;
            body.plan.validate()?;
            Ok(HostRequest::Prepare {
                request_id: call.id.clone(),
                plan: body.plan,
            })
        }
        "host_apply" => {
            let body: Apply =
                serde_json::from_str(&call.arguments).map_err(|error| error.to_string())?;
            if body.id < 1 {
                return Err("Expected a positive host operation id.".into());
            }
            Ok(HostRequest::Apply { id: body.id })
        }
        _ => Err("Unknown host administration tool".into()),
    }
}
pub fn describe(call: &ToolCall) -> String {
    match request(call) {
        Ok(HostRequest::Prepare { plan, .. }) => {
            format!("Prepare {} on {}", plan.title, plan.target)
        }
        Ok(HostRequest::Apply { id }) => format!("Apply host operation #{id}"),
        Ok(HostRequest::Inspect) => "Inspect administration host".into(),
        Ok(_) => "Read host operation history".into(),
        Err(error) => error,
    }
}
pub async fn preview<B: BridgeClient>(bridge: &B, call: &ToolCall) -> Option<ToolPreview> {
    let note = match request(call).ok()? {
        HostRequest::Prepare { plan, .. } => plan.preview(),
        HostRequest::Apply { id } => {
            match bridge.host_admin(&HostRequest::Operation { id }).await {
                Ok(HostResponse::Operation(operation)) => format!(
                    "{}\nOperation #{id}\n\nThis operation continues if you close chat or press Stop.",
                    operation.plan.preview()
                ),
                Ok(_) => "Cannot load the stored host operation; execution will fail.".into(),
                Err(error) => format!("Cannot load the stored host operation: {error}"),
            }
        }
        _ => return None,
    };
    Some(ToolPreview {
        diff: None,
        note: Some(note),
    })
}
pub async fn execute<B: BridgeClient>(bridge: &B, call: &ToolCall) -> ToolOutcome {
    let result = match request(call) {
        Ok(request) => bridge.host_admin(&request).await,
        Err(error) => Err(error),
    };
    match result {
        Ok(response) => ToolOutcome {
            ok: true,
            content: serde_json::to_string(&response).unwrap_or_default(),
            summary: describe(call),
            diff: None,
        },
        Err(error) => ToolOutcome {
            ok: false,
            content: format!("error: {error}"),
            summary: format!("{} failed: {error}", call.name),
            diff: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{NoopWebClient, ToolExecutor};
    struct Host;
    impl BridgeClient for Host {
        async fn host_admin(&self, _request: &HostRequest) -> Result<HostResponse, String> {
            Ok(HostResponse::Operations(vec![]))
        }
        async fn execute_command(
            &self,
            _command: &str,
            _timeout: u64,
        ) -> Result<openwebide_core::CommandOutcome, String> {
            panic!("Workspace commands cannot be used for host administration")
        }
    }
    #[test]
    fn host_tools_are_projectless_only_in_both_project_modes() {
        futures::executor::block_on(async {
            let call = ToolCall {
                id: "call".into(),
                name: "host_operations".into(),
                arguments: "{}".into(),
            };
            for mode in [
                openwebide_core::WorkspaceMode::Local,
                openwebide_core::WorkspaceMode::Remote,
            ] {
                let workspace = crate::VfsToolExecutor::new(openwebide_core::MemoryVfs::new());
                let executor = crate::vfs_executor::SessionToolExecutor::new(
                    Some(workspace),
                    NoopWebClient,
                    openwebide_core::RunEnvironment {
                        mode: Some(mode),
                        ..Default::default()
                    },
                )
                .with_host(Host);
                assert!(!executor.execute(&call).await.ok);
            }
            let executor = crate::vfs_executor::SessionToolExecutor::<
                crate::VfsToolExecutor<openwebide_core::MemoryVfs>,
                _,
            >::new(None, NoopWebClient, Default::default())
            .with_host(Host);
            assert!(executor.execute(&call).await.ok);
            let apply = ToolCall {
                name: "host_apply".into(),
                arguments: "{\"id\":1}".into(),
                ..call
            };
            assert!(crate::requires_approval(&apply));
            assert!(!crate::policy::always_approvable("host_apply"));
            assert!(
                request(&ToolCall {
                    arguments: "{\"id\":1,\"command\":\"malicious replacement\"}".into(),
                    ..apply
                })
                .is_err()
            );
        });
    }
}
