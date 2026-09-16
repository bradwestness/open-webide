# Running Open WebIDE as a Podman quadlet

Quadlet lets systemd manage Podman containers with unit files. This turns
Open WebIDE into a normal Linux service: built from the repo's Dockerfile,
started at boot, SQLite persisted on disk.

**Requirements:** Linux with systemd, `podman` (quadlet is built into
recent podman/systemd — `systemd --version` ≥ 252 or a distro that ships
quadlet). Not available on macOS/Windows.

## Unit files

### `open-webide.image` — builds the image from the Dockerfile

```ini
[Image]
Image=open-webide:local
BuildFile=Dockerfile
# Absolute path to the repo checkout (the unit file itself lives in
# ~/.config/containers/systemd/, so the default context is wrong):
BuildContextDirectory=/path/to/open-webide
```

If the image does not exist, quadlet runs `podman build` for it before
starting anything that requires it. Rebuild after code changes with
`systemctl --user restart open-webide.image` after deleting the image
(`podman rmi open-webide:local`), or just tag a new version.

### `open-webide.container` — runs the app

```ini
[Container]
Image=open-webide:local
ContainerName=open-webide
# Host port 8080 keeps it clear of bare `spin build --up` (3000); the
# container itself always listens on 3000.
PublishPort=127.0.0.1:8080:3000
Volume=%h/.local/state/open-webide/data:/app/.spin:Z
Requires=open-webide.image

[Service]
Restart=always

[Install]
WantedBy=default.target
```

Notes:

- `PublishPort=127.0.0.1:8080:3000` binds to loopback only. Use
  `8080:3000` to expose it on all interfaces (then put auth/reverse proxy
  in front — the API has no authentication).
- The volume maps the SQLite data directory (`/app/.spin` inside the
  container) to a host directory. `:Z` relabels it for SELinux; drop the
  suffix on systems without SELinux.
- The first start downloads `spin_static_fs.wasm` (the static file server
  component) from GitHub; after that it is cached in the container image's
  Spin home.

## Install (user-level service)

```sh
mkdir -p ~/.config/containers/systemd
# copy both unit files there, fixing BuildContextDirectory
systemctl --user daemon-reload
systemctl --user start open-webide.container
```

Useful commands:

```sh
systemctl --user status open-webide.container
journalctl --user -u open-webide.container -f   # logs
systemctl --user enable open-webide.container   # start at login
```

For a system-wide service, put the units in `/etc/containers/systemd/`
instead and drop `--user` (use an absolute path for the data volume, e.g.
`/var/lib/open-webide/data:/app/.spin:z`).

## Access

- Frontend: <http://localhost:8080/>
- API: <http://localhost:8080/api/health>
