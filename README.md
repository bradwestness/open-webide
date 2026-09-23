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
   Pure Rust across the entire codebase. No Node.js runtime, no Python daemon,
   no heavy container orchestration. Instant startup and an image under 50MB.
4. **Dual Workspace Modes:**
   - **Remote mode (Primary):** The workspace lives on the host machine
     (your workstation, home lab, or server). Dynamic access to any repository
     under your home directory with multi-device shared sessions out of the box.
   - **Local mode:** The workspace lives on the browser's machine, accessed
     directly via the File System Access API (Chromium). Keep client repositories
     strictly on your laptop's local SSD without mounting them to the host.
5. **Agentic Coding with Safety First:**
   The model inspects code, calls workspace-confined tools, and presents
   syntax-highlighted diffs with human-in-the-loop permission gates and
   accept/reject controls.

## Status

Working:
- **Agentic coding loop (`crates/agent`):** Model → tool calls (`read_file`, `write_file`, `list_dir`, `search`, `run_command`, `git_status`/`git_diff`/`git_commit`/`git_branch`) → execute → review cycle with turn and tool call budgets
- **Permission handshake & cancellation:** Gated tool approval before destructive file writes and shell commands, and server-side run cancellation
- **Core IDE surface:**
  - In-browser code editor with a pure-Rust syntax-highlighter overlay (16 languages; zero JS dependencies), cursor/selection tracking, and diff viewing against Git HEAD
  - Diff-first file viewer with toggleable display modes: inline diff, side-by-side diff, updated content, and markdown/image preview
  - Accept/reject controls for agent file modifications
  - Multi-project tabs (Rider-style) with per-project state preservation
- **TUI-driven chat surface:** a terminal-native stream layout, slash commands (`/model`, `/tokens`, `/clear`, `/test`, `/diff`, `/commit`, `/checkout`, `/branch`, `/sync`), active-editor-context injection, and a statusline with live token/speed telemetry and a context-window gauge, backed by real per-call provider usage and an optional per-connection **context limit** (sent to Ollama as `options.num_ctx`)
- **Virtual File System (`Vfs`):** one shared abstraction over Remote (host filesystem) and Local (browser File System Access API) workspaces, including workspace-wide search, live web search, and a documentation-page reader
- **WebSocket terminal bridge:** a native `openwebide-bridge` daemon giving the UI an interactive terminal and the agent a `run_command` tool, plus host Git operations against the real repository
- **Git integration:** branch/ahead-behind status bar, file tree status badges, and diff/branch/commit/checkout/sync
- **Both workspace modes:** Remote host mounts via Spin filesystem preopens, and Local browser mode via the File System Access API (persisted via IndexedDB)
- **Local user accounts:** Self-hosted user registration and login with argon2id password hashing, session tokens, and user-scoped data
- **Custom dialogs & remote file browser:** Themed confirmation, prompt, and remote host file browser modals replacing browser-native dialogs
- **Single-container deployment:** Multi-stage Dockerfile and docker-compose packaging frontend, backend, and SQLite into one image

See [docs/roadmap.md](docs/roadmap.md) for what's in flight and queued up next
(security/bridge hardening and agent streaming now; code intelligence,
approval modes, and a secondary fast model next), and
[CHANGELOG.md](CHANGELOG.md) for the full list of finished work.

## Prerequisites

