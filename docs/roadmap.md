# Roadmap

What's left, grouped by how soon it's coming: **Next** (queued up), **Later**
(planned, not yet started). Finished work —
phases 1 through 14, telemetry, hardening, streaming, `/test`, database-backed
theme and prompt history, and frontend performance & polish — moved to [CHANGELOG.md](../CHANGELOG.md).

## Next

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

### Remaining manual verification

The model setup and code hardening follow-ups are implemented. Native and browser
contracts cover both modes, failures, stale results and fallbacks. Docker checks
cover authenticated WebSocket PTYs, real-model streaming, remote file writes,
host-owned Git workspaces, supervisor shutdown and bridge-disabled SSE fallback.
[Reload recovery](reload-recovery.md) covers mid-run and multi-window checks, plus
the user-confirmed restoration of an OS-picked local folder.

The following checks still require hands-on device or environment testing:

- First-paint theme and both themes visually; terminal dock hide/show.
- Docker browser terminal and Files/Changes interactions.
- A phone on the LAN; podman/systemd.
- Editor IME composition, paste and caret behaviour with a real input method.
- Windows runtime process cleanup. The Windows adapter compiles without TLS locally;
  the Windows CI job checks the full TLS build.

### Full code editor: editing, structure and navigation

Build out the existing syntax-highlighted Edit view into a daily-use code editor.
Keep numbered Inline/Split diffs, previews for supported formats (Markdown/images/PDF
and plain-text documents),
including Markdown gutters on changed blocks/items/rows and inline prose differences for Git HEAD and pending agent edits,
Find, pending-edit review and agent editor
context intact. Editing must work without a host language server in both modes.

The shared Rust document/transaction engine, grouped undo/redo, per-file/project
history, indentation/EditorConfig controls, block-aware Enter and paired typing
are in place. Line movement/duplication/deletion, snippet duplication, line/block
comments, explicit indentation-matching paste, selected-line reindent, folding,
reading/navigation and Find/Replace use the same engine. See [editor controls](editor.md). Continue with:

- **Reliable edits and history:** build on grouped transactions and per-document history
  with caret/selection/scroll restoration now retained across file, project and view
  switches. Explicit EditorConfig newline,
  line-ending and trailing-whitespace save policies are undoable; finish real
  input-method/clipboard verification, preserving Unicode and LF/CRLF without
  rewriting unrelated text.
- **Syntax-aware editing:** the shared incremental parser now has extensible grammar
  providers and fold descriptors for Rust, TypeScript/TSX, JavaScript/JSX, Python,
  Java, C#, C++, PHP, Shell, C, Go, HTML and CSS. Native/browser contracts cover
  parsed folds, incremental Unicode/CRLF updates, cancellation and size fallbacks.
  HTML script/style bodies now use separate incremental JavaScript/CSS parsers
  with full-file fold coordinates, declared-type selection and shared limits.
  Paired typing/deletion, Enter, selected-line reindent, line/block comments,
  structural selection expansion and bracket navigation now consume parser-backed
  contexts, including template interpolation and HTML embedded JavaScript/CSS.
  Selection expansion uses validated named syntax-node ranges and retains shrink
  history and direction. Edit highlighting now uses cached providers for grammar
  classifications, literals/comments, template interpolation and embedded bodies.
  Inline/Split diffs, recovery reviews, Git previews and chat diff previews now
  share full-document syntax paint while retaining word-change highlights.
  PHP heredoc/nowdoc and shell heredoc contexts now protect text while exposing
  executable interpolation, including PHP braces and nested shell string wrappers.
  Shared contracts also cover Python and C# interpolation and nested literals.
  Fold, structural-command and highlight consumers now share one immutable,
  source-bound preparation per document/tab width, with cancellation and stale
  account/read/source/indentation guards. This preparation now runs in a dedicated
  Rust/WASM worker, with coalesced requests, validated replies, bounded shared LRU
  retention and synchronous/lexical fallbacks. Native/browser contracts and the
  built-worker Chrome check cover providers, incremental Unicode/CRLF, stale
  scopes, transport failure, size limits and PWA asset inclusion. Finish viewport
  paint and end-to-end latency/memory measurements below.
  Lexical JSON/TOML/YAML/SQL/Markdown paint also uses this cache and worker,
  preserving multiline state and discarding partial rows on cancellation.
  Reindent preserves
  existing Python block depth; language formatting remains in Code intelligence.

