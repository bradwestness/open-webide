# Architecture

## Goals

1. **Full stack in WebAssembly.** The frontend is a Leptos WASM app; the
   backend is a Spin component (`wasm32-wasip2`). A native Rust bridge
   runs beside Spin for terminals, host tools, and chat/agent execution.
2. **Local-LLM first.** Ollama and llama.cpp sit behind one provider
   interface so the UI never cares which engine is serving.
3. **File-based persistence.** SQLite, the same shape Open WebUI uses,
   storing preferences, LLM connections, settings, and system prompts.
4. **Single shared core, thin boundary shims.** Avoid dual implementations.
   Everything above the I/O boundary (the agent loop, VFS tool execution,
   editor surface, diff computation, and in-memory WASM linting) is 100% shared
   code. Local and Remote modes are strictly thin shims bridging the physical
   I/O boundary (Spin REST vs. browser File System Access handles), ensuring
   features never drift between modes.

## Shared feature boundaries

UI actions, slash commands, and background refresh use the same feature entry points:

| Feature | Shared behavior | Host primitives |
| --- | --- | --- |
| Files and search | `Workspace`, core `Vfs`, shared path validation and ordering | Browser `BrowserFsaVfs`, backend `HostFsVfs`, bridge `NativeFsVfs` |
| Git and terminals | `ProjectGit` / `ProjectHost`, shared Git types and bridge execution | Backend REST or authenticated browser bridge transport |
| Chat and agent runs | `ProjectRuns`, agent `session::plan`, `chat_events`, `events`, agent loop and approval policy | Browser, Spin and bridge persistence, cancellation, permission and HTTP adapters |
| Model requests | `ModelRuntime::apply_to`, provider wire messages and stream state machines | Protocol parsers and browser/Spin/native HTTP clients |
| History recovery | Core interrupted-run validation and tool-history reconstruction | Persisted messages and tool results, UI presentation mapping |

Mode checks belong to selecting these adapters and browser folder permissions. New
features extend the shared entry point; adapters supply operations, never a second
workflow. Cancellation, fallback, result shaping and limits stay above the adapters.
Browser futures retain checked `SendWrapper` ownership. Asynchronous UI updates must
validate the originating account, project/session and host revision before applying.
The filesystem creation contract runs against memory, native and browser adapters;
provider contracts run the same success and failure cases against both protocols.

## Components

```
browser
  └── frontend (Leptos → wasm32-unknown-unknown, built by Trunk)
        │  REST + SSE /api/...
        ├── shared WebSocket → native bridge → model HTTP + host tools
        │                      └── REST → backend (secret + acting user)
        ▼
  Spin (wasm32-wasip2)
        ├── static file server component  → serves frontend/dist at /
        └── backend component (this repo) → /api/...
              ├── spin-sdk http  (request/response)
              ├── spin-sdk sqlite (the "default" database)
              └── outbound HTTP → localhost (Ollama / llama.cpp)
```

### `crates/core`