- Rust via [rustup](https://rustup.rs/). The toolchain (currently 1.98.1) and its
  `wasm32-wasip2` / `wasm32-unknown-unknown` targets are pinned in
  `rust-toolchain.toml`, so rustup installs them on the first build.
- [Spin](https://spinframework.dev/docs/latest/installation/) (4.x)
- [Trunk](https://trunkrs.dev/getting-started/install/)

## Run it

```sh
spin build --up
```

This builds the backend component (Rust → WASM) and the frontend (Trunk →
static files served by Spin's static file server), then starts Spin on
`http://localhost:3000`.

- Frontend: <http://localhost:3000/>
- API: <http://localhost:3000/api/health>

## Run in a container

One image, one port, one volume for the database — the whole stack
(frontend + backend + SQLite) runs in a single Spin container. The
Dockerfile builds the components itself (multi-stage), so only Docker is
needed:

The container listens on 3000 inside; it is mapped to **8080** on the host
so it can run side by side with a bare `spin build --up` (3000):

```sh
docker compose up --build
# or manually:
docker build -t open-webide .
docker run -d -p 8080:3000 -v openwebide-data:/app/.spin --name open-webide open-webide
```

- Frontend: <http://localhost:8080/>
- API: <http://localhost:8080/api/health>

To work on a folder on the host machine, mount it and enable the backend's
filesystem capability (see [docs/architecture.md](docs/architecture.md) →
"Workspace: local and remote modes"):

```sh
docker run -d -p 8080:3000 -v openwebide-data:/app/.spin -v ~/projects:/workspace open-webide
```

On Linux with systemd, it can also run as a Podman quadlet service — see
[docs/podman-quadlet.md](docs/podman-quadlet.md).

### Try the API

```sh
curl -s localhost:3000/api/health

# add a connection
curl -s -X POST localhost:3000/api/connections \
  -H 'content-type: application/json' \
  -d '{"name":"local-ollama","kind":"ollama","base_url":"http://localhost:11434","model":"qwen2.5-coder:7b"}'

curl -s localhost:3000/api/connections

# settings
curl -s -X PUT localhost:3000/api/settings \
  -H 'content-type: application/json' \
  -d '{"key":"theme","value":"dark"}'
curl -s localhost:3000/api/settings

# system prompts
curl -s -X POST localhost:3000/api/system-prompts \
  -H 'content-type: application/json' \
  -d '{"name":"coder","content":"You are a coding agent."}'

# provider endpoints (need a running engine at the connection's base_url)
curl -s 'localhost:3000/api/models?connection_id=1'

curl -s -X POST localhost:3000/api/chat \
  -H 'content-type: application/json' \
  -d '{"connection_id":1,"messages":[{"role":"user","content":"hello"}]}'
```

## Host egress

The backend is a Spin component, so it runs in a sandbox that can only
reach the network hosts you explicitly allow. Outbound requests to any
other host are denied by Spin *before* they leave the sandbox — the app
can't change this at runtime.

Allowed hosts are declared in [spin.toml](spin.toml) under
`[component.backend]`:

```toml
allowed_outbound_hosts = [
    # Localhost — engines running on the machine running Spin.
    "*://localhost:*",
    "*://127.0.0.1:*",
    # Tailscale MagicDNS — engines on a tailnet.
    "*://*.ts.net:*",
    # Cloudflare Quick Tunnel — no-account tunnels.
    "*://*.trycloudflare.com:*",
    # ngrok — free tier.
    "*://*.ngrok-free.app:*",
    # NetBird — WireGuard mesh (Tailscale-style).
    "*://*.netbird.cloud:*",
    # Nord — NordLynx.
    "*://*.nord:*",
    # Cloudflare Mesh.
    "*://*.cloudflaremesh.com:*",
]
```

The defaults cover engines on the local machine plus the common
MagicDNS-style / tunnel services used to reach engines on other machines
(Tailscale, Cloudflare, ngrok, NetBird, Nord).

### Allowing another host

Add the host to `allowed_outbound_hosts` in [spin.toml](spin.toml), then
**restart** Spin (`spin build --up`) — manifest changes only take effect on
startup.

- **A LAN IP** (e.g. llama.cpp on `192.168.1.50:8080`):
  ```toml
  "http://192.168.1.50:8080",
  ```
- **A Headscale tailnet** (your own Tailscale control server): use the
  tailnet's DNS domain, e.g. `"*://*.my-tailnet.example:*"`.
- **A custom tunnel / domain**: `"*://*.my-tunnel.example:*"`.

Pattern format is `scheme://host:port` with `*` as a wildcard for each
part. **Wildcards only work as DNS subdomains, not IP octets** — so
`*://192.168.*:*` is rejected by Spin; list LAN hosts by their specific IP.

When a request is denied, the API returns an actionable error pointing at
this section:

```
HTTP error: outbound to http://10.0.0.99:9/v1/models is blocked by Spin's
`allowed_outbound_hosts` allowlist. Add the host to spin.toml (see README
→ Host egress) and restart Spin
```

## Execution bridge

The native bridge daemon (`openwebide-bridge`) runs on the host to provide interactive PTY terminals, process execution (`POST /exec`), and host Git operations for the web frontend and coding agents.

### Running the bridge

```sh
cargo build -p openwebide-bridge
./target/debug/openwebide-bridge --port 3001 --workspace /path/to/project
```

- `--workspace <DIR>` (or env `OPENWEBIDE_BRIDGE_WORKSPACE`): sets the workspace root directory for command execution and repository operations (defaults to the current working directory).
- `-p, --port <PORT>` (or env `OPENWEBIDE_BRIDGE_PORT`): port to bind on (default: `3001`).
- `--host <HOST>` (or env `OPENWEBIDE_BRIDGE_HOST`): host address to bind on (default: `127.0.0.1`). To expose the bridge to your local network (e.g. phone/tablet use over LAN or Tailscale), bind to `0.0.0.0` or a specific LAN IP:
  ```sh
  ./target/debug/openwebide-bridge --host 0.0.0.0 --port 3001
  ```

> [!WARNING]
> Exposing the bridge on `0.0.0.0` allows Origin-less HTTP requests from the local network (used by the Spin backend and command-line tools). While cross-origin browser requests are strictly restricted, any device on the trusted LAN that can reach port 3001 can send Origin-less HTTP commands.

### Host and Origin security baseline

To protect against DNS rebinding and malicious websites opened in the user's browser, the bridge validates incoming HTTP and WebSocket requests:

1. **Host Header Rules:**
   - Allowed if the `Host` is an **IP literal** (IPv4 or IPv6, e.g. `127.0.0.1`, `[::1]`, `192.168.1.50`).
   - Allowed if `localhost`, this machine's hostname (e.g. `mymachine`), or `<hostname>.local`.
   - Allowed if explicitly added via `--allowed-host <HOSTNAME>` (or `OPENWEBIDE_BRIDGE_ALLOWED_HOSTS` comma-separated list).
   - Any unrecognized or rebinding domain name is rejected with `403 Forbidden`.

2. **Origin & CORS Rules:**
   - Requests without an `Origin` header (such as Spin backend calls, `curl`, and local daemon tools) are permitted.
   - Browser requests with an `Origin` header are permitted only if:
     - The origin is in the allowed origins list (default: `http://localhost:3000`, `http://127.0.0.1:3000`, `http://localhost:8080`, `http://127.0.0.1:8080`, plus any `--allowed-origin` entries); **or**
     - The origin's hostname matches the request's `Host` hostname (allowing phone/LAN access when the frontend and bridge are accessed on the same host machine).
   - `Origin: null` and untrusted cross-origin requests are rejected with `403 Forbidden`.
   - Wildcard `Access-Control-Allow-Origin: *` is disabled; allowed origins receive their exact origin echoed with `Vary: Origin`.

3. **JSON-Only Browser POSTs:**
   - Browser POST requests carrying an `Origin` header require `Content-Type: application/json`; simple browser requests (e.g. `text/plain`, form-urlencoded) are rejected with `415 Unsupported Media Type` to prevent browser CSRF.

## Development

```sh
cargo test-native      # native tests for core/llm/storage (rusqlite, in-memory)
cargo build-backend    # backend component → target/wasm32-wasip2/release
cargo build-frontend   # frontend → target/wasm32-unknown-unknown/release
cd frontend && trunk serve   # frontend dev server on :8080
```

When developing the frontend against a running `spin up` instance, point it
at the API with `?api=http://localhost:3000/api`.

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
