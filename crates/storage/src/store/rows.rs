//! Shared decoding for rows returned by both database adapters.

use super::*;
use crate::db::QueryRow;

pub(super) fn connection_from_row(row: &QueryRow) -> Result<Connection, StorageError> {
    let kind = row.get_text(2)?;
    Ok(Connection {
        id: row.get_int(0)?,
        name: row.get_text(1)?.to_string(),
        kind: ProviderKind::parse(kind)
            .ok_or_else(|| StorageError::InvalidValue(format!("unknown provider kind: {kind}")))?,
        base_url: row.get_text(3)?.to_string(),
        model: row.get_text_opt(4).map(str::to_string),
        enabled: row.get_int(5)? != 0,
        context_limit: row
            .get_int_opt(6)
            .filter(|&n| n > 0)
            .and_then(|n| usize::try_from(n).ok()),
        tool_stream_unsupported: row.get_int(7)? != 0,
        tool_stream_revision: row.get_int(8)?,
    })
}

pub(super) fn prompt_from_row(row: &QueryRow) -> Result<SystemPrompt, StorageError> {
    Ok(SystemPrompt {
        id: row.get_int(0)?,
        name: row.get_text(1)?.to_string(),
        content: row.get_text(2)?.to_string(),
    })
}

pub(super) fn opt_int(
    row: &QueryRow,
    idx: usize,
    field: &str,
) -> Result<Option<i64>, StorageError> {
    match &row.values[idx] {
        DbValue::Int(i) => Ok(Some(*i)),
        DbValue::Null => Ok(None),
        other => Err(StorageError::InvalidValue(format!(
            "{field} is not an integer: {other:?}"
        ))),
    }
}

pub(super) fn session_from_row(row: &QueryRow) -> Result<ChatSession, StorageError> {
    Ok(ChatSession {
        id: row.get_int(0)?,
        name: row.get_text(1)?.to_string(),
        connection_id: opt_int(row, 2, "connection_id")?,
        system_prompt_id: opt_int(row, 3, "system_prompt_id")?,
        project_id: opt_int(row, 4, "project_id")?,
        user_id: opt_int(row, 5, "user_id")?.map(UserId::new),
        created_at: row.get_int(6)?,
    })
}

pub(super) fn project_from_row(row: &QueryRow) -> Result<Project, StorageError> {
    let mode = row.get_text(2)?;
    Ok(Project {
        id: row.get_int(0)?,
        name: row.get_text(1)?.to_string(),
        mode: WorkspaceMode::parse(mode)
            .ok_or_else(|| StorageError::InvalidValue(format!("unknown workspace mode: {mode}")))?,
        path: row.get_text_opt(3).map(str::to_string),
        user_id: opt_int(row, 4, "user_id")?.map(UserId::new),
        created_at: row.get_int(5)?,
    })
}

pub(super) fn user_from_row(row: &QueryRow) -> Result<UserRecord, StorageError> {
    let role = row.get_text(3)?;
    Ok(UserRecord {
        id: UserId::new(row.get_int(0)?),
        username: row.get_text(1)?.to_string(),
        password_hash: row.get_text(2)?.to_string(),
        role: UserRole::parse(role)
            .ok_or_else(|| StorageError::InvalidValue(format!("unknown role: {role}")))?,
        created_at: row.get_int(4)?,
        token_epoch: row.get_int(5).unwrap_or(0),
    })
}

pub(super) fn message_from_row(row: &QueryRow) -> Result<ChatMessage, StorageError> {
    let role = row.get_text(2)?;
    let prompt_tokens = row.get_int_opt(5);
    let completion_tokens = row.get_int_opt(6);
    let usage = if prompt_tokens.is_some() || completion_tokens.is_some() {
        Some(TurnTelemetry {
            prompt_tokens: usize::try_from(prompt_tokens.unwrap_or(0).max(0)).unwrap_or(usize::MAX),
            completion_tokens: usize::try_from(completion_tokens.unwrap_or(0).max(0))
                .unwrap_or(usize::MAX),
            eval_duration_ms: u64::try_from(row.get_int_opt(7).unwrap_or(0)).unwrap_or(0),
            estimated: row.get_int_opt(8).map(|v| v != 0).unwrap_or(false),
        })
    } else {
        None
    };
    Ok(ChatMessage {
        id: row.get_int(0)?,
        session_id: row.get_int(1)?,
        role: Role::parse(role)
            .ok_or_else(|| StorageError::InvalidValue(format!("unknown role: {role}")))?,
        content: row.get_text(3)?.to_string(),
        created_at: row.get_int(4)?,
        tool_calls: row
            .get_text_opt(9)
            .map(serde_json::from_str)
            .transpose()
            .map_err(|e| StorageError::InvalidValue(e.to_string()))?,
        tool_call_id: None,
        usage,
    })
}

pub(super) fn tool_step_from_row(row: &QueryRow) -> Result<ToolStep, StorageError> {
    let ok = match &row.values[3] {
        DbValue::Int(i) => Some(*i != 0),
        DbValue::Null => None,
        other => {
            return Err(StorageError::InvalidValue(format!(
                "tool step ok is not an integer: {other:?}"
            )));
        }
    };
    let diff = row
        .get_text_opt(5)
        .map(serde_json::from_str::<FileDiff>)
        .transpose()
        .map_err(|e| StorageError::InvalidValue(format!("bad tool step diff: {e}")))?;
    Ok(ToolStep {
        tool_call_id: row.get_text(0)?.to_string(),
        name: row.get_text(1)?.to_string(),
        summary: row.get_text(2)?.to_string(),
        ok,
        result_summary: row.get_text_opt(4).map(str::to_string),
        diff,
        anchor_message_id: row.get_int(6)?,
        checkpoint: row
            .get_text_opt(7)
            .map(serde_json::from_str)
            .transpose()
            .map_err(|e| StorageError::InvalidValue(e.to_string()))?,
    })
}

pub(super) fn edit_decision_text(decision: EditDecision) -> &'static str {
    match decision {
        EditDecision::Pending => "pending",
        EditDecision::Accepted => "accepted",
        EditDecision::Rejected => "rejected",
    }
}

pub(super) fn persisted_edit_from_row(row: &QueryRow) -> Result<PersistedEdit, StorageError> {
    let decision = match row.get_text(3)? {
        "pending" => EditDecision::Pending,
        "accepted" => EditDecision::Accepted,
        "rejected" => EditDecision::Rejected,
        _ => return Err(StorageError::InvalidValue("invalid edit decision".into())),
    };
    Ok(PersistedEdit {
        file: row
            .get_text_opt(5)
            .map(serde_json::from_str)
            .transpose()
            .map_err(|e| StorageError::InvalidValue(e.to_string()))?,
        project_id: row.get_int(0)?,
        path: row.get_text(1)?.into(),
        revision: row.get_int(2)?,
        decision,
        diff: serde_json::from_str(row.get_text(4)?)
            .map_err(|e| StorageError::InvalidValue(e.to_string()))?,
    })
}
