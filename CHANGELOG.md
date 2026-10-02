# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
The workspace is at `0.1.0` with no tags and no releases yet, so everything
to date lives under [Unreleased](#unreleased). See [docs/roadmap.md](docs/roadmap.md)
for what's still ahead.

## [Unreleased]

### Added

- Handle CRLF and CR fallback chat streams and multi-line SSE data fields correctly.

- Clarify Open local / Open remote with device-based tooltips, a remote folder-picker subtitle, and matching documentation.

- Reduce the release WebAssembly download size with size-focused optimization.

- Debounce file-search typing by 250 ms while keeping clearing and ignored-folder toggles immediate.

- Load workspace settings and lists in parallel, and avoid redundant model requests when switching or renaming sessions on the same connection.

- Render terminal output incrementally with 10,000 lines of scrollback, split ANSI colour support, recovery from unfinished controls when processes exit or restart, progress-line updates, and scrolling that follows only near the bottom.

- Coalesce editor syntax highlighting to one animation frame while keeping typing and saving immediate.

- Keep conversation rows mounted while replies stream, reducing chat update work and preserving expanded reasoning and diff previews.

- Show streamed model reasoning and mark replies cut off by the output token limit; omit prior reasoning from model context.
- Preview file diffs before approving agent edits, expand long previews, and flag unreadable overwrites.
- Run `/test [filter]` in the terminal with shell-quoted filters and the project directory.
- Bundle the execution bridge in Docker and Podman deployments for terminals, Git operations, and streamed chat on port 3001.
- Resume interrupted local-mode runs from their saved conversation, preserving user messages and tool-step numbering.
- Add bridge-hosted chat/agent runs and streamed completions, with reconnect replay, cancellation, approvals, and TLS enabled by default.
- Add provider streaming for tool-call turns, with automatic non-streaming fallback for older llama.cpp servers.
- Add a frontend component test harness with a fake backend and headless Chrome tests in CI.
- **System theme option:** the default follows the OS light/dark preference and reacts live to changes; explicit theme choices are stored in per-user database settings.
- **Search "include ignored folders" toggle:** the file tree's search box now has a toggle button that also searches the ignored folders (`.git`, `target`, `node_modules`, `dist`, `.spin`); the hit and byte caps still apply, and toggling re-runs the current query. The flag is per query — not persisted, off on reload.
- **Bridge confinement:** canonicalize the workspace root and enforce lexical cwd bounds, while permitting directory symlinks.
- Agent edits back up original bytes before overwriting unreadable (binary or large) files; refusing an unreadable file edit now safely restores it from a backup rather than deleting it.
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
  hashing, HttpOnly cookie sessions, and user-scoped projects/sessions.
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
- Bridge `hello` authentication with short-lived backend-minted tokens (`/api/bridge/token`), a `bridge_url` user setting, and optional `OPENWEBIDE_BRIDGE_TOKEN` pairing credentials.
- **Local mode integrity:** validate UTF-8 with a read-only fallback for unreadable files, surface missing directory permissions, and abort local runs without waiting for server round-trips.

### Changed

- Enforce selected pedantic Clippy lints across the workspace, remove unused code, and trim bridge Tokio features.
- Split bridge terminals, tool execution, and agent runs into modules; share an executor trait for host tools and add structured logs with `RUST_LOG` filtering.
- Refactor backend routing and typed errors; hide internal 500 details, surface project lookup failures, and accept raw binary file writes.
- Stop interrupts running commands, web requests, and searches while allowing file writes and Git mutations to finish.
- Sync prompt history and theme through user settings, import legacy history once, and fit panel widths to the viewport.
- Split frontend state into per-feature stores provided through Leptos context.
- Run local-mode command and git tools in the picked folder, discovered and verified through a temporary bridge probe.
- Persist interim tool calls for follow-up history and remember servers that reject streamed tool calls.
- Unify SSE and WebSocket chat events in one shared protocol; deploy frontend and backend together.
- Use the bridge for frontend chat and local completion streaming, with SSE fallback, run resume, and project availability notices.
- Share one authenticated bridge connection across terminal, run, and completion traffic; reconnect terminals automatically and open a fresh shell after a bridge restart.
- Stream server-side agent replies token by token, retain text before tool calls in conversation history, and add a run-plan API.
- Added session expiry handling (auto-logout) and statusline model switcher dropdown.
- **Bridge process lifecycle:** every command the bridge spawns now runs in its own process group, so timeout, request disconnect, Kill, and shutdown signal that group. Interactive shells may place background jobs in separate groups that survive shell cleanup. `Kill` sends the requested signal (`TERM`/`HUP`/`KILL`, default `KILL`) to the whole group; on a PTY, `INT` instead writes `^C` to the terminal like a real Ctrl+C. `/exec` output over ~1 MiB per stream now keeps the first 256 KiB and last 768 KiB with an omission marker instead of buffering unbounded output. Exited terminal/process sessions are now removed 30 minutes after they exit; running sessions are never reaped. The daemon now shuts down gracefully on Ctrl+C or `SIGTERM`, terminating every session's process group first.
- **Bridge HTTP resilience:** The execution bridge's HTTP and WebSocket server is now built on `hyper` instead of a hand-written parser, fixing header/body misreads under fragmented or chunked writes and case-sensitive `Upgrade` header matching. Requests are capped (16 KiB head, 1 MiB `/exec`/Git body, 16 MiB WebSocket message), idle sockets close after 10 s without a request head, WebSocket connections are pinged every 30 s and closed after 90 s without a reply, and the accept loop now backs off and keeps serving instead of exiting on the first accept error (e.g. EMFILE), bounded by 256 concurrent connections.
- Backend git operations now run within the project context, and bridge failures cleanly propagate rather than showing false success.
- **Backend resource bounds:** API request bodies are now capped per route (64 KiB auth, 1 MiB JSON, 4 MiB settings, 16 MiB chat, 10 MiB file write) and rejected with 413 Payload Too Large when exceeded. File search is bounded to 500 hits / 64 MiB / 20,000 entries, and the `include_ignored` flag is honored server-side.
- Database methods that update multiple interrelated tables (e.g. deleting a project, first-admin creation) now run within strict SQLite `BEGIN IMMEDIATE` ... `COMMIT` transactions on the backend. This guarantees complete rollback if an operation fails or if the WASM task cancels or panics midway, fixing race conditions and half-deleted states without relying on manual cascading deletion code or risking lock deadlocks.
- The user registration API endpoint (`POST /api/register`) now correctly isolates creation by atomically using the transaction, preventing race conditions where multiple parallel signups could occur on first setup.
- Constraint-violation errors are now reliably returned as HTTP 409 Conflict.
- **Direct Workspace Mounts:** Spin development servers and Docker deployments now require the `--direct-mounts --allow-transient-write` flags. This ensures file modifications write to the native host directory instead of a temporary Spin sandbox. Project creation validates paths, and the `.spin/` configuration directory is explicitly forbidden from file API and agent access.
- **Web fetch & redirect safety:** Hardened web fetching against SSRF by refusing requests to cloud metadata endpoints (`169.254.169.254`, `fd00:ec2::254`, their IPv4-mapped representations, and `metadata.google.internal`) on initial requests and redirect hops while keeping LAN and private access open. Hand-rolled RFC 3986 §5.2 redirect reference resolution to correctly handle absolute paths, network-path (`//`) references, relative paths, port preservation, and query-only redirects. Capped streaming HTTP response bodies directly at 512 KiB for web pages and 2 MiB for JSON to prevent memory exhaustion.
- **Bridge Git safety:** Protected native Git execution against command-line argument injection. Branch and remote names are validated (`git check-ref-format --branch`, remote membership) with option-like leading `-` and pathspecs like `.` rejected with 400 Bad Request. Branch switching now strictly uses `git switch` (requiring Git ≥ 2.23), commit operations with specified paths only stage and commit those paths, Git output is untrimmed, push/pull commit counts are calculated accurately via `rev-list`, and phantom `origin` branches from `origin/HEAD` symrefs are ignored.
- **Approval UX:** "Always approve" is now scoped per-session (cleared on logout) instead of globally, and explicitly never covers `run_command`. Keyboard approval shortcuts moved from bare `y`/`n`/`a` to `Alt+Y / Alt+N / Alt+A` (typed responses no longer trigger approvals). Stopping a run now cancels any pending tool prompt instead of leaving it clickable, fixing an issue where later runs' shortcuts acted on dead prompts.
- Enabled running unit tests for `openwebide-backend` natively via an `AppDb`
  abstraction backed by in-memory SQLite (`rusqlite`) on native targets and Spin
  SQLite (`SpinDb`) on WASM targets.
- Toolchain pinned to Rust 1.98.1; `argon2` 0.6, `rand` 0.10, `base64` 0.23,
  `hmac` 0.13, `sha2` 0.11, and `tokio-tungstenite` 0.30 (bridge only), with
  existing password hashes and bearer tokens verifying unchanged.
- `/tokens` now shows the context-window gauge as the *latest* call's token
  count against the resolved context limit, instead of a running total
  summed across the whole session; the Input/Output rows stay cumulative.
- The context-window fallback, used when a connection has no configured
  limit and provider discovery finds none, is `4,096` tokens (was a
  hard-coded `32,768`), and is marked estimated (`~`) in the UI.
- Deliver terminal output through a cursor-based SeqRing with ordered replay, retained exit events, truncation notices, and duplicate session-ID rejection.
- Apply numbered, idempotent database migrations transactionally via `PRAGMA user_version`; refuse databases newer than the build.
- Decode typed tool arguments and report malformed calls as tool errors; cap file, directory, and search tool results and support `include_ignored` in agent searches.
- Reconcile setup, architecture, bridge protocol, roadmap, changelog, and `/help` documentation with current behavior.

### Fixed

- Keep split terminal controls intact when busy or recoverable-error notices appear during a running process.

- Preview SVG files with uppercase extensions.
- Preserve binary Git HEAD contents and hide Revert with a clear binary-file notice in text diffs.
- Isolate SSE run cancellation and permission cleanup so a quick resend preserves Stop and concurrent runs retain their decisions.
- Editor: Added confirmation dialogs before discarding unsaved edits and conditionally offered the Revert button only when HEAD is known.
- **Web fetch HTML→Markdown conversion:** the converter is now built on the `html5ever` tokenizer (same version `ammonia` already uses, so no second copy in the dependency tree), so fetched pages keep text the old hand-written scanner dropped — an unescaped `<` no longer swallows the rest of the line, bare `&` in text (`Q&A`, `AT&T`) survives, HTML5 entities are decoded fully, self-closing skip tags (`<svg/>`) no longer swallow everything after them, and `<head>` content (e.g. `<title>`) no longer leaks into the output. Links with unsafe `href` schemes (`javascript:`, `data:`, …) are now removed (link text is kept), relative/fragment links are kept, and `data-href` is no longer mistaken for `href`.
- **Line-ending and final-newline diffs:** the inline and side-by-side diffs now align lines with a line-level LCS that compares each line including its ending, so a CRLF→LF conversion or an added/removed trailing newline shows up as a change (with a small "⏎ CRLF → LF" / "no newline at end of file" note) instead of being invisible, and an inserted line no longer marks every line below it as changed. A whole-file line-ending change collapses to a single summary line.
- **Git status for paths with spaces, unicode, and unborn branches:** the bridge now reads `git status` with `--porcelain=v1 -b -z` (NUL-separated, unquoted paths) and the parser consumes rename/copy source records and recognizes the `No commits yet on` / `Initial commit on` / `HEAD (no branch)` headers, so files with spaces or non-ASCII names report the correct status and new repositories show their branch instead of failing to parse.
- **Stale search results no longer overwrite the view:** file-content searches are now guarded by a per-run generation, so a slow search for an older query can no longer land after a newer search for the same project and clobber its results, and clearing the search box can no longer be undone by a search that resolves after the clear.
- **Stale chat history no longer overwrites the view:** loading a session's message history is now guarded by a per-run generation, so a slow history request can no longer land after a newer request — including one for the same session — and clobber the conversation on screen. The first send in a brand-new chat also no longer duplicates the user message, since the redundant history fetch that fired the instant the session was created is now skipped.
- **Stale file-browser listings no longer overwrite the view:** the folder picker's directory listing is now guarded, so a slow browse response for an old directory can no longer land after a newer navigation and show the wrong listing.
- **\"New file\" no longer truncates an existing file:** creating a file at a path that already exists now returns an error (HTTP 409 in remote mode, an error banner in local mode) and leaves the file's content untouched. Agent `write_file` to a new local-mode path now works correctly.
- **Auth correctness:** the token-signing secret is now created with an atomic insert-if-absent, so concurrent first requests can no longer write different secrets and invalidate issued tokens, and token expiry checks fail closed with a 500 instead of accepting every token when the system clock is unavailable.
- **Docker project-path migration:** Remote project paths created before the `/workspace` mount (stored relative to the container root, e.g. `workspace/foo`) are rewritten automatically on first start, so projects from older Docker installs no longer appear missing after an upgrade.
- Plain-chat SSE streams always emit a terminal done/error event, including persistence failures.
- **Truncated replies are kept, not silently saved or dropped:** a reply cut short by a crashed or disconnected provider is now kept with a visible "[reply truncated]" marker instead of being silently saved as complete or dropped, and llama-server `error:` events now surface their message.
- Database foreign keys (`PRAGMA foreign_keys = ON`) are now correctly enabled when opening the Spin WASM SQLite database connection.
- Calling `delete_project` now relies natively on SQLite's cascading deletes, vastly simplifying the query footprint.
- **Frontend panics and terminal cleanup:** Fixed a panic in editor context capture when truncating selections lands inside a multi-byte character by mapping UTF-16 selection offsets to byte offsets and truncating at a UTF-8 char boundary. The terminal dock's WebSocket connection task is now cancelled and its spawned sessions killed when the dock closes, and its output auto-scroll no longer panics on an already-disposed DOM node. The status bar no longer panics on a stale Git status signal. Added a browser panic hook that logs to the console instead of showing an opaque `unreachable` error. Typed shell commands with arguments are now run via `sh -lc` instead of failing.
- **Provider stream robustness:** The Ollama/llama.cpp line-splitter now decodes UTF-8 across the whole buffered line instead of per network chunk, so a multi-byte character split across chunks no longer corrupts the reply. The buffer is capped at 16 MiB per line instead of growing without bound. A `tool_calls: []` response is now treated as a text reply instead of an empty tool-call turn that looped until the turn budget ran out.
- **Highlighter and diff robustness:** Fixed panics in the syntax highlighter on non-ASCII source lines and in HTML-to-Markdown truncation on multi-byte character boundaries. Lines over 10,000 bytes now render as a single unhighlighted token instead of freezing. Word-level diffs fall back to a whole-line delete/insert past a size budget instead of using unbounded `O(m·n)` memory.
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
- Return CORS headers on API errors and reject extra path segments on session update/delete routes.
- Match file extensions without case sensitivity and use Markdown fences longer than any backtick run in injected editor context.

### Security

- **Bridge Security**: Added `Authorization: Bearer <SECRET>` for Origin-less `/exec` and `/git/*` routes. The bridge generates a 32-byte secret at startup and the backend auto-fetches it over loopback via `POST /secret`, with caching in SQLite.
- Hardened login system: transitioned to HttpOnly cookie sessions, added brute-force lockout, and implemented global logout (invalidates sessions on all devices). Added CSRF protection via custom header.
- Default-deny tool approval policy: agent tools now default to requiring user approval unless explicitly allow-listed as auto-approved (`read_file`, `list_dir`, `search`, `grep_search`, `git_status`, `git_diff`, `search_web`). `fetch_web_page` now requires user approval, and `write_file` rejects any path with a `.git` component (case-insensitive). Tool descriptions and summaries updated for `write_file` (showing line and byte counts) and `git_commit` (clarifying all tracked changes vs specific paths).
- Bridge exposure baseline: Enforced strict `Host` and `Origin` validation, rejecting foreign cross-origin browser requests and DNS rebinding. Removed wildcard `Access-Control-Allow-Origin: *` and restricted preflight responses. Enforced `application/json` Content-Type on browser POSTs (`POST /exec`, `/git/*`) to prevent CSRF, and added `--allowed-origin` and `--allowed-host` CLI/env configuration.
- `files/raw` non-image requests are now forced to download instead of rendering inline.
- Directly opened SVG/HTML files via `files/raw` are now sandboxed.
- Dropped support for `?token=` query parameter authentication. Media preview URLs now use short-lived blob URLs instead of exposing the bearer token.
- Markdown previews sanitize raw HTML with `ammonia` and no longer execute unsafe elements (e.g., `<script>`, `<style>`,
  `<iframe>`, `onload`/`onerror`, `javascript:` links). Safe elements like
  `<details>`, `<sub>`, and inline `<img>` remain. SVG previews display as
  images and no longer execute scripts. Added `openwebide-frontend` as a lib
  crate for a native test harness.
