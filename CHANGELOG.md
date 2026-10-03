# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
The workspace is at `0.1.0` with no tags and no releases yet, so everything
to date lives under [Unreleased](#unreleased). See [docs/roadmap.md](docs/roadmap.md)
for what's still ahead.

## [Unreleased]

### Changed

- Simplify model setup to one **Detect settings** action per chat model and a single modal **Cancel / Save**. Preview and testing leave configuration untouched until Save atomically commits server options, credentials, profiles and initial defaults. Keep only **Edit** on server rows and **New server** in the Servers heading.
- Add server presets and shared detection for Ollama, llama.cpp, LM Studio, vLLM, LiteLLM, OpenRouter, SGLang and KoboldCpp. Discover standard host endpoints on first setup, cache model facts by transport revision, preserve manual overrides, surface sampling, runtime context, size, quantization, loaded state, server version, CPU spill and tokenizer details when reported, and exclude embedding-only models from chat.
- Add optional model testing for structured and streamed tools, first-token latency and tokens/sec. Probe failures fall back to plain chat with a notice; tool streaming capability is scoped to each server and model. Shared project detection suggests test commands and linters in local and remote workspaces.
- Harden bridge lifecycle and HTTP handling: reap idle detached PTYs, send SIGHUP before forced termination, await delayed process cleanup at shutdown, time out stalled body reads and response writes, close HTTP/1.0 connections, support half-close responses, enforce WebSocket message limits, avoid response-body copies and bound displayed bridge errors. Compile Windows adapters in CI.
- Bound unterminated terminal lines, respect composer IME composition, restore background editor content after backup rejection while preserving new input, mark the active `/model` and support `/model default`. Normalize provider URLs with query strings or fragments without losing nested proxy prefixes.
- Extend auth, search, database rollback, model detection, transport, process lifecycle and browser parity regression coverage. Fake-backend deletions cascade, component click helpers accept combined classes, and frontend tests build natively without `--lib`. Revert the WASM size optimization that did not improve compressed size or startup.

- Establish shared feature facades for local and remote files, Git, execution hosts and session runs. UI send/stop/approval/resume actions use the run facade; planning, context, history recovery, streaming, persistence, permission policy and tool workflows live above thin browser, Spin and bridge adapters.
- Share provider model precedence, wire messages, plain and tool-stream lifecycles; split core domains and diff rendering, share line/word alignment and storage row mapping, and remove storage’s agent dependency. Use typed VFS creation/permission errors and Git responses while retaining browser thread checks and legacy wire formats.
- Verify common provider and filesystem contracts across adapters, including failure and fallback behavior. Browser chat-only runs share server reply persistence and telemetry; populated-directory deletion and binary Git previews behave consistently in both modes.

### Added

- Set up model servers through a rerunnable wizard: choose Ollama or OpenAI-compatible, enter URL and optional write-only auth token, then discover models and review/customize settings. Retry discovery without duplicate servers, preserve saved tokens and manual model overrides, and choose an initial default model when applying reviewed settings. Launch setup from Servers, model configuration or the workspace toolbar in either mode.

- Automatically compact local and remote chat/agent context before model requests and tool continuations, using the configured threshold (85% by default, 0 disables). Reserve response and summary space, use provider tokenization with an estimate fallback, summarize through the fast model or primary fallback, and persist reusable summaries while retaining original history. Failed or cancelled summaries never replace history.

- Render Markdown tables with aligned columns, themed headers and borders, and horizontal scrolling for wide tables in agent replies and file previews. Support strikethrough while retaining sans-serif prose and monospace code in both modes.

- Distinguish user prompts with a compact, rounded, theme-aware background inset from the TUI panel edges and aligned on the right with tool and thought panels; remove repeated assistant headings while preserving a shared text alignment for prompts and replies.

- Keep the file tree ordered at every level: directories first, then files, using case-insensitive natural name sorting consistently across local and remote loads and refreshes.

- New chat immediately creates and selects a session, persisting its project startup context in both modes before the first prompt.


- Choose Default, Auto-accept edits, Auto, or YOLO from the TUI or with Shift+Tab. Persist session choices in user-scoped database settings and enforce them through one shared policy gate, with thin browser, bridge, and SSE adapters. Auto uses the configured fast model or the primary model and falls back to manual approval on uncertain, malformed, failed, or timed-out classification.

- Configure primary and optional fast models, per-model context/sampling/output/thinking/tool overrides, and an 85% auto-compaction threshold in database-backed settings shared by local and remote mode. Background work falls back to the primary model when no fast model is selected..
- Add model servers by URL, discover common local endpoints, detect model context/capabilities, and configure write-only API keys, proxy headers, timeouts, and Ollama keep-alive.

- Choose a session’s connection and model from a tiered TUI menu, with models discovered on expansion and new sessions starting from the configured default.

- Generate fresh startup context in local, backend, and bridge runs: environment and available tools, root and nested project instructions, and relative imports. Save the exact context as a collapsible session entry, with visible size and import limits.
- Automatically refresh the file tree in local and remote projects, preserving expanded folders and editor contents while pausing background tabs.
- Restore the last-used session per project when opening a project or a fresh browser window, using user-scoped database settings.

- Reload pending agent edits per project from the database and persist Accept/Reject decisions, retaining failed reviews and backups for retry.

- Store pending agent edits and review decisions per project in the database, with replay protection and revision checks; review UI integration follows separately.

- Keep terminal shells and output when hiding the dock, and start new shells in the active project folder with a visible workspace-root fallback notice, including when a remote folder no longer exists.

- Guard local browser file operations and agent completions with originating-thread checks, including cancellation when a completion stream is dropped.

- Reduce repeated recent-project filtering and statusline formatting work while preserving project order, telemetry text, and saved themes.

- Make dialogs keyboard accessible with focus containment, stacked Escape handling, focus restoration, and keyboard folder navigation.

- Apply the saved database theme before first paint without browser preference storage, and unify action buttons and theme-aware warning/diff colors.

- Reuse the browser database connection and remove saved local folder handles when projects are deleted, including stale handles owned by the signed-in account found at startup; preserve other accounts’ folders and handles saved during startup, including when the system clock changes.

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

- Keep personal default/fast model choices in Settings; move shared model configuration to a separate dialog opened from Servers. Model profiles, context detection and compaction thresholds are shared per server/model, with existing preferences migrated.

- Share content/file search policy and budgets across browser, WASI and native filesystems, and share run planning and persisted agent events across browser, SSE and bridge runs.
- Route Git, terminal and agent startup through a common project execution-host facade. Model discovery uses shared probes on the backend or the local companion host.
- Normalize file paths and enforce a shared 10 MiB read limit across editor and agent adapters; browser filesystem failures retain typed error categories.


- Keep assistant thinking blocks collapsed while the model is thinking (click the header to expand the live trace), move the spinner after the "Thinking..." label with a live elapsed-time counter, and show the final elapsed time in the collapsed "Thought" summary; the spinner now cycles proper braille frames. Elapsed time scales with the run (e.g. `42s`, `2m05s`, `1h03m20s`), and aborting a turn clears the "Thinking..." state so the partial trace collapses into the summary.
- Raise the agent run budget from 12 turns / 24 tool calls to 256 turns / 512 tool calls; the budget is a runaway-loop guard (runs stay cancellable), so nontrivial tasks no longer exhaust it mid-task.
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

- Delete nested remote files with writable directory descriptors and support recursive folder deletion without following symlinks, matching browser/native adapters.

- Recover recent local projects when browser folder access is missing: request permission or re-pick the original folder, starting at the saved handle when available and retaining the project and sessions.
- Defer oversized CLAUDE.md/AGENTS.md instructions with scoped read-file directions instead of injecting truncated fragments; retain imports beyond the inline limit.


- Route file-tree Git indicators, HEAD diffs, branch/sync controls, and Git slash commands through a shared project Git facade in both modes. Local projects use the verified local bridge instead of the remote-only project API; background refresh includes Git status and stale results are discarded.

- Keep editor syntax highlighting aligned during rapid scrolling and bottom-to-top scrolling by translating the highlight layer from the textarea’s offsets, matching scroll gutters, and showing only one text layer at a time.

- **Side-by-side diff alignment:** the detailed side-by-side diff is now built on the same line-level LCS as the inline diff, so an inserted or deleted line no longer shifts every line below it into a false pair (previously the whole changed middle rendered as insertions); unchanged lines stay aligned as context, and paired changed lines keep intra-line word highlighting and line-ending notes.

- Disable the agent chat composer until a project is open, with shortcuts to open a local or remote project.

- Accept llama.cpp base URLs with or without a trailing `/v1` for model discovery, chat completions, and context limits.

- Stop bridge agent runs when a tool result cannot be saved, delivering the completed result before the error and preventing overlapping runs in the same session.

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
