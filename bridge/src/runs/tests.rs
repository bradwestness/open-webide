use super::*;
use futures::stream;
use openwebide_agent::PermissionGate;
use openwebide_core::{
    ChatCompletion, ChatRequest, ChatResponse, Connection, FileDiff, ModelInfo, ProviderKind,
    ToolCall, WebSearchResult,
};
use openwebide_llm::ToolStreamChunk;
use std::pin::Pin;

#[derive(Default)]
struct FakeBackend {
    messages: Mutex<Vec<ChatMessage>>,
    operations: Mutex<Vec<String>>,
    kind: Mutex<Option<RunKind>>,
    fail_final: AtomicBool,
    fail_completion: AtomicBool,
    fail_interim: AtomicBool,
    fail_plan: AtomicBool,
}

impl RunBackend for FakeBackend {
    async fn run_plan(
        &self,
        _user_id: i64,
        _session_id: i64,
        content: &str,
        _model: Option<&str>,
        _editor_context: Option<&EditorContext>,
    ) -> Result<RunPlan, String> {
        if self.fail_plan.load(Ordering::SeqCst) {
            return Err("plan failed".into());
        }
        Ok(plan(
            self.kind.lock().unwrap().clone().unwrap_or(RunKind::Chat),
            content,
        ))
    }
    async fn persist_message(
        &self,
        _user_id: i64,
        session_id: i64,
        role: Role,
        content: &str,
        usage: Option<&TurnTelemetry>,
        tool_calls: Option<&[openwebide_core::ToolCall]>,
    ) -> Result<ChatMessage, String> {
        if (content == "final" && self.fail_final.load(Ordering::SeqCst))
            || (content == "interim" && self.fail_interim.load(Ordering::SeqCst))
        {
            return Err("offline".into());
        }
        let mut messages = self.messages.lock().unwrap();
        let message = ChatMessage {
            id: i64::try_from(messages.len()).unwrap() + 10,
            session_id,
            role,
            content: content.into(),
            usage: usage.copied(),
            created_at: 1,
            tool_calls: tool_calls.map(<[ToolCall]>::to_vec),
            tool_call_id: None,
        };
        messages.push(message.clone());
        self.operations
            .lock()
            .unwrap()
            .push(format!("message:{}", message.id));
        Ok(message)
    }
    async fn upsert_tool_step(
        &self,
        _user_id: i64,
        _session_id: i64,
        anchor_id: i64,
        id: &str,
        _name: &str,
        _summary: &str,
        _diff: Option<&FileDiff>,
    ) -> Result<(), String> {
        self.operations
            .lock()
            .unwrap()
            .push(format!("step:{anchor_id}:{id}"));
        Ok(())
    }
    async fn complete_tool_step(
        &self,
        _user_id: i64,
        _session_id: i64,
        id: &str,
        _ok: bool,
        _summary: &str,
        _diff: Option<&FileDiff>,
    ) -> Result<(), String> {
        self.operations
            .lock()
            .unwrap()
            .push(format!("complete:{id}"));
        if self.fail_completion.load(Ordering::SeqCst) {
            return Err("offline".into());
        }
        Ok(())
    }
    async fn set_tool_stream_unsupported(
        &self,
        user_id: i64,
        connection_id: i64,
        tool_stream_revision: i64,
    ) -> Result<(), String> {
        self.operations.lock().unwrap().push(format!(
            "memo:{user_id}:{connection_id}:{tool_stream_revision}"
        ));
        Ok(())
    }
    async fn list_connections(&self, _user_id: i64) -> Result<Vec<Connection>, String> {
        Ok(vec![])
    }
    async fn web_search(
        &self,
        _user_id: i64,
        _query: &str,
        _limit: usize,
    ) -> Result<Vec<WebSearchResult>, String> {
        Ok(vec![])
    }
    async fn web_fetch(&self, _user_id: i64, _url: &str) -> Result<String, String> {
        Ok(String::new())
    }
}

fn plan(kind: RunKind, content: &str) -> RunPlan {
    RunPlan {
        kind,
        user_content: content.into(),
        request: ChatRequest {
            connection_id: 1,
            system_prompt: None,
            model: None,
            messages: vec![],
            tools: vec![],
        },
        connection: Connection {
            id: 1,
            name: "model".into(),
            kind: ProviderKind::Ollama,
            base_url: "http://model".into(),
            model: Some("model".into()),
            enabled: true,
            context_limit: None,
            tool_stream_unsupported: false,
            tool_stream_revision: 0,
        },
    }
}

#[derive(Default)]
struct FakeProvider {
    chat: Mutex<Vec<Result<StreamChunk, ProviderError>>>,
    tools: Mutex<Vec<Vec<Result<ToolStreamChunk, ProviderError>>>>,
    pending: bool,
}

