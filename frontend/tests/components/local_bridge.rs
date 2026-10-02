use leptos::prelude::*;
use std::sync::{Arc, Mutex};

use openwebide_agent::{BridgeClient, policy::BRIDGE_TOOLS};
use openwebide_core::{CommandOutcome, Vfs};
use openwebide_frontend::{
    bridge::{BridgeConfig, BridgeCredentials},
    local_agent::{
        BRIDGE_FOLDER_NOTICE, BrowserBridgeClient, local_tools, resolve_bridge_cwd_with,
    },
    local_fs::BrowserFsaVfs,
    util::sleep_ms,
};
use wasm_bindgen::{JsCast, prelude::*};
use wasm_bindgen_test::*;

use super::support::{chat_view, mount_test, settle};

#[wasm_bindgen(inline_js = r#"
export function probeFolder() {
    const files = new Map();
    const deleted = [];
    return {
        files, deleted,
        queryPermission: async () => 'granted',
        getFileHandle: async (name, options) => {
            if (!files.has(name) && !options?.create) throw new DOMException('missing', 'NotFoundError');
            files.set(name, files.get(name) || '');
            return Object.assign(Object.create(FileSystemFileHandle.prototype), { getFile: async () => new File([files.get(name)], name), createWritable: async () => Object.assign(Object.create(FileSystemWritableFileStream.prototype), {
                write: async value => { files.set(name, value); },
                close: async () => {}
            }) });
        },
        removeEntry: async name => { deleted.push(name); files.delete(name); }
    };
}
export function fakeBridgeHttp() {
    const original = window.fetch;
    const mock = { found: true, invalid: false, hanging: false, aborted: 0, calls: [], restore: () => { window.fetch = original; } };
    window.fetch = async request => {
        if (!request.url.startsWith('http://bridge.test:3001/')) return original(request);
        const body = JSON.parse(await request.text());
        const path = new URL(request.url).pathname;
        mock.calls.push({ path, body, authorization: request.headers.get('Authorization') });
        if (mock.hanging) return new Promise((resolve, reject) => {
            const abort = () => {
                mock.aborted++;
                reject(new DOMException('aborted', 'AbortError'));
            };
            if (request.signal.aborted) abort();
            else request.signal.addEventListener('abort', abort, { once: true });
        });
        if (mock.invalid) return new Response(JSON.stringify({ error: 'cwd does not exist: repos/x' }), { status: 400 });
        if (body.branch === 'fix-cwd..mapping') return new Response(JSON.stringify({ error: `invalid branch name '${body.branch}': invalid ref` }), { status: 400 });
        const probe = body.command?.match(/\.openwebide-probe-[0-9a-f]+/)?.[0];
        const stdout = body.command?.startsWith('find ') && mock.found ? `./repos/x/${probe}\n` : '';
        return new Response(JSON.stringify({ exit_code: mock.found ? 0 : 1, stdout, stderr: '' }));
    };
    return mock;
}
export function restoreBridgeHttp(mock) { mock.restore(); }
export function bridgeCalls(mock) { return JSON.stringify(mock.calls); }
export function bridgeFound(mock, found) { mock.found = found; }
export function bridgeHanging(mock, hanging) { mock.hanging = hanging; }
export function bridgeAborted(mock) { return mock.aborted; }
export function bridgeInvalid(mock) { mock.invalid = true; }
export function denyFolder(folder) {
    folder.getFileHandle = async () => { throw new DOMException('denied', 'NotAllowedError'); };
}
export function folderEmpty(folder) { return folder.files.size === 0 && folder.deleted.length > 0; }
export function probeDeleted(folder, name) {
    return folder.deleted.includes(name) && !folder.files.has(name);
}
"#)]
extern "C" {
    #[wasm_bindgen(js_name = fakeBridgeHttp)]
    fn fake_bridge_http() -> JsValue;
    #[wasm_bindgen(js_name = restoreBridgeHttp)]
    fn restore_bridge_http(mock: &JsValue);
    #[wasm_bindgen(js_name = bridgeCalls)]
    fn bridge_calls(mock: &JsValue) -> String;
    #[wasm_bindgen(js_name = bridgeFound)]
    fn bridge_found(mock: &JsValue, found: bool);
    #[wasm_bindgen(js_name = bridgeHanging)]
    fn bridge_hanging(mock: &JsValue, hanging: bool);
    #[wasm_bindgen(js_name = bridgeAborted)]
    fn bridge_aborted(mock: &JsValue) -> u32;
    #[wasm_bindgen(js_name = bridgeInvalid)]
    fn bridge_invalid(mock: &JsValue);
    #[wasm_bindgen(js_name = denyFolder)]
    fn deny_folder(folder: &JsValue);
    #[wasm_bindgen(js_name = folderEmpty)]
    fn folder_empty(folder: &JsValue) -> bool;
    #[wasm_bindgen(js_name = probeFolder)]
    fn probe_folder() -> JsValue;
    #[wasm_bindgen(js_name = probeDeleted)]
    fn probe_deleted(folder: &JsValue, name: &str) -> bool;
}

