# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
The workspace is at `0.1.0` with no tags and no releases yet, so everything
to date lives under [Unreleased](#unreleased). See [docs/roadmap.md](docs/roadmap.md)
for what's still ahead.

## [Unreleased]

### Changed
- **Bridge Git safety:** Protected native Git execution against command-line argument injection. Branch and remote names are validated (`git check-ref-format --branch`, remote membership) with option-like leading `-` and pathspecs like `.` rejected with 400 Bad Request. Branch switching now strictly uses `git switch` (requiring Git ≥ 2.23), commit operations with specified paths only stage and commit those paths, Git output is untrimmed, push/pull commit counts are calculated accurately via `rev-list`, and phantom `origin` branches from `origin/HEAD` symrefs are ignored.
- **Approval UX:** "Always approve" is now scoped per-session (cleared on logout) instead of globally, and explicitly never covers `run_command`. Keyboard approval shortcuts moved from bare `y`/`n`/`a` to `Alt+Y / Alt+N / Alt+A` (typed responses no longer trigger approvals). Stopping a run now cancels any pending tool prompt instead of leaving it clickable, fixing an issue where later runs' shortcuts acted on dead prompts.
- Default-deny tool approval policy: agent tools now default to requiring user approval unless explicitly allow-listed as auto-approved (`read_file`, `list_dir`, `search`, `grep_search`, `git_status`, `git_diff`, `search_web`). `fetch_web_page` now requires user approval, and `write_file` rejects any path with a `.git` component (case-insensitive). Tool descriptions and summaries updated for `write_file` (showing line and byte counts) and `git_commit` (clarifying all tracked changes vs specific paths).
- Bridge exposure baseline: Enforced strict `Host` and `Origin` validation, rejecting foreign cross-origin browser requests and DNS rebinding. Removed wildcard `Access-Control-Allow-Origin: *` and restricted preflight responses. Enforced `application/json` Content-Type on browser POSTs (`POST /exec`, `/git/*`) to prevent CSRF, and added `--allowed-origin` and `--allowed-host` CLI/env configuration.
- `files/raw` non-image requests are now forced to download instead of rendering inline.
- Directly opened SVG/HTML files via `files/raw` are now sandboxed.
- Dropped support for `?token=` query parameter authentication. Media preview URLs now use short-lived blob URLs instead of exposing the bearer token.

### Added

- **Scaffold:** Rust workspace (`core`, `llm`, `storage`), a Spin (`wasm32-wasip2`)
  backend, a Leptos WASM frontend shell, and CI running fmt, clippy, native
  tests, and both WASM builds.
- **Provider HTTP:** real Ollama and llama.cpp calls (`/api/models`,
  `/api/chat`) over Spin outbound HTTP, with a fake `HttpClient` for native
  tests and actionable errors for unreachable engines.
- **Chat sessions:** a sidebar of sessions (new/switch/rename/delete), SQLite
  message persistence, SSE token streaming, and a per-session system prompt
  and connection.
- **Container deployment:** a single-image multi-stage `Dockerfile`,
  `docker-compose.yml`, and Podman quadlet units, with SQLite on a named
  volume.
- **Workspace modes:** Remote mode (Spin-mounted host folder, `/api/files`)
  and Local mode (File System Access API, directory handle persisted in
  IndexedDB), plus Rider-style multi-project tabs.
- **Agentic coding loop:** tool-calling agent (`read_file`, `write_file`,
  `list_dir`, `search`) confined to the workspace, with turn/tool budgets, a
  permission handshake for gated tool calls, and server-side run
  cancellation.
- **IDE surface:** an in-browser syntax-highlighting code editor, a
  diff-first file viewer (inline diff, side-by-side diff, updated content,
  markdown/image preview), full-text file search, a per-session model
  picker, a Settings dialog (theme, default connection, default system
  prompt), and a system prompt manager.
- **Local user accounts:** registration and login, argon2id password
  hashing, signed bearer session tokens, and user-scoped projects/sessions.
- **Custom dialogs & remote file browser:** themed confirmation and prompt
  dialogs, and a host file browser for picking a remote project folder,
  replacing browser-native `alert`/`confirm`/`prompt`.
- **Virtual File System (VFS):** a shared `Vfs` trait with `HostFsVfs`
  (remote) and `BrowserFsaVfs` (local) implementations, a universal
  `VfsToolExecutor` shared by both modes, a browser-driven local-mode agent
  loop against `/api/chat-tools`, workspace-wide `grep_search`, `search_web`,
  `fetch_web_page`, and zero-turn temporal context injection.
- **Process execution & WebSocket terminal bridge:** the native
  `openwebide-bridge` daemon (PTY sessions, `run_command` agent tool) and an
  integrated terminal pane, with Git operations forwarded to the bridge
  against the real host repository.
- **TUI-driven chat surface:** a terminal-native linear stream layout,
  collapsible `<think>` reasoning blocks, readline-style prompt history,
  an in-stream keyboard permission handshake, a slash-command engine
  (`/model`, `/tokens`, `/clear`, `/test`, `/diff`, `/help`, `/commit`,
  `/checkout`, `/branch`, `/sync`), and active-editor-context injection via
  a context pill and `Ctrl+L`/`Cmd+L`.
- **Git integration:** passive `.git/HEAD`/`.git/refs` status telemetry with
  a bridge-backed active engine for status, diff, branch, commit, checkout,
  and sync against the real host repository; a status bar branch/ahead-behind
  widget, file tree status badges, and `git_status`/`git_diff`/`git_commit`/
  `git_branch` agent tools.
- **Editor enhancements:** cursor/selection tracking (`EditorContext`) shared
  between the editor and chat prompt, diff viewing against Git HEAD, a
  markdown/image preview mode, and syntax highlighting expanded from 5 to 16
  languages.
- **TUI telemetry meters:** the statusline's `t/s` and `Ctx:` gauge are now
  backed by real provider usage. Ollama (`prompt_eval_count`, `eval_count`,
  `eval_duration`) and llama.cpp (`usage`, `timings`) report token counts and
  timing per call; missing fields fall back to an estimator and are marked
  `~` in the statusline and `/tokens`. Usage is forwarded as a `telemetry` SSE
  event, recorded into `SessionTelemetry`, persisted on the assistant
  message, and replayed on session reload.
- **Per-connection context limit:** an optional `context_limit` on each
  connection, resolved from the configured value, then provider discovery
  (`GET /api/models/context` — Ollama's `POST /api/show`, llama.cpp's
  `GET /props`), then a fallback. Ollama connections send it as
  `options.num_ctx` on every request so the gauge and the runtime context
  window agree; for llama.cpp the value drives the gauge display only,
  since its context size is fixed when `llama-server` starts.

### Changed

- Enabled running unit tests for `openwebide-backend` natively via an `AppDb`
  abstraction backed by in-memory SQLite (`rusqlite`) on native targets and Spin
  SQLite (`SpinDb`) on WASM targets.
- Toolchain pinned to Rust 1.98.1; `argon2` 0.6, `rand` 0.10, `base64` 0.23,
  `hmac` 0.13, `sha2` 0.11, and `tokio-tungstenite` 0.30 (bridge only), with
  existing password hashes and bearer tokens verifying unchanged.
- Markdown previews no longer execute raw HTML (e.g., `<script>`, `<style>`,
  `<iframe>`, `onload`/`onerror`, `javascript:` links). Safe elements like
  `<details>`, `<sub>`, and inline `<img>` remain. SVG previews display as
  images and no longer execute scripts. Added `openwebide-frontend` as a lib
  crate for a native test harness.
- `/tokens` now shows the context-window gauge as the *latest* call's token
  count against the resolved context limit, instead of a running total
  summed across the whole session; the Input/Output rows stay cumulative.
- The context-window fallback, used when a connection has no configured
  limit and provider discovery finds none, is `4,096` tokens (was a
  hard-coded `32,768`), and is marked estimated (`~`) in the UI.

### Fixed

- Approving one tool call could silently approve a later, different call
  (Ollama reuses `call_0` every turn), letting e.g. `run_command` run without a
  prompt. Tool calls now get session-unique ids and each approval is used once.
  This also stops later runs from overwriting earlier tool steps in history.
- Agent-directed file writes on the streaming write path no longer produce
  0-byte files.
- Remote file API path handling for nested and root-relative paths.
- Filesystem and outbound-denial error messages now point at the actionable
  fix (the host egress allowlist in `spin.toml`, or the remote mount
  configuration) instead of an opaque error.
- Duplicate projects sharing the same mode/path are deduplicated on
  startup, reassigning their sessions to the surviving project.