impl LlmProvider for FakeProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Ollama
    }
    async fn list_models(&self) -> Result<Vec<ModelInfo>, ProviderError> {
        Ok(vec![])
    }
    async fn chat(&self, _request: &ChatRequest) -> Result<String, ProviderError> {
        unreachable!()
    }
    async fn chat_tools(&self, _request: &ChatRequest) -> Result<ChatCompletion, ProviderError> {
        unreachable!()
    }
    async fn context_limit(&self, _model: Option<&str>) -> Result<Option<usize>, ProviderError> {
        Ok(None)
    }
    fn chat_stream(
        &self,
        _request: &ChatRequest,
    ) -> Pin<Box<dyn Stream<Item = Result<StreamChunk, ProviderError>> + Send + 'static>> {
        let chunks = std::mem::take(&mut *self.chat.lock().unwrap());
        if self.pending {
            Box::pin(stream::iter(chunks).chain(stream::pending()))
        } else {
            Box::pin(stream::iter(chunks))
        }
    }
    fn chat_tools_stream(
        &self,
        _request: &ChatRequest,
    ) -> Pin<Box<dyn Stream<Item = Result<ToolStreamChunk, ProviderError>> + Send + 'static>> {
        let chunks = self.tools.lock().unwrap().remove(0);
        Box::pin(stream::iter(chunks))
    }
}

fn start(id: &str) -> StartRun {
    StartRun {
        run_id: id.into(),
        session_id: 1,
        content: "go".into(),
        model: None,
        editor_context: None,
    }
}
fn user(id: i64) -> Principal {
    Principal::User { user_id: id }
}
async fn finished(run: &Run) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while run.running.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
fn events(run: &Run) -> Vec<RunEvent> {
    run.delivery
        .lock()
        .unwrap()
        .ring
        .read_after(0, 4096)
        .0
        .into_iter()
        .map(|(_, e)| e)
        .collect()
}

#[tokio::test]
async fn buffered_start_cancel_survives_delayed_start_task() {
    use openwebide_core::BridgeClientMessage;

    let dir = tempfile::tempdir().unwrap();
    let registry = Arc::new(RunRegistry::default());
    let backend = Arc::new(FakeBackend::default());
    let mut frames = stream::iter([
        r#"{"type":"run_start","run_id":"r","session_id":1,"content":"go"}"#,
        r#"{"type":"run_cancel","run_id":"r"}"#,
    ]);
    let (release, delayed) = tokio::sync::oneshot::channel::<()>();
    let mut delayed = Some(delayed);
    let mut task = None;
    while let Some(frame) = frames.next().await {
        match serde_json::from_str::<BridgeClientMessage>(frame).unwrap() {
            BridgeClientMessage::RunStart {
                run_id,
                session_id,
                content,
                model,
                editor_context,
            } => {
                let start = StartRun {
                    run_id,
                    session_id,
                    content,
                    model,
                    editor_context,
                };
                let run = registry.reserve(&user(1), &start).unwrap();
                let registry = registry.clone();
                let backend = backend.clone();
                let workspace = dir.path().to_path_buf();
                let delayed = delayed.take().unwrap();
                task = Some(tokio::spawn(async move {
                    delayed.await.unwrap();
                    registry
                        .prepare(
                            run,
                            start,
                            &workspace,
                            backend,
                            |_| -> FakeProvider {
                                panic!("cancelled start must not call the model")
                            },
                            Arc::new(crate::exec::HostExecution),
                        )
                        .await
                        .unwrap()
                }));
            }
            BridgeClientMessage::RunCancel { run_id } => {
                registry.get(&user(1), &run_id).unwrap().cancel.cancel();
            }
            _ => unreachable!(),
        }
    }
    assert!(registry.get(&user(1), "r").unwrap().cancel.is_cancelled());
    assert!(backend.messages.lock().unwrap().is_empty());
    release.send(()).unwrap();
    let run = task.unwrap().await.unwrap();
    finished(&run).await;
    assert_eq!(events(&run), [RunEvent::Cancelled]);
    assert!(backend.messages.lock().unwrap().is_empty());
    assert!(!registry.list(&user(1), 1)[0].running);
}

