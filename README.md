<img src="frontend/pwa/favicon.svg" width="80" height="80" alt="Open WebIDE house logo" />

# Open WebIDE

A self-hosted browser IDE for coding with local LLMs: edit files, run an agent,
review its changes, and use Git and a terminal in one workspace. Connect to
Ollama or OpenAI-compatible model servers such as llama.cpp and LM Studio.

The Rust frontend and backend run on WebAssembly; a native execution bridge
provides terminals, commands, Git operations, and persistent agent runs. Projects,
sessions, and preferences live in SQLite, so you can resume from another browser.

## What you can do

- Edit code with syntax highlighting, search, Git diffs, and Markdown/image previews.
- Work across project tabs, with an automatically refreshed file tree and the
  last-used session restored when opening a project.
- Chat with a coding agent that reads, searches, edits, runs commands, and uses Git;
  review pending edits with Accept/Reject controls.
- Choose Default, Auto-accept edits, Auto, or YOLO approval modes with `Shift+Tab`.
  Auto uses an optional fast model, falling back to the primary model.
- Choose personal primary/fast defaults in Settings. Configure shared servers,
  credentials, context, sampling, output, thinking, and tool settings from Servers. The saved auto-compaction
  threshold defaults to 85% (0 disables it). Local and remote runs compact before model requests, reserve output space, and retain original history alongside saved summaries.
  Server setup uses a rerunnable provider → URL/auth → model discovery wizard; review and customize detected settings before applying. Re-run it from Servers or the model setup toolbar button.
- Use slash commands such as `/model`, `/test`, `/diff`, `/commit`, and `/sync`,
  with live token/speed telemetry and a context-window gauge.

See the [roadmap](docs/roadmap.md) for remaining work and the
[changelog](CHANGELOG.md) for completed changes.

The [project site guide](docs/project-site.md) explains how the landing page and documentation
are built and published to GitHub Pages from this repository.