#[wasm_bindgen_test]
async fn local_vfs_clones_read_write_and_drop_unpolled_operations() {
    let folder = probe_folder();
    let vfs = BrowserFsaVfs::new(folder.unchecked_into());
    let clone = vfs.clone();
    vfs.write("note.txt", "first λ").await.unwrap();
    assert_eq!(clone.read("note.txt").await.unwrap(), "first λ");
    drop(clone.write("note.txt", "must not run"));
    assert_eq!(vfs.read("note.txt").await.unwrap(), "first λ");
    clone.write("note.txt", "latest").await.unwrap();
    drop(clone);
    assert_eq!(vfs.read("note.txt").await.unwrap(), "latest");
    vfs.delete("note.txt").await.unwrap();
    assert!(matches!(
        vfs.read("note.txt").await,
        Err(openwebide_core::VfsError::NotFound(_))
    ));
}

#[derive(Clone)]
struct FakeBridgeHttp {
    cwd: String,
    replies: Arc<Mutex<std::collections::VecDeque<Result<CommandOutcome, String>>>>,
    calls: Arc<Mutex<Vec<(String, String, u64)>>>,
}

impl BridgeClient for FakeBridgeHttp {
    async fn execute_command(&self, command: &str, timeout: u64) -> Result<CommandOutcome, String> {
        self.calls
            .lock()
            .unwrap()
            .push((self.cwd.clone(), command.into(), timeout));
        self.replies
            .lock()
            .unwrap()
            .pop_front()
            .expect("scripted probe response")
    }
}

fn bridge(replies: Vec<Result<CommandOutcome, String>>) -> FakeBridgeHttp {
    FakeBridgeHttp {
        cwd: String::new(),
        replies: Arc::new(Mutex::new(replies.into())),
        calls: Arc::new(Mutex::new(Vec::new())),
    }
}

fn outcome(stdout: &str, exit_code: i32) -> CommandOutcome {
    CommandOutcome {
        exit_code: Some(exit_code),
        stdout: stdout.into(),
        stderr: String::new(),
    }
}

#[wasm_bindgen_test]
async fn discovery_with_traversal_error_is_saved_and_enables_commands() {
    let mounted = mount_test(|state| {
        state.seed_session();
        chat_view(state)
    });
    let folder = probe_folder();
    let vfs = BrowserFsaVfs::new(folder.clone().unchecked_into());
    let http = bridge(vec![Ok(outcome("./repos/x/.openwebide-probe-ab12\n", 1))]);
    let cwd = resolve_bridge_cwd_with(mounted.state.api, &vfs, 12, "ab12", |cwd| FakeBridgeHttp {
        cwd,
        ..http.clone()
    })
    .await;
    assert_eq!(cwd.as_deref(), Some("repos/x"));
    assert_eq!(
        mounted
            .state
            .fake
            .settings
            .borrow()
            .get("local_bridge_cwd.12")
            .map(String::as_str),
        Some("repos/x")
    );
    assert!(
        local_tools(cwd.as_deref())
            .iter()
            .any(|tool| tool.name == "run_command")
    );
    assert!(probe_deleted(&folder, ".openwebide-probe-ab12"));
    let calls = http.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "");
    assert!(calls[0].1.starts_with("find . -maxdepth 5"));
    assert_eq!(calls[0].2, 10);
}

