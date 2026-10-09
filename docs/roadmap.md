# Roadmap

What's left, grouped by how soon it's coming: **Next** (queued up), **Later**
(planned, not yet started). Finished work —
phases 1 through 14, telemetry, hardening, streaming, `/test`, database-backed
theme and prompt history, frontend performance & polish, app branding and the chat
welcome — moved to [CHANGELOG.md](../CHANGELOG.md).

## Next

### Remaining manual verification

The model setup and code hardening follow-ups are implemented. Native and browser
contracts cover both modes, failures, stale results and fallbacks. Chat and agent
reply budgets now use remaining context capacity per request unless explicitly
limited; compaction keeps a separate planning reserve. Docker checks
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

The everyday editing baseline is implemented. The goal remains open until the
performance, input/device and verification requirements below are finished.
Completed implementation details belong in [editor controls](editor.md),
[performance evidence](editor-performance.md) and the changelog; this section
tracks the remaining work rather than every optimization already shipped.

**Implemented baseline** (shared local/remote behavior, with automated coverage):

| Area | Available now |
| --- | --- |
| Editing and history | Tab/space and indent/outdent; block-aware Enter; paired typing/deletion; grouped undo/redo; line move/duplicate/delete; comments; selected-line reindent; indentation-matching paste; EditorConfig save policies. |
| Structure and languages | Syntax highlighting, parser-backed folding, bracket matching and structural navigation; extensible Rust/WASM language support including Rust, TypeScript/TSX, Python, JavaScript/JSX, Java, C#, C++, PHP, Shell, C, Go, HTML, CSS, JSON/JSONC, YAML/YML, TOML, INI/EditorConfig, XML ecosystem configs and Markdown with inline/fenced code. Pending and unsupported source stays plain. |
| Navigation and review | Find/Replace; line numbers; horizontal scrolling and linked split scrolling; Edit/Inline/Split diffs; supported previews and Markdown change gutters/word differences; pending-edit review and agent context. |
| Tabs, appearance and recovery | Tab context actions; five Monaspace families; texture healing and ligature toggles enabled by default; retained caret/selection/scroll and database-backed editor recovery. Real folder-permission recovery still needs device verification. |
| Selection and browser input | Multiple/rectangular selections and clipboard transactions; scoped native windows for eligible unwrapped views; automated Chromium composition and pointer checks. The recorded line-end caret bug is fixed and user-verified in the localhost PWA. |
| Preparation and ownership | Rust/WASM worker and cooperative fallback; retained source/token/structure allocations; bounded eligible unwrapped paragraph probes, exact overlap validation, exact carets from retained anchors/current painted coverage and conservative complete-layout fallback. Current-file progress is shown during preparation. Matching trusted font notifications retain current in-flight work. |

**Remaining implementation**:

- [ ] **Cold startup and native input:** finish bounded initial input for wrapped,
  nonuniform and unsupported long tabbed/bidirectional rows, and remove remaining
  initial full-source shaping. Eligible unwrapped uniform files now use bounded
  startup input regardless of the initial caret or long-row position. Long-row tab
  admission and paragraph preparation reuse immutable coordinate metadata;
  wrapped, nonuniform and unsupported
  rows still require complete layout. Finish cold
  wrapped input, touch pointer selection,
  source-owned caret/selection and complete document extents. Preserve composition
  mappings and complete-native fallback where a bounded view cannot be proved.
- [ ] **Tabbed, wrapped and bidirectional layout:** finish bounded preparation,
  bidirectional visual-run windows, fine long-row paint and incremental glyph
  measurement. Current tabbed/wrapped/bidi paths still use complete preparation;
  retained DOM nodes, canvas widths and approximate Rust advances do not establish
  exact browser geometry.
- [ ] **Incremental paragraph updates:** extend shifted suffix reuse to changed
  long-token and plain-run boundaries, and avoid repeated prefix segmentation for
  over-limit styled run tables. Unchanged styled suffixes now map exact source and
  glyph positions in bounded replay batches, validate incoming browser overlap,
  and freshly measure the final extent. Ordered coordinate queries share cluster
  traversal while preserving every old/new source-byte check. Styled paragraph
  anchor construction also shares traversal across ordered paint boundaries,
  omitting boundaries inside clusters as before. Shifted tabs retain
  fresh measurement. Validated retained
  probes now share rectangle allocations directly and compare each prefix
  run-boundary interval once. Probe target/commit lookup now visits only sparse
  anchors within the measured glyph range. Initial run-table
  construction and unsupported-boundary fallbacks still scan complete rows;
  capped metadata attempts now stop once their run budget is exceeded and retain
  that result within their bounded exact-source/style cache. Uncapped fallback
  construction still segments full rows.
