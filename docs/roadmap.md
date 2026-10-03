# Roadmap

What's left, grouped by how soon it's coming: **Next** (queued up), **Later**
(planned, not yet started). Finished work —
phases 1 through 14, telemetry, hardening, streaming, `/test`, database-backed
theme and prompt history, and frontend performance & polish — moved to [CHANGELOG.md](../CHANGELOG.md).

## Next

### Model setup follow-ups

Finish the setup experience through shared logic and thin local/remote adapters:

- Relabel the llama.cpp kind as **OpenAI-compatible**, retaining the stored
  `llamacpp` value. Offer server presets that choose the initial detection probe.
- Extend context detection to LiteLLM `/model/info` (`max_input_tokens`) and
  OpenRouter `/models` (`context_length`).
- Run discovery automatically on first use (manual discovery supports both hosts); identify
  server versions and prompt for credentials when a discovered server returns 401.
  Select sensible primary and fast defaults when nothing is configured.
- Extend model details with default sampling, embedding/FIM capabilities, CPU
  spill information and provider tokenization. Surface size, quantization and
  loaded state, hide irrelevant capability toggles, and exclude embedding-only
  models from the chat picker.
- Add an optional **Test model** action for structured/streamed tool calls,
  time to first token and tokens/sec. When a model rejects tools, fall back to
  plain chat with a notice and remember that capability per server + model.
- Extend detection beyond the model editor: run it when adding a server, allow
  server/workspace re-runs, and show the current probe and fallback source.
  Offer a current → detected review with Apply selected / Apply all / Dismiss;
  manually configured values are unchecked by default.
- Detect project type to suggest the default `/test` command and linters.
- Show only the model name unless duplicate names need an `@ host` suffix.

### Auto-compaction engine

Activate the saved threshold (85% by default, with per-model overrides and
an explicit disable option) in the shared agent loop. Check before every model
request, including tool continuations within a user turn. Compact at safe
boundaries after outstanding tool-call/result pairs, without interrupting streams.

Use the detected/configured context limit and treat the threshold as a ceiling:
compact earlier when the remaining space cannot fit the reserved response budget
or compaction request, especially on small context windows. Count instructions,
tool schemas, history and tool results using provider tokenization where available
and a conservative estimate otherwise.

Use the fast model or primary fallback, retain original history in the database,
and persist the summary used for subsequent requests. Support both local and
remote runs. Until the engine is active, keep the UI clear that the threshold is
configured but auto-compaction is not yet running.

### Ephemeral chat without a project

