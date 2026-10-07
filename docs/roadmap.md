# Roadmap

What's left, grouped by how soon it's coming: **Next** (queued up), **Later**
(planned, not yet started). Finished work —
phases 1 through 14, telemetry, hardening, streaming, `/test`, database-backed
theme and prompt history, frontend performance & polish, app branding and the chat
welcome — moved to [CHANGELOG.md](../CHANGELOG.md).

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

The shared Rust document/transaction engine, grouped undo/redo, per-file/project
history, indentation/EditorConfig controls, block-aware Enter and paired typing
are in place. Line movement/duplication/deletion, snippet duplication, line/block
comments, explicit indentation-matching paste, selected-line reindent, folding,
reading/navigation and Find/Replace use the same engine. See [editor controls](editor.md). Continue with:

- **Reliable edits and history:** build on grouped transactions and per-document history
  with caret/selection/scroll restoration now retained across file, project and view
  switches. Explicit EditorConfig newline,
  line-ending and trailing-whitespace save policies are undoable. Release-app
  Chromium composition commit/cancel and undo/redo now verify Unicode and LF/CRLF
  in both modes, with pending syntax and bounded native windows. Finish physical
  input-method/clipboard verification without rewriting unrelated text.
- **Syntax-aware editing:** shared parsers, source-bound preparation and the
  Rust/WASM worker are in place; see [syntax behavior](editor.md#syntax-and-language-behavior).
  Pending worker paint now borrows source rows and retains scoped styled frames.
  Cold horizontal and wrapped paint reuse the height/width probe’s source anchors;
  identical pending/plain rows retain dimensions and glyph anchors after syntax resolves.
  Cold probes briefly await the selected font, with bounded fallback and fresh ownership checks.
  The [cold layout candidate check](editor-performance.md#cold-layout-candidate-check)
  records why Rust shaping and canvas widths cannot replace current DOM geometry directly.
  Terminal lexical paint prepares in cooperative, source-owned batches; worker and
  fallback updates reuse exact source/context rows and share their immutable token
  allocations. Worker replies reference validated unchanged token rows and publish
  source replacement spans in both directions, with a bounded full-snapshot resync
  when a worker base is unavailable. Consecutive unchanged token rows transfer as
  validated runs; replies and warm queries retain the facade's source snapshot.
  Structural metadata now transfers changed list spans against a validated ticket,
  avoiding complete wire copies for retained records. Structural extraction, list
  reconstruction/validation and row-table construction still visit the whole file.
  Finish those incremental paths and remaining source snapshot
  ownership,
  uncached initial/cold shaping, bidirectional visual-run windows, fine long-row paint
  and incremental measurement. Language formatting remains in Code intelligence.
- **Bounded native input:** prepared larger fine-pointer editors bind scoped
  surrounding text to the textarea, with full-source selections and source-owned
  scrolling. Finish initial/cold and touch pointer selection, caret ownership and
  source extents, then verify physical Chrome/Edge PWA input methods. Reduce
  full-source access in remaining input paths. Ownership checks, ordinary typing
  selection dispatch and motion scheduling now borrow source; motion queues retain
  document identity instead of another source copy. Pointer gestures now borrow current
  source and retain document identity, rejecting replacement documents even at the
  same revision. Release-app Chromium checks cover highlighted token/line-end clicks,
  far-right blank space and held pointer movement in Rust/C#/JSON and scrolled
  bounded input in both modes. Click/drag hits share measured-boundary validation
  and row-padding normalization. Composition baselines share source text,
  line indexes and prepared projections; edits detach the changed version and
  cancellation restores the original allocations. Reduce remaining buffer/projection
  publication. History snapshots
  share immutable steps and transaction payloads rather than copying retained edits,
  and document/composition snapshots share saved-text baselines. Cancellation now
  borrows preview/restored source and copies only matching UI destinations. Admission now skips
  untouched complete rows; long boundary rows still scan, and storage still shifts
  suffix bytes and coordinates.
  See [viewport preparation](editor.md#preparation-and-viewport-rendering).
- **Editor performance verification:** input, cold paint, wrapped layout and
  process-memory stalls remain. Cold-probe traces separate rendering, DOM installation,
  row layout and source geometry; use those measurements, then repeat admitted byte,
  row-count and long-line boundary workloads in both modes, including Linux Chrome
  PSS and the unresponsive wrapped cases. Broader startup scroll latency and
  initial-shaping samples remain unverified despite bounded steady-state paint.
  See [recorded measurements](editor-performance.md).
- **Selections and files:** finish real-device input/IME/clipboard verification
  for multiple cursors, permission/error regression coverage and native folder
  permission/recovery checks. Shared selection, tab/buffer and database recovery
  behavior is documented in [editor controls](editor.md); the disposable recovery
  check covers both modes but uses no native local folder handle. Coordinate
  draft/reload recovery with Offline & error-state recovery below.

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

### Editor Git annotations and file-tree changes

- GitLens-style editor annotations showing line authorship, commit details and
  history, with navigation to the relevant commit or diff.
- GitHub-style colored folder/file icons in the file tree instead of the `M`/`U`/`A`
  Git indicators; show added/removed line counts for changed files.
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

### TUI goals and common controls

- Support `/goal` in the TUI through shared agent goal orchestration, with visible
  progress, cancellation and session recovery.
- Review other common TUI features and prioritize the missing controls; reuse
  shared command behavior across the TUI and browser wherever applicable.

### Universal command palette omnibar

Extend the existing searchable command palette into a universal omnibar, replacing
the **Commands** button and its current UI. Combine commands, file navigation and
project/session navigation behind one searchable entry point, with keyboard and
touch access, consistent focus behavior and context-aware action availability.
Reuse shared actions in both modes.

### In-app About and open-source software

- Add an About page showing the running build version and commit.
- Include an in-app open-source software inventory with license notices, including
  the bundled Monaspace fonts. Reuse the project site's generated credits inventory
  and extend it to cover bundled assets as well as dependencies.
- Make build details and notices available in both modes and the offline PWA.

### Compact desktop layout and phone navigation

Combine related bars and reveal occasional controls on demand, preserving control
sizes and usable hit targets. Aim for two app rows above editor content and one
footer, excluding the browser/PWA window title bar.

- **App navigation:** combine branding and project tabs into one row. Make the logo
  an app-menu dropdown with a discoverable chevron: Open local folder, Open remote
  folder, Recent projects, Settings, Servers, Help/Keyboard shortcuts and
  About/Open-source software. Remove the separate opening controls from the project
  strip. Coordinate the searchable command entry point with the Universal command
  palette omnibar item above. Keep Sessions focused on conversation navigation;
  move server and system-prompt management into app-level configuration.
- **Editor header:** replace the separate **Editor** heading with file tabs on the
  left; right-align the view selector, Save, Find and finally the `⋯` pane menu.
  Combine the current title, tabs and path/action bars rather than shrink them.
  Align Files and Chat headers with the editor tab/action row. Show the full file
  path through tab tooltips or a breadcrumb popover, with copy-path/reveal-in-tree
  actions, rather than reserve a permanent path row. Save and Find can use icons
  with tooltips and accessible labels; keep view choices readable on desktop and
  collapse them to an active-view selector on narrow screens.
- **Changes view:** replace the awkward **Diff HEAD** control with an **Edit /
  Changes / Preview** view selector (Preview only for supported files). Keep file
  tabs, navigation and toolbar positions consistent across views. Show **Inline /
  Split** as a small display control when viewing Changes; describe the comparison
  as **Against last commit**, with `HEAD` available in a tooltip/advanced selector.
  Move the prominent red **Revert to HEAD** action into `⋯` and the command palette
  as **Discard changes…**, retaining confirmation and unsaved-buffer/review guards.
- **Files controls:** keep Explorer/Changes visible. Put the search icon in the
  pane header; clicking it reveals and focuses the search UI, and Escape dismisses
  it. Move Include hidden files and folders, refresh and tree preferences into the
  pane menu. Editor Find likewise appears only when invoked by icon or shortcut.
- **Status and occasional settings:** combine the editor and app footers. Show
  cursor position, one indentation control (**Spaces: 4** / **Tabs: 4**), connection,
  Git status and Output access. Put separate indent/tab widths and detection/default
  details behind the indentation control; move **Convert indentation** into `⋯`
  and the command palette. Keep chat model and approval-mode state visible, while
  detailed telemetry can open from the context gauge. Review duplicate pane-collapse
  buttons where dock toggles already provide the same action. Use one app logo/menu
  and consistent `⋯` pane menus for app navigation versus pane options.
- **Chat density and narrow layouts:** keep model and approval controls readable
  without clipping or horizontal overflow. On phones, reduce telemetry to a compact
  context indicator that opens details. Group consecutive collapsed thought/tool
  events into an expandable activity summary (for example, **4 steps · 6.3s**),
  retaining access to individual events and keeping pending approvals and failures
  visible without expanding the group.
- **Phone navigation:** use a full-width bottom bar with four equal, justified
  icon-and-label destinations: Sessions, Files, Editor and Chat. Selecting one
  makes that pane the main view, with a clear active state. The logo opens the
  Material-style offcanvas app/project drawer. Output can open as a sheet from
  its status indicator. Respect safe-area insets and choose keyboard behavior that
  preserves composer/editor space without obscuring controls.
- **Visual state consistency:** use shared selected-state styling across project/file
  tabs, pane navigation and segmented controls; distinguish keyboard focus from
  selection. Coordinate file-tree status with the Editor Git annotations and
  file-tree changes item: replace cryptic change dots/letter indicators with colored
  icons and added/removed counts, retaining accessible status descriptions.
- **Shared behavior and verification:** use shared feature actions, components and
  theme tokens in both modes. Keep frequent actions accessible outside menus;
  retain keyboard shortcuts, focus restoration, touch targets and account-synced
  layout preferences. Verify tab overflow, narrow layouts, keyboard/IME behavior,
  accessibility, Changes/review safeguards and real phones in both modes.

### Build, test and process output

Prioritize the existing terminal pane as a useful place to follow build, test and
other process output: readable streamed logs, separate runs, exit status, cancellation,
copy/search and file/line links back to the editor. Preserve output during reconnects
and keep runs scoped to their project/session. Use shared output behavior with thin
runtime adapters; local output is available only for browser-supported execution,
while host-native builds/tests initially run in remote mode.

- Default an empty, idle output pane to collapsed. When a process starts, reveal
  it at a modest desktop height while honoring explicit hide/resize choices.
  On phones, show output in a sheet rather than stack it beneath the active pane
  and squeeze the conversation/editor into the remaining space.
- Combine the pane title, connection state and `⋯` actions into one header.
  Show command input only when manual execution is requested; avoid reserving
  separate connection/input bands around an empty output area. Keep running/failure
  status visible through the shared footer's Output indicator when the pane is hidden.

Prioritize idle output behavior, phone control overflow and grouped chat activity
alongside the compact-layout work above. Verify both-mode output contracts and
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
