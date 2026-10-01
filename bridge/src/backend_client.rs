use std::future::Future;
use std::sync::Arc;

use bytes::Bytes;
use http_body_util::Full;
use hyper::Request;
use openwebide_core::{
    ChatMessage, Connection, EditorContext, FileDiff, Role, RunPlan, TurnTelemetry, WebSearchResult,
};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::http_client::ReqwestHttpClient;

pub trait RunBackend: Send + Sync {
    fn run_plan(
        &self,
        user_id: i64,
        session_id: i64,
        content: &str,
        model: Option<&str>,
        editor_context: Option<&EditorContext>,
    ) -> impl Future<Output = Result<RunPlan, String>> + Send;
    fn persist_message(
        &self,
        user_id: i64,
        session_id: i64,
        role: Role,
        content: &str,
        usage: Option<&TurnTelemetry>,
        tool_calls: Option<&[openwebide_core::ToolCall]>,
    ) -> impl Future<Output = Result<ChatMessage, String>> + Send;
    fn upsert_tool_step(
        &self,
        user_id: i64,
        session_id: i64,
        anchor_id: i64,
        id: &str,
        name: &str,
        summary: &str,
    ) -> impl Future<Output = Result<(), String>> + Send;
    fn complete_tool_step(
        &self,
        user_id: i64,
        session_id: i64,
        id: &str,
        ok: bool,
        summary: &str,
        diff: Option<&FileDiff>,
    ) -> impl Future<Output = Result<(), String>> + Send;
    fn set_tool_stream_unsupported(
        &self,
        user_id: i64,
        connection_id: i64,
        tool_stream_revision: i64,
    ) -> impl Future<Output = Result<(), String>> + Send;
    fn list_connections(
        &self,
        user_id: i64,
    ) -> impl Future<Output = Result<Vec<Connection>, String>> + Send;
    fn web_search(
        &self,
        user_id: i64,
        query: &str,
        limit: usize,
    ) -> impl Future<Output = Result<Vec<WebSearchResult>, String>> + Send;
    fn web_fetch(
        &self,
        user_id: i64,
        url: &str,
    ) -> impl Future<Output = Result<String, String>> + Send;
}

#[derive(Clone, Debug)]
pub struct BackendClient {
    url: String,
    secret: Arc<str>,
    http: ReqwestHttpClient,
}

impl BackendClient {
    pub fn new(url: String, secret: Arc<str>, http: ReqwestHttpClient) -> Self {
        Self { url, secret, http }
    }

    async fn call<T: DeserializeOwned>(
        &self,
        user_id: i64,
        method: &str,
        path: &str,
        body: Value,
    ) -> Result<T, String> {
        let body = if method == "GET" {
            Bytes::new()
        } else {
            Bytes::from(serde_json::to_vec(&body).map_err(|e| e.to_string())?)
        };
        let request = Request::builder()
            .method(method)
            .uri(format!("{}{}", self.url.trim_end_matches('/'), path))
            .header("authorization", format!("Bearer {}", self.secret))
            .header("x-openwebide-user", user_id.to_string())
            .header("content-type", "application/json")
            .body(Full::new(body))
            .map_err(|e| e.to_string())?;
        serde_json::from_value(self.http.json(request).await.map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())
    }
}