With no project open the composer is disabled today ("Open a project to start a
session"). Later: allow a plain, non-coding chat there — no tools and no file
access — in a session that isn't tied to a project, kept or discarded on close.

### Git over SSH in the Docker image

Git (status, commit, pull, push, sync) runs through the bridge in both local and remote
mode, with `GIT_TERMINAL_PROMPT=0` and `ssh -o BatchMode=yes`, so it uses whatever SSH
setup the bridge's machine has. A natively run bridge already works with the host's
`~/.ssh` and ssh-agent; the bridge in the Docker image has no `ssh` client, agent, or
SSH config. Make it work there without private keys ever entering the container:

- Install `openssh-client` in the image.
- Opt-in ssh-agent forwarding in compose (Docker Desktop's
  `/run/host-services/ssh-auth.sock`; the host's `$SSH_AUTH_SOCK` on Linux and podman,
  with matching quadlet `Volume=`/`Environment=` lines).
- Mount `~/.ssh/config`, `known_hosts` and the `*.pub` files read-only; the entrypoint
  copies them into `/root/.ssh` with the permissions ssh requires, so host aliases and
  `IdentitiesOnly` keys resolve through the agent.
- First contact with an unknown host: `StrictHostKeyChecking=accept-new` in the
  bridge's `GIT_SSH_COMMAND`, or stay strict and rely on the mounted `known_hosts`.
- Plain-language errors for `Permission denied (publickey)` and
  `Host key verification failed` (load the key into the agent / no agent forwarded).
- README: git over SSH for native vs Docker, plus HTTPS with a credential helper or
  token as the alternative.

### Follow-ups from the hardening sequence

Small items the review rounds left open, plus the manual checks nobody has run yet.

- **Bridge process cleanup:** kill detached PTY sessions after an idle timeout (a lost
  `Kill` after logout or on a dying socket otherwise leaves a shell until bridge restart);
  SIGHUP first for PTY sessions, SIGKILL after a grace period (on bash hosts a closed
  dock or bridge shutdown leaves the shell's background jobs running); shutdown should
  honour the guard's delayed SIGKILL. Windows code paths have never been compiled.
- **Bridge HTTP edges:** no body-read / response-write timeout (a stalled client holds
  a connection permit); HTTP/1.0 `keep-alive` clients bypass `Connection: close`;
  half-closing clients get no response; the 16 MiB WebSocket message limit is untested;
  `respond` copies every body; unbounded fallback error-body length in
  `parse_bridge_response`.
- **Frontend:** the terminal buffer doesn't cap a single unterminated line; the
  composer's keydown ignores IME composition (`isComposing`); rejecting a background
  edit restored from a backup doesn't restore the editor; `/model` with no argument
  doesn't mark the active model and `/model default` doesn't reset the override;
  remaining model-picker polish.
- **Providers:** base URLs with a query string or fragment break endpoint joining.
- **Tests:** missing-header rejection on register/login/logout; the
  `get_or_create_secret` race; a valid-token case in the backend expiry test;
  empty search query → 400; component-test helper `click_text` matches `class_name`
  exactly; fake-backend deletes don't cascade; `cargo test -p openwebide-frontend`
  without `--lib` doesn't build natively.
- **Bundle:** `data-wasm-opt="z"` cut the raw WASM ~3% but gzip only ~0.2% and cold
  load didn't improve — keep or revert.
- **Manual verification pass:** first-paint theme and both themes visually; the
  terminal against a real PTY (colours, `\r` progress, dock hide/show, cwd); streaming
  and reconnect against a real model; resume after a mid-run reload with a slow model;
  pending edits across two browsers; the Docker image end to end (browser terminal, git
  panel, WebSocket chat with no SSE, `OPENWEBIDE_BRIDGE=0` fallback); a phone on the
  LAN; podman/systemd; editor IME, paste and caret behaviour.

### File tree: context menus

Add right-click menus for files and folders, with the same actions in local and
remote projects through shared workspace and Git facades:

- File actions: create, rename, move, copy path and delete, with confirmation for
  destructive operations and clear handling of open or dirty editor tabs.
- Git actions: add/track, ignore, stage, unstage and revert changes. Show actions
  appropriate to the selected file's status and support folders where applicable.
- Chat TUI / agent actions: explain or summarize a file/folder, review changes,
  and similar shortcuts. Inject an editable prompt with the selected paths into
  the chat composer so the user can review and send it.
- Make menus keyboard-accessible and usable with a touch-friendly alternative.

### File tree: Explorer / Changes mode

Today's file tree (`frontend/src/components/file_tree.rs`) is a single
"Explorer" view — the full project tree with git status badges inline. Add a
toggle between **Explorer** (unchanged) and **Changes** (only files with a
git status, badges still shown), for jumping straight to what's dirty
without scrolling a large tree.

### Code intelligence: in-browser WASM linters & LSP

Run lightweight WebAssembly linters directly in the browser for instant diagnostics, with no
language runtimes on the host, and optionally bridge to host language servers.

Bring real-time code intelligence (syntax errors, lint squiggles, tooltips,
autocomplete) into the editor while keeping the core diagnostics engine
100% shared between Remote and Local mode:

- Universal in-browser WASM linters running in a Web Worker against the
  active editor buffer: `ruff-wasm` (Python), `oxc-wasm`/`biome-wasm`
  (JS/TS), `syn`/`rustc_lexer` (Rust), `serde_json`/`toml` (configs) — all
  producing a shared `Diagnostic` struct for the editor's squiggle overlay.
- Progressive-enhancement host LSP multiplexed over the Phase 11 bridge
  (`rust-analyzer`, `pyright`, `vtsls`) for cross-file go-to-definition,
  hover, and autocomplete when a host toolchain is available.
- One diagnostics UI regardless of whether a diagnostic came from the
  in-browser linter or a remote host LSP.

### Process execution: MCP client & headless browser

The rest of the Phase 11 bridge work that
isn't built yet — the bridge's PTY sessions, `run_command` tool, and
terminal pane are done and in the changelog, but:

- **Model Context Protocol (MCP) client:** the bridge spawning configured
  MCP servers (`~/.openwebide/mcp.json` / `.openwebide/mcp.json`) as host
  child processes over `stdio`, translating `tools/list` into the agent's
  `ToolDefinition` schema, and forwarding `tools/call`.
- **Deferred tool loading (`tool_search`):** once MCP servers push the tool count up, keep the core
  file/search/shell tools always loaded and, when the remaining schemas would exceed a share of the
  connection's context limit, send only their names plus a `tool_search` tool (keyword or `select:`
  lookup) that loads the matching full definitions for the next turn. Off below the threshold, so
  small tool sets and small-context local models pay nothing extra.
- **Tool budget for small-context models:** measure the token cost of the tool schemas, tighten their
  descriptions, and let each connection pick which tools it sends (or none, for chat-only use) — on
  4k-context local models a dozen schemas can eat a large share of the window, and those models are
  the least likely to use `tool_search` well.
- **Headless browser via Chrome DevTools Protocol (CDP):** the bridge
  attaching to a host or sidecar Chrome/Chromium instance to give the agent
  `browser_navigate`/`browser_screenshot`/`browser_click`/`browser_type`/
  `browser_console_logs` tools, failing open to `fetch_web_page` when no CDP
  browser is reachable.

### Fully fleshed-out TUI

The chat/TUI surface grows into a full agent workspace. In priority order:

1. **Checkpoints & rewind** — snapshot the files a turn touches; "rewind to
   here" restores both the files and the conversation to that point.
2. **Per-run changes panel** — every file the run touched in one view, with
   accept/reject per file or per hunk, and editor gutter markers for pending
   agent edits.
3. **Context visibility** — `/context` shows a breakdown bar (system prompt,
   files, tool output, history), alongside the auto-compaction engine above.
4. **`@`-mentions** (`@file`, `@folder`, `@diff`, with autocomplete) and
   drag/drop or paste of images for vision models.
5. **Message queue & steering** — type while the agent runs; queue the next
   prompt or interrupt with guidance. Also edit-and-resend and forking from
   an earlier prompt.
6. **Browser notifications** when a run finishes or needs approval (pairs
   with the phone layout).
7. **Todo/plan panel** — an agent-maintained checklist via a `todo_write`
   tool, pinned above the composer.
8. **Reasoning polish** — a live timer and token count while the model
   thinks, and a collapsed "Thought for 3.2s · 1.4k tokens" summary
   (extends the inline `<think>` blocks and streamed provider reasoning).
9. **Tool-step polish** — a running spinner with elapsed time, durations,
   show-more for long output, ANSI colors, a copy button, and a per-turn
   summary line ("5 tools · 2 files changed · 12.3s").
10. **Command palette** (`Ctrl+Shift+P`) and a keyboard-shortcut overlay.
11. **Session management** — search, pin/archive, fast-model auto-titles,
    and export to Markdown.
12. **Sub-agents** — a `task` tool that spawns child agents with their own
    context, shown as nested collapsible runs with status, elapsed time,
    tokens and tool count, runnable in parallel.

Build on the existing fast-model selection and primary fallback, WebSocket
streaming, and frontend state stores.

### Mobile support & collapsible tool windows

- **Tool windows:** every panel (file tree, editor, chat/TUI, terminal,
  git/diff, search, …) becomes a collapsible tool window docked in a tabbed
  side strip, like JetBrains Rider: click a tab to show or hide it, drag or
  pin it to a side, with the layout persisted per user in the database
  settings (per AGENTS.md — no localStorage).
- **Phone layout:** the TUI chat is the whole app, like the Claude or Codex
  mobile apps — a full-screen chat stream and composer, statusline,
  approvals, and model/approval-mode dropdowns; the other panels open as
  full-screen sheets (file viewer, diffs, terminal) instead of side by side.
- Responsive breakpoints pick the layout automatically, with a manual
  override; touch-friendly targets; works over the LAN through the bridge
  (the phone/LAN access the hardening sequence keeps working).

The existing per-feature state stores make layouts swappable. Saved panel widths
already fit the viewport.

## Later

### Multi-user

Only needed once more than one account can exist (today registration closes
after the first account):

- **Admin-only writes** for connections and system prompts (they stay
  shared, admin-owned), and admin-only `browse` and remote-project paths
  (`require_admin`).
- **Ownership columns** where rows are per user, and session-data store
  methods that take the `user_id` (or an owned-session token) instead of
  relying on every call site to check.
- **Registration setup token** with a loopback-only default when none is
  set (a Spin variable; verify Spin sets `spin-client-addr`).
- **Per-user bridge terminals:** sessions owned by the `user_id` in the
  bridge token, so one user can't list, attach to or kill another's shells;
  and the bridge's acting-user header limited to the connection's own user.

### Installable PWA & HTTPS

- **PWA:** a web app manifest (name, icons, `display: standalone`, theme
  colors from the design tokens) and a small JavaScript service worker
  (copied in by Trunk) that caches only the app shell (wasm/js/css) for a
  fast launch, never caches `/api` or bridge traffic, shows a "can't reach
  the server" screen when offline, and busts its cache per build.
- **HTTPS is required** — browsers only allow install and service workers
  on secure origins (localhost excepted) — and then the bridge must be
  `wss://` too, since an https page can't open `ws://`.
- **Default design:** the bridge sits behind the same HTTPS proxy as the app
  at the same-origin path `/bridge` (`wss://<host>/bridge`), with the bridge
  bound to `127.0.0.1` only (no LAN exposure). The frontend defaults the
  bridge URL to `wss://<same host>/bridge` when the page is served over
  https. Spin can't proxy WebSockets, so this needs a proxy in front of it
  rather than living in the Spin app.
- **Tailscale path:** a one-time tailnet admin toggle (MagicDNS + HTTPS
  Certificates — manual; the app detects the certificate failure and points
  to the toggle), then `tailscale serve --bg --https=443
  http://127.0.0.1:3000` and `--set-path /bridge http://127.0.0.1:3001`.
  Automation: an opt-in bridge `--tailscale-serve` flag that sets up both
  routes at startup and prints the URL; for Docker, an optional Tailscale
  sidecar in `docker-compose.yml` with a checked-in serve config (the user
  supplies only an auth key).
- **Alternative:** Caddy with an internal CA. Built-in bridge TLS
  (`--tls-cert`/`--tls-key` via tokio-rustls) only if a need appears.
- **Optional:** Web Push for "run finished / needs approval" (installed
  PWAs, including iOS 16.4+), tied to the TUI notifications item.

### Sandboxed tool execution (optional)

The VFS already confines the file tools (`read_file`, `write_file`,
`list_dir`, `search`) to the working directory, but `run_command` and the
git tools spawn real host processes as the user, which the VFS doesn't
cover. Approval modes now allow commands to run without asking. Offer an
opt-in mode that runs the agent's tool
executor in a container or as a restricted user with only the project folder
mounted and optionally no network, while the user's own terminal keeps full
access. Off by default; recommended alongside YOLO mode. Use the existing
shared tool-execution trait.

### Productionization & public release (1.0)

The milestone that marks **1.0** — the first version tagged for public use. A hygiene and packaging pass once the hardening sequence, refactors and the main Next features have
landed — before sharing the repo publicly.

- **Repo hygiene:** remove cruft and unused artifacts (stray scratch files, dead code, stale specs
  and docs, unused dependencies and features, leftover config), make sure `.gitignore` covers build
  and tool output, and that CI enforces fmt/clippy/tests on every crate.
- **Setup story:** a quick start that works in minutes — pull an image and point it at a model
  server — plus bare-metal, Docker/Podman and Tailscale guides, a configuration reference (flags,
  Spin variables, settings), upgrade/migration notes and troubleshooting.
- **Documentation & architecture diagrams** (Mermaid in `docs/`, rendered on GitHub, updated in the
  same PR as the code they describe):
  - Architecture overview: the browser app (Leptos → wasm32-unknown-unknown), the Spin backend
    (wasm32-wasip2) with SQLite, the native `openwebide-bridge` daemon, the model servers, and the
    shared crates (`core`, `llm`, `agent`, `storage`, `auth`) — which binary each compiles into and
    what runs where in Remote vs Local mode.
  - Transport map: REST + SSE to the backend, the multiplexed bridge WebSocket (hello/auth,
    terminal, agent runs), the SSE fallback, and the HTTPS proxy / same-origin `/bridge` path.
  - Request-flow sequence diagrams: a prompt from the composer through the transport to the agent
    loop, the provider call and streamed tokens back, a tool call through the approval gate
    (`ApprovalMode`, single-use decisions) into the tool executor and the VFS (`HostFsVfs` /
    `BrowserFsaVfs` / `MemoryVfs`, path confinement), and results/diffs back to the UI and SQLite.
  - Data model and security model (trust boundaries — model output is untrusted — and what each
    check protects).
- **Install without cloning:**
  - GitHub Actions building multi-arch images (`linux/amd64`, `linux/arm64`) and publishing to
    GHCR only (`ghcr.io/<owner>/open-webide`, public package linked to the repo), tagged by SemVer
    plus `latest`. Auth via the built-in `GITHUB_TOKEN` (`packages: write`) — no extra accounts or
    secrets. Build each arch on a native runner (`ubuntu-24.04-arm` for arm64) instead of QEMU, then
    merge into one multi-arch manifest.
  - A published `docker-compose.yml` and Podman quadlet (`.image` + `.container`) that reference
    the registry image, so users download one file and start it.
  - The bridge shipped inside the image (sequence step 47) and as prebuilt release binaries for
    macOS/Linux (amd64/arm64) for laptop-companion use.
  - SemVer releases with release notes generated from `CHANGELOG.md`; this item ships as `v1.0.0`
    (the workspace is `0.1.0` until then), and `[Unreleased]` in the changelog becomes `[1.0.0]`.

### Offline & error-state recovery

- Frontend heartbeat to `/api/health` with exponential backoff reconnection.
- Preserving unsaved editor state and draft prompts across connection dropouts.
- Graceful re-authorization flow for local File System Access API directory handles.

### Parking lot

Ideas without a phase yet:

- Multi-model comparison for a single prompt (an Open WebUI classic).
- Bridge child-process environment allow-list with `--pass-env` — parked until the bridge
  is shared; commands inherit the user's environment on purpose (cloud creds, toolchains).
- Private-address egress blocking / an explicit outbound allow-list for the backend —
  parked; conflicts with LAN model hosts and LAN web fetch. Would also need resolve-then-pin
  to stop a hostname rebinding to the metadata address.

## Explicitly out of scope

- **Model installation and lifecycle management:** Pulling, downloading, or
  deleting model weights on disk (e.g. `ollama pull`, GGUF management). Open
  WebIDE is an IDE and agentic client runtime, not a model engine manager.
  Users deploy their own inference engines (Ollama, llama.cpp in Podman
  quadlets, or OpenAI-compatible endpoints) and point Open WebIDE at them via
  connections.