- [ ] **Incremental syntax and structure:** finish larger retained-container reuse,
  warm semantic list assembly, paint-table iteration, shifted suffix metadata,
  final context assembly and bounded metadata publication,
  larger fenced-code workloads and changed-source plain row-table
  reconstruction/validation. Cooperative source-change comparison now
  shares the resumable UTF-8 comparison used by plain-row preparation; admission
  and changed row/index assembly still need bounded work. Resolved worker parser
  replacements reuse their validated spans only
  for the exact retained base allocation. Intersecting plain fallback rows retain
  raw-byte boundary validation. Fallback range/opaque-point checks now use ordered
  embedded-scope lookup. Unchanged embedded bodies retain relative fallback contexts;
  outer/new-body scanning now resumes across worker tasks, including long literal
  and comment bodies, YAML headers/scalars and speculative row replay. Final
  context/selection-list assembly remains. Grammar-free SQL plain-row preparation
  now yields within long rows while scanning, validating retained text and copying
  plain tokens; the browser fallback uses the same bounded job. Comparing retained
  plain sources now also resumes within that job's byte budget, preserving exact
  UTF-8 replacement boundaries and unchanged table reuse. Whole-row capacity
  allocation and final publication still need bounded
  work. The final bracket pass
  skips opaque
  regions and languages without brackets while preserving scope boundaries.
- [ ] **Source ownership and storage:** finish remaining external parser snapshots,
  changed-revision source comparisons, transport serialization, diff shaping and
  native-text materialization. Parser-free entry equality now uses the same
  resumable comparison and passes validated replacements to plain-row reuse;
  paint/transport comparisons still sit outside those jobs.
  Folded/bounded projection tables still assemble
  independently; retained row/coordinate tables copy on edits, changed guide values
  copy retained columns, and storage shifts suffix bytes/coordinates. Long boundary
  rows still scan for admission. Revisit measured storage candidates when the
  viewport/worker access pattern supports the change, preserving exact coordinates,
  immutable retained views and cancellation behavior.

**Remaining completion gates**:

- [ ] **Responsiveness and memory:** remove the remaining input, cold-paint, wrapped
  layout and process-memory stalls. Repeat admitted byte, row-count and long-line
  boundary workloads in both modes, including formerly unresponsive wrapped cases
  and Linux Chrome PSS. Require actual near-1-MiB String styling through load,
  scrolling and input, plus repeated startup-scroll, initial-shaping and beginning
  edit samples. Existing multi-second results do not satisfy this gate.
- [ ] **Exact geometry and fallbacks:** retain complete-renderer extent/anchor/hit
  comparisons, font/feature/whitespace matrices, Unicode/caret mapping and failed-proof
  fallback contracts. Current painted coverage and exact retained caret anchors avoid complete movement
  probes; unpainted sparse gaps and unsupported layouts still require
  complete-renderer fallback.
  Improvements must preserve source/account/project ownership,
  pending edits, themes, supported previews, agent context and both adapter contracts.
- [ ] **Reliable CI and release/PWA checks:** pass the complete native/WASI/WASM,
  platform, browser and release-app checks reliably. Repeat near-limit Linux
  readiness and the complete suites; a local run or one green checkpoint is insufficient.
  Current checkpoints and measured results are in [performance evidence](editor-performance.md).
- [ ] **Physical Chrome/Edge PWA input:** verify real input-method commit/cancel,
  Unicode and LF/CRLF undo/redo, multiple-cursor clipboard behavior and touch input
  without rewriting unrelated text. Automated CDP composition is supporting evidence,
  not physical input-method verification.
- [ ] **Folder permissions and recovery:** expand permission/error regressions and
  verify real local directory-handle permission loss/regrant and reload recovery.
  Existing held-write contracts cover failures and stale account/folder/bridge/project
  results, background saves and newer edits; the disposable recovery check has no
  native local folder handle. Coordinate with Offline & error-state recovery below.
- [ ] **Accessibility and integration:** verify keyboard focus/Tab escape, assistive
  technology, touch, theme integration and PWA loading across the completed editor,
  including the cold/fallback paths. File-specific input names, current Tab guidance
  and live Ctrl+M announcements have browser contracts in both modes. Release-app
  checks verify trusted forward/backward Tab escape and accessible input names with
  pending syntax, bounded native input and LF/CRLF sources. Physical assistive-
  technology verification remains. Evaluate whether the current projected native
  input and source-paint surface adequately supports the richer multiple-selection view.