pub fn encode_query(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

impl RunBackend for BackendClient {
    async fn run_plan(
        &self,
        user_id: i64,
        session_id: i64,
        content: &str,
        model: Option<&str>,
        editor_context: Option<&EditorContext>,
    ) -> Result<RunPlan, String> {
        self.call(
            user_id,
            "POST",
            &format!("/sessions/{session_id}/run-plan"),
            json!({"content":content,"model":model,"editor_context":editor_context}),
        )
        .await
    }
    async fn persist_message(
        &self,
        user_id: i64,
        session_id: i64,
        role: Role,
        content: &str,
        usage: Option<&TurnTelemetry>,
        tool_calls: Option<&[openwebide_core::ToolCall]>,
    ) -> Result<ChatMessage, String> {
        self.call(
            user_id,
            "POST",
            &format!("/sessions/{session_id}/messages/persist"),
            json!({"role":role,"content":content,"usage":usage,"tool_calls":tool_calls}),
        )
        .await
    }
    async fn upsert_tool_step(
        &self,
        user_id: i64,
        session_id: i64,
        anchor_id: i64,
        id: &str,
        name: &str,
        summary: &str,
    ) -> Result<(), String> {
        let _: Value = self.call(user_id, "POST", &format!("/sessions/{session_id}/tool-steps/upsert"), json!({"anchor_message_id":anchor_id,"tool_call_id":id,"name":name,"summary":summary})).await?;
        Ok(())
    }
    async fn complete_tool_step(
        &self,
        user_id: i64,
        session_id: i64,
        id: &str,
        ok: bool,
        summary: &str,
        diff: Option<&FileDiff>,
    ) -> Result<(), String> {
        let _: Value = self
            .call(
                user_id,
                "POST",
                &format!("/sessions/{session_id}/tool-steps/complete"),
                json!({"tool_call_id":id,"ok":ok,"result_summary":summary,"diff":diff}),
            )
            .await?;
        Ok(())
    }
    async fn set_tool_stream_unsupported(
        &self,
        user_id: i64,
        connection_id: i64,
        tool_stream_revision: i64,
    ) -> Result<(), String> {
        let _: Value = self
            .call(
                user_id,
                "POST",
                &format!("/connections/{connection_id}/tool-stream-unsupported"),
                json!({"tool_stream_revision": tool_stream_revision}),
            )
            .await?;
        Ok(())
    }
    async fn list_connections(&self, user_id: i64) -> Result<Vec<Connection>, String> {
        self.call(user_id, "GET", "/connections", Value::Null).await
    }
    async fn web_search(
        &self,
        user_id: i64,
        query: &str,
        limit: usize,
    ) -> Result<Vec<WebSearchResult>, String> {
        self.call(
            user_id,
            "GET",
            &format!("/web/search?query={}&limit={limit}", encode_query(query)),
            Value::Null,
        )
        .await
    }
    async fn web_fetch(&self, user_id: i64, url: &str) -> Result<String, String> {
        let response: Value = self
            .call(
                user_id,
                "GET",
                &format!("/web/fetch?url={}", encode_query(url)),
                Value::Null,
            )
            .await?;
        response
            .get("content")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| "missing page content".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn records_streamed_tools_flag_at_connection_route() {
        let (url, captured) = crate::http_client::tests::capture(
            "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}",
        )
        .await;
        let client = BackendClient::new(
            format!("{url}/api"),
            "shared-secret".into(),
            ReqwestHttpClient::default(),
        );
        client.set_tool_stream_unsupported(42, 7, 3).await.unwrap();
        let request = captured.await.unwrap().to_ascii_lowercase();
        assert!(
            request.starts_with("post /api/connections/7/tool-stream-unsupported http/1.1\r\n")
        );
        assert!(request.contains("x-openwebide-user: 42\r\n"));
        assert!(request.contains("\"tool_stream_revision\":3"));
    }

    #[tokio::test]
    async fn sends_secret_and_acting_user() {
        let (url, captured) = crate::http_client::tests::capture(
            "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n[]",
        )
        .await;
        let client = BackendClient::new(
            format!("{url}/api"),
            "shared-secret".into(),
            ReqwestHttpClient::default(),
        );
        assert!(client.list_connections(42).await.unwrap().is_empty());
        let request = captured.await.unwrap().to_ascii_lowercase();
        assert!(request.starts_with("get /api/connections http/1.1\r\n"));
        assert!(request.contains("authorization: bearer shared-secret\r\n"));
        assert!(request.contains("x-openwebide-user: 42\r\n"));
    }
}