#[tokio::test]
async fn agent_mapping_persists_in_order_and_reanchors_with_last_usage() {
    let backend = FakeBackend::default();
    let run = Run::new("r".into(), 1, 1, 4096);
    let first = TurnTelemetry {
        prompt_tokens: 10,
        ..Default::default()
    };
    let last = TurnTelemetry {
        prompt_tokens: 20,
        ..Default::default()
    };
    map_agent_events(
        &run,
        &backend,
        7,
        stream::iter(vec![
            AgentEvent::ToolCall {
                id: "a".into(),
                name: "read_file".into(),
                summary: "a".into(),
            },
            AgentEvent::ToolResult {
                id: "a".into(),
                name: "read_file".into(),
                ok: true,
                summary: "read".into(),
                diff: None,
            },
            AgentEvent::Telemetry(first),
            AgentEvent::TurnCalls {
                text: "interim".into(),
                calls: vec![],
            },
            AgentEvent::PermissionRequest {
                id: "b".into(),
                name: "write_file".into(),
                summary: "b".into(),
                diff: None,
                note: None,
            },
            AgentEvent::Telemetry(last),
            AgentEvent::FinalText("final".into()),
        ]),
    )
    .await;
    assert_eq!(
        *backend.operations.lock().unwrap(),
        [
            "step:7:a",
            "complete:a",
            "message:10",
            "step:10:b",
            "message:11"
        ]
    );
    let messages = backend.messages.lock().unwrap();
    assert_eq!(messages[0].usage, Some(first));
    assert_eq!(messages[1].usage, Some(last));
    assert!(
        matches!(events(&run).last(), Some(RunEvent::Done { message }) if message.usage == Some(last))
    );
}

#[tokio::test]
async fn completion_persistence_failure_stops_agent_before_releasing_run() {
    struct StreamGuard<'a> {
        registry: &'a RunRegistry,
        run: &'a Run,
        dropped: &'a AtomicBool,
    }

    impl Drop for StreamGuard<'_> {
        fn drop(&mut self) {
            assert!(self.run.running.load(Ordering::SeqCst));
            assert_eq!(
                self.registry.reserve(&user(1), &start("r2")).unwrap_err().0,
                RunRejectCode::Busy
            );
            self.dropped.store(true, Ordering::SeqCst);
        }
    }

    let backend = FakeBackend::default();
    backend.fail_completion.store(true, Ordering::SeqCst);
    let registry = RunRegistry::default();
    let run = registry.reserve(&user(1), &start("r")).unwrap();
    let dropped = AtomicBool::new(false);
    let guard = StreamGuard {
        registry: &registry,
        run: &run,
        dropped: &dropped,
    };
    let mut result = Some(AgentEvent::ToolResult {
        id: "t".into(),
        name: "write_file".into(),
        ok: true,
        summary: "written".into(),
        diff: None,
    });
    let stream = stream::poll_fn(move |_| {
        let _ = &guard;
        match result.take() {
            Some(event) => std::task::Poll::Ready(Some(event)),
            None => std::task::Poll::Pending,
        }
    });
    tokio::time::timeout(
        Duration::from_secs(2),
        map_agent_events(&run, &backend, 7, stream),
    )
    .await
    .unwrap();
    assert!(dropped.load(Ordering::SeqCst));
    assert!(!run.running.load(Ordering::SeqCst));
    assert!(registry.reserve(&user(1), &start("r2")).is_ok());
    assert_eq!(*backend.operations.lock().unwrap(), ["complete:t"]);

    let (sender, mut receiver) = mpsc::channel(8);
    run.clone().forward(Some(0), sender).await;
    assert!(matches!(
        recv(&mut receiver).await,
        BridgeServerMessage::RunEvent {
            seq: 1,
            event: RunEvent::ToolResult { id, .. },
            ..
        } if id == "t"
    ));
    assert!(matches!(
        recv(&mut receiver).await,
        BridgeServerMessage::RunEvent {
            seq: 2,
            event: RunEvent::Error { message },
            ..
        } if message == "failed to save tool result: offline"
    ));
    assert!(receiver.recv().await.is_none());
}

#[tokio::test]
async fn mapping_cancel_error_and_failed_persistence() {
    for event in [AgentEvent::Cancelled, AgentEvent::Error("boom".into())] {
        let backend = FakeBackend::default();
        let run = Run::new("r".into(), 1, 1, 4096);
        map_agent_events(&run, &backend, 7, stream::iter(vec![event])).await;
        assert!(backend.messages.lock().unwrap().is_empty());
    }
    let backend = FakeBackend::default();
    backend.fail_final.store(true, Ordering::SeqCst);
    let run = Run::new("r".into(), 1, 1, 4096);
    map_agent_events(
        &run,
        &backend,
        7,
        stream::iter(vec![AgentEvent::FinalText("final".into())]),
    )
    .await;
    assert!(
        matches!(events(&run).last(), Some(RunEvent::Error { message }) if message == "failed to save reply: offline")
    );
    backend.fail_final.store(false, Ordering::SeqCst);
    backend.fail_interim.store(true, Ordering::SeqCst);
    let run = Run::new("r2".into(), 1, 1, 4096);
    map_agent_events(
        &run,
        &backend,
        7,
        stream::iter(vec![
            AgentEvent::TurnCalls {
                text: "interim".into(),
                calls: vec![],
            },
            AgentEvent::ToolCall {
                id: "t".into(),
                name: "read_file".into(),
                summary: "read".into(),
            },
            AgentEvent::FinalText("final".into()),
        ]),
    )
    .await;
    assert!(
        backend
            .operations
            .lock()
            .unwrap()
            .contains(&"step:7:t".to_string())
    );
    assert!(matches!(&events(&run)[0], RunEvent::Interim { message } if message.id == 0));
}

