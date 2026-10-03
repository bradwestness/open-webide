//! Provider message serialization; only protocol fields differ.

use openwebide_core::{ChatRequest, ProviderKind, Role, ToolDefinition};
use serde_json::{Value, json};

/// Provider wire format for chat messages: `role` + `content` pairs, with
/// the system prompt first when one is set.
pub(crate) fn chat_messages(request: &ChatRequest) -> Vec<serde_json::Value> {
    let mut messages = Vec::new();
    if let Some(system) = &request.system_prompt
        && !system.is_empty()
    {
        messages.push(json!({ "role": "system", "content": system }));
    }
    for message in &request.messages {
        messages.push(json!({ "role": message.role.as_str(), "content": message.content }));
    }
    messages
}

/// Provider wire format for the `tools` array, shared by both providers
/// (Ollama and the OpenAI-compatible llama.cpp API use the same shape).
pub(crate) fn tools_wire(tools: &[ToolDefinition]) -> serde_json::Value {
    serde_json::Value::Array(
        tools
            .iter()
            .map(|tool| {
                json!({
                    "type": "function",
                    "function": {
                        "name": tool.name,
                        "description": tool.description,
                        "parameters": tool.parameters,
                    }
                })
            })
            .collect(),
    )
}

/// Serialize tool history once, adapting only provider-specific wire fields.
pub(crate) fn tool_messages(request: &ChatRequest, kind: ProviderKind) -> Vec<Value> {
    let mut messages = Vec::new();
    if let Some(system) = &request.system_prompt
        && !system.is_empty()
    {
        messages.push(json!({ "role": "system", "content": system }));
    }
    for message in &request.messages {
        let mut value = json!({ "role": message.role.as_str(), "content": message.content });
        if message.role == Role::Tool && kind == ProviderKind::LlamaCpp {
            value["tool_call_id"] = json!(message.tool_call_id.as_deref().unwrap_or_default());
        } else if message.role == Role::Assistant
            && let Some(calls) = &message.tool_calls
        {
            if message.content.is_empty() {
                value["content"] = Value::Null;
            }
            value["tool_calls"] = Value::Array(
                calls
                    .iter()
                    .map(|call| match kind {
                        ProviderKind::Ollama => {
                            let args: Value =
                                serde_json::from_str(&call.arguments).unwrap_or_else(|_| json!({}));
                            json!({ "function": { "name": call.name, "arguments": args } })
                        }
                        ProviderKind::LlamaCpp => json!({
                            "id": call.id, "type": "function",
                            "function": { "name": call.name, "arguments": call.arguments }
                        }),
                    })
                    .collect(),
            );
        }
        messages.push(value);
    }
    messages
}
