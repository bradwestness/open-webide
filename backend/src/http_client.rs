//! Spin's outbound HTTP, adapted to the provider-facing [`HttpClient`] trait.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::Bytes;
use futures::Stream;
use http_body_util::BodyExt;
use openwebide_llm::{HttpClient, ProviderError};
#[cfg(target_arch = "wasm32")]
use spin_sdk::http;
use spin_sdk::http::Response;

/// Outbound HTTP client backed by Spin's WASI HTTP handler.
#[derive(Clone, Default)]
pub struct SpinHttpClient {
    transport: openwebide_core::ServerTransport,
}
impl SpinHttpClient {
    pub fn with_transport(mut self, transport: openwebide_core::ServerTransport) -> Self {
        self.transport = transport;
        self
    }
    #[cfg(target_arch = "wasm32")]
    async fn send(&self, method: &str, url: &str, body: Bytes) -> Result<Response, ProviderError> {
        let mut builder = http::Request::builder()
            .method(method)
            .uri(url)
            .header("content-type", "application/json");
        for (name, value) in &self.transport.headers {
            builder = builder.header(name, value);
        }
        if let Some(key) = &self.transport.api_key {
            builder = builder.header("authorization", format!("Bearer {key}"));
        }
        let request = builder
            .body(http::FullBody::new(body))
            .map_err(|_| ProviderError::Http("Invalid server headers or URL.".into()))?;
        let timeout = spin_sdk::time::sleep(std::time::Duration::from_secs(u64::from(
            self.transport.timeout_seconds,
        )));
        futures::pin_mut!(timeout);
        let send = http::send(request);
        futures::pin_mut!(send);
        match futures::future::select(send, timeout).await {
            futures::future::Either::Left((result, _)) => {
                result.map_err(|e| outbound_error(url, e))
            }
            futures::future::Either::Right(_) => Err(ProviderError::Http(
                "Model server request timed out.".into(),
            )),
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    async fn send(
        &self,
        _method: &str,
        _url: &str,
        _body: Bytes,
    ) -> Result<Response, ProviderError> {
        Err(ProviderError::NotImplemented(
            "Spin HTTP requires a WASI host.".into(),
        ))
    }
    fn payload(&self, url: &str, body: &serde_json::Value) -> Result<Bytes, ProviderError> {
        let mut body = body.clone();
        if url.ends_with("/api/chat")
            && let Some(keep_alive) = &self.transport.keep_alive
        {
            body["keep_alive"] = serde_json::json!(keep_alive);
        }
        serde_json::to_vec(&body)
            .map(Bytes::from)
            .map_err(|e| ProviderError::Parse(e.to_string()))
    }
}

/// Turn a failed outbound request into an actionable error. Spin denies
/// requests to hosts not in `allowed_outbound_hosts` with a
/// `HttpRequestDenied` error — that's the one failure where we can tell the
/// user exactly what to do.
fn outbound_error(url: &str, e: impl std::fmt::Display) -> ProviderError {
    let detail = e.to_string();
    if detail.contains("HttpRequestDenied") {
        ProviderError::Http(format!(
            "outbound to {url} is blocked by Spin's `allowed_outbound_hosts` allowlist. Add the host to spin.toml (see README → Host egress) and restart Spin"
        ))
    } else {
        ProviderError::Http(format!("cannot reach {url}: {detail}"))
    }
}

impl HttpClient for SpinHttpClient {
    async fn get_json(&self, url: &str) -> Result<serde_json::Value, ProviderError> {
        let response = self.send("GET", url, Bytes::new()).await?;
        json_from_response(response).await
    }

    async fn post_json(
        &self,
        url: &str,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value, ProviderError> {
        let response = self.send("POST", url, self.payload(url, body)?).await?;
        json_from_response(response).await
    }

    fn post_stream(
        &self,
        url: &str,
        body: &serde_json::Value,
    ) -> Pin<Box<dyn Stream<Item = Result<Bytes, ProviderError>> + Send + 'static>> {
        let url = url.to_string();
        let payload = self.payload(&url, body);
        let client = self.clone();
        Box::pin(SpinStream {
            url: url.clone(),
            state: SpinStreamState::Request(Box::pin(async move {
                client
                    .send("POST", &url, payload.map_err(|e| e.to_string())?)
                    .await
                    .map_err(|e| e.to_string())
            })),
        })
    }
}

async fn json_from_response(response: Response) -> Result<serde_json::Value, ProviderError> {
    let status = response.status();
    let collected = response
        .into_body()
        .collect()
        .await
        .map_err(|e| ProviderError::Http(format!("read response body: {e}")))?;
    let text = String::from_utf8(collected.to_bytes().to_vec())
        .map_err(|_| ProviderError::Parse("response body is not valid UTF-8".into()))?;

    if matches!(status.as_u16(), 401 | 403) {
        return Err(ProviderError::Authentication);
    }
    if !status.is_success() {
        // Ollama and llama.cpp both report failures as {"error": "..."}.
        let detail = serde_json::from_str::<serde_json::Value>(&text)
            .ok()
            .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(str::to_string))
            .unwrap_or(text);
        return Err(ProviderError::Http(format!("{status}: {detail}")));
    }

    serde_json::from_str(&text).map_err(|e| ProviderError::Parse(format!("invalid JSON: {e}")))
}

/// Outbound streaming POST: awaits the response, then yields body chunks as
/// they arrive. Non-success statuses are read in full and reported as a
/// single [`ProviderError::Http`], mirroring `json_from_response`.
struct SpinStream {
    url: String,
    state: SpinStreamState,
}

enum SpinStreamState {
    Request(Pin<Box<dyn Future<Output = Result<Response, String>> + Send>>),
    ErrorBody(Pin<Box<dyn Future<Output = ProviderError> + Send>>),
    Body(Pin<Box<dyn Stream<Item = Result<Bytes, ProviderError>> + Send>>),
    Done,
}

impl Stream for SpinStream {
    type Item = Result<Bytes, ProviderError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        loop {
            // Take the state out so the arms can put a new one back.
            let state = std::mem::replace(&mut self.state, SpinStreamState::Done);
            match state {
                SpinStreamState::Request(mut future) => match Future::poll(future.as_mut(), cx) {
                    Poll::Ready(Ok(response)) => {
                        let status = response.status();
                        let body = response.into_body();
                        self.state = if status.is_success() {
                            SpinStreamState::Body(Box::pin(BodyStream::new(body)))
                        } else {
                            SpinStreamState::ErrorBody(Box::pin(error_body(status.as_u16(), body)))
                        };
                    }
                    Poll::Ready(Err(e)) => {
                        return Poll::Ready(Some(Err(outbound_error(&self.url, e))));
                    }
                    Poll::Pending => {
                        self.state = SpinStreamState::Request(future);
                        return Poll::Pending;
                    }
                },
                SpinStreamState::ErrorBody(mut future) => match Future::poll(future.as_mut(), cx) {
                    Poll::Ready(error) => return Poll::Ready(Some(Err(error))),
                    Poll::Pending => {
                        self.state = SpinStreamState::ErrorBody(future);
                        return Poll::Pending;
                    }
                },
                SpinStreamState::Body(mut stream) => match Stream::poll_next(stream.as_mut(), cx) {
                    Poll::Ready(Some(chunk)) => {
                        self.state = SpinStreamState::Body(stream);
                        return Poll::Ready(Some(chunk));
                    }
                    Poll::Ready(None) => return Poll::Ready(None),
                    Poll::Pending => {
                        self.state = SpinStreamState::Body(stream);
                        return Poll::Pending;
                    }
                },
                SpinStreamState::Done => return Poll::Ready(None),
            }
        }
    }
}

