//! Shared host capability adaptation; a verified local binding supplies native primitives.
pub fn authorize_host_plan(plan: &mut openwebide_core::RunPlan, path: &str) {
    plan.environment.project_root = Some(path.into());
    if plan.request.model_settings.tools == Some(false) {
        return;
    }
    let mut tools = crate::session::tools_for_host(true);
    tools.extend(
        plan.request
            .tools
            .iter()
            .filter(|tool| !tools_contains(&tool.name))
            .cloned(),
    );
    plan.connection.tool_selection.apply(&mut tools);
    plan.request.tools = tools;
    plan.kind = if plan.request.tools.is_empty() {
        openwebide_core::RunKind::Chat
    } else {
        openwebide_core::RunKind::Agent {
            project_path: path.into(),
        }
    };
}
fn tools_contains(name: &str) -> bool {
    crate::vfs_tools().iter().any(|tool| tool.name == name)
}
use crate::{ToolExecutor, ToolOutcome, ToolPreview};
use openwebide_core::{
    FileDiff, ToolCall, ToolDefinition,
    rewind::ProjectSnapshot,
    scheduled::{ScheduledTask, TaskCommand},
};
use std::future::Future;
pub const TOOL_NAMES: &[&str] = &[
    "monitor",
    "schedule_list",
    "schedule_create",
    "schedule_update",
    "schedule_delete",
];
pub fn is_scheduled_tool(name: &str) -> bool {
    TOOL_NAMES.contains(&name)
}
pub fn configure(tools: &mut Vec<ToolDefinition>) {
    tools.retain(|tool| !is_scheduled_tool(&tool.name));
    tools.extend(TOOL_NAMES.iter().map(|name| definition(name)));
}
pub fn definition(name: &str) -> ToolDefinition {
    if name == "monitor" {
        return ToolDefinition {
            name: name.into(),
            description: "Start, list or cancel ephemeral host-owned follow-up checks in this conversation. Use for checking an async process later, not saved recurring tasks or sleeping in this run. Checks survive browser closure; local projects need a user-authorized paired host. A check uses normal approvals and session context. Cancel by id/revision once the condition is met. Repeats stop after max_checks or 24 hours; failures stop repeats. Cancellation stops future checks; Stop controls an active response.".into(),
            parameters: serde_json::json!({"oneOf":[
                {"type":"object","properties":{"action":{"const":"start"},"prompt":{"type":"string","minLength":1,"maxLength":32768},"delay_seconds":{"type":"integer","minimum":5,"maximum":86399},"interval_seconds":{"type":"integer","minimum":5,"maximum":86400,"default":600},"max_checks":{"type":"integer","minimum":1,"maximum":24,"default":1}},"required":["action","prompt","delay_seconds"],"additionalProperties":false},
                {"type":"object","properties":{"action":{"const":"list"}},"required":["action"],"additionalProperties":false},
                {"type":"object","properties":{"action":{"const":"cancel"},"id":{"type":"integer","minimum":1},"revision":{"type":"integer","minimum":1}},"required":["action","id","revision"],"additionalProperties":false}
            ]}),
        };
    }
    let schedule = serde_json::json!({"oneOf":[{"type":"object","properties":{"kind":{"const":"once"},"at":{"type":"integer","description":"UTC epoch seconds"}},"required":["kind","at"],"additionalProperties":false},{"type":"object","properties":{"kind":{"const":"cron"},"expression":{"type":"string","description":"Five fields: minute hour day month weekday"},"timezone":{"type":"string","description":"IANA timezone, such as America/Chicago"}},"required":["kind","expression","timezone"],"additionalProperties":false}]});
    let draft = serde_json::json!({"type":"object","properties":{"title":{"type":"string","maxLength":120},"prompt":{"type":"string","maxLength":32768},"session_target":{"type":"string","enum":["existing","new","latest"],"description":"Existing (default), new session each run, or latest unarchived session at delivery"},"session_id":{"type":"integer","description":"Used only for existing; 0 means this session"},"model":{"description":"Optional run override; omit or null to use the current session model","oneOf":[{"type":"null"},{"type":"object","properties":{"server_id":{"type":"integer","minimum":1},"model":{"type":"string","minLength":1,"maxLength":256}},"required":["server_id","model"],"additionalProperties":false}]},"schedule":schedule,"enabled":{"type":"boolean"}},"required":["title","prompt","schedule","enabled"],"additionalProperties":false});
    let parameters = match name {
        "schedule_list" => {
            serde_json::json!({"type":"object","properties":{},"additionalProperties":false})
        }
        "schedule_create" => {
            serde_json::json!({"type":"object","properties":{"draft":draft},"required":["draft"],"additionalProperties":false})
        }
        "schedule_update" => {
            serde_json::json!({"type":"object","properties":{"id":{"type":"integer","minimum":1},"revision":{"type":"integer","minimum":1},"draft":draft},"required":["id","revision","draft"],"additionalProperties":false})
        }
        _ => {
            serde_json::json!({"type":"object","properties":{"id":{"type":"integer","minimum":1},"revision":{"type":"integer","minimum":1}},"required":["id","revision"],"additionalProperties":false})
        }
    };
    ToolDefinition{name:name.into(),description:match name{"schedule_list"=>"List saved scheduled prompts for this project or projectless chat.","schedule_create"=>"Schedule a future user prompt. Omit title to let the assistance model name it; provide a title when the user requests one. Runs use the current session model unless model is overridden, and normal approvals. Local projects need a user-authorized paired host.","schedule_update"=>"Replace a saved scheduled prompt using its current revision. Set enabled false to pause. Changes cancel undelivered prompts.",_=>"Remove a scheduled prompt using its current revision. Delivered runs remain in chat."}.into(),parameters}
}
pub trait TaskStore: Send + Sync {
    fn execute(
        &self,
        command: &TaskCommand,
    ) -> impl Future<Output = Result<Vec<ScheduledTask>, String>> + Send;
}
pub struct ScheduledTools<E, S> {
    executor: E,
    store: S,
}
impl<E, S> ScheduledTools<E, S> {
    pub fn new(executor: E, store: S) -> Self {
        Self { executor, store }
    }
}
impl<E: ToolExecutor + Sync, S: TaskStore> ToolExecutor for ScheduledTools<E, S> {
    fn has_context(&self) -> bool {
        self.executor.has_context()
    }
    async fn context(
        &mut self,
        tools: &[ToolDefinition],
        call: Option<&ToolCall>,
    ) -> Option<String> {
        self.executor.context(tools, call).await
    }
    fn describe(&self, call: &ToolCall) -> String {
        if is_scheduled_tool(&call.name) {
            crate::tools::parse(call).map_or_else(|error| error.to_string(), |tool| tool.describe())
        } else {
            self.executor.describe(call)
        }
    }
    async fn preview(&self, call: &ToolCall) -> Option<ToolPreview> {
        if !is_scheduled_tool(&call.name) {
            return self.executor.preview(call).await;
        }
        let note = match crate::tools::parse(call).ok()? {
            crate::tools::Tool::Scheduled(TaskCommand::Monitor { command, .. }) => {
                Some(serde_json::to_string(&command).ok()?)
            }
            crate::tools::Tool::Scheduled(
                TaskCommand::Create { draft } | TaskCommand::Update { draft, .. },
            ) => Some(format!(
                "Scheduled prompt: {}\nSchedule: {}\n\n{}",
                draft.title,
                serde_json::to_string(&draft.schedule).ok()?,
                draft.prompt
            )),
            crate::tools::Tool::Scheduled(TaskCommand::Delete { id, .. }) => {
                Some(format!("Remove scheduled task #{id}"))
            }
            _ => None,
        };
        note.map(|note| ToolPreview {
            diff: None,
            note: Some(note),
        })
    }
    async fn checkpoint(&self, call: &ToolCall) -> Result<Option<FileDiff>, String> {
        self.executor.checkpoint(call).await
    }
    async fn project_checkpoint(&self, call: &ToolCall) -> Result<Option<ProjectSnapshot>, String> {
        self.executor.project_checkpoint(call).await
    }
    fn acknowledge(&self, id: &str) {
        self.executor.acknowledge(id);
    }
    async fn execute(&self, call: &ToolCall) -> ToolOutcome {
        if !is_scheduled_tool(&call.name) {
            return self.executor.execute(call).await;
        }
        let result:Result<String,String>=async{let crate::tools::Tool::Scheduled(command)=crate::tools::parse(call).map_err(|error|error.to_string())?else{return Err("Invalid scheduled task".into());};let tasks=self.store.execute(&command).await?;Ok(serde_json::json!(tasks.iter().take(100).map(|task|serde_json::json!({"id":task.id,"revision":task.revision,"title":task.draft.title,"prompt_excerpt":task.draft.prompt.chars().take(256).collect::<String>(),"session_id":task.draft.session_id,"session_target":task.draft.session_target,"schedule":task.draft.schedule,"enabled":task.draft.enabled,"next_run":task.next_run,"host_available":task.host_available,"last_run":task.last_run})).collect::<Vec<_>>()).to_string())}.await;
        match result {
            Ok(content) => ToolOutcome {
                ok: true,
                content,
                summary: format!("{} complete", self.describe(call)),
                diff: None,
            },
            Err(error) => ToolOutcome {
                ok: false,
                content: error.clone(),
                summary: format!("Scheduled task failed: {error}"),
                diff: None,
            },
        }
    }
}