#[tokio::test]
async fn chat_success_truncation_and_errors() {
    let dir = tempfile::tempdir().unwrap();
    for (tail, expected) in [
        (None, Some("reply")),
        (
            Some(ProviderError::Incomplete),
            Some("reply\n\n[reply truncated]"),
        ),
        (Some(ProviderError::Http("boom".into())), None),
    ] {
        let registry = RunRegistry::default();
        let backend = Arc::new(FakeBackend::default());
        let usage = TurnTelemetry {
            prompt_tokens: 10,
            ..Default::default()
        };
        let mut chunks = vec![
            Ok(StreamChunk::Delta("reply".into())),
            Ok(StreamChunk::Usage(usage)),
        ];
        if let Some(error) = tail {
            chunks.push(Err(error));
        }
        let run = registry
            .start(&user(1), start("r"), dir.path(), backend.clone(), |_| {
                FakeProvider {
                    chat: Mutex::new(chunks),
                    ..Default::default()
                }
            })
            .await
            .unwrap();
        finished(&run).await;
        let messages = backend.messages.lock().unwrap();
        assert_eq!(messages[0].role, Role::User);
        assert_eq!(messages.len(), if expected.is_some() { 2 } else { 1 });
        if let Some(text) = expected {
            assert_eq!(messages[1].content, text);
            assert_eq!(messages[1].usage, Some(usage));
        }
    }
}

#[tokio::test]
async fn busy_scoping_cancel_and_reaping() {
    let dir = tempfile::tempdir().unwrap();
    let registry = RunRegistry::default();
    let backend = Arc::new(FakeBackend::default());
    let run = registry
        .start(&user(1), start("r"), dir.path(), backend.clone(), |_| {
            FakeProvider {
                pending: true,
                ..Default::default()
            }
        })
        .await
        .unwrap();
    assert_eq!(
        registry
            .start(&user(1), start("r2"), dir.path(), backend.clone(), |_| {
                FakeProvider::default()
            })
            .await
            .unwrap_err()
            .0,
        RunRejectCode::Busy
    );
    assert_eq!(registry.get(&user(2), "r").unwrap_err(), "run not found");
    assert!(registry.list(&user(2), 1).is_empty());
    assert!(registry.get(&Principal::Paired, "r").is_err());
    assert_eq!(registry.list(&user(1), 1).len(), 1);
    registry.get(&user(1), "r").unwrap().cancel.cancel();
    finished(&run).await;
    assert_eq!(backend.messages.lock().unwrap().len(), 1);
    assert_eq!(events(&run).last(), Some(&RunEvent::Cancelled));
    registry.reap();
    assert!(registry.get(&user(1), "r").is_ok());
    *run.finished_at.lock().unwrap() = Some(Instant::now() - Duration::from_secs(601));
    registry.reap();
    assert!(registry.get(&user(1), "r").is_err());
}

#[tokio::test]
async fn rejection_never_persists_a_user_message() {
    let dir = tempfile::tempdir().unwrap();
    let backend = Arc::new(FakeBackend::default());
    let registry = RunRegistry::default();
    let rejected = registry
        .start(
            &Principal::Paired,
            start("r"),
            dir.path(),
            backend.clone(),
            |_| FakeProvider::default(),
        )
        .await
        .unwrap_err();
    assert_eq!(rejected.0, RunRejectCode::Unauthorized);
    for path in ["missing", "../outside"] {
        *backend.kind.lock().unwrap() = Some(RunKind::Agent {
            project_path: path.into(),
        });
        assert_eq!(
            registry
                .start(&user(1), start("r"), dir.path(), backend.clone(), |_| {
                    FakeProvider::default()
                })
                .await
                .unwrap_err()
                .0,
            RunRejectCode::ProjectUnavailable
        );
    }
    backend.fail_plan.store(true, Ordering::SeqCst);
    assert_eq!(
        registry
            .start(&user(1), start("r"), dir.path(), backend.clone(), |_| {
                FakeProvider::default()
            })
            .await
            .unwrap_err()
            .0,
        RunRejectCode::PlanFailed
    );
    assert!(backend.messages.lock().unwrap().is_empty());
    assert!(registry.list(&user(1), 1).is_empty());
}

