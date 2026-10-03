use std::pin::Pin;
use std::time::Duration;

use bytes::Bytes;
use futures::{Stream, StreamExt, stream};
use http_body_util::{BodyExt, Full};
use hyper::{Request, Response, body::Incoming};
use hyper_util::client::legacy::{Client, connect::HttpConnector};
use hyper_util::rt::TokioExecutor;
use openwebide_llm::{HttpClient, ProviderError};

#[cfg(feature = "tls")]
type Connector = hyper_rustls::HttpsConnector<HttpConnector>;
#[cfg(not(feature = "tls"))]
type Connector = HttpConnector;

#[derive(Clone, Debug)]
pub struct ReqwestHttpClient {
    client: Client<Connector, Full<Bytes>>,
    transport: openwebide_core::ServerTransport,
}

impl Default for ReqwestHttpClient {
    fn default() -> Self {
        let mut http = HttpConnector::new();
        http.set_connect_timeout(Some(Duration::from_secs(10)));
        #[cfg(feature = "tls")]
        let connector = {
            http.enforce_http(false);
            hyper_rustls::HttpsConnectorBuilder::new()
                .with_webpki_roots()
                .https_or_http()
                .enable_http1()
                .wrap_connector(http)
        };
        #[cfg(not(feature = "tls"))]
        let connector = http;
        Self {
            client: Client::builder(TokioExecutor::new()).build(connector),
            transport: Default::default(),
        }
    }
}

impl ReqwestHttpClient {
    pub fn with_transport(mut self, transport: openwebide_core::ServerTransport) -> Self {
        self.transport = transport;
        self
    }
    pub async fn send(
        &self,
        mut request: Request<Full<Bytes>>,
    ) -> Result<Response<Incoming>, ProviderError> {
        for (name, value) in &self.transport.headers {
            request.headers_mut().insert(
                name.parse::<hyper::header::HeaderName>()
                    .map_err(|_| ProviderError::Http("Invalid server header.".into()))?,
                value
                    .parse()
                    .map_err(|_| ProviderError::Http("Invalid server header.".into()))?,
            );
        }
        if let Some(key) = &self.transport.api_key {
            request.headers_mut().insert(
                "authorization",
                format!("Bearer {key}")
                    .parse()
                    .map_err(|_| ProviderError::Http("Invalid API key.".into()))?,
            );
        }
        tokio::time::timeout(
            Duration::from_secs(u64::from(self.transport.timeout_seconds)),
            self.client.request(request),
        )
        .await
        .map_err(|_| ProviderError::Http("Model server request timed out.".into()))?
        .map_err(|_| ProviderError::Http("Cannot reach model server.".into()))
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
    pub async fn json(
        &self,
        request: Request<Full<Bytes>>,
    ) -> Result<serde_json::Value, ProviderError> {
        let response = self.send(request).await?;
        let status = response.status();
        let body = response
            .into_body()
            .collect()
            .await
            .map_err(|e| ProviderError::Http(e.to_string()))?
            .to_bytes();
        if !status.is_success() {
            return Err(status_error(status, &body));
        }
        serde_json::from_slice(&body)
            .map_err(|e| ProviderError::Parse(format!("invalid JSON: {e}")))
    }
}

fn status_error(status: hyper::StatusCode, body: &[u8]) -> ProviderError {
    if matches!(status.as_u16(), 401 | 403) {
        return ProviderError::Authentication;
    }
    let text = String::from_utf8_lossy(body).into_owned();
    let detail = serde_json::from_str::<serde_json::Value>(&text)
        .ok()
        .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(str::to_string))
        .unwrap_or(text);
    ProviderError::Http(format!("{status}: {detail}"))
}

fn request(method: &str, url: &str, body: Bytes) -> Result<Request<Full<Bytes>>, ProviderError> {
    Request::builder()
        .method(method)
        .uri(url)
        .header("content-type", "application/json")
        .body(Full::new(body))
        .map_err(|e| ProviderError::Http(e.to_string()))
}

impl HttpClient for ReqwestHttpClient {
    async fn get_json(&self, url: &str) -> Result<serde_json::Value, ProviderError> {
        self.json(request("GET", url, Bytes::new())?).await
    }