Documentation and project updates: [openwebide.com](https://openwebide.com/).

## Run with Docker

The first public image release is being prepared. Once published, download the
versioned Compose file from [GitHub Releases](https://github.com/openwebide/openwebide/releases)
and run `docker compose up -d` without cloning or building. See
[releases, configuration and upgrades](docs/releases.md) for installation and backup
instructions. The source-build path below remains available now.

Existing installs should follow the [deployment-name migration notes](docs/releases.md#existing-installations-with-the-old-project-name)
to retain their database and Tailscale identity when adopting the `openwebide` name.

From a checkout of this repository:

```sh
docker compose up --build
```

Open <http://localhost:8080/> and register the first account. Compose mounts
`~/source` as `/workspace`, persists the database in the `openwebide-data` volume,
and exposes the bundled bridge on port 3001. Choose a different workspace with:

```sh
OPENWEBIDE_WORKSPACE=/path/to/projects docker compose up --build
```

For a standalone container:

```sh
docker build -t openwebide .
docker run -d --name openwebide \
  -p 8080:3000 -p 3001:3001 \
  -v openwebide-data:/app/.spin -v ~/source:/workspace openwebide
```

The workspace mount is accessible to file tools and the agent; choose its scope
accordingly. `.spin/` is excluded from file access. The bundled bridge persists
its secret at `/app/.spin/bridge-secret`; set `OPENWEBIDE_BRIDGE=0` to disable it
and use SSE chat instead. If overriding the container command, retain
`spin up --direct-mounts --allow-transient-write` so edits reach the mounted files.
Legacy remote paths are migrated automatically on startup.

For Linux services, see the [Podman quadlet guide](docs/podman-quadlet.md).
For private HTTPS access from phones and other devices, the recommended approach
is [Tailscale Serve in a separate container](docs/tailscale.md).

## Run from source

Install [Rust](https://rustup.rs/), [Spin 4.x](https://spinframework.dev/docs/latest/installation/),
and [Trunk](https://trunkrs.dev/getting-started/install/). Rust's toolchain and WASM
targets are pinned in `rust-toolchain.toml` and installed by rustup on first build.
The browser syntax parser also needs Clang (Xcode Command Line Tools on macOS;
`sudo apt-get install clang` on Debian/Ubuntu). Rust's pinned `llvm-tools` component
supplies the WASM archiver; the build selects it automatically, including on macOS
where Apple's archiver does not retain WASM objects.
Host Git operations require Git 2.23 or newer.

Build from the repository root:

```sh
NO_COLOR=true spin build
cargo build -p openwebide-bridge
```

Start Spin and the bridge in separate terminals:

```sh
spin up --direct-mounts --allow-transient-write
```

```sh
./target/debug/openwebide-bridge --port 3001 --workspace ../.. --host 127.0.0.1 \
  --backend-url http://127.0.0.1:3000/api --secret-file /tmp/openwebide-bridge-secret
```

Open <http://localhost:3000/>. The bridge workspace must match the backend's
`files.source` in `spin.toml` (currently `../..`). Both services need normal host
filesystem access: launching them inside a coding agent's sandbox can prevent
writes to other projects. Keep the Spin flags above to avoid editing a temporary
copy of the workspace.

## Open a project

After signing in, add your model server in Settings, choose a primary model,
and open a project. Both workspace modes share the editor and agent features:

| Mode | Folder picker | File access |
| --- | --- | --- |
| Remote | Folders on the Open WebIDE host | Backend workspace mount |
| Local | Folders on the browser's device | Chromium File System Access API |

Local mode stores browser directory handles in IndexedDB. For commands and Git,
a bridge on that device must also see the selected folder: start it inside the
project or with a workspace containing it within five directory levels. A temporary
probe file verifies the match and is removed afterward. If the folder is unreachable,
those tools are hidden and chat explains how to enable them. An interrupted local
run can be continued with **Resume**.

## Connect over the LAN

Use `http://<host-ip>:8080` for Docker or port 3000 for a source build. For a native
bridge, bind it to `0.0.0.0` instead of `127.0.0.1` so browsers can reach port 3001.

When using a custom hostname, set `OPENWEBIDE_BRIDGE_ALLOWED_HOSTS=<hostname>`.
If the page and bridge use different hostnames, also set
`OPENWEBIDE_BRIDGE_ALLOWED_ORIGINS=http://<page-host>:<port>`. Both accept
comma-separated lists and are passed through by Compose. A backend connecting to
a bridge over a non-loopback address also needs an explicitly configured shared
secret; see the [bridge reference](docs/execution-bridge.md).

## Accounts and agent execution

Accounts use argon2id password hashing and HttpOnly session cookies. Registration
closes after the first account. Logging out invalidates that user's sessions across
devices. Preferences and session state are database-backed rather than localStorage.

File tools are confined to the workspace; commands and Git run as the bridge's
host user. Approval modes control when the agent asks permission. Before overwriting
binary or large files, the agent keeps originals in `.openwebide/backups/` so Reject
can restore them; retain those backups until the edits no longer need restoration.

The backend can reach LAN and public model servers, search, and documentation
pages. Web fetching blocks cloud metadata addresses. `fetch_web_page` goes through
the approval policy; `search_web` is auto-approved. The bridge checks request hosts,
origins, and authentication to protect against malicious browser tabs.

## Development

```sh
cargo test -p openwebide-storage -p openwebide-core -p openwebide-auth -p openwebide-agent -p openwebide-llm -p openwebide-bridge -p openwebide-backend
cargo test -p openwebide-frontend --lib
CHROMEDRIVER=<path> cargo test -p openwebide-frontend --target wasm32-unknown-unknown
```

Browser tests require Chrome, chromedriver, and `wasm-bindgen-cli` matching
`Cargo.lock`. For frontend-only development, run `cd frontend && trunk serve`
and open `http://localhost:8080/?api=http://localhost:3000/api` with Spin running.
Use the same hostname for both services so session cookies work; the API override
is accepted only from `localhost:8080` or `127.0.0.1:8080`.

| Directory | Purpose |
| --- | --- |
| `frontend/` | Leptos WASM app, built with Trunk |
| `backend/` | Spin REST/SSE component |
| `bridge/` | Native terminals, execution, and agent hosting |
| `crates/` | Shared domain, agent, provider, authentication, and storage code |

Further reference: [architecture](docs/architecture.md),
[execution bridge](docs/execution-bridge.md), and [API examples](docs/api-examples.md).

## License

[MIT](LICENSE).