#[wasm_bindgen_test]
async fn missing_folder_hides_bridge_tools_and_notifies_once() {
    let mounted = mount_test(|state| {
        state.seed_session();
        chat_view(state)
    });
    for reply in [Ok(outcome("", 0)), Err("bridge unreachable".into())] {
        let folder = probe_folder();
        let vfs = BrowserFsaVfs::new(folder.clone().unchecked_into());
        let http = bridge(vec![reply]);
        let cwd =
            resolve_bridge_cwd_with(mounted.state.api, &vfs, 12, "ab12", |cwd| FakeBridgeHttp {
                cwd,
                ..http.clone()
            })
            .await;
        assert_eq!(cwd, None);
        assert!(
            local_tools(cwd.as_deref())
                .iter()
                .all(|tool| !BRIDGE_TOOLS.contains(&tool.name.as_str()))
        );
        assert!(
            local_tools(cwd.as_deref())
                .iter()
                .any(|tool| tool.name == "read_file")
        );
        mounted.state.chat.notify_bridge_folder_once(1);
        assert!(probe_deleted(&folder, ".openwebide-probe-ab12"));
    }
    settle().await;
    let text = mounted.root.text_content().unwrap();
    assert_eq!(text.matches("Command and git tools are off").count(), 1);
    assert!(text.contains("bridge can't see this folder"));
    assert_eq!(mounted.state.chat.messages.get_untracked().len(), 1);
    assert!(BRIDGE_FOLDER_NOTICE.contains("--workspace"));
}

#[wasm_bindgen_test]
async fn candidate_is_verified_each_run_and_rediscovered_when_stale() {
    let mounted = mount_test(|state| {
        state.seed_session();
        chat_view(state)
    });
    mounted
        .state
        .fake
        .settings
        .borrow_mut()
        .insert("local_bridge_cwd.12".into(), "repos/x".into());
    let folder = probe_folder();
    let vfs = BrowserFsaVfs::new(folder.clone().unchecked_into());
    let http = bridge(vec![
        Ok(outcome("", 0)),
        Ok(outcome("", 1)),
        Ok(outcome("./.openwebide-probe-ab12", 0)),
    ]);
    let first =
        resolve_bridge_cwd_with(mounted.state.api, &vfs, 12, "ab12", |cwd| FakeBridgeHttp {
            cwd,
            ..http.clone()
        })
        .await;
    assert_eq!(first.as_deref(), Some("repos/x"));
    let second =
        resolve_bridge_cwd_with(mounted.state.api, &vfs, 12, "ab12", |cwd| FakeBridgeHttp {
            cwd,
            ..http.clone()
        })
        .await;
    assert_eq!(second.as_deref(), Some(""));
    assert_eq!(
        mounted
            .state
            .fake
            .settings
            .borrow()
            .get("local_bridge_cwd.12")
            .map(String::as_str),
        Some("")
    );
    let calls = http.calls.lock().unwrap();
    assert_eq!(calls.len(), 3);
    assert_eq!(
        calls[0],
        ("repos/x".into(), "test -f .openwebide-probe-ab12".into(), 5)
    );
    assert_eq!(calls[1], calls[0]);
    assert_eq!(calls[2].0, "");
    assert!(probe_deleted(&folder, ".openwebide-probe-ab12"));
}

struct HttpGuard(JsValue);
impl Drop for HttpGuard {
    fn drop(&mut self) {
        restore_bridge_http(&self.0);
    }
}