#[tokio::test]
async fn cancel_while_agent_waits_for_permission_emits_cancelled() {
    let dir = tempfile::tempdir().unwrap();
    let backend = Arc::new(FakeBackend::default());
    *backend.kind.lock().unwrap() = Some(RunKind::Agent {
        project_path: "".into(),
    });
    let registry = RunRegistry::default();
    let provider = FakeProvider {
        tools: Mutex::new(vec![vec![Ok(ToolStreamChunk::Response(
            ChatResponse::ToolCalls(vec![ToolCall {
                id: "p".into(),
                name: "write_file".into(),
                arguments: r#"{"path":"a","content":"x"}"#.into(),
            }]),
        ))]]),
        ..Default::default()
    };
    let run = registry
        .start(&user(1), start("r"), dir.path(), backend.clone(), |_| {
            provider
        })
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        while !events(&run)
            .iter()
            .any(|e| matches!(e, RunEvent::PermissionRequest { .. }))
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    run.cancel.cancel();
    finished(&run).await;
    assert_eq!(events(&run).last(), Some(&RunEvent::Cancelled));
    let messages = backend.messages.lock().unwrap();
    assert_eq!(messages.len(), 2);
    assert!(messages[1].content.is_empty());
    assert!(messages[1].tool_calls.is_some());
    assert!(!dir.path().join("a").exists());
}

async fn recv(receiver: &mut mpsc::Receiver<WriterCmd>) -> BridgeServerMessage {
    match tokio::time::timeout(Duration::from_secs(2), receiver.recv())
        .await
        .unwrap()
        .unwrap()
    {
        WriterCmd::Send(message) => *message,
        WriterCmd::Attach { .. } => unreachable!(),
    }
}

#[tokio::test]
async fn attach_replays_without_gaps_or_duplicates_and_snapshots_outside_ring() {
    for cursor in [Some(1), None, Some(0)] {
        let run = Arc::new(Run::new("r".into(), 1, 1, 2));
        for text in ["a", "b", "c"] {
            run.emit(RunEvent::Delta {
                content: text.into(),
            });
        }
        let (sender, mut receiver) = mpsc::channel(8);
        let forward = tokio::spawn(run.clone().forward(cursor, sender));
        if cursor == Some(1) {
            for expected in [2, 3] {
                assert!(
                    matches!(recv(&mut receiver).await, BridgeServerMessage::RunEvent { seq, .. } if seq == expected)
                );
            }
        } else {
            assert!(
                matches!(recv(&mut receiver).await, BridgeServerMessage::RunSnapshot { seq: 3, snapshot, .. } if snapshot.text == "abc")
            );
        }
        run.emit(RunEvent::Delta {
            content: "d".into(),
        });
        assert!(matches!(
            recv(&mut receiver).await,
            BridgeServerMessage::RunEvent { seq: 4, .. }
        ));
        run.emit(RunEvent::Cancelled);
        assert!(matches!(
            recv(&mut receiver).await,
            BridgeServerMessage::RunEvent {
                seq: 5,
                event: RunEvent::Cancelled,
                ..
            }
        ));
        forward.await.unwrap();
        assert!(receiver.recv().await.is_none());
    }
}

#[tokio::test]
async fn lagging_attachment_resyncs_snapshot_then_live() {
    let run = Arc::new(Run::new("r".into(), 1, 1, 2));
    let (sender, mut receiver) = mpsc::channel(1);
    let forward = tokio::spawn(run.clone().forward(None, sender));
    tokio::task::yield_now().await;
    for _ in 0..8 {
        run.emit(RunEvent::Delta {
            content: "x".into(),
        });
    }
    tokio::task::yield_now().await;
    assert!(matches!(
        recv(&mut receiver).await,
        BridgeServerMessage::RunSnapshot { seq: 0, .. }
    ));
    assert!(
        matches!(recv(&mut receiver).await, BridgeServerMessage::RunSnapshot { seq: 8, snapshot, .. } if snapshot.text == "xxxxxxxx")
    );
    run.emit(RunEvent::Cancelled);
    assert!(matches!(
        recv(&mut receiver).await,
        BridgeServerMessage::RunEvent { seq: 9, .. }
    ));
    forward.await.unwrap();
}

#[tokio::test]
async fn gate_default_policy_matches_other_hosts() {
    let gate = BridgeGate::new(BridgeCancel::default());
    assert!(gate.needs_approval(&ToolCall {
        id: "t".into(),
        name: "write_file".into(),
        arguments: "{}".into()
    }));
    assert!(!gate.needs_approval(&ToolCall {
        id: "t".into(),
        name: "read_file".into(),
        arguments: "{}".into()
    }));
}