    async fn post_json(
        &self,
        url: &str,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value, ProviderError> {
        let bytes = self.payload(url, body)?;
        self.json(request("POST", url, bytes)?).await
    }

    fn post_stream(
        &self,
        url: &str,
        body: &serde_json::Value,
    ) -> Pin<Box<dyn Stream<Item = Result<Bytes, ProviderError>> + Send + 'static>> {
        let client = self.clone();
        let req = self
            .payload(url, body)
            .and_then(|body| request("POST", url, body));
        Box::pin(
            stream::once(async move {
                let response = client.send(req?).await?;
                let status = response.status();
                let body = response.into_body();
                if !status.is_success() {
                    let bytes = body
                        .collect()
                        .await
                        .map_err(|e| ProviderError::Http(e.to_string()))?
                        .to_bytes();
                    return Err(status_error(status, &bytes));
                }
                Ok(body)
            })
            .flat_map(
                |result| -> Pin<Box<dyn Stream<Item = Result<Bytes, ProviderError>> + Send>> {
                    match result {
                        Err(error) => Box::pin(stream::once(async { Err(error) })),
                        Ok(body) => Box::pin(
                            body.into_data_stream()
                                .map(|chunk| chunk.map_err(|e| ProviderError::Http(e.to_string()))),
                        ),
                    }
                },
            ),
        )
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    pub async fn capture(response: &'static str) -> (String, tokio::task::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let request = read_request(&mut socket).await;
            socket.write_all(response.as_bytes()).await.unwrap();
            request
        });
        (url, task)
    }

    async fn read_request(socket: &mut tokio::net::TcpStream) -> String {
        let mut bytes = Vec::new();
        loop {
            let mut buf = [0u8; 4096];
            let n = socket.read(&mut buf).await.unwrap();
            assert!(n > 0);
            bytes.extend_from_slice(&buf[..n]);
            if let Some(head_end) = bytes.windows(4).position(|b| b == b"\r\n\r\n") {
                let head = String::from_utf8_lossy(&bytes[..head_end]).to_ascii_lowercase();
                let length = head
                    .lines()
                    .find_map(|l| l.strip_prefix("content-length: "))
                    .map(|n| n.parse::<usize>().unwrap())
                    .unwrap_or(0);
                if bytes.len() >= head_end + 4 + length {
                    break;
                }
            }
        }
        String::from_utf8(bytes).unwrap()
    }

    #[tokio::test]
    async fn server_credentials_and_keep_alive_reach_native_requests() {
        let (url, task) =
            capture("HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}").await;
        let client =
            ReqwestHttpClient::default().with_transport(openwebide_core::ServerTransport {
                api_key: Some("test-key".into()),
                headers: std::collections::BTreeMap::from([(
                    "X-Proxy-Key".into(),
                    "test-header".into(),
                )]),
                keep_alive: Some("10m".into()),
                ..Default::default()
            });
        client
            .post_json(
                &format!("{url}/api/chat"),
                &serde_json::json!({"model": "main"}),
            )
            .await
            .unwrap();
        let request = task.await.unwrap();
        assert!(
            request
                .to_ascii_lowercase()
                .contains("authorization: bearer test-key")
        );
        assert!(
            request
                .to_ascii_lowercase()
                .contains("x-proxy-key: test-header")
        );
        assert!(request.contains("\"keep_alive\":\"10m\""));
        let (url, task) =
            capture("HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .await;
        assert!(matches!(
            client.get_json(&url).await,
            Err(ProviderError::Authentication)
        ));
        task.await.unwrap();
    }

    #[tokio::test]
    async fn http_errors_extract_json_detail() {
        let (url, task) = capture("HTTP/1.1 500 Internal Server Error\r\nContent-Length: 13\r\nConnection: close\r\n\r\n{\"error\":\"x\"}").await;
        let error = ReqwestHttpClient::default()
            .get_json(&url)
            .await
            .unwrap_err();
        assert!(matches!(error, ProviderError::Http(ref s) if s == "500 Internal Server Error: x"));
        task.await.unwrap();
    }

    #[tokio::test]
    async fn chunked_body_streams_before_eof() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let (release, wait) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            read_request(&mut socket).await;
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n")
                .await
                .unwrap();
            wait.await.unwrap();
            socket.write_all(b"3\r\ndef\r\n0\r\n\r\n").await.unwrap();
        });
        let mut stream = ReqwestHttpClient::default().post_stream(&url, &serde_json::json!({}));
        let first = tokio::time::timeout(Duration::from_secs(2), stream.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(first, "abc");
        release.send(()).unwrap();
        assert_eq!(stream.next().await.unwrap().unwrap(), "def");
        assert!(stream.next().await.is_none());
        task.await.unwrap();
    }
}
