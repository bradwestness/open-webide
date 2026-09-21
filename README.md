# Open WebIDE

A WebAssembly-based IDE for working with local-LLM coding agents. Think
Open WebUI, but for agentic coding — and the whole stack (frontend *and*
backend) runs on WebAssembly.

- **Frontend:** Rust + [Leptos](https://leptos.dev) compiled to WASM, built with [Trunk](https://trunkrs.dev)
- **Backend:** a [Spin](https://spinframework.dev) component (Rust → `wasm32-wasip2`) exposing a small REST API
- **Persistence:** SQLite via Spin's `sqlite` capability (file-backed, like Open WebUI)
- **LLM providers:** Ollama and llama.cpp behind a common provider interface

## Status

Chat sessions and both workspace modes are in: multi-turn conversations
stream token-by-token from the backend and persist across reloads, and you
can open a folder — on the machine running Spin (remote) or in the browser
via the File System Access API (local) — browse it, and create/edit/save
files, with several projects open in tabs at once.

Working:
- Repo layout and workspace wiring
- Domain types (`crates/core`)
- Provider interface with Ollama / llama.cpp implementations (`crates/llm`), tested against a fake HTTP client
- SQLite-backed storage with migrations and typed repositories (`crates/storage`), tested natively with rusqlite
- Backend REST API: health, connections CRUD, settings, system prompts, models, chat, sessions, message streaming (SSE), projects, and remote-mode file access (list/read/write/create/search)
- Frontend: top bar with live backend health, sidebar (projects with a remote/local mode picker, sessions: new/switch/rename/delete, connections), file tree explorer (browse, create file/folder, search), code editor with save, multi-project tabs (open/switch/close), chat pane with markdown rendering + streaming, input, send, stop, status bar
- Local workspace mode: open a folder in the browser via the File System Access API, with the directory handle persisted in IndexedDB and permission re-requested on reload
- CI: fmt, clippy, native tests, WASM builds, Trunk build

Not yet:
- Agentic coding (tool calls + agent loop)

## Prerequisites

- Rust (stable) with targets `wasm32-wasip2` and `wasm32-unknown-unknown`
  (`rustup target add wasm32-wasip2 wasm32-unknown-unknown`)
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
docs/roadmap.md        phase-by-phase plan
```

See [docs/architecture.md](docs/architecture.md) for the architecture and
[docs/roadmap.md](docs/roadmap.md) for the plan.

## License

MIT — see [LICENSE](LICENSE).