#[cfg(feature = "tls")]
#[tokio::test]
async fn websocket_scopes_attach_cancel_permission_and_list() {
    use futures::SinkExt;
    use tokio_tungstenite::tungstenite::Message;
    let dir = tempfile::tempdir().unwrap();
    let config = crate::ServerConfig::new(dir.path().to_path_buf(), "secret".into(), None);
    let backend = Arc::new(FakeBackend::default());
    let run = config
        .runs
        .start(&user(1), start("owned"), dir.path(), backend, |_| {
            FakeProvider {
                pending: true,
                ..Default::default()
            }
        })
        .await
        .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(crate::run_server(listener, config));
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{address}/"))
        .await
        .unwrap();
    let token =
        openwebide_auth::sign_token_expires("secret", 2, i64::try_from(now()).unwrap() + 120, 0);
    ws.send(Message::Text(
        serde_json::to_string(&openwebide_core::BridgeClientMessage::Hello { token })
            .unwrap()
            .into(),
    ))
    .await
    .unwrap();
    ws.next().await.unwrap().unwrap();
    for command in [
        openwebide_core::BridgeClientMessage::RunAttach {
            run_id: "owned".into(),
            last_seq: None,
        },
        openwebide_core::BridgeClientMessage::RunCancel {
            run_id: "owned".into(),
        },
        openwebide_core::BridgeClientMessage::RunPermission {
            run_id: "owned".into(),
            tool_call_id: "t".into(),
            approved: true,
        },
    ] {
        ws.send(Message::Text(
            serde_json::to_string(&command).unwrap().into(),
        ))
        .await
        .unwrap();
        let message: BridgeServerMessage =
            serde_json::from_str(ws.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
        assert!(
            matches!(message, BridgeServerMessage::Error { id, message } if id == "owned" && message == "run not found")
        );
    }
    assert!(!run.cancel.is_cancelled());
    ws.send(Message::Text(
        r#"{"type":"run_list","session_id":1}"#.into(),
    ))
    .await
    .unwrap();
    let message: BridgeServerMessage =
        serde_json::from_str(ws.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
    assert!(matches!(message, BridgeServerMessage::Runs { runs, .. } if runs.is_empty()));
    run.cancel.cancel();
    finished(&run).await;
    server.abort();
}

#[tokio::test]
async fn agent_body_executes_native_tools_and_persists_interim_then_final() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a"), "native file").unwrap();
    let backend = Arc::new(FakeBackend::default());
    *backend.kind.lock().unwrap() = Some(RunKind::Agent {
        project_path: "".into(),
    });
    let first = TurnTelemetry {
        prompt_tokens: 10,
        ..Default::default()
    };
    let last = TurnTelemetry {
        prompt_tokens: 20,
        ..Default::default()
    };
    let provider = FakeProvider {
        tools: Mutex::new(vec![
            vec![
                Ok(ToolStreamChunk::Delta("interim".into())),
                Ok(ToolStreamChunk::Usage(first)),
                Ok(ToolStreamChunk::Response(ChatResponse::ToolCalls(vec![
                    ToolCall {
                        id: "p".into(),
                        name: "read_file".into(),
                        arguments: r#"{"path":"a"}"#.into(),
                    },
                ]))),
            ],
            vec![
                Ok(ToolStreamChunk::Delta("final".into())),
                Ok(ToolStreamChunk::Usage(last)),
                Ok(ToolStreamChunk::Response(ChatResponse::Text(
                    "final".into(),
                ))),
            ],
        ]),
        ..Default::default()
    };
    let registry = RunRegistry::default();
    let run = registry
        .start(&user(1), start("r"), dir.path(), backend.clone(), |_| {
            provider
        })
        .await
        .unwrap();
    finished(&run).await;
    let messages = backend.messages.lock().unwrap();
    assert_eq!(
        messages
            .iter()
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>(),
        ["go", "interim", "final"]
    );
    assert_eq!(messages[1].usage, Some(first));
    assert_eq!(messages[2].usage, Some(last));
    assert_eq!(
        *backend.operations.lock().unwrap(),
        [
            "message:10",
            "message:11",
            "step:11:a10t1c0",
            "complete:a10t1c0",
            "message:12"
        ]
    );
    assert!(
        events(&run)
            .iter()
            .any(|event| matches!(event, RunEvent::ToolResult { ok: true, .. }))
    );
}

#[tokio::test]
async fn seeded_memo_and_new_detection_record_only_once() {
    let mut connection = plan(RunKind::Chat, "test").connection;
    connection.kind = ProviderKind::LlamaCpp;
    connection.tool_stream_unsupported = true;
    let memos = openwebide_llm::ToolStreamMemos::default();
    let backend = FakeBackend::default();
    let seeded = memos.get_or_insert(&connection);
    assert!(seeded.unsupported());
    record_tool_stream_memo(
        &backend,
        7,
        connection.id,
        connection.tool_stream_revision,
        &seeded,
    )
    .await;
    assert!(backend.operations.lock().unwrap().is_empty());
    connection.base_url = "http://new-server".into();
    connection.tool_stream_revision += 1;
    connection.tool_stream_unsupported = false;
    let fresh = memos.get_or_insert(&connection);
    assert!(!fresh.unsupported());
    fresh.mark_unsupported();
    record_tool_stream_memo(
        &backend,
        7,
        connection.id,
        connection.tool_stream_revision,
        &fresh,
    )
    .await;
    record_tool_stream_memo(
        &backend,
        7,
        connection.id,
        connection.tool_stream_revision,
        &memos.get_or_insert(&connection),
    )
    .await;
    assert_eq!(*backend.operations.lock().unwrap(), vec!["memo:7:1:1"]);
    connection.kind = ProviderKind::Ollama;
    assert!(!memos.get_or_insert(&connection).unsupported());
}

#[tokio::test]
async fn empty_interim_persists_wire_calls_before_step_rows() {
    let backend = FakeBackend::default();
    let run = Run::new("r".into(), 1, 2, 4096);
    let calls = vec![ToolCall {
        id: "wire-id".into(),
        name: "read_file".into(),
        arguments: "{}".into(),
    }];
    map_agent_events(
        &run,
        &backend,
        7,
        stream::iter([
            AgentEvent::TurnCalls {
                text: String::new(),
                calls: calls.clone(),
            },
            AgentEvent::ToolCall {
                id: "a7t1c0".into(),
                name: "read_file".into(),
                summary: "read".into(),
            },
        ]),
    )
    .await;
    let messages = backend.messages.lock().unwrap();
    assert_eq!(messages[0].content, "");
    assert_eq!(messages[0].tool_calls.as_ref(), Some(&calls));
    assert_eq!(
        *backend.operations.lock().unwrap(),
        vec!["message:10", "step:10:a7t1c0"]
    );
}

#[tokio::test]
async fn cancel_running_command_kills_group_before_marker() {
    let dir = tempfile::tempdir().unwrap();
    let backend = Arc::new(FakeBackend::default());
    *backend.kind.lock().unwrap() = Some(RunKind::Agent {
        project_path: "".into(),
    });
    let provider = FakeProvider {
        tools: Mutex::new(vec![vec![Ok(ToolStreamChunk::Response(ChatResponse::ToolCalls(vec![ToolCall {
            id: "p".into(), name: "run_command".into(),
            arguments: serde_json::json!({"command": "touch started; sleep 30; touch marker", "timeout_seconds": 60}).to_string(),
        }])) )]]), ..Default::default()
    };
    let registry = RunRegistry::default();
    let run = registry
        .start(&user(1), start("r"), dir.path(), backend, |_| provider)
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let Some(RunEvent::PermissionRequest { id, .. }) = events(&run)
                .iter()
                .find(|e| matches!(e, RunEvent::PermissionRequest { .. }))
            {
                run.gate.decide(id, true).unwrap();
                break;
            }
            tokio::task::yield_now().await;
        }
        while !dir.path().join("started").exists() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let started = Instant::now();
    registry.get(&user(1), "r").unwrap().cancel.cancel();
    finished(&run).await;
    assert!(started.elapsed() < Duration::from_secs(2));
    let emitted = events(&run);
    assert!(
        matches!(&emitted[emitted.len()-2], RunEvent::ToolResult { ok: false, summary, .. } if summary == "cancelled")
    );
    assert_eq!(emitted.last(), Some(&RunEvent::Cancelled));
    tokio::time::sleep(Duration::from_secs(31)).await;
    assert!(!dir.path().join("marker").exists());
}

