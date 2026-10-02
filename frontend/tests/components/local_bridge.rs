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