/// Adapts an `http_body::Body` to a byte-chunk stream.
struct BodyStream<B> {
    body: B,
}

impl<B> BodyStream<B> {
    fn new(body: B) -> Self {
        Self { body }
    }
}

impl<B> Stream for BodyStream<B>
where
    B: http_body::Body + Unpin,
    B::Data: AsRef<[u8]>,
    B::Error: std::fmt::Display,
{
    type Item = Result<Bytes, ProviderError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match http_body::Body::poll_frame(Pin::new(&mut self.body), cx) {
            Poll::Ready(Some(Ok(frame))) => {
                // A frame without data carries trailers only: end of stream.
                match frame.into_data() {
                    Ok(data) => Poll::Ready(Some(Ok(Bytes::copy_from_slice(data.as_ref())))),
                    Err(_) => Poll::Ready(None),
                }
            }
            Poll::Ready(Some(Err(e))) => Poll::Ready(Some(Err(ProviderError::Http(e.to_string())))),
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}

/// Collect a non-success response body and report it as a provider error.
async fn error_body<B>(status: u16, body: B) -> ProviderError
where
    B: http_body::Body + Unpin + Send + 'static,
    B::Data: AsRef<[u8]>,
    B::Error: std::fmt::Display,
{
    if matches!(status, 401 | 403) {
        return ProviderError::Authentication;
    }
    let collected = match body.collect().await {
        Ok(collected) => collected,
        Err(e) => return ProviderError::Http(format!("read error body: {e}")),
    };
    let text = String::from_utf8_lossy(&collected.to_bytes()).to_string();
    let detail = serde_json::from_str::<serde_json::Value>(&text)
        .ok()
        .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(str::to_string))
        .unwrap_or(text);
    ProviderError::Http(format!("{status}: {detail}"))
}