#[tokio::test]
async fn reasoning_is_streamed_and_persisted_per_agent_turn() {
    let backend = FakeBackend::default();
    let run = Run::new("r".into(), 1, 1, 4096);
    let answer = format!("answer{}", openwebide_core::REPLY_CUT_OFF_MARKER);
    map_agent_events(
        &run,
        &backend,
        7,
        stream::iter([
            AgentEvent::ReasoningDelta("first".into()),
            AgentEvent::TurnCalls {
                text: "checking".into(),
                calls: vec![],
            },
            AgentEvent::ReasoningDelta("r".into()),
            AgentEvent::FinalText(answer.clone()),
        ]),
    )
    .await;
    let messages = backend.messages.lock().unwrap();
    assert_eq!(messages[0].content, "<think>first</think>checking");
    assert_eq!(messages[1].content, format!("<think>r</think>{answer}"));
    assert!(matches!(&events(&run)[0], RunEvent::ReasoningDelta { content } if content == "first"));
}

#[tokio::test]
async fn incomplete_reasoning_only_chat_is_persisted() {
    let dir = tempfile::tempdir().unwrap();
    let registry = RunRegistry::default();
    let backend = Arc::new(FakeBackend::default());
    let reasoning = "analysis with </think> literal";
    let run = registry
        .start(&user(1), start("r"), dir.path(), backend.clone(), |_| {
            FakeProvider {
                chat: Mutex::new(vec![
                    Ok(StreamChunk::Reasoning(reasoning.into())),
                    Err(ProviderError::Incomplete),
                ]),
                ..Default::default()
            }
        })
        .await
        .unwrap();
    finished(&run).await;
    let messages = backend.messages.lock().unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(
        messages[1].content,
        openwebide_core::with_reasoning(reasoning, REPLY_TRUNCATED_MARKER)
    );
    assert!(matches!(events(&run).last(), Some(RunEvent::Done { .. })));
}

