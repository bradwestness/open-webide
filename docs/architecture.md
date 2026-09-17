# Architecture

## Goals

1. **Full stack in WebAssembly.** The frontend is a Leptos WASM app; the
   backend is a Spin component (`wasm32-wasip2`). No server-side runtime
   that isn't WASM.
2. **Local-LLM first.** Ollama and llama.cpp sit behind one provider
   interface so the UI never cares which engine is serving.
3. **File-based persistence.** SQLite, the same shape Open WebUI uses,
   storing preferences, LLM connections, settings, and system prompts.

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

Plain data types shared by every other crate: `Connection`, `ChatSession`,
`ChatMessage`, `SystemPrompt`, `ModelInfo`, `ChatRequest`, `Health`, and
the enums `ProviderKind` / `Role`. All `serde`-serializable; no I/O.

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

**Remote mode** — the folder is on the machine running Spin:

- Spin `filesystem` capability on the backend component:
  `filesystem = [{ permissions = "readwrite", guest_path = "/workspace",
  host_path = "/workspace" }]`
- Container deployment mounts a host directory at `/workspace` (compose
  volume); bare `spin up` uses a local directory
- The backend exposes a file API (`/api/files`: list, read, write, search)
  and the frontend calls it like any other REST endpoint
- The agent edits the mounted folder on the host; the LLM also runs on the
  host (outbound HTTP to localhost is already permitted)

**Local mode** — the folder is on the machine running the browser:

- File System Access API (`window.showDirectoryPicker()`) via web-sys /
  `wasm_bindgen` interop; the directory handle is persisted in IndexedDB and
  re-authorized on reload
- File operations stay entirely in the browser; the backend is used only
  for settings, connections, and system prompts
- LLM connections point at `http://localhost:11434` and the *frontend*
  calls the provider directly via gloo-net (mirroring the backend's
  `HttpClient`); Ollama needs `OLLAMA_ORIGINS` set to the page's origin

Constraints:

- The File System Access API is Chromium-only (Chrome/Edge). Firefox/Safari
  get a read-only fallback (`<input webkitdirectory>`) or no local mode.
- Keep the modes coherent: remote = files + LLM on the host, local = files +
  LLM on the laptop. A mixed split (remote files, local LLM) is deferred.

## Roadmap

The phase-by-phase plan (scaffold → provider HTTP → chat sessions →
container deployment → workspace modes → agentic coding → IDE surface →
distribution) lives in [roadmap.md](roadmap.md).
