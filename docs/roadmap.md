# Roadmap

What's left, grouped by how soon it's coming: **Now** (in flight), **Next**
(queued up after it), **Later** (planned, not yet started). Finished work —
phases 1 through 14, the TUI telemetry meters, and the per-connection context
limit — moved to [CHANGELOG.md](../CHANGELOG.md).

## Now

### Security hardening, bridge robustness & agent streaming

A sequence of correctness and hardening passes across every crate, plus
switching the agent loop from one-shot SSE requests to a single multiplexed
bridge WebSocket connection:

- **Security hardening:** default-deny tool approval policy, bridge
  Host/Origin/CORS checks, path-confinement and git argument-injection
  fixes, markdown/SVG XSS closure (safe raw HTML still renders), sandboxed
  raw-file responses, request/response size bounds, and auth correctness.
- **Sign-in:** an HttpOnly cookie session instead of a token in
  localStorage (with a CSRF header), failed-login throttling, and logout
  that signs out every device.
- **Bridge robustness & auth:** an HTTP server on `hyper`, process lifecycle
  (kill/reap/shutdown), output delivery ordering, a shared secret between
  the backend and the bridge, and a `hello` handshake with short-lived
  bridge tokens — phone/LAN access keeps working with no extra setup.
- **Agent streaming over the bridge WebSocket:** provider and agent-loop
  streaming of tool-call turns, a typed run protocol over one multiplexed
  bridge connection, and a frontend fallback to SSE when the bridge is
  unavailable; afterwards one event type for SSE and WebSocket, resumable
  local-mode runs, and the bridge bundled into the Docker image.
- **Frontend structure & tests:** the `App` component split into
  per-feature state stores, and a component-test harness running in
  headless Chrome in CI.
- **Agent quality:** typed tool arguments, approval cards that show the full
  diff, Stop that interrupts a running command, search that can opt into
  ignored folders, truncated or cut-off replies kept with a visible marker,
  model reasoning surfaced, and diffs that show line-ending changes.
- **Cleanup:** backend and bridge refactors (typed errors, module
  structure), dead-code removal, and selected pedantic clippy lints enforced
  in CI.

## Next

### Frontend performance & polish

Follow-ups that build on the per-feature state stores from the hardening
sequence:

- **Performance first:** stop re-rendering the whole conversation on every
  streamed token (per-message signals / a keyed `reactive_stores` store) and
  stop re-highlighting the whole file on every keystroke (highlight only the
  visible window, or debounce to the next animation frame); a terminal line
  buffer instead of re-rendering all output.
- Design tokens and buttons: settle the canonical token names (rename the CSS
  to match AGENTS.md or amend AGENTS.md), add the missing tokens, move to
  `color-mix`, and merge `ui::Button` into `.btn`.
- `data-wasm-opt="z"` for a smaller bundle (measure before/after).
- A 250 ms debounce on search; parallel initial load and a model-refetch
  memo; ANSI and scroll fixes in the terminal; CRLF and multi-line `data:`
  handling in the SSE reader.
- IndexedDB connection caching, and deleting a local project's directory
  handle when the project is deleted.
- Keyboard accessibility for modals; smaller duplications (recent-projects
  filter, telemetry formatting on `SessionTelemetry`) and idiom cleanups
  (`.with()` instead of cloning `.get()`, typed enums for stringly values,
  one guarded `Send` wrapper).
- Keep the terminal dock mounted so shells survive a toggle; default the
  terminal's working directory to the project folder.
- Persist accept/reject on agent edits so pending edits survive a reload.

### Secondary "fast model" per connection