- **Selections and files:** the shared document now normalizes overlapping selections,
  selects next/all occurrences, maps column selections using graphemes and tab stops,
  expands/shrinks selections and replaces multiple ranges in one grouped transaction.
  The editor facade retains secondary selections during commands and native input.
  IME previews change the primary range, then commit all ranges in one undo step;
  cancellation and stale file/project/account events preserve document state.
  Copy/cut use full source selections and preserve multiline fragment boundaries
  through clipboard metadata; foreign or stripped metadata falls back to matching
  clipboard lines or repeating the complete text in one transaction.
  Occurrence/vertical/expand/shrink
  controls, Alt-click/column gestures and secondary selection paint share the same
  facade. Arrow movement retains all cursors with grapheme, word, logical-line and
  sticky-column behavior. Wrapped Up/Down now use measured visual rows, retained
  pixel goals and soft-wrap caret affinity, skipping folds and rejecting stale
  source/projection measurements. Pending-paint motion now queues in order and
  flushes before edits, IME and clipboard actions, with bounded retries and stale
  account/file/projection guards. Long wrapped lines now use shared Unicode
  indexing and bounded DOM searches to prepare only neighboring visual rows,
  reusing measurements across cursors. Finish real-device input/clipboard
  verification for multiple cursors. File navigation now
  retains independent dirty buffers, history, caret and scroll state in memory,
  with protected reads and filesystem mutation guards. File tabs share selected-tab
  styling, keyboard navigation and guarded close/discard controls. Validated recovery
  snapshots, revision-guarded user-scoped SQLite/API storage, typed frontend
  transport and active/hidden buffer collection are in place. Transactional
  hydration and shared disk reconciliation preserve baselines and reject stale
  editor activity. The shared frontend facade now loads tabs/selected files and
  drafts and serializes debounced database saves. Failed writes retain drafts;
  explicit restore/keep choices resolve database revision conflicts. Disk checks
  block unsafe host saves and use the existing folder-access flow. Recovered-file
  reviews reuse inline diffs and offer guarded reload or explicit draft saves,
  including missing/empty file recreation. A disposable Spin/SQLite and Chrome
  check verifies real browser edits, autosave, server restarts and reload/new-window
  selected-tab/draft restoration in both modes; local recovery is checked without
  a native folder handle. Finish permission/error regression coverage and native
  folder-permission/real-device verification.
  Coordinate draft/reload recovery with the existing Offline & error-state recovery
  item instead of implementing separate persistence.

**Rust/WebAssembly architecture:** keep the editor in Rust/Leptos compiled to WASM;
do not embed CodeMirror, Monaco or another substantial JavaScript editor client.
Those editors are feature references only. Build reusable Rust document, selection,
edit-transaction, undo/history, command and fold-range primitives, with one Leptos
editor component. Evaluate WASM-compatible Rust crates for text storage and syntax
parsing before choosing dependencies. [Native/browser storage measurements](editor-performance.md)
compare the current document with Crop/Ropey edits, display materialization and
UTF-16 indexing; retain current production storage until viewport/worker work
changes its access pattern. Keep browser glue thin: DOM events, input/IME,
selection, clipboard, measurements and worker transport; editing policy and algorithms
belong in Rust. Preserve native browser input behavior where possible, and evaluate
a richer view for multiple selections beyond the current projected textarea/paint
surface.

Workspace reads/writes, settings and review policy stay in shared facades. Save user
preferences in database settings, never localStorage. Verify touch/IME/accessibility,
Unicode/caret mapping, incremental parsing, large-file responsiveness, theme integration
and PWA loading. Benchmark viewport rendering and establish large-file fallbacks before
claiming completion; run behavioral contracts in both modes.

Completion, diagnostics, hover, symbol navigation, rename, formatting and code actions
belong to the following Code intelligence item, using the editor's extension points.
Minimap, Vim/Emacs emulation and similar extras are optional later work.

Research references: [VS Code editing](https://code.visualstudio.com/docs/editing/codebasics),
[CodeMirror baseline](https://codemirror.net/examples/basic/),
[CodeMirror Tab accessibility](https://codemirror.net/examples/tab/),
[CodeMirror features](https://codemirror.net/),
[Monaco support/architecture](https://github.com/microsoft/monaco-editor),
and [EditorConfig](https://editorconfig.org/).

### Code intelligence: in-browser WASM linters & LSP

Run lightweight WebAssembly linters directly in the browser for instant diagnostics, with no
language runtimes on the host, and optionally bridge to host language servers.

Build on the Full code editor component and its document/selection/extension APIs.
Bring real-time code intelligence (syntax errors, lint squiggles, tooltips,
autocomplete) into the editor while keeping the core diagnostics engine
100% shared between Remote and Local mode:

- Universal in-browser WASM linters running in a Web Worker against the
  active editor buffer: `ruff-wasm` (Python), `oxc-wasm`/`biome-wasm`
  (JS/TS), `syn`/`rustc_lexer` (Rust), `serde_json`/`toml` (configs) — all
  producing a shared `Diagnostic` struct for the editor's squiggle overlay.
- Progressive-enhancement host LSP multiplexed over the Phase 11 bridge
  (`rust-analyzer`, `pyright`, `vtsls`) for cross-file go-to-definition,
  hover, autocomplete, document symbols/outline, references, rename, code actions
  and document/selection formatting when a host toolchain is available.
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

### Web Push (optional)

- Notify installed apps when a run finishes or needs approval, with user-scoped
  subscriptions and permission controls. This is separate from PWA installation.

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