**Rust/WebAssembly architecture:** keep the editor in Rust/Leptos compiled to WASM;
do not embed CodeMirror, Monaco or another substantial JavaScript editor client.
Those editors are feature references only. Build reusable Rust document, selection,
edit-transaction, undo/history, command and fold-range primitives, with one Leptos
editor component. Evaluate WASM-compatible Rust crates for text storage and syntax
parsing before choosing dependencies. [Native/browser storage measurements](editor-performance.md)
compare the current document with Crop/Ropey edits, display materialization and
UTF-16 indexing; retain current production storage until viewport/worker work
changes its access pattern.

Keep browser glue thin: DOM events, input/IME,
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

### Editor Git annotations

- GitLens-style editor annotations showing line authorship, commit details and
  history, with navigation to the relevant commit or diff.
- Use shared Git orchestration and the existing bridge adapters in both modes;
  guard asynchronous results against file, project and account changes.

### Test discovery, running and debugging

- Discover tests for supported languages and frameworks in both modes. Initially,
  expose inline **Run test** actions in remote projects, in the style of Rider/Visual
  Studio; local projects can show discovered tests with execution unavailable.
- In remote mode, run individual tests, suites or all project tests through the
  existing server-side bridge, with results and failure
  locations linked back to the editor. Stream runner stdout/stderr to the terminal,
  retaining output alongside structured test results. Define supported frameworks
  and runtime requirements explicitly.
