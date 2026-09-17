# Open WebIDE

A WebAssembly-based IDE for working with local-LLM coding agents. Think
Open WebUI, but for agentic coding — and the whole stack (frontend *and*
backend) runs on WebAssembly.

- **Frontend:** Rust + [Leptos](https://leptos.dev) compiled to WASM, built with [Trunk](https://trunkrs.dev)
- **Backend:** a [Spin](https://spinframework.dev) component (Rust → `wasm32-wasip2`) exposing a small REST API
- **Persistence:** SQLite via Spin's `sqlite` capability (file-backed, like Open WebUI)
- **LLM providers:** Ollama and llama.cpp behind a common provider interface

## Status

Chat sessions are in: multi-turn conversations stream token-by-token from
the backend, persist across reloads, and show history in the sidebar.

Working:
- Repo layout and workspace wiring
- Domain types (`crates/core`)
- Provider interface with Ollama / llama.cpp implementations (`crates/llm`), tested against a fake HTTP client
- SQLite-backed storage with migrations and typed repositories (`crates/storage`), tested natively with rusqlite
- Backend REST API: health, connections CRUD, settings, system prompts, models, chat, sessions, message streaming (SSE)
- Frontend: top bar with live backend health, sidebar (sessions: new/switch/rename/delete, connections), chat pane with markdown rendering + streaming, input, send, stop, status bar
- CI: fmt, clippy, native tests, WASM builds, Trunk build

Not yet:
- Workspace modes (open/browse/edit a project folder)
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