#[wasm_bindgen_test]
async fn local_runs_use_discovered_tools_and_hide_them_when_bridge_cannot_see_folder() {
    let http = HttpGuard(fake_bridge_http());
    let previous = openwebide_frontend::idb::get_bridge_pairing_token()
        .await
        .unwrap();
    openwebide_frontend::idb::set_bridge_pairing_token("probe-token")
        .await
        .unwrap();
    let folder = probe_folder();
    let handle = folder.clone();
    let mounted = mount_test(move |state| {
        state.seed_project();
        state.seed_connection();
        state.seed_session();
        state
            .projects
            .projects
            .update(|projects| projects[0].mode = openwebide_core::WorkspaceMode::Local);
        state.projects.local_handles.update(|handles| {
            handles.insert(1, handle.unchecked_into());
        });
        state
            .settings
            .bridge_url
            .set("ws://bridge.test:3001".into());
        chat_view(state)
    });
    settle().await;
    for (found, hanging) in [(true, false), (false, false), (false, true), (true, false)] {
        bridge_hanging(&http.0, hanging);
        bridge_found(&http.0, found);
        mounted
            .state
            .fake
            .scripted_completions
            .borrow_mut()
            .push_back(openwebide_core::ChatCompletion {
                reasoning: String::new(),
                stop_reason: openwebide_core::StopReason::Complete,
                response: openwebide_core::ChatResponse::Text("Reply".into()),
                preamble: String::new(),
                usage: None,
            });
        let before = mounted.state.fake.completion_requests.borrow().len();
        mounted.input("check this folder");
        mounted.key("Enter", "Enter", false);
        for _ in 0..4000 {
            sleep_ms(5).await;
            settle().await;
            if mounted.state.fake.completion_requests.borrow().len() > before
                && !mounted.state.chat.streaming.get_untracked()
            {
                break;
            }
        }
        let requests = mounted.state.fake.completion_requests.borrow();
        assert_eq!(requests.len(), before + 1);
        let tools = &requests.last().unwrap().tools;
        assert_eq!(tools.iter().any(|tool| tool.name == "run_command"), found);
        if !found {
            assert!(
                tools
                    .iter()
                    .all(|tool| !BRIDGE_TOOLS.contains(&tool.name.as_str()))
            );
        }
        assert!(folder_empty(&folder));
        if hanging {
            assert_eq!(bridge_aborted(&http.0), 2);
        }
    }
    assert_eq!(
        mounted
            .state
            .fake
            .settings
            .borrow()
            .get("local_bridge_cwd.1")
            .map(String::as_str),
        Some("repos/x")
    );
    assert_eq!(
        mounted
            .root
            .text_content()
            .unwrap()
            .matches("Command and git tools are off")
            .count(),
        1
    );
    restore_bridge_http(&http.0);
    let http = HttpGuard(fake_bridge_http());
    let credentials = BridgeCredentials::new(mounted.state.api);
    for cwd in ["repos/x", ""] {
        let client = BrowserBridgeClient::for_project(
            BridgeConfig::new("ws://bridge.test:3001").http_url,
            cwd.into(),
            credentials.clone(),
        );
        client.execute_command("pwd", 5).await.unwrap();
        let _ = client.git_status().await;
        let _ = client.git_diff(Some("file.rs")).await;
        let _ = client
            .git_commit(&openwebide_core::GitCommitRequest {
                message: "commit".into(),
                paths: None,
                include_untracked: false,
            })
            .await;
        let _ = client
            .git_checkout(&openwebide_core::GitCheckoutRequest {
                branch: "main".into(),
                create_if_missing: false,
            })
            .await;
        let calls: Vec<serde_json::Value> = serde_json::from_str(&bridge_calls(&http.0)).unwrap();
        for call in &calls[calls.len() - 5..] {
            let expected = if call["path"] == "/exec" || !cwd.is_empty() {
                cwd
            } else {
                "."
            };
            assert_eq!(call["body"]["cwd"], expected);
            assert_eq!(call["authorization"], "Bearer probe-token");
        }
    }
    let client = BrowserBridgeClient::for_project(
        "http://bridge.test:3001".into(),
        "repos/x".into(),
        credentials,
    );
    let error = client
        .git_checkout(&openwebide_core::GitCheckoutRequest {
            branch: "fix-cwd..mapping".into(),
            create_if_missing: false,
        })
        .await
        .unwrap_err();
    assert!(error.contains("invalid branch name"));
    client.execute_command("pwd", 5).await.unwrap();
    bridge_invalid(&http.0);
    assert!(
        client
            .execute_command("pwd", 5)
            .await
            .unwrap_err()
            .contains("HTTP 400")
    );
    let before = bridge_calls(&http.0);
    assert!(
        client
            .execute_command("pwd", 5)
            .await
            .unwrap_err()
            .contains("no longer verified")
    );
    assert_eq!(bridge_calls(&http.0), before);
    if let Some(token) = previous {
        openwebide_frontend::idb::set_bridge_pairing_token(&token)
            .await
            .unwrap();
    } else {
        openwebide_frontend::idb::delete_bridge_pairing_token()
            .await
            .unwrap();
    }
}

