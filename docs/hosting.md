# Hosting and administration

The container bundles the Spin backend and native execution bridge. Mount your
project directory and persist the app's database before using it for ongoing work.

- [Releases, configuration and backups](releases.md): installation files,
  environment settings, release channels, upgrades and restoration.
- [Podman services](podman-quadlet.md): rootless systemd services and automatic
  registry updates.
- [HTTPS with Tailscale](tailscale.md): private access across devices without
  exposing the app publicly.
- [Execution bridge](execution-bridge.md): native execution, authentication,
  browser pairing and connection configuration.

For a first installation, begin with [the quick start](../README.md#run-with-docker).
For an existing installation using the old deployment name, read the
[volume migration notes](releases.md#existing-installations-with-the-old-project-name)
before changing Compose configuration.
