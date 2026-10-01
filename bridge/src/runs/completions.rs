use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use futures::StreamExt;
use openwebide_core::{BridgeServerMessage, ChatRequest, with_temporal_context};
use openwebide_llm::{LlmProvider, ToolStreamMemos, registry::Provider};
use tokio::sync::mpsc;

use crate::auth::Principal;
use crate::runs::backend_client::{BackendClient, RunBackend};
use crate::runs::http_client::ReqwestHttpClient;
use crate::server::WriterCmd;

pub(crate) async fn complete(
    principal: Principal,
    id: String,
    mut request: ChatRequest,
    backend: Arc<BackendClient>,
    http: ReqwestHttpClient,
    memos: Arc<ToolStreamMemos>,
    sender: mpsc::Sender<WriterCmd>,
) {
    let result = async {
        let Principal::User { user_id } = principal else {
            return Err("unauthorized".into());
        };
        let connection = backend
            .list_connections(user_id)
            .await?
            .into_iter()
            .find(|c| c.id == request.connection_id)
            .ok_or_else(|| "connection not found".to_string())?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        request.system_prompt = Some(with_temporal_context(request.system_prompt, now));
        let memo = memos.get_or_insert(&connection);
        let provider = Provider::for_connection_with_memo(&connection, http, memo.clone());
        let mut stream = provider.chat_tools_stream(&request);
        while let Some(chunk) = stream.next().await {
            crate::runs::record_tool_stream_memo(
                &*backend,
                user_id,
                connection.id,
                connection.tool_stream_revision,
                &memo,
            )
            .await;
            let chunk = chunk.map_err(|e| e.to_string())?;
            sender
                .send(WriterCmd::send(BridgeServerMessage::CompletionChunk {
                    id: id.clone(),
                    chunk,
                }))
                .await
                .map_err(|_| "connection closed".to_string())?;
        }
        Ok::<_, String>(())
    }
    .await;
    let _ = sender
        .send(WriterCmd::send(BridgeServerMessage::CompletionEnd {
            id,
            error: result.err(),
        }))
        .await;
}