#[wasm_bindgen_test]
async fn terminal_local_spawns_resolve_cwd_and_refuse_stale_probes() {
    use openwebide_core::{BridgeClientMessage, BridgeServerMessage};
    use openwebide_frontend::{
        bridge::BridgeConn, components::TerminalPane, testing::fake_transport::FakeTransport,
    };
    use std::{cell::RefCell, rc::Rc};
    let previous = openwebide_frontend::idb::get_bridge_pairing_token()
        .await
        .unwrap();
    openwebide_frontend::idb::set_bridge_pairing_token("probe-token")
        .await
        .unwrap();
    for scenario in [
        "shell",
        "parallel",
        "empty",
        "typed",
        "test",
        "restart",
        "denied",
        "missing",
        "unresolved",
        "switch",
        "deleted",
        "logout",
        "url",
        "unmount",
    ] {
        let http = HttpGuard(fake_bridge_http());
        let folder = probe_folder();
        if scenario == "denied" {
            deny_folder(&folder);
        }
        let handle = folder.clone();
        let fake = Rc::new(FakeTransport::default());
        let transport = fake.clone();
        let slot = Rc::new(RefCell::new(None));
        let connection_slot = slot.clone();
        let auth_slot = Rc::new(RefCell::new(None));
        let auth_context = auth_slot.clone();
        let command_slot = Rc::new(RefCell::new(None));
        let command_context = command_slot.clone();
        let mounted = mount_test(move |state| {
            state.seed_project();
            state.projects.projects.update(|projects| {
                projects[0].mode = if scenario == "restart" {
                    openwebide_core::WorkspaceMode::Remote
                } else {
                    openwebide_core::WorkspaceMode::Local
                };
            });
            if scenario != "missing" {
                state.projects.local_handles.update(|handles| {
                    handles.insert(1, handle.unchecked_into());
                });
            }
            state
                .settings
                .bridge_url
                .set("ws://bridge.test:3001".into());
            *command_context.borrow_mut() = Some(
                expect_context::<openwebide_frontend::state::layout::LayoutState>().terminal_cmd,
            );
            *auth_context.borrow_mut() =
                Some(expect_context::<openwebide_frontend::state::auth::AuthState>());
            let bridge = BridgeConn::with_transport(
                BridgeConfig::new("ws://bridge.test:3001"),
                transport,
                Rc::new(|| Box::pin(async { Ok("token".into()) })),
            );
            *connection_slot.borrow_mut() = Some(bridge.clone());
            view! { <TerminalPane bridge=bridge on_close=|| () /> }
        });
        settle().await;
        fake.reply(BridgeServerMessage::HelloOk {
            user_id: Some(1),
            protocol: 1,
            runs: false,
        });
        settle().await;
        if scenario == "unresolved" {
            bridge_found(&http.0, false);
        }
        let deferred = matches!(
            scenario,
            "switch" | "deleted" | "logout" | "url" | "unmount"
        );
        let (tx, rx) = futures::channel::oneshot::channel();
        if deferred {
            mounted
                .state
                .fake
                .settings_load_results
                .borrow_mut()
                .push_back(rx);
        }
        match scenario {
            "empty" | "typed" => {
                let input = mounted
                    .root
                    .query_selector(".terminal-input")
                    .unwrap()
                    .unwrap()
                    .unchecked_into::<web_sys::HtmlInputElement>();
                input.set_value(if scenario == "typed" {
                    "echo local"
                } else {
                    ""
                });
                input
                    .dispatch_event(&web_sys::Event::new("input").unwrap())
                    .unwrap();
                mounted.click_text("Send");
            }
            "restart" => {
                mounted.click_text("+ Shell");
                let id = fake
                    .sent()
                    .into_iter()
                    .find_map(|message| match message {
                        BridgeClientMessage::Spawn { id, .. } => Some(id),
                        _ => None,
                    })
                    .unwrap();
                fake.reply(BridgeServerMessage::Spawned {
                    id: id.clone(),
                    pid: 1,
                    pty: true,
                });
                mounted
                    .state
                    .projects
                    .projects
                    .update(|projects| projects[0].mode = openwebide_core::WorkspaceMode::Local);
                fake.disconnect();
                sleep_ms(1100).await;
                fake.reply(BridgeServerMessage::HelloOk {
                    user_id: Some(1),
                    protocol: 1,
                    runs: false,
                });
                settle().await;
                fake.reply(BridgeServerMessage::Error {
                    id,
                    message: "session not found".into(),
                });
            }
            "test" => command_slot
                .borrow()
                .unwrap()
                .set(Some("cargo test".into())),
            "parallel" => {
                mounted.click_text("+ Shell");
                mounted.click_text("+ Shell");
            }
            _ => mounted.click_text("+ Shell"),
        }
        settle().await;
        match scenario {
            "switch" => mounted.state.projects.active_project.set(None),
            "deleted" => mounted.state.projects.projects.set(Vec::new()),
            "logout" => auth_slot.borrow().unwrap().logout(),
            "url" => mounted.state.settings.bridge_url.set("ws://other".into()),
            "unmount" => {
                drop(mounted);
                tx.send(Ok(Default::default())).unwrap();
                for _ in 0..200 {
                    sleep_ms(5).await;
                    settle().await;
                    if folder_empty(&folder) {
                        break;
                    }
                }
                assert!(
                    !fake
                        .sent()
                        .iter()
                        .any(|message| matches!(message, BridgeClientMessage::Spawn { .. }))
                );
                slot.borrow_mut().take().unwrap().close();
                continue;
            }
            _ => {}
        }
        if deferred {
            tx.send(Ok(Default::default())).unwrap();
        }
        for _ in 0..200 {
            sleep_ms(5).await;
            settle().await;
            if fake
                .sent()
                .iter()
                .filter(|message| matches!(message, BridgeClientMessage::Spawn { .. }))
                .count()
                > usize::from(matches!(scenario, "restart" | "parallel"))
                || (matches!(scenario, "logout" | "url") && folder_empty(&folder))
            {
                break;
            }
        }
        let frame = js_sys::Promise::new(&mut |resolve, _| {
            web_sys::window()
                .unwrap()
                .request_animation_frame(&resolve)
                .unwrap();
        });
        wasm_bindgen_futures::JsFuture::from(frame).await.unwrap();
        settle().await;
        let spawns = fake
            .sent()
            .into_iter()
            .filter_map(|message| match message {
                BridgeClientMessage::Spawn { cwd, .. } => Some(cwd),
                _ => None,
            })
            .skip(usize::from(scenario == "restart"))
            .collect::<Vec<_>>();
        if matches!(scenario, "logout" | "url") {
            assert!(spawns.is_empty(), "{scenario}");
        } else {
            let fallback = matches!(scenario, "missing" | "unresolved" | "deleted" | "denied");
            assert_eq!(
                spawns,
                vec![
                    if fallback {
                        None
                    } else {
                        Some("repos/x".into())
                    };
                    if scenario == "parallel" { 2 } else { 1 }
                ],
                "{scenario}"
            );
            assert_eq!(
                mounted
                    .root
                    .text_content()
                    .unwrap()
                    .contains("starting in the bridge workspace root"),
                fallback,
                "{scenario}"
            );
        }
        if scenario != "missing" {
            assert!(folder_empty(&folder), "{scenario}");
        }
        drop(mounted);
        slot.borrow_mut().take().unwrap().close();
    }
    if let Some(previous) = previous {
        openwebide_frontend::idb::set_bridge_pairing_token(&previous)
            .await
            .unwrap();
    } else {
        openwebide_frontend::idb::delete_bridge_pairing_token()
            .await
            .unwrap();
    }
}

