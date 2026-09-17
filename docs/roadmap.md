# Roadmap

Phase-by-phase plan. Phases 1–6 are the core arc (scaffold → a working
agentic IDE in a container); 7+ are the IDE surface and distribution work
that makes it feel like a product. Each phase ends in something runnable.

Status: ✅ done · 🔜 next · ⬜ planned

## 1. Scaffold ✅

Repo layout and a compiling, tested skeleton.

- Rust workspace: `core` (domain types), `llm` (provider interface +
  stubs), `storage` (Db abstraction, migrations, repositories)
- Spin backend: REST API (health, connections, settings, system prompts,
  models/chat as 501 stubs), SQLite via Spin's `sqlite` capability
- Leptos frontend shell (top bar, sidebar, chat pane, status bar)
- CI: fmt, clippy (native + both WASM targets), native tests, WASM builds
- Verified end-to-end with `spin build --up`

## 2. Provider HTTP ✅

Make the LLM calls real. The UI never cares which engine serves.

- Implement `HttpClient` for the backend (Spin outbound HTTP) and a fake
  for native tests
- Ollama: `GET /api/tags` (models), `POST /api/chat` (completion)
- llama.cpp: `GET /v1/models`, `POST /v1/chat/completions`
  (OpenAI-compatible)
- Wire `/api/models` and `/api/chat` through `registry::Provider`
- Friendly errors for unreachable engines (connection refused, bad base
  URL) instead of opaque 500s

**Done when:** with Ollama running locally, `/api/models` lists real
models and `/api/chat` returns a completion for a stored connection.

## 3. Chat sessions ✅

The Open WebUI-shaped core: conversations you can come back to.

- Sessions in the sidebar: new, switch, rename, delete
- Message persistence (the `sessions`/`messages` tables already exist)
- Chat pane: markdown rendering, input box, send, stop
- Streaming responses (SSE from backend to frontend)
- Per-session system prompt and connection selection

**Done when:** a multi-turn conversation streams token-by-token and
survives a page reload, with history in the sidebar.

## 4. Container deployment ✅

One image, one port, one volume — the whole stack in a single container.

- Multi-stage `Dockerfile` (builder runs `spin build`; runtime is Spin +
  prebuilt components)
- `docker-compose.yml`: host 8080 → container 3000, named volume for
  SQLite
- Podman quadlet units documented for Linux (`docs/podman-quadlet.md`)
- Verified: all endpoints on 8080, data persists across container
  recreation

## 5. Workspace modes 🔜

The IDE operates on a project folder. Where it lives defines the mode;
the UI sits behind a `Workspace` trait with one impl per mode.

**Progress:** Remote mode (backend file API + frontend file tree, browse,
create/edit/save) and multi-project tabs are done and verified end-to-end.
Local mode (File System Access API) is the remaining piece.

- **Remote** (folder on the machine running Spin): Spin `filesystem`
  capability, backend `/api/files` API (list, read, write, search), host
  folder mounted in the container
- **Local** (folder on the machine running the browser): File System
  Access API via web-sys, directory handle in IndexedDB, frontend calls
  the provider directly (Ollama needs `OLLAMA_ORIGINS`)
- File tree in the UI for both modes
- Mode picker in the sidebar; coherent modes only (remote = files + LLM
  on host; local = files + LLM on laptop)
- **Multi-project tabs (Rider-style):** multiple projects open at once,
  each tab = one project (its workspace + chat sessions); tab bar with
  new/switch/close. Project is a first-class entity in the data model —
  sessions and settings belong to a project

**Done when:** you can open a folder in either mode, browse it, and
create/edit/save a file from the UI, with two projects open in tabs and
switching between them.

## 6. Agentic coding

The point of the project: the model edits your code.

- Tool-call protocol through both providers (Ollama `tools`, llama.cpp
  OpenAI-compatible function calling)
- Agent loop in the backend: model → tool call → execute → result →
  model, until done or budget exhausted
- Tools: read file, write file, list directory, search — all confined to
  the workspace
- UI: agent steps as cards (which file, what action), diffs for edits
- Safety: path confinement to the workspace, turn/tool budget, visible
  stop button

**Done when:** "fix the failing test in this folder" → the agent reads,
edits, and reports, with every step visible in the conversation.

## 7. IDE surface

From "chat that can edit files" to "IDE".

- In-browser code editor (CodeMirror 6, WASM) with syntax highlighting
- File viewer **defaults to diff mode** for agent edits, with accept/reject;
  toggle between display modes: **inline diff**, **side-by-side diff**,
  **updated content** (plain file), **preview** (markdown rendered to HTML,
  images viewable)
- Full-text file search (backend, remote mode)
- Settings UI: theme, default connection, default system prompt
- System prompt manager (the API already exists)
- Model picker per session

## 8. Hardening & distribution

- Auth (at minimum a shared token) so the container can be exposed beyond
  loopback safely
- Releases: versioned multi-arch images to GHCR, CHANGELOG
- Token/cost accounting per session (local models: just tokens)
- Better offline/error states (engine down, workspace lost)

## Parking lot

Ideas without a phase yet:

- Terminal in the UI (needs a WebSocket/SSH bridge to the host — the one
  thing that doesn't fit "full stack in WASM")
- Model management (pull/delete models through the Ollama API)
- Multi-model comparison for a single prompt (an Open WebUI classic)
- Conversation export (markdown)
