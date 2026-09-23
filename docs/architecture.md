# Architecture

## Goals

1. **Full stack in WebAssembly.** The frontend is a Leptos WASM app; the
   backend is a Spin component (`wasm32-wasip2`). No server-side runtime
   that isn't WASM.
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

## Components

```
browser
  └── frontend (Leptos → wasm32-unknown-unknown, built by Trunk)
        │  fetch /api/...
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

`LlmProvider` is the seam:

```rust
pub trait LlmProvider: Send + Sync {
    fn kind(&self) -> ProviderKind;
    async fn list_models(&self) -> Result<Vec<ModelInfo>, ProviderError>;
    async fn chat(&self, request: &ChatRequest) -> Result<String, ProviderError>;
}
```

`OllamaProvider` and `LlamaCppProvider` implement it; `registry::Provider::for_connection`
builds the right one from a stored `Connection`. The HTTP layer (a small
`HttpClient` trait so tests can fake it) is the next milestone — for now
the methods return `ProviderError::NotImplemented`, which the API maps to
HTTP 501.

### `crates/storage`

A tiny target-agnostic `Db` trait (execute → rows + last_insert_rowid +
changes) with two backends:

- `SpinDb` — Spin's `sqlite` capability (wasm builds)
- `RusqliteDb` — rusqlite (native builds and tests)

`migrations` is a list of idempotent DDL statements re-applied at startup
(Spin has no migration runner). `Store<D: Db>` holds the typed
repositories: settings, connections, system prompts, sessions, messages.
All repository logic is tested natively against an in-memory rusqlite DB.

### `backend`

A single `#[http_service]` entry point; routing is manual on
`(method, path)`. Each request opens a fresh DB connection (Spin components
are stateless) and re-applies the idempotent schema. Handlers:

| Method & path            | Purpose                          |
| ------------------------ | -------------------------------- |
| `GET /api/health`        | liveness + version               |
| `GET/POST /api/connections` | list / create connections     |
| `PUT/DELETE /api/connections/:id` | update / delete          |
| `GET/PUT /api/settings`  | all settings / upsert one        |
| `GET/POST /api/system-prompts` | list / create              |
| `DELETE /api/system-prompts/:id` | delete                     |
| `GET /api/models?connection_id=N` | models from a provider (501 until HTTP lands) |
| `POST /api/chat`         | one-shot chat (501 until HTTP lands) |

Responses are JSON built with `serde_json` into `http::Response<String>`
(http-body implements `Body` for `String`). CORS is permissive so the
frontend can be served from a different origin during development.

### `frontend`

Leptos (CSR) app: `App` fetches `/api/health` on mount, then
`/api/connections`, and renders the shell — top bar (health dot), sidebar
(connections list), empty chat pane, status bar. The API base is
same-origin by default, overridable with `?api=<url>` for dev.

## Spin wiring (`spin.toml`)

- `spin_manifest_version = 2`
- Two HTTP triggers: `/api/...` → backend component, `/...` → static file
  server (prebuilt `spin_static_fs.wasm`) serving `frontend/dist`
- Backend declares `sqlite_databases = ["default"]` and
  `allowed_outbound_hosts` for localhost (where Ollama/llama.cpp run)
- Build commands: `cargo build -p openwebide-backend --target
  wasm32-wasip2 --release` and `cd frontend && trunk build --release`
  (Trunk 0.21 is run from the frontend directory; the `data-trunk` rust
  link's `href` is the manifest path)

## Container deployment

A multi-stage `Dockerfile` makes the whole app one image: the builder stage
is the Spin image plus Rust and Trunk, and runs `spin build` (so the
`spin.toml` build commands stay the single source of truth); the runtime
stage copies `spin.toml`, the backend WASM, and `frontend/dist` into a fresh
Spin image. One port (3000) serves frontend and API; SQLite lives in
`/app/.spin` (a `VOLUME`, backed by a named volume in `docker-compose.yml`).

`docs/podman-quadlet.md` documents running the same image as a systemd
service on Linux via Podman quadlet (`.image` + `.container` units).

## Workspace: local and remote modes

The IDE operates on a project folder. Where that folder lives defines two
modes; the UI is mode-agnostic, sitting behind a `Workspace` trait in the
frontend with one impl per mode.

**Remote mode** — the folder lives on the machine running Open WebIDE / the host:

- Operates on repositories on the host machine, accessed either via Spin's mounted
  filesystem or dynamically via the host bridge Unix socket (`~/.openwebide/bridge.sock`).
- Dynamic `~/` Access: Instead of being confined to a single static folder, Remote mode
  allows opening, browsing, and switching between any repository under the user's home
  directory (`~/source/...`, `~/projects/...`) in multi-project tabs.
- The backend exposes a file API (`/api/files`: list, read, write, search) and git API,
  and the frontend calls it like any other REST/WebSocket endpoint.
- The agent edits project files and executes commands directly in the native host
  directory with full access to host Git identity and toolchains.
- The LLM also runs on the host (outbound HTTP to localhost is already permitted).

**Local mode** — the folder is on the machine running the browser:

- File System Access API (`window.showDirectoryPicker()`) via web-sys /
  `wasm_bindgen` interop; the directory handle is persisted in IndexedDB and
  re-authorized on reload
- File operations stay entirely in the browser; the backend is used only
  for settings, connections, and system prompts
- LLM connections point at `http://localhost:11434` and the *frontend*
  calls the provider directly via gloo-net (mirroring the backend's
  `HttpClient`); Ollama needs `OLLAMA_ORIGINS` set to the page's origin

Constraints & Device Roles:

- **Desktop + Mobile Workflow (Shared Sessions):** Remote mode is not just for
  remote servers. When running Open WebIDE on a workstation alongside local LLMs,
  using Remote mode on your desktop (`http://localhost:3000`) and on your phone or
  tablet (`http://workstation:3000` via LAN/Tailscale) means both devices view the
  exact same projects and chat sessions stored in backend SQLite. You can prompt
  the agent at your desk, walk away, and monitor streaming tool steps and review
  diffs on your mobile device without any session desynchronization.
- **Local Mode Role:** Local mode is specifically designed for accessing an
  Open WebIDE deployment from a laptop with private, on-disk repositories that you
  do not want to mount or upload to the host.
- The File System Access API is Chromium-only (Chrome/Edge). Mobile browsers
  (iOS Safari, Android Chrome) do not support directory picking, making mobile
  devices natural Remote-mode control clients.
- Keep the modes coherent: remote = files + LLM on the host, local = files +
  LLM on the laptop. A mixed split (remote files, local LLM) is deferred.

### Virtual File System (VFS) & process execution

To keep the agent completely decoupled from the underlying storage mechanism, file
tools operate against a unified `Vfs` abstraction (Phase 10). Whether backed by
Spin's mounted filesystem on the host or browser directory handles in local mode,
the agent interacts with standard POSIX paths without mode-specific branching.

For process execution and terminal access (Phase 11), a thin native WebSocket
bridge handles PTY sessions and command execution outside Spin's WASI sandbox,
providing execution capabilities (`cargo test`, interactive shell) to both the
agent and the user.

## Roadmap

What's left — grouped as Now / Next / Later — lives in
[roadmap.md](roadmap.md); finished work (scaffold through the TUI telemetry
meters and per-connection context limit) is in
[CHANGELOG.md](../CHANGELOG.md).