#[wasm_bindgen_test]
async fn resume_after_failed_history_persistence_reuses_write_id() {
    use super::support::editor_view;
    use openwebide_core::{
        ChatCompletion, ChatResponse, EditDecision, FileDiff, PersistedEdit, StopReason, ToolCall,
        WorkspaceMode,
    };

    let http = HttpGuard(fake_bridge_http());
    bridge_found(&http.0, false);
    let folder = probe_folder();
    let vfs = BrowserFsaVfs::new(folder.clone().unchecked_into());
    vfs.write("file.rs", "original").await.unwrap();
    let mounted = mount_test(move |state| {
        state.seed_project();
        state.seed_connection();
        state.seed_session();
        state
            .projects
            .projects
            .update(|projects| projects[0].mode = WorkspaceMode::Local);
        state.projects.local_handles.update(|handles| {
            handles.insert(1, folder.unchecked_into());
        });
        state
            .settings
            .bridge_url
            .set("ws://bridge.test:3001".into());
        let mut second = state.chat.sessions.get_untracked()[0].clone();
        second.id = 2;
        state.fake.sessions.borrow_mut().push(second.clone());
        state.chat.sessions.update(|sessions| sessions.push(second));
        state
            .chat
            .set_approval_mode(1, openwebide_agent::policy::ApprovalMode::AlwaysForSession);
        state.workspace.open_file.set(Some("file.rs".into()));
        state.workspace.content.set("dirty draft".into());
        state.workspace.dirty.set(true);
        let edit = PersistedEdit {
            project_id: 1,
            path: "file.rs".into(),
            revision: 1,
            decision: EditDecision::Pending,
            diff: FileDiff {
                path: "file.rs".into(),
                old: Some("original".into()),
                new: "changed".into(),
                old_unavailable: false,
                backup_path: None,
            },
        };
        state
            .fake
            .persisted_edits
            .borrow_mut()
            .insert((1, edit.path.clone()), edit.clone());
        state.workspace.set_persisted_edits(1, vec![edit]);
        view! { {chat_view(state.clone())} {editor_view(state)} }
    });
    settle().await;
    mounted
        .state
        .fake
        .message_save_results
        .borrow_mut()
        .extend([Ok(()), Err("offline".into()), Err("offline".into())]);
    *mounted.state.fake.step_save_error.borrow_mut() = Some("offline".into());
    for content in ["first write", "resumed write"] {
        mounted
            .state
            .fake
            .scripted_completions
            .borrow_mut()
            .extend([
                ChatCompletion {
                    reasoning: String::new(),
                    stop_reason: StopReason::Complete,
                    response: ChatResponse::ToolCalls(vec![ToolCall {
                        id: "wire".into(),
                        name: "write_file".into(),
                        arguments: serde_json::json!({"path": "file.rs", "content": content})
                            .to_string(),
                    }]),
                    preamble: String::new(),
                    usage: None,
                },
                ChatCompletion {
                    reasoning: String::new(),
                    stop_reason: StopReason::Complete,
                    response: ChatResponse::Text("done".into()),
                    preamble: String::new(),
                    usage: None,
                },
            ]);
    }
    mounted.input("edit file");
    mounted.key("Enter", "Enter", false);
    for _ in 0..1000 {
        sleep_ms(5).await;
        settle().await;
        if !mounted.state.chat.streaming.get_untracked() {
            break;
        }
    }
    assert!(!mounted.state.chat.streaming.get_untracked());
    assert_eq!(vfs.read("file.rs").await.unwrap(), "first write");
    assert_eq!(
        mounted.state.workspace.agent_writes.get_untracked()[&(1, "file.rs".into())],
        1
    );
    assert!(
        mounted
            .state
            .workspace
            .counted_agent_writes
            .get_untracked()
            .contains(&(1, "a1t1c0".into()))
    );
    assert_eq!(mounted.state.fake.messages.borrow()[&1].len(), 1);
    assert!(mounted.state.fake.tool_sources.borrow().is_empty());
    assert!(mounted.state.fake.message_save_results.borrow().is_empty());
    mounted.state.chat.active_session.set(Some(2));
    settle().await;
    mounted.state.chat.active_session.set(Some(1));
    settle().await;
    let resume = mounted.state.chat.interrupted_run.get_untracked().unwrap();
    assert_eq!((resume.anchor_id, resume.first_turn), (1, 1));
    let (release, response) = futures::channel::oneshot::channel();
    mounted
        .state
        .fake
        .resolution_response_results
        .borrow_mut()
        .push_back(response);
    mounted.click_text("✓ Accept");
    settle().await;
    assert_eq!(
        mounted.state.fake.persisted_edits.borrow()[&(1, "file.rs".into())].decision,
        EditDecision::Accepted
    );
    *mounted.state.fake.step_save_error.borrow_mut() = None;
    mounted.click_text("Resume");
    for _ in 0..1000 {
        sleep_ms(5).await;
        settle().await;
        if !mounted.state.chat.streaming.get_untracked() {
            break;
        }
    }
    assert!(!mounted.state.chat.streaming.get_untracked());
    assert_eq!(vfs.read("file.rs").await.unwrap(), "resumed write");
    assert_eq!(
        mounted
            .state
            .fake
            .tool_sources
            .borrow()
            .get(&(1, "a1t1c0".into())),
        Some(&true)
    );
    assert_eq!(
        mounted.state.fake.persisted_edits.borrow()[&(1, "file.rs".into())].revision,
        2
    );
    release.send(Ok(())).unwrap();
    settle().await;
    assert_eq!(
        mounted.state.workspace.agent_writes.get_untracked()[&(1, "file.rs".into())],
        2
    );
    assert_eq!(
        mounted.state.workspace.content.get_untracked(),
        "dirty draft"
    );
    assert!(mounted.state.workspace.dirty.get_untracked());
}
