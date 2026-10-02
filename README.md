# Open WebIDE

A WebAssembly-based IDE for working with local-LLM coding agents. Think
**Open WebUI, but for agentic coding** — and the whole stack (frontend *and*
backend) runs on WebAssembly with zero client install.

- **Frontend:** Rust + [Leptos](https://leptos.dev) compiled to WASM, built with [Trunk](https://trunkrs.dev)
- **Backend:** a [Spin](https://spinframework.dev) component (Rust → `wasm32-wasip2`) exposing a REST & SSE API
- **Persistence:** SQLite via Spin's `sqlite` capability (file-backed, single-volume)
- **LLM providers:** Ollama and llama.cpp behind a common OpenAI-compatible provider interface

## Vision & Workflow

Open WebIDE turns your workstation into a self-hosted agentic dev engine and
any phone, tablet, or laptop into a seamless remote control:

1. **Workstation as Engine, Mobile as Remote Control:**
   Run Open WebIDE on your workstation alongside your local LLM (e.g.
   `qwen2.5-coder` via Ollama) and your git repositories. Use it at your desk on
   `http://localhost:3000`, or connect from your phone or tablet on the couch
   via LAN or Tailscale (`http://workstation:3000`). Because chat sessions,
   projects, and messages live in backend SQLite, your phone and desktop stay
   100% in sync. Kick off a task at your desk, walk away, and monitor streaming
   tool steps, review diffs, and approve changes from your phone.
2. **Zero Client Footprint:**
   No binaries, Daemons, or toolchains to install on your client device. Just a
   clean browser tab that feels fast and responsive even on mobile devices.
3. **100% WebAssembly:**
   The frontend and backend run on WebAssembly, with a native Rust execution
   bridge bundled beside Spin. No Node.js runtime or Python daemon is needed.
4. **Dual Workspace Modes:**
   - **Remote mode (Primary):** remote = a project on the device hosting Open WebIDE
     (your workstation, home lab, or server). Access to repositories under the configured workspace
     mount, with multi-device shared sessions out of the box.
   - **Local mode:** local = a project on this device (the browser's File System Access API).
     Requires Chromium. Keep client repositories
     strictly on your laptop's local SSD without mounting them to the host.
     Command and git tools find the picked folder using a temporary probe file,
     verified at each run and removed afterwards. Start the bridge with
     `--workspace` pointing to a folder containing the project (within five
     directory levels), or start it inside the project. If the bridge cannot see
     the folder, those tools are hidden and chat explains how to enable them.
     After a reload interrupts a local run, use Resume in its chat session to
     continue without adding another user message.
5. **Agentic Coding with Safety First:**
   The model inspects code, calls workspace-confined tools, and presents
   syntax-highlighted diffs with human-in-the-loop permission gates and
   accept/reject controls.

## Status

Working:
- **Agentic coding loop (`crates/agent`):** Model → tool calls (`read_file`, `write_file`, `list_dir`, `search`, `run_command`, `git_status`/`git_diff`/`git_commit`/`git_branch`) → execute → review cycle with turn and tool call budgets
- **Permission handshake & cancellation:** Gated tool approval before destructive file writes and shell commands, and server-side run cancellation. Stop interrupts running commands, web requests, and workspace searches. File writes, commits, and branch changes finish before the run stops. Bridge commands stop immediately; SSE and local runs check during tools every 250 ms and 100 ms respectively.
- **Core IDE surface:**
  - In-browser code editor with a pure-Rust syntax-highlighter overlay (16 languages; zero JS dependencies), cursor/selection tracking, and diff viewing against Git HEAD
  - Diff-first file viewer with toggleable display modes: inline diff, side-by-side diff, updated content, and markdown/image preview
  - Accept/reject controls for agent file modifications
  - Multi-project tabs (Rider-style) with per-project state preservation
- **TUI-driven chat surface:** a terminal-native stream layout, slash commands (`/model`, `/tokens`, `/clear`, `/test`, `/diff`, `/commit`, `/checkout`, `/branch`, `/sync`), active-editor-context injection, and a statusline with live token/speed telemetry and a context-window gauge, backed by real per-call provider usage and an optional per-connection **context limit** (sent to Ollama as `options.num_ctx`)
- **Virtual File System (`Vfs`):** one shared abstraction over workspaces: remote = a project on the device hosting Open WebIDE; local = a project on this device (the browser's File System Access API). Includes workspace-wide search, live web search, and a documentation-page reader
- **WebSocket terminal bridge:** a native `openwebide-bridge` daemon giving the UI an interactive terminal and the agent a `run_command` tool, plus host Git operations against the real repository
- **Git integration:** branch/ahead-behind status bar, file tree status badges, and diff/branch/commit/checkout/sync
- **Both workspace modes:** remote = a project on the device hosting Open WebIDE (Spin filesystem preopens); local = a project on this device (the browser's File System Access API), with directory handles persisted via IndexedDB
- **Local user accounts:** Self-hosted user registration and login with argon2id password hashing, HttpOnly cookie sessions, and user-scoped data
- **Custom dialogs & remote file browser:** Themed confirmation, prompt, and file browser modals for a project on the device hosting Open WebIDE, replacing browser-native dialogs
- **Single-container deployment:** Multi-stage Dockerfile and docker-compose packaging frontend, backend, SQLite, and the execution bridge into one image

See [docs/roadmap.md](docs/roadmap.md) for what's in flight and queued up next
(frontend polish, code intelligence, approval modes, and a secondary fast model next), and
[CHANGELOG.md](CHANGELOG.md) for the full list of finished work.

## Prerequisites

- Rust via [rustup](https://rustup.rs/). The toolchain (currently 1.98.1) and its
  `wasm32-wasip2` / `wasm32-unknown-unknown` targets are pinned in
  `rust-toolchain.toml`, so rustup installs them on the first build.
- [Spin](https://spinframework.dev/docs/latest/installation/) (4.x)
- [Trunk](https://trunkrs.dev/getting-started/install/)

## Run it

```sh
spin build --up --direct-mounts --allow-transient-write
```

This builds the backend component (Rust → WASM) and the frontend (Trunk →
static files served by Spin's static file server), then starts Spin on
`http://localhost:3000`.

- Frontend: <http://localhost:3000/>
- API: <http://localhost:3000/api/health>

## Run in a container

One image, two ports, one volume for the database — the whole stack
(frontend + backend + SQLite + execution bridge) runs in a single container. The
Dockerfile builds the components itself (multi-stage), so only Docker is
needed:

The container listens on 3000 inside; it is mapped to **8080** on the host
so it can run side by side with a bare `spin build --up` (3000):

```sh
docker compose up --build
# or manually:
docker build -t open-webide .
docker run -d -p 8080:3000 -p 3001:3001 -v openwebide-data:/app/.spin --name open-webide open-webide
```

- Frontend: <http://localhost:8080/>
- API: <http://localhost:8080/api/health>
- Bridge: `ws://localhost:3001` (terminal, Git operations, and streamed chat runs).

The bundled bridge uses `/workspace` and persists its secret at
`/app/.spin/bridge-secret`; the backend discovers it over loopback. Set
`OPENWEBIDE_BRIDGE=0` to disable it and use SSE chat instead. Compose passes
this environment variable through; with `docker run`, use `-e OPENWEBIDE_BRIDGE=0`.

IP literals and `localhost` work by default, including LAN access at
`http://<host-ip>:8080`. For a host name, set
`OPENWEBIDE_BRIDGE_ALLOWED_HOSTS=<host-name>`. If the page uses a different
host name from the bridge, also set
`OPENWEBIDE_BRIDGE_ALLOWED_ORIGINS=http://<page-host>:8080`. Both accept
comma-separated lists and are passed through by Compose.

To work on a folder on the host machine, mount it and enable the backend's
filesystem capability (see [docs/architecture.md](docs/architecture.md) →
"Workspace: local and remote modes"):

```sh
docker run -d -p 8080:3000 -p 3001:3001 -v openwebide-data:/app/.spin -v ~/source:/workspace open-webide
```

> [!NOTE]
> The workspace root (`/workspace`) is exposed to the file API and the agent — keep secrets out of it. The `.spin/` path is explicitly refused. Arguments starting with `-` are appended to the default `spin up` flags. A command override (for example, `spin up`) runs directly; overriding the command must keep `--direct-mounts --allow-transient-write`. Remote project paths created before the `/workspace` mount (stored relative to the container root, e.g. `workspace/foo`) are migrated automatically on first start — no manual SQL needed.

On Linux with systemd, it can also run as a Podman quadlet service — see
[docs/podman-quadlet.md](docs/podman-quadlet.md).

### Try the API

```sh
curl -s localhost:3000/api/health

# Sign in (register the first account in the UI first). Keep this cookie jar private.
curl -s -c /tmp/openwebide-cookies -X POST localhost:3000/api/login \
  -H 'x-openwebide: 1' -H 'content-type: application/json' \
  -d '{"username":"your-user","password":"your-password"}'

# add a connection
curl -s -b /tmp/openwebide-cookies -H 'x-openwebide: 1' -X POST localhost:3000/api/connections \
  -H 'content-type: application/json' \
  -d '{"name":"local-ollama","kind":"ollama","base_url":"http://localhost:11434","model":"qwen2.5-coder:7b"}'

curl -s -b /tmp/openwebide-cookies -H 'x-openwebide: 1' localhost:3000/api/connections

# settings
curl -s -b /tmp/openwebide-cookies -H 'x-openwebide: 1' -X PUT localhost:3000/api/settings \
  -H 'content-type: application/json' \
  -d '{"key":"theme","value":"dark"}'
curl -s -b /tmp/openwebide-cookies -H 'x-openwebide: 1' localhost:3000/api/settings

# system prompts
curl -s -b /tmp/openwebide-cookies -H 'x-openwebide: 1' -X POST localhost:3000/api/system-prompts \
  -H 'content-type: application/json' \
  -d '{"name":"coder","content":"You are a coding agent."}'

# provider endpoints (need a running engine at the connection's base_url)
curl -s -b /tmp/openwebide-cookies -H 'x-openwebide: 1' 'localhost:3000/api/models?connection_id=1'

curl -s -b /tmp/openwebide-cookies -H 'x-openwebide: 1' -X POST localhost:3000/api/chat \
  -H 'content-type: application/json' \
  -d '{"connection_id":1,"messages":[{"role":"user","content":"hello"}]}'
```

## Host egress

Outbound network egress from the backend component is open via wildcards (`https://*:*` and `http://*:*` in [spin.toml](spin.toml)). This is needed to connect to LLM providers across local networks (LAN model hosts, loopback, private tunnels) and to support web search and documentation fetching.

Web-tool protections:

- **Approval gate:** The agent's `fetch_web_page` tool requires explicit human approval before any external page is fetched. `search_web` stays auto-approved.
- **Cloud metadata protection:** Outbound web fetching explicitly refuses requests to cloud metadata addresses (`169.254.169.254`, `fd00:ec2::254`, their IPv4-mapped representations, and `metadata.google.internal`). Everything else (LAN, loopback, public internet) remains accessible.

## Agent

When the agent edits files that cannot be read as text (binary or large files), it backs up the original contents to the `.openwebide/backups/` directory inside the project root before overwriting. Backups are git-ignored and retained for Reject to restore the original; delete them only when those edits no longer need restoration.

## Execution bridge

The native bridge daemon (`openwebide-bridge`) runs on the host to provide interactive PTY terminals, process execution (`POST /exec`), and host Git operations for the web frontend and coding agents. The backend reaches the bridge at `127.0.0.1:3001` and sends project-relative `cwd`.

> [!NOTE]
> Git operations performed by the bridge require Git ≥ 2.23 on the host machine for branch switching via `git switch`.

The bridge's HTTP/1 and WebSocket server (built on `hyper`) enforces request limits: a 16 KiB
HTTP request head, a 1 MiB `/exec`/Git request body, a 16 MiB WebSocket message/frame, and 256
concurrent connections. Idle sockets are closed after 10 s without a request head; WebSocket
connections are pinged every 30 s and closed after 90 s without an inbound frame.

### Running the bridge

```sh
cargo build -p openwebide-bridge
./target/debug/openwebide-bridge --port 3001 --workspace ../.. --host 127.0.0.1 \
  --backend-url http://127.0.0.1:3000/api --secret-file /tmp/openwebide-bridge-secret
```

Signed-in connections (hello with a bridge token) can run chat and agents over the bridge when it shares the backend's secret. Runs continue after a browser disconnect and can replay their events on reconnect. Otherwise, runs use SSE and completions use `/api/chat-tools`. HTTP-only builds (`cargo build -p openwebide-bridge --no-default-features`) advertise no run support; terminal and Git operations remain available.

**Chat over the bridge:** the frontend uses WebSocket runs after `hello_ok` advertises run support. In browser DevTools, prompts send `run_start` and replies arrive as `run_event` frames without an SSE request. Local folders keep their agent loop in the browser and stream model completions through `completion_start`. Reloading a running session attaches its snapshot; reconnecting resumes from the last sequence. If the bridge is unavailable, rejects authentication, or doesn't finish hello within about two seconds, chat uses SSE and local completions use `/api/chat-tools`. A project the bridge cannot see shows an info notice before server fallback; busy sessions and planning failures show errors. LAN clients use the same flow.

- `--backend-url <URL>` (or env `OPENWEBIDE_BRIDGE_BACKEND_URL`): backend API URL for run plans and persistence (default: `http://127.0.0.1:3000/api`); set it on the daemon, never in a client request. A frontend `?api=` development override needs a matching `--backend-url`.
- `--workspace <DIR>` (or env `OPENWEBIDE_BRIDGE_WORKSPACE`): sets the workspace root directory for command execution and repository operations (defaults to the current working directory). This directory must match the `files` source mount used by the Spin backend (e.g. `~/source` locally or `/workspace` in Docker). All `cwd` arguments passed to the bridge are evaluated relative to this root, and requests escaping the root are rejected (lexical confinement).
- `-p, --port <PORT>` (or env `OPENWEBIDE_BRIDGE_PORT`): port to bind on (default: `3001`).
- `--host <HOST>` (or env `OPENWEBIDE_BRIDGE_HOST`): host address to bind on (default: `127.0.0.1`). To expose the bridge to your local network (e.g. phone/tablet use over LAN or Tailscale), bind to `0.0.0.0` or a specific LAN IP:
  ```sh
  ./target/debug/openwebide-bridge --host 0.0.0.0 --port 3001
  ```
- `--secret-file <FILE>`: persistent shared-secret file for non-browser API commands. `OPENWEBIDE_BRIDGE_SECRET` overrides the file and must contain at least 32 characters. Otherwise a persistent random 32-byte secret is stored at `--secret-file`, `$XDG_CONFIG_HOME/openwebide/bridge-secret`, or `~/.config/openwebide/bridge-secret`. The backend lazily fetches it over loopback via `POST /secret` and caches it in SQLite.
- `--token <TOKEN>` (or env `OPENWEBIDE_BRIDGE_TOKEN`): Optional pairing token (≥ 16 characters) to authenticate local companion apps without needing the backend's minted session tokens. Allows local-mode execution: `openwebide-bridge --token <16+ chars> --workspace <path>`.

> [!WARNING]
> When the backend reaches a remote bridge through a non-loopback URL, set both `SPIN_VARIABLE_BRIDGE_URL=http://<bridge-host>:3001` and `SPIN_VARIABLE_BRIDGE_SECRET` to the bridge secret from its secret file or `OPENWEBIDE_BRIDGE_SECRET`. The daemon logs the secret source, never the secret itself. Binding to `0.0.0.0` still permits automatic bootstrap when the backend connects over loopback.

### Host and Origin security baseline

To protect against DNS rebinding and malicious websites opened in the user's browser, the bridge validates incoming HTTP and WebSocket requests:

1. **Host Header Rules:**
   - Allowed if the `Host` is an **IP literal** (IPv4 or IPv6, e.g. `127.0.0.1`, `[::1]`, `192.168.1.50`).
   - Allowed if `localhost`, this machine's hostname (e.g. `mymachine`), or `<hostname>.local`.
   - Allowed if explicitly added via `--allowed-host <HOSTNAME>` (or `OPENWEBIDE_BRIDGE_ALLOWED_HOSTS` comma-separated list).
   - Any unrecognized or rebinding domain name is rejected with `403 Forbidden`.

2. **Origin & CORS Rules:**
   - Requests without an `Origin` header (such as Spin backend calls, `curl`, and local daemon tools) are permitted, but API routes require authorization via `Authorization: Bearer <SECRET>`. For example:
     ```sh
     curl -X POST http://127.0.0.1:3001/exec \
       -H "Authorization: Bearer <your-secret>" \
       -H "Content-Type: application/json" \
       -d '{"command": "echo test"}'
     ```
   - Browser requests with an `Origin` header are permitted only if:
     - The origin is in the allowed origins list (default: `http://localhost:3000`, `http://127.0.0.1:3000`, `http://localhost:8080`, `http://127.0.0.1:8080`, plus any `--allowed-origin` entries); **or**
     - The origin's hostname matches the request's `Host` hostname (allowing phone/LAN access when the frontend and bridge are accessed on the same host machine).
   - `Origin: null` and untrusted cross-origin requests are rejected with `403 Forbidden`.
   - Wildcard `Access-Control-Allow-Origin: *` is disabled; allowed origins receive their exact origin echoed with `Vary: Origin`.

3. **JSON-Only Browser POSTs:**
   - Browser POST requests carrying an `Origin` header require `Content-Type: application/json`; simple browser requests (e.g. `text/plain`, form-urlencoded) are rejected with `415 Unsupported Media Type` to prevent browser CSRF.

### Process lifecycle

Every command the bridge spawns (`/exec`, `run_command`, PTY shells, Git subprocesses) starts in its own process group. Signals target that group; interactive shells can put background jobs into separate groups, which may survive shell cleanup:

- **Kill semantics:** sending `Kill` with no signal (or `KILL`/`SIGKILL`) terminates the whole process group immediately. `TERM`/`SIGTERM` and `HUP`/`SIGHUP` signal the group directly. `INT`/`SIGINT` on a PTY session instead writes `^C` to the terminal, matching a real Ctrl+C — it interrupts whatever's in the foreground rather than killing the shell itself.
- **Timeouts:** a `run_command`/`/exec` timeout, or the client disconnecting mid-request, terminates the command's process group (SIGTERM, then SIGKILL after a 2 s grace period) instead of leaving it (and any children) running.
- **Output capping:** `/exec` keeps the first 256 KiB and last 768 KiB of each of stdout/stderr per stream, with an omission marker in between, so a runaway command can't exhaust memory.
- **Session reaping:** exited terminal/process sessions are removed 30 minutes after they exit, freeing their output buffers; running sessions are never reaped, however long they've been open.
- **Graceful shutdown:** on Ctrl+C or `SIGTERM`, the bridge stops accepting new connections and terminates every session's process group (with the same grace period) before exiting.

## Authentication

Open WebIDE uses local accounts (with argon2id password hashing) and secure HttpOnly cookie sessions.
There is no reliance on localStorage for tokens. Note that logging out clears the session globally across all devices by rolling the user's token epoch. Rate-limiting is enforced against failed logins to prevent brute force attacks.

## Development

```sh
cargo test -p openwebide-storage -p openwebide-core -p openwebide-auth -p openwebide-agent -p openwebide-llm -p openwebide-bridge -p openwebide-backend
cargo test -p openwebide-frontend --lib
CHROMEDRIVER=<path> cargo test -p openwebide-frontend --target wasm32-unknown-unknown
cargo build-backend    # backend component → target/wasm32-wasip2/release
cargo build-frontend   # frontend → target/wasm32-unknown-unknown/release
cd frontend && trunk serve   # frontend dev server on :8080
```

UI tests require Chrome, chromedriver, and `wasm-bindgen-cli` 0.2.128 matching the lockfile.

When developing the frontend against `spin up --direct-mounts --allow-transient-write`,
point it at the API with `?api=http://localhost:3000/api`. This override is accepted
only from `localhost:8080` or `127.0.0.1:8080`. Note: because sessions use `SameSite=Strict` cookies, cross-origin requests from Trunk (`localhost:8080`) to the backend (`127.0.0.1:3000`) might fail to attach cookies in some browsers; use `localhost` for both to avoid cross-site issues.

## Layout

```
crates/core       shared domain types (serde)
crates/llm        LlmProvider trait + Ollama/llama.cpp providers
crates/storage    Db abstraction, migrations, Store repositories
backend           Spin HTTP component (REST API)
frontend          Leptos WASM app (Trunk)
docs/architecture.md   design notes
docs/roadmap.md        what's left (Now / Next / Later)
CHANGELOG.md           what's already shipped
```

See [docs/architecture.md](docs/architecture.md) for the architecture,
[docs/roadmap.md](docs/roadmap.md) for what's left, and
[CHANGELOG.md](CHANGELOG.md) for what's already shipped.

## License

MIT — see [LICENSE](LICENSE).