- Assess remote debugger integration through Debug Adapter Protocol (DAP): the
  server-side bridge hosts debugger adapters, and the browser provides breakpoints,
  stepping, stack frames and variable inspection. Route debuggee output to the
  terminal. Add inline **Debug test** only for validated runner/debugger combinations,
  including test-process launch/attach, source mapping and clean session shutdown.
  See [DAP architecture](https://microsoft.github.io/debug-adapter-protocol/overview.html).
- **Initial scope exception:** test execution and debugging are remote-only at first.
  A local companion bridge is outside this feature's scope. Keep local execution and
  debugging parity as remaining work requiring a browser-compatible runtime; do not
  mark the overall feature complete when only remote execution ships. Source-based
  discovery uses `Workspace` in both modes; unavailable actions explain the runtime
  limitation.
- Keep discovery, action availability and result handling shared across modes;
  use thin runtime adapters for execution and explain missing capabilities.
  Coordinate editor integration with Code intelligence.

### Selection context menu and agent actions

- Right-click highlighted code to open a shared context menu with editor and
  agent actions, such as explain, refactor or generate tests.
- Carry the selected text, file path and line range into the agent request and
  pending-edit review; reject stale selections after document or project changes.
- Offer the same actions through keyboard and touch controls in both modes.


### Phone device verification

The compact app/editor rows, universal search, logo drawer, single status footer,
on-demand pane controls, grouped menus, responsive dialogs, nested file groups, automatic tree density, folder-only creation menus, middle-click tab closing, pointer-stable tab scrolling, tree controls and Git icons/counts are implemented in
both modes, with browser coverage for narrow layouts, focus restoration, keyboard
navigation, inline header search with viewport-bounded results, stable file-tab geometry across preview availability changes and loading locks, contained tab context-menu controls, non-overlapping tree disclosure touch targets, tooltip dismissal during pointer activation and scroll restoration, and review safeguards. Remaining checks need physical devices:

- Verify density, model/approval controls, activity-group touch controls and the
  compact context indicator on real phones in both modes.
- Check virtual-keyboard and IME behavior on iOS/Android so the composer/editor
  space and controls remain accessible as the keyboard opens and closes.
- Check safe-area spacing, the app drawer and Output sheet on real devices.

### Build, test and process output

Prioritize the existing terminal pane as a useful place to follow build, test and
other process output: readable streamed logs, separate runs, exit status, cancellation,
copy/search and file/line links back to the editor. Preserve output during reconnects
and keep runs scoped to their project/session. Use shared output behavior with thin
runtime adapters; local output is available only for browser-supported execution,
while host-native builds/tests initially run in remote mode.

- When a process starts, reveal output at a modest desktop height while honoring
  explicit hide/resize choices. Verify Output sheet focus/keyboard behavior on
  real devices.
- Keep running/failure status visible through the shared footer's Output indicator
  when the pane is hidden. Add output search, separate run histories and file/line
  links.

Prioritize idle output behavior, phone control overflow and activity-group density
on phones alongside the compact-layout work above. Verify both-mode output contracts and
mobile focus/keyboard behavior before considering these refinements complete.

A fully interactive terminal (cursor movement, direct keyboard input and shell/TUI
programs) is optional later work, pending a concrete need beyond process output.

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
- **Headless browser via Chrome DevTools Protocol (CDP):** the bridge
  attaching to a host or sidecar Chrome/Chromium instance to give the agent
  `browser_navigate`/`browser_screenshot`/`browser_click`/`browser_type`/
  `browser_console_logs` tools, failing open to `fetch_web_page` when no CDP
  browser is reachable.

## Later

### Agent questions and structured user replies

- Add an `AskUserQuestion`-style agent tool for clarification and choices during
  a run. Present concise questions, optional suggested answers and free-text
  replies; support a small related group of questions when useful.
- Persist pending questions and user replies in the database so they survive
  reconnects and device changes. Resume the requesting run with the user's
  answer as a tool result; handle cancellation and reject stale replies after
  session/account changes. A suggested default is not an answer.
- Share question policy, validation and result shaping across local and remote
  project runs and project-less host chat, with thin transport adapters and
  matching behavioral contracts. Use the shared form/component system, with
  keyboard and phone support. Keep ordinary questions separate from tool
  approvals and private terminal/password input.

### Multi-user

Only needed once more than one account can exist (today registration closes
after the first account):

- **Admin-only writes** for connections and server/model configuration (they stay
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

The milestone that marks **1.0** — the first stable version tagged for public use. A hygiene and packaging pass once the hardening sequence, refactors and the main Next features have
landed — before sharing the repo publicly.

Release automation, download-only Compose/Quadlet files, support issue forms,
configuration/upgrade/backup guidance, architecture diagrams and font credits are
prepared. See [releases and upgrades](releases.md). Version/changelog gates,
archive/checksum contracts, workflow lint and Compose configuration checks cover
this preparation; publication and release-host verification remain outstanding.
Documentation now has [section overviews](index.md), first-session/workspace guides
and curated site navigation. Complete the remaining task guides and installation
walkthroughs against the final app before public launch.

- **Release gates:** finish the hardening/editor dependencies and the Remaining
  manual verification checks above. Do not declare 1.0 while both-mode contracts,
  device checks or known responsiveness issues remain unresolved.
- **Repo hygiene:** finish the unused-code/dependency/configuration sweep and remove
  confirmed scratch files and stale specs once concurrent development is finished.
  Generated Python caches, browser test configuration and output are now ignored;
  existing native/WASI/WASM fmt/clippy/tests remain release gates.
- **User documentation:** expand model/provider setup, approval modes and agent
  workflows, sessions, Git, commands/output and troubleshooting into task guides.
  Validate them on a clean installation and on both workspace modes; refresh
  editor and mobile walkthroughs after the ongoing UI work lands.
- **Release rehearsal:** run the Release workflow's nonpublishing manual build and
  verify its native Linux amd64/arm64 images and Linux/macOS bridge archives. Exercise
  download-only Compose, rootless Quadlet (including registry auto-updates), HTTPS,
  database backup/restore/upgrade and both workspace modes. CI validates generated
  Quadlet services; actual systemd service and update-timer checks remain hands-on.
- **Prerelease publication:** the prepared pipeline also accepts `-alpha.N`, `-beta.N`
  and `-rc.N`, publishes versioned images plus the matching rolling channel, and marks
  GitHub Releases as prereleases. Validate a first prerelease install and make GHCR
  public before using alpha/beta distribution for feedback; this does not complete 1.0.
- **First stable publication:** move shipped changelog entries to `[1.0.0]`, bump workspace,
  Spin and lockfile versions together, and publish `v1.0.0` after rehearsal passes.
  The prepared workflow runs CI, merges native platform images into
  `ghcr.io/openwebide/openwebide:v1.0.0` and `latest`, and publishes changelog-derived
  notes, archives, install files and checksums using `GITHUB_TOKEN`. Make the GHCR
  package public and verify anonymous pulls and clean installs on both architectures.
  The workspace currently remains `0.1.0`; a prerelease version can be adopted before
  the stable-release gates pass.
- **Public launch:** update the [site landing page](project-site.md) and pre-release
  documentation wording for the actual release, then prepare community announcements
  and directory submissions. Support uses the new GitHub issue forms; an optional
  demo remains future launch work.

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
