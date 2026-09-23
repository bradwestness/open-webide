# AGENTS.md

Instructions, conventions, and architectural principles for AI agents working on the Open WebIDE codebase.

---

## 1. Core Principles

### Always Use the Component System for UI Elements
- **Consistency First**: Always use unified UI component patterns and shared styling classes across the frontend. Never introduce ad-hoc, isolated button/input styles that clash with the rest of the application.
- **Design Tokens & Theme Variables**: Rely strictly on the established CSS variables defined in `frontend/styles.css`:
  - Backgrounds: `var(--bg-main)`, `var(--bg-panel)`, `var(--bg-card)`, `var(--bg-hover)`, `var(--bg-editor)`
  - Borders: `var(--border)`, `var(--border-subtle)`
  - Accents & Actions: `var(--accent)`, `var(--accent-hover)`, `var(--online)`, `var(--offline)`, `var(--warn)`
  - Typography & Code: `var(--font-main)`, `var(--mono)`
- **Shared Components & Buttons**: Use standard button classes (`.btn`, `.btn.accent`, `.btn.send`, `.btn.stop`, `.btn.approve`, `.btn.deny`, etc.) or reusable Leptos component abstractions. When new interactive controls are added, integrate them into the shared styling system so themes (dark/light) and visual hierarchy remain coherent.

### Always Use the Database for Persistence (No LocalStorage)
- **Seamless Multi-Device Continuity**: The core product philosophy is that a user must be able to switch machines, devices, or browsers and immediately pick up right where they left off without losing state.
- **Never rely on `localStorage`** for user preferences, workspaces, layout configurations, or session state.
- **User-Scoped Database Settings**:
  - Persist all user layout preferences, panel dimensions (`panel_sidebar_width`, `panel_tree_width`, `panel_chat_width`), active tabs (`open_tabs`), active project (`active_project`), theme, and default configurations in the SQLite database via the `/api/settings` endpoints (`crates/storage/src/store.rs` -> `user_settings` table).
  - All settings queries and mutations must be scoped to the authenticated `user_id`.
  - The only browser-specific storage permitted is IndexedDB for local file system directory handles (`FileSystemDirectoryHandle`) when running in browser local-mode, where native browser permissions require origin-bound handles.

---

## 2. Architecture Overview

- **Frontend (`frontend/`)**:
  - Built with **Leptos 0.8** compiling to WebAssembly (`wasm32-unknown-unknown`) via **Trunk**.
  - Single-page application providing code editing, terminal multiplexing, git status/diff visualization, collapsible file tree, and chat/agent interactions.
- **Backend (`backend/`)**:
  - WebAssembly microservices running on the **Fermyon Spin** runtime (`wasm32-wasip2`).
  - Implements REST and SSE endpoints for auth, workspace management, file I/O, LLM streaming, sessions, and settings.
- **Storage (`crates/storage/`)**:
  - SQLite storage layer. Spin has no migration runner, so `crates/storage/src/migrations.rs` re-applies a list of idempotent DDL statements at startup (`CREATE TABLE IF NOT EXISTS`, plus probe-`pragma_table_info`-then-`ALTER TABLE ADD COLUMN` for new columns); there is no schema-version table.
  - Supports user isolation, session histories, project metadata, and key-value user settings.
- **Execution Bridge (`bridge/`)**:
  - Native daemon (`openwebide-bridge`) providing PTY terminal emulation, process execution, and host Git operations over WebSockets.
- **Shared Crates (`crates/`)**:
  - `openwebide-core`: Shared domain types, diff algorithms, syntax highlighting, TUI telemetry, and VFS abstractions.
  - `openwebide-auth`: Password hashing and JWT/token verification.
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
  cargo test -p openwebide-storage -p openwebide-core -p openwebide-llm -p openwebide-auth -p openwebide-agent -p openwebide-bridge
  ```
- **Execution Bridge**:
  ```bash
  cargo build -p openwebide-bridge
  ./target/debug/openwebide-bridge --port 3001
  ```
- **Spin Dev Server**:
  ```bash
  spin up --allow-transient-write
  ```

---

## 4. Code Conventions for Agents

1. **Reactive State**: In Leptos components, prefer `RwSignal`, `Signal::derive`, and `Callback` with clear ownership. Avoid cloning heavy state needlessly inside reactive closures.
2. **Error Handling**: Use structured error responses and bubble errors with `Result<T, ApiError>` in the backend and user-friendly error banners or notifications in the frontend.
3. **Database Migrations**: Add new database changes as idempotent migrations in `crates/storage/src/migrations.rs` and update `Store` methods with corresponding unit tests in `crates/storage/src/store.rs`.
4. **Resilience & Safe Layouts**: When implementing layout resizing, enforce sane minimum and maximum bounds to ensure critical panels (like the code editor or diff viewer) are never crushed.