Plain data types shared by every other crate: `Connection` (with an optional
per-connection `context_limit` override for the model's context window),
`ChatSession`, `ChatMessage`, `SystemPrompt`, `ModelInfo`, `ChatRequest`,
`Health`, and the enums `ProviderKind` / `Role`. Also `tui::TurnTelemetry` /
`SessionTelemetry`, the token/speed/context-window accounting behind the TUI
statusline. All `serde`-serializable; no I/O.

### `crates/llm`

`LlmProvider` provides model listing, plain chat, text streaming, tool-call chat,
streamed tool-call turns, and context-window discovery. `OllamaProvider` uses
Ollama's native API; `LlamaCppProvider` uses its OpenAI-compatible API.
`registry::Provider::for_connection` selects the provider for a stored connection.
A shared `HttpClient` boundary has Spin, native bridge, and test implementations.
Streams require a provider completion marker; an unexpected EOF is incomplete.
Older servers that reject streamed tool calls fall back to non-streaming turns,
and the connection remembers that capability until its URL or provider kind changes.

### `crates/storage`

A tiny target-agnostic `Db` trait (execute → rows + last_insert_rowid +
changes) with two backends:

- `SpinDb` — Spin's `sqlite` capability (wasm builds)
- `RusqliteDb` — rusqlite (native builds and tests)

`migrations` applies version-gated steps (Spin has no migration runner):
`PRAGMA user_version` is compared against `SCHEMA_VERSION`, and each numbered
step in `apply_step` runs once while staying idempotent, so a pre-versioning
database (version 0) replays every step. A database newer than the build
refuses to start. `Store<D: Db>` holds the typed
repositories: settings, connections, system prompts, sessions, messages.
All repository logic is tested natively against an in-memory rusqlite DB.

### `backend`

A single `#[http_service]` entry point dispatches through a segment router, matching
HTTP methods and path slices. Handlers live in `backend/src/api/{auth,connections,
prompts,settings,projects,files,git,sessions,chat,web,bridge}.rs`; query parameters
are percent-decoded once per request. The router authenticates once and passes
`AuthedUser` to protected handlers. Storage and backend use the transparent
`UserId` type; bridge wire IDs remain integers. Typed filesystem, bridge, and
storage errors convert explicitly to `ApiError`; 500 responses say "internal error"
and log the route and full detail. Each request opens a fresh DB connection (Spin components
are stateless) and checks the schema version (migrations are version-gated,
so an up-to-date schema costs one read). Handlers:

| Method & path            | Purpose                          |
| ------------------------ | -------------------------------- |
| `GET /api/health`        | liveness + version               |
| `GET/POST /api/connections` | list / create connections     |
| `PUT/DELETE /api/connections/:id` | update / delete          |
| `GET/PUT /api/settings`  | all settings / upsert one        |
| `GET/POST /api/system-prompts` | list / create              |
| `DELETE /api/system-prompts/:id` | delete                     |
| `GET /api/models?connection_id=N` | models from the configured provider |
| `POST /api/chat`         | one-shot provider chat |

JSON and SSE responses share `http::Response<BoxBody>`. Cookie-authenticated API requests require `x-openwebide: 1` for CSRF protection.
Session cookies are HttpOnly and SameSite=Strict (Secure on HTTPS); logout rolls the
user epoch to invalidate sessions on every device. Failed logins trigger a lockout.
Registration closes after the first account. Development CORS
allows the frontend on `localhost:8080` and `127.0.0.1:8080`.

### `frontend`

Leptos (CSR) app. `App` creates the feature stores, provides them through Leptos context, checks
the session, and renders the layout shell. The stores in `frontend/src/state/` own UI notices and
dialogs, panel layout, settings and authentication, projects and per-project workspaces, Git state,
and chat runs. Components read the matching store from context instead of receiving app-wide
signal bundles through every intermediate component. Browser and backend actions stay in the
WASM-only frontend binary; the signals and pure transitions are also available to native tests.

The API base is same-origin. `?api=<url>` is a development override accepted only
on `localhost:8080` or `127.0.0.1:8080`. Preferences, theme, prompt history, and layout
live in user-scoped database settings. IndexedDB holds only directory handles and
the local bridge pairing token; legacy localStorage entries are removed on import.

## Spin wiring (`spin.toml`)

- `spin_manifest_version = 2`
- Two HTTP triggers: `/api/...` → backend component, `/...` → static file
  server (prebuilt `spin_static_fs.wasm`) serving `frontend/dist`
- Backend declares `sqlite_databases = ["default"]` and
  `allowed_outbound_hosts` for localhost and tunnels, plus unchanged HTTP/HTTPS
  wildcards for LAN model servers, web search, and documentation fetching
- Backend mounts the host workspace directory via the `files` array (requires
  `--direct-mounts --allow-transient-write` so edits hit the real disk, otherwise
  they go to a temporary copy)
- Build commands: `cargo build -p openwebide-backend --target
  wasm32-wasip2 --release` and `cd frontend && trunk build --release`
  (Trunk 0.21 is run from the frontend directory; the `data-trunk` rust
  link's `href` is the manifest path)

## Container deployment

A multi-stage `Dockerfile` makes the whole app one image: the builder stage
is the Spin image plus Rust and Trunk, and runs `spin build` (so the
`spin.toml` build commands stay the single source of truth); the runtime
stage copies `spin.toml`, the backend WASM, `frontend/dist`, and the native bridge
into a fresh Spin image. Port 3000 serves frontend and API; port 3001 serves the
bridge. SQLite and the bridge secret live in `/app/.spin`; the host workspace is
mounted at `/workspace`. A bash supervisor stops both children if either exits and
forwards shutdown signals. A non-flag command override runs directly.
`OPENWEBIDE_BRIDGE=0` disables the bridge; chat uses SSE. Old container project
paths are migrated automatically to the `/workspace` mount.

`docs/podman-quadlet.md` documents running the same image as a systemd
service on Linux via Podman quadlet (`.image` + `.container` units).

## Workspace: local and remote modes

The IDE operates on a project folder. Where that folder lives defines two
modes; the UI is mode-agnostic, sitting behind a `Workspace` trait in the
frontend with one impl per mode.

**Remote mode** — remote = a project on the device hosting Open WebIDE:

- Operates on repositories inside Spin's configured filesystem mount. The bridge
  uses the same workspace root for commands and Git. Multi-project tabs switch
  between folders under that root. Unix-domain sockets are not implemented.
- The backend exposes a file API (`/api/files`: list, read, write, search) and git API,
  and the frontend calls it like any other REST/WebSocket endpoint.
- The agent edits project files and executes commands directly in the native host
  directory with full access to host Git identity and toolchains.
- The LLM also runs on the host (outbound HTTP to localhost is already permitted).

**Local mode** — local = a project on this device (the browser's File System Access API):

- File System Access API (`window.showDirectoryPicker()`) via web-sys /
  `wasm_bindgen` interop; the directory handle is persisted in IndexedDB and
  re-authorized on reload
- File operations and the agent loop stay in the browser. The backend persists
  settings, sessions, messages, and tool steps. Model completions stream through
  the companion bridge, falling back to backend `/api/chat-tools`.
- A temporary probe file discovers the picked folder under the bridge root (up
  to five levels), verifies it at each run, and is removed afterwards. Command
  and Git tools are hidden if the bridge cannot see it. Interrupted local runs
  can resume from persisted history without another user message.

Constraints & Device Roles:

- **Desktop + Mobile Workflow (Shared Sessions):** Remote mode is not just for
  remote servers. When running Open WebIDE on a workstation alongside local LLMs,
  using Remote mode on your desktop (`http://localhost:3000`) and on your phone or
  tablet (`http://workstation:3000` via LAN/Tailscale) means both devices view the
  exact same projects and chat sessions stored in backend SQLite. You can prompt
  the agent at your desk, walk away, and monitor streaming tool steps and review
  diffs on your mobile device without any session desynchronization.
- **Local Mode Role:** local = a project on this device (the browser's File System Access API).
  Access an Open WebIDE deployment with private, on-disk repositories that you
  do not want to mount or upload to the host.
- The File System Access API is Chromium-only (Chrome/Edge). Mobile browsers
  (iOS Safari, Android Chrome) do not support directory picking, making mobile
  devices natural Remote-mode control clients.
- Keep the modes coherent: remote = a project on the device hosting Open WebIDE;
  local = a project on this device (the browser's File System Access API).
  The LLM runs on the hosting device in remote mode and on this device in local mode.
  A mixed split (remote files, local LLM) is deferred.

### Chat execution and streaming

The frontend shares one authenticated WebSocket connection for terminals, runs, and completions.
With `hello_ok.runs = true`, remote chat and agent loops execute in the native bridge. The bridge
loads plans and persists messages and tool steps through backend REST using its shared secret
`S` and the hello-verified user ID in `x-openwebide-user`; it never forwards the hello token.
Browser local-mode agent loops still execute against the browser's directory handle, while model
completions stream over the bridge. Without run support, chat executes in the backend over SSE
and local-mode completions use `/api/chat-tools`.

Chat and agent streams share core's `RunEvent` type across SSE, WebSocket runs, and browser
local-mode execution. SSE frames use `event: <kind>` and `data: <tagged RunEvent JSON>`, followed
by a blank line. For example, `event: delta` carries `{"kind":"delta","content":"hi"}`;
`reasoning_delta` carries separate model reasoning in `content`. It is persisted as a leading
`<think>…</think>` block, stripped from assistant history sent back to the model. Length stops
append a plain-text reply-cutoff marker. Message and done events wrap the persisted message in `message`. The frontend parses the data's
`kind` tag. Frontend and backend must be deployed together for this frame format.

Sending waits up to about two seconds for a connecting bridge. Unavailable or unauthorized
bridges fall back silently. A project unavailable to the bridge shows a notice and falls back;
busy or failed run plans show an error. Stops and approvals address the active WS run. History
loads discover running bridge runs and merge snapshots by message and step IDs, including pending
approvals and live text. Socket reconnects attach using the last received sequence; an unknown
run clears streaming, reloads history, and adds an interrupted-run notice if no reply was saved.
Runs survive socket disconnects; a daemon restart loses its in-memory run registry.

### Virtual File System (VFS) & process execution

To keep the agent completely decoupled from the underlying storage mechanism, file
tools operate against a unified `Vfs` abstraction (Phase 10). Whether backed by
Spin's mounted filesystem on the host or browser directory handles in local mode,
the agent interacts with standard POSIX paths without mode-specific branching.

For process execution and terminal access (Phase 11), a thin native WebSocket
bridge handles PTY sessions and command execution outside Spin's WASI sandbox,
providing execution capabilities (`cargo test`, interactive shell) to both the
agent and the user. Origin-less HTTP tool requests require the backend/bridge shared secret. Browser
HTTP tool requests and WebSocket hello use a short-lived HMAC bridge token or
local pairing token. Host/Origin allowlists reject foreign origins and DNS
rebinding; browser tool POSTs require JSON. `POST /secret` bootstraps only on
loopback. Remote bridges use `SPIN_VARIABLE_BRIDGE_URL` and
`SPIN_VARIABLE_BRIDGE_SECRET`. Working directories are confined lexically under
the canonical workspace root; directory symlinks are permitted. Agent file tools
refuse `.spin/` and `.git` writes. Shell commands still have host access and
require approval; confinement is not an OS sandbox.

The native bridge separates `server/` (HTTP parsing, routes, WebSocket connections),
`terminals/` (PTY/headless sessions and replay rings), `exec/` (command and Git execution),
and `runs/` (agent/chat runs, completions, backend clients, native VFS). `ServerConfig`
holds an `Arc<dyn ToolExecution>` shared by `/exec`, `/git/*`, and the in-process agent
bridge client. `HostExecution` supplies today's host behavior; a sandbox can implement
the same object-safe trait. `SpawnSpec` carries command, arguments, cwd, environment,
timeout, and cancellation; interactive terminals use it but stay outside the executor
trait. `BridgeError` maps typed failures to HTTP status codes. Bridge logging uses
`tracing`, with connection/run spans and `RUST_LOG` filtering (default `info`).

## Roadmap

What's left — grouped as Next / Later — lives in
[roadmap.md](roadmap.md); finished work, including hardening and streaming, is in
[CHANGELOG.md](../CHANGELOG.md).


## Agent turn streaming

Server-side agent runs stream model text token by token over SSE, alongside
telemetry and tool steps. Text preceding tool calls is persisted as an interim
assistant message with wire tool calls and emitted as an `interim` event; tool steps follow that message
in the conversation. The same text is included in the assistant tool-call message
sent back to the model. Tool-call ids use the initiating user message id throughout
the run, while the display anchor moves to each persisted interim message.
History reconstructs tool replies from anchored step summaries; incomplete row sets
fall back to plain text. Empty interim messages carrying calls are hidden in the UI.
Local-mode runs use the same loop, persist interim text and calls, and stream
model turns over bridge completions when available. The `/api/chat-tools` fallback
delivers a complete turn.

`POST /api/sessions/<id>/run-plan` prepares a run without persisting a message or
clearing run flags. It checks session ownership and returns the connection, chat
request with prior history and temporal context, editor-prefixed user content, and
run kind (`chat` or `agent` with a project path). The SSE message handler uses the
same builder before persisting the new user message and starting the run.
