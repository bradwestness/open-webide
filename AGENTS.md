# AGENTS.md

Instructions, conventions, and architectural principles for AI agents working on the Open WebIDE codebase.

---

## 1. Core Principles

### Always Use the Component System for UI Elements
- **Consistency First**: Always use unified UI component patterns and shared styling classes across the frontend. Never introduce ad-hoc, isolated button/input styles that clash with the rest of the application.
- **Design Tokens & Theme Variables**: Rely strictly on the established CSS variables defined in `frontend/styles.css`:
  - Backgrounds: `var(--bg)`, `var(--bg-panel)`, `var(--bg-hover)`
  - Borders: `var(--border)`
  - Accents & Actions: `var(--accent)`, `var(--online)`, `var(--offline)`, `var(--warn)`, `var(--git-modified)`, `var(--on-accent)`
  - Typography & Code: `var(--text)`, `var(--text-muted)`, `var(--mono)`; body text uses the system font stack
- **Shared Components & Buttons**: Use standard button classes (`.btn`, `.btn.send`, `.btn.stop`, `.btn.approve`, `.btn.deny`, etc.) or reusable Leptos component abstractions. When new interactive controls are added, integrate them into the shared styling system so themes (dark/light) and visual hierarchy remain coherent.

### Always Use the Database for Persistence (No LocalStorage)
- **Seamless Multi-Device Continuity**: The core product philosophy is that a user must be able to switch machines, devices, or browsers and immediately pick up right where they left off without losing state.
- **No user state is persisted in localStorage**: theme and prompt history use user-scoped database settings; legacy keys are removed after import.
- **Never rely on `localStorage`** for user preferences, workspaces, layout configurations, or session state. The session is an HttpOnly cookie.
- **User-Scoped Database Settings**:
  - Persist all user layout preferences, panel dimensions (`panel_sidebar_width`, `panel_tree_width`, `panel_chat_width`), active tabs (`open_tabs`), active project (`active_project`), theme, `bridge_url`, and default configurations in the SQLite database via the `/api/settings` endpoints (`crates/storage/src/store.rs` -> `user_settings` table).
  - All settings queries and mutations must be scoped to the authenticated `user_id`.
  - Tools that depend on the `bridge_url` user setting must authenticate natively via `BridgeCredentials`.
  - The only browser-specific storage permitted is IndexedDB for local file system directory handles (`FileSystemDirectoryHandle`) and the bridge pairing token when running in browser local-mode, where native browser permissions or security requirements require origin-bound data. (There are no exceptions to this rule.)

---

## 2. Architecture Overview

- **Frontend (`frontend/`)**:
  - Built with **Leptos 0.8** compiling to WebAssembly (`wasm32-unknown-unknown`) via **Trunk**.
  - Feature stores in `frontend/src/state/*` are created by `App` and provided through Leptos context.
  - Single-page application providing code editing, terminal multiplexing, git status/diff visualization, collapsible file tree, and chat/agent interactions.
- **Backend (`backend/`)**:
  - WebAssembly microservices running on the **Fermyon Spin** runtime (`wasm32-wasip2`).
  - Implements REST and SSE endpoints for auth, workspace management, file I/O, LLM streaming, sessions, and settings.
- **Storage (`crates/storage/`)**:
  - SQLite storage layer. Spin has no migration runner, so `crates/storage/src/migrations.rs` applies version-gated migrations: `PRAGMA user_version` tracks the schema version against `SCHEMA_VERSION`, and each numbered step in `apply_step` runs once (every step stays idempotent, since pre-versioning databases start at 0 and replay). A database whose version is newer than the build refuses to start.
  - Supports user isolation, session histories, project metadata, and key-value user settings.
- **Execution Bridge (`bridge/`)**:
  - Native daemon (`openwebide-bridge`) with separate `terminals` (PTY/headless sessions), `exec` (host command/Git execution), `runs` (agent/chat runs), and `server` (HTTP/WebSocket routing) modules.
  - Uses a hyper HTTP server, a backend shared secret, and a WebSocket `hello` gate with short-lived bridge tokens (or a local pairing token). Chat/agent runs and completions stream over the shared WebSocket; the daemon is bundled alongside Spin in the Docker image.
  - HTTP tools and agent runs share `Arc<dyn ToolExecution>` on `ServerConfig`; terminals retain full host shell access outside that trait.