#[derive(Default)]
struct FakeExecution {
    calls: Mutex<Vec<(String, PathBuf)>>,
}

impl crate::exec::ToolExecution for FakeExecution {
    fn run_command(
        &self,
        spec: crate::exec::SpawnSpec,
    ) -> crate::exec::ExecutionFuture<crate::exec::ExecOutput> {
        self.calls
            .lock()
            .unwrap()
            .push((spec.args.last().unwrap().clone(), spec.cwd));
        Box::pin(async {
            Ok(openwebide_core::CommandOutcome {
                exit_code: Some(0),
                stdout: "fake execution".into(),
                stderr: String::new(),
            })
        })
    }

    fn git(
        &self,
        request: crate::exec::GitRequest,
    ) -> crate::exec::ExecutionFuture<crate::exec::GitResponse> {
        assert!(matches!(
            request.operation,
            crate::exec::GitOperation::Status
        ));
        self.calls
            .lock()
            .unwrap()
            .push(("git status".into(), request.cwd));
        Box::pin(async {
            Ok(serde_json::to_value(openwebide_core::GitRepoStatus {
                branch: "fake".into(),
                ..Default::default()
            })
            .unwrap())
        })
    }
}

#[tokio::test]
async fn http_and_agent_run_use_configured_execution() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let execution = Arc::new(FakeExecution::default());
    let mut config = crate::ServerConfig::new(root.clone(), "secret".into(), None);
    config.execution = execution.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let server_config = config.clone();
    let server = tokio::spawn(crate::run_server_until(listener, server_config, async {
        let _ = stopped.await;
    }));
    for (path, body, expected) in [
        (
            "/exec",
            r#"{"command":"touch http-marker"}"#,
            "fake execution",
        ),
        ("/git/status", r#"{"cwd":"."}"#, "fake"),
    ] {
        let mut socket = tokio::net::TcpStream::connect(addr).await.unwrap();
        let request = format!(
            "POST {path} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nAuthorization: Bearer secret\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            addr.port(),
            body.len()
        );
        socket.write_all(request.as_bytes()).await.unwrap();
        let mut response = Vec::new();
        tokio::time::timeout(Duration::from_secs(2), socket.read_to_end(&mut response))
            .await
            .unwrap()
            .unwrap();
        let response = String::from_utf8(response).unwrap();
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        assert!(response.contains(expected), "{response}");
    }
    let backend = Arc::new(FakeBackend::default());
    *backend.kind.lock().unwrap() = Some(RunKind::Agent {
        project_path: "".into(),
    });
    let provider = FakeProvider {
        tools: Mutex::new(vec![
            vec![Ok(ToolStreamChunk::Response(ChatResponse::ToolCalls(
                vec![
                    ToolCall {
                        id: "command".into(),
                        name: "run_command".into(),
                        arguments: r#"{"command":"touch run-marker"}"#.into(),
                    },
                    ToolCall {
                        id: "git".into(),
                        name: "git_status".into(),
                        arguments: "{}".into(),
                    },
                ],
            )))],
            vec![Ok(ToolStreamChunk::Response(ChatResponse::Text(
                "final".into(),
            )))],
        ]),
        ..Default::default()
    };
    let start = start("fake-run");
    let run = config.runs.reserve(&user(1), &start).unwrap();
    let run = config
        .runs
        .prepare(
            run,
            start,
            &config.workspace_root,
            backend,
            |_| provider,
            config.execution.clone(),
        )
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let Some(RunEvent::PermissionRequest { id, .. }) = events(&run)
                .iter()
                .find(|event| matches!(event, RunEvent::PermissionRequest { .. }))
            {
                run.gate.decide(id, true).unwrap();
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    finished(&run).await;
    assert!(matches!(events(&run).last(), Some(RunEvent::Done { .. })));
    assert_eq!(
        events(&run)
            .iter()
            .filter(|event| matches!(event, RunEvent::ToolResult { ok: true, .. }))
            .count(),
        2
    );
    assert_eq!(
        *execution.calls.lock().unwrap(),
        [
            ("touch http-marker".into(), root.clone()),
            ("git status".into(), root.clone()),
            ("touch run-marker".into(), root.clone()),
            ("git status".into(), root.clone()),
        ]
    );
    assert!(!root.join("http-marker").exists());
    assert!(!root.join("run-marker").exists());
    stop.send(()).unwrap();
    server.await.unwrap();
}