A second, smaller/faster model configurable per user or per connection
(the same idea as Qwen Code's second model), used for background work that
doesn't need the main model's quality: approval classification (see below),
session naming, and autocomplete/suggestion-style tasks. Falls back to the
main model when none is configured. Nothing here exists yet — there is no
secondary-model setting anywhere in `Connection`, `user_settings`, or the
providers today.

### Approval modes

A real CLI/TUI-style approval mode selector instead of the current
`Alt+A` per-session "always" choice. This builds on the `ApprovalMode`
enum in `openwebide_agent::policy` (which currently supports `Default`
and `AlwaysForSession` — the latter explicitly never auto-approving
`run_command`):

- **Default** — prompt for every gated tool call (today's behavior).
- **Auto-accept edits** — file edits are auto-approved; shell commands still
  prompt.
- **Auto with a classifier** — the fast model (above) judges whether a given
  call is safe to auto-approve.
- **YOLO** — approve everything, including `run_command`, no prompts (like Qwen Code's YOLO mode).

Cycle modes with `Shift+Tab`; show the current mode in the TUI statusline
(replacing the current `[ALWAYS]` segment), which is also clickable and
opens a dropdown to pick a mode directly. Depends on the fast model
(classifier mode).

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

### Server / model settings split

Split today's single `Connection` (kind + name + URL + model) into
**servers** and **per-model settings**, and generalize the llama.cpp kind:

- **Servers:** kind, URL, an optional API key/extra headers, and a request
  timeout. No name field — the label is derived from the host (e.g.
  `ollama @ 192.168.1.20`). Ollama servers also get a `keep_alive` setting.
- Rename the llama.cpp connection kind to **OpenAI-compatible**, with
  llama.cpp kept as a preset; the same kind covers LM Studio, llama-swap,
  vLLM, SGLang, MLX (`mlx_lm.server`), KoboldCpp, TabbyAPI, LocalAI, Jan,
  llamafile, Docker Model Runner, Lemonade, text-generation-webui, and
  LiteLLM. Ollama keeps its native provider (needed for `num_ctx`,
  `keep_alive`, and capability detection).
- **Context-limit and capability detection:** the OpenAI-compatible provider
  runs a short chain of server-specific probes — llama.cpp `/props`
  (`n_ctx`), vLLM `/v1/models` (`max_model_len`), LM Studio's model info
  (loaded/max context), SGLang's model info, KoboldCpp's true-max-context
  endpoint — then falls back to the configured value, then 4,096. The preset
  only picks which probe runs first; the server type is auto-detected where
  possible. If a server rejects tool calls, fall back to plain chat with a
  notice.
  - **Visible and re-runnable:** detection runs automatically when a server
    is added, and a **Detect** button on the server (and each model) re-runs
    it at any time. While it runs, the settings form shows a spinner with the
    probe currently being tried (e.g. "Checking /props…"); when it finishes
    it shows what was found and from where (e.g. "Context 32,768 · from
    llama.cpp /props · tools ✓"), or which fallback was used. Detected values
    are cached per server + model and never overwrite a value you set by
    hand.
- **Per-model settings** (optional overrides, keyed by server + model):
  context limit (moves here from the connection, see the per-connection
  context limit in the changelog), sampling (temperature, top_p, top_k,
  min_p, repeat penalty, seed), max output tokens, thinking on/off, and tool
  calling on/off.
- Auto-detect model capabilities where possible (Ollama's `POST /api/show`
  `capabilities`: tools, vision, thinking) to hide irrelevant toggles and
  fall back to plain chat for models that don't support tools.
- The UI shows just the model name, adding `@ host` only when two servers
  offer the same model name.
- **Principle (stated explicitly, applies to all of the above):** works out
  of the box with sensible defaults. Adding a server needs only a URL (kind
  auto-detected where possible); every setting is optional with a
  documented fallback; nothing blocks sending the first prompt.

This also reframes the secondary "fast model" above as a per-model setting
rather than a bare per-connection one.

The `core`/`llm` refactor happens together with this split, since it
reshapes the same code: provider deduplication (`resolve_model`,
`delta_stream`, `messages_wire`), a typed `ProviderError`, splitting the
core god-file and deduplicating its diff helpers, storage row-mapping
helpers, enums/`FromStr`/`thiserror` for `VfsError` and git status, and a
`MaybeSend` alias so the frontend can drop its `unsafe impl Send/Sync`.

### Auto-discovery & configuration

Discover and configure as much as possible so the first prompt works with no
setup. Builds on the server/model split and its context-limit detection.

- **Server discovery:** on first run (and on demand), probe `localhost` and
  the bridge host on default ports — Ollama 11434, llama.cpp/llama-swap 8080,
  LM Studio 1234, vLLM 8000, KoboldCpp 5001, TabbyAPI 5000, Jan 1337.
  Fingerprint the server type and version from its telltale endpoint
  (Ollama `/api/version`, llama.cpp `/props`, vLLM `/version`, …). A 401
  prompts for an API key instead of failing.
- **Model details:** capabilities (chat, tools, vision, thinking, embedding,
  fill-in-the-middle), size and quantization, the model's own default
  sampling settings, loaded-vs-cold state (Ollama `/api/ps`, including the
  context actually running and whether the model spilled to CPU), and exact
  token counts where the server can tokenize (llama.cpp `/tokenize`).
  Embedding-only models stay out of the chat picker.
- **Automatic defaults:** pick a main model and a fast model (the smallest
  tool-capable chat model) when none is configured.
- **Active test:** an optional "Test model" request checks structured and
  streamed tool calls and measures time to first token and tokens/sec.
- **Workspace & host (via the bridge):** project type → default `/test`
  command and linters; installed tools (`git`, `cargo`, `node`, `python`, …)
  → what the agent's system prompt says is available.
- **Review before apply:** discovery never changes settings silently. It
  shows a list of detected values that differ from the current ones —
  current → detected, with where each came from (e.g. "Context 4,096 →
  32,768 · llama.cpp /props") — each with a checkbox, plus Apply selected /
  Apply all / Dismiss. Values the user set by hand are unchecked by default.
  First-run discovery with nothing configured applies directly.
- **Visible and re-runnable:** every detection shows a spinner with the
  current probe, can be re-run at any time from the server, model or
  workspace settings, and caches its results.

### Fully fleshed-out TUI

The chat/TUI surface grows into a full agent workspace. In priority order:

1. **Checkpoints & rewind** — snapshot the files a turn touches; "rewind to
   here" restores both the files and the conversation to that point.
2. **Per-run changes panel** — every file the run touched in one view, with
   accept/reject per file or per hunk, and editor gutter markers for pending
   agent edits.
3. **Auto-compaction** — near the context limit, summarize older turns with
   the fast model; `/context` shows a breakdown bar (system prompt, files,
   tool output, history).
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
   (extends the inline `<think>` blocks and the hardening sequence's
   provider-reasoning step).
9. **Tool-step polish** — a running spinner with elapsed time, durations,
   show-more for long output, ANSI colors, a copy button, and a per-turn
   summary line ("5 tools · 2 files changed · 12.3s").
10. **Command palette** (`Ctrl+Shift+P`) and a keyboard-shortcut overlay.
11. **Session management** — search, pin/archive, fast-model auto-titles,
    and export to Markdown.
12. **Sub-agents** — a `task` tool that spawns child agents with their own
    context, shown as nested collapsible runs with status, elapsed time,
    tokens and tool count, runnable in parallel.

Dependencies: the fast model (auto-compaction, session auto-titles,
sub-agents), WebSocket streaming (sub-agents), and the frontend state-store
split from the hardening sequence for the UI-heavy items.

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

Best done after the frontend state-store split (per-feature stores make
layouts swappable). It subsumes the sequence's viewport clamp for saved panel
widths.

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
cover — today the user's approval is the sandbox (always-approve never
covers `run_command`). Once commands can run without asking (YOLO or other
auto-approve modes), offer an opt-in mode that runs the agent's tool
executor in a container or as a restricted user with only the project folder
mounted and optionally no network, while the user's own terminal keeps full
access. Off by default; recommended alongside YOLO mode. Depends on the
approval-modes item (and uses the tool-execution trait the bridge refactor
introduces).

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

## Explicitly out of scope

- **Model installation and lifecycle management:** Pulling, downloading, or
  deleting model weights on disk (e.g. `ollama pull`, GGUF management). Open
  WebIDE is an IDE and agentic client runtime, not a model engine manager.
  Users deploy their own inference engines (Ollama, llama.cpp in Podman
  quadlets, or OpenAI-compatible endpoints) and point Open WebIDE at them via
  connections.
