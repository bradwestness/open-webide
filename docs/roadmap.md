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
  fixes, auth correctness (secret rotation, clock skew), request/response
  size bounds, and markdown/SVG XSS closure.
- **Bridge robustness:** HTTP parsing resilience, process lifecycle
  (kill/reap/shutdown), output delivery ordering, and a `hello`
  auth handshake for the bridge connection.
- **Agent streaming over the bridge WebSocket:** provider and agent-loop
  streaming of tool-call turns, a typed run protocol over one multiplexed
  bridge connection, and a frontend fallback to SSE when the bridge is
  unavailable.

## Next

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
per-session single on/off flag (`always_approve_all` in `frontend/src/app.rs`,
set from the permission prompt's "always" choice):

- **Default** — prompt for every gated tool call (today's behavior).
- **Auto-accept edits** — file edits are auto-approved; shell commands still
  prompt.
- **Auto with a classifier** — the fast model (above) judges whether a given
  call is safe to auto-approve.
- **YOLO** — approve everything, no prompts (like Qwen Code's YOLO mode).

Cycle modes with `Shift+Tab`; show the current mode in the TUI statusline's
`[NORMAL]`/`[RUNNING]`/`[AWAITING]` segment, which is also clickable and
opens a dropdown to pick a mode directly. Depends on the fast model
(classifier mode) and on the default-deny approval-policy work in the
hardening entry above.

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

## Later

### Hardening, multi-arch distribution & releases

Production distribution and resilient offline handling for the complete
stack.

- **Automated multi-arch container releases:**
  - GitHub Actions CI/CD building multi-arch images (`linux/amd64` and
    `linux/arm64`) via QEMU/Buildx and publishing to GitHub Container
    Registry (`GHCR`).
  - SemVer release tagging and automated changelog generation.
  - Podman quadlet `.image` + `.container` systemd service definitions.
- **Offline & error state recovery:**
  - Frontend heartbeat to `/api/health` with exponential backoff
    reconnection.
  - Preserving unsaved editor state and draft prompts across connection
    dropouts.
  - Graceful re-authorization flow for local File System Access API
    directory handles.

### Parking lot

Ideas without a phase yet:

- Multi-model comparison for a single prompt (an Open WebUI classic).
- Conversation export (markdown).

## Explicitly out of scope

- **Model installation and lifecycle management:** Pulling, downloading, or
  deleting model weights on disk (e.g. `ollama pull`, GGUF management). Open
  WebIDE is an IDE and agentic client runtime, not a model engine manager.
  Users deploy their own inference engines (Ollama, llama.cpp in Podman
  quadlets, or OpenAI-compatible endpoints) and point Open WebIDE at them via
  connections.