- **Shared Crates (`crates/`)**:
  - `openwebide-core`: Shared domain types, diff algorithms, syntax highlighting, TUI telemetry, and VFS abstractions.
  - `openwebide-auth`: argon2id hashing and HMAC session/bridge tokens (not JWT).
  - `openwebide-agent`: Tool-calling agent loop, tool execution, and permission gating.
  - `openwebide-llm`: Provider integrations (Ollama, and llama.cpp via its OpenAI-compatible API). There are no OpenAI or Anthropic providers.

---

## 3. Build & Test Commands

- **Toolchain**: pinned in `rust-toolchain.toml` (currently `1.98.1`). A toolchain bump moves
  `rust-toolchain.toml`, `.github/workflows/ci.yml`, and the `Dockerfile` builder stage together.
- **Backend WASM (Spin)**:
  ```bash
  cargo build -p openwebide-backend --target wasm32-wasip2 --release
  ```
- **Frontend Bundle (Trunk)**:
  ```bash
  cd frontend && trunk build --release
  ```
- **Unit & Integration Tests**:
  ```bash
  # Run tests across native crates
  cargo test -p openwebide-storage -p openwebide-core -p openwebide-auth -p openwebide-agent -p openwebide-llm -p openwebide-bridge -p openwebide-backend
  cargo test -p openwebide-frontend --lib
  ```
- **Frontend UI Tests** (needs Chrome + chromedriver and `wasm-bindgen-cli` matching
  `Cargo.lock`'s `wasm-bindgen`):
  ```bash
  CHROMEDRIVER=<path> cargo test -p openwebide-frontend --target wasm32-unknown-unknown
  ```
- **Execution Bridge**:
  ```bash
  cargo build -p openwebide-bridge
  ./target/debug/openwebide-bridge --port 3001 --workspace ../.. --host 127.0.0.1 \
    --backend-url http://127.0.0.1:3000/api --secret-file /tmp/openwebide-bridge-secret
  ```
- **Spin Dev Server**:
  ```bash
  spin up --direct-mounts --allow-transient-write
  ```

### Build disk usage

`target/` grows fast here: native, `wasm32-unknown-unknown` and `wasm32-wasip2` builds, each
with its own test and clippy artifacts. A full repo copy that builds its own `target/` adds tens
of GB, and agents running several copies at once have filled the disk before.

- Experiments, review copies and worktrees reuse the repo's build directory: set
  `CARGO_TARGET_DIR=<repo>/target` (cargo locks it, so concurrent builds are safe). Never let a
  scratch copy create its own `target/`.
- Prefer `git worktree add` over copying the repo, and remove copies and worktrees when done.
- Dev builds keep only line tables (`[profile.dev] debug = "line-tables-only"`, and no debug info
  for dependencies) — don't turn full debug info back on in the workspace profile.
- Run `cargo clean` after finishing a feature, once it's pushed, and also whenever free space
  drops under ~50 GB between batches. Never run it while a build is running.

---

## 4. Code Conventions for Agents

1. **Reactive State**: In Leptos components, prefer `RwSignal`, `Signal::derive`, and `Callback` with clear ownership. Avoid cloning heavy state needlessly inside reactive closures. Shared frontend state lives in `frontend/src/state/*`, is provided from `App` with context, and belongs in the matching feature store.
2. **Error Handling**: Backend handlers live in `backend/src/api/*` and return `Result<T, ApiError>` through typed `FsError`, `BridgeError`, and `StorageError` conversions; never classify errors by substring. Internal errors log their detail with the route and return a generic 500 body. Use user-friendly error banners or notifications in the frontend.
3. **Database Migrations**: Append a numbered step in `apply_step` (`crates/storage/src/migrations.rs`), bump `SCHEMA_VERSION`, keep every step idempotent, and never edit a shipped step; update `Store` methods with corresponding unit tests in `crates/storage/src/store.rs`.
4. **Resilience & Safe Layouts**: When implementing layout resizing, enforce sane minimum and maximum bounds to ensure critical panels (like the code editor or diff viewer) are never crushed.

5. **Lint Policy**: Clippy pedantic picks are enforced via `[workspace.lints]` — every crate inherits it with `[lints] workspace = true`, and CI enforces it with `-D warnings` on native crates, the WASI backend, the WASM frontend, and the bridge. Add a lint there, not per crate; `#[allow]` needs a reason.

## 5. Threat model

Single-user home lab on a trusted LAN. Keep what bites at home (a malicious website in a
browser tab, including another port on the same host; prompt-injected model output; data loss;
crashes; broken contracts), but security fixes must not remove features. Deliberately not done
for that reason: bridge commands inherit the user's environment (no env allow-list), and the
backend may reach any LAN host (no private-address egress block; only cloud-metadata addresses
are blocked and `fetch_web_page` needs approval).
