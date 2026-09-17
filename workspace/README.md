# Remote workspace

This folder is mounted into the Spin backend as the remote-mode workspace
root (see the `files` entry in `spin.toml`).

Remote-mode projects store their `path` relative to this directory. The
`/api/projects/<id>/files...` endpoints list, read, write, create, and search
files under it.
