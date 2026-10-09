# Plugins and marketplaces

The **Plugins** button beside **Output** in the status bar opens discovery and
plugin management. Browse the official marketplace and any custom public Git
marketplaces configured for your account. Installed and available plugins appear
in compact rows with publisher/version details and inline actions. Search filters
both sections. Listings show their marketplace as a Source label, with the official
source named Open WebIDE. Click a row’s name to inspect plugin details
and use the gear menu to choose a release. Install it on the open project's
execution host, or on the server host when no project is open. Plugins currently
contribute agent skills.

Installation does not enable a plugin. **Enable for project** loads the verified
instructions/resources into the database as managed project skills. They appear
in Skills and use the existing `skill_list` and `skill_read` agent tools. Their
package provenance is visible; update, disable or remove them through Plugins.
Personal skills remain independent, and a duplicate skill name aborts activation
without overwriting it. The project's global Skills switch still applies.

Use **Manage marketplace sources** in the Plugins hamburger menu to open
Settings → Plugins. That settings tab
only configures marketplace sources; search, install, uninstall and project
enable/disable controls live in the Plugins interface.

The official source is `https://github.com/openwebide/plugins.git`, using its
default branch and root `marketplace.json`. It is built in and cannot be removed.
Additional sources accept a public Git repository URL, optional branch/tag/commit
reference, and catalog file path.
Releases declare only their immutable commit and package directory; every package
inherits its marketplace's repository. Refreshing catalogs updates discovery,
while installed versions remain pinned. A failed refresh preserves cached
releases and reports the failed source. Removing a custom source stops discovery
without uninstalling its packages. Sources and caches are user-scoped database settings.

Select another catalog release to update or roll back an installation, then
**Apply installed version** to change this project's active skills. Other projects
retain their enabled versions. **Disable for project** keeps the package installed
and its managed skills saved but inactive. **Uninstall** asks for confirmation,
then removes the logical installation and its managed skills from all your
projects. It retains host snapshots for active runs. Runs pin enabled package
instructions and resources at startup, so changes apply to subsequent runs.

For packages outside a catalog, choose **Install a pinned package manually** from
the Plugins hamburger menu and
enter a public Git repository URL (HTTP, HTTPS, SSH or Git protocol), a full
lowercase commit ID, and the directory containing `plugin.json`; use `.` for the
repository root. The reference PR Review 0.1.0 package uses:

- Repository: `https://github.com/openwebide/plugins.git`
- Commit: `e79185c2b25f713503b70e23ee7e91e66c5af208`
- Package directory: `plugins/pr-review`

Local projects need their paired native bridge and folder access. Remote projects
use the server bridge, including when opened from a phone. The browser does not
clone packages or execute plugin code. Fetching uses the bridge host's existing Git
credentials; credentials cannot be embedded in repository URLs.

The shared plugin facade selects the project host. Both transports use the same
core validation and installation policy. The native bridge reads Git objects from
a bare cache, validates the draft API 1 skills manifest and skill resources, and
publishes a commit/content-addressed snapshot by atomic rename. Hooks, checkout
filters, package scripts and runtime installers are not run. Symlinks, submodules,
path collisions and unsupported contributions are rejected. Packages are limited
to 2,048 files, 128 KiB per file and 16 MiB in total; contributed skills must also
meet the existing project skill import limits.

Caches live outside project folders. Native bridges use
`$XDG_DATA_HOME/openwebide/plugins` or `~/.local/share/openwebide/plugins`;
`OPENWEBIDE_PLUGIN_DIR` overrides that path with an absolute host cache path. The container uses
`/app/.spin/plugins` on the existing persistent volume. Backend users have separate
cache namespaces; locally paired clients use the paired host namespace.

Logical installation records are user-scoped database settings, shared across
clients. They include the source commit, validated manifest, content digest and
historical preparation receipts for each host. Receipts describe a completed
preparation, not ongoing host availability. Reinstalling on another host prepares
that same version there. Preparing the same cached version verifies it before
returning success and works offline. A modified snapshot is rejected rather than
silently repaired in place.

To change the selected commit, refresh the installation list and install the new
commit. Database updates check the installation revision, and preparation failures
leave the previous record and snapshot intact. Account, project, session or host
changes prevent stale browser results from recording an installation. An already
submitted server transaction may finish for its authenticated account.

Automatic updates, runtime dependency preparation and agent-facing plugin
management remain on the roadmap. Language, MCP, UI and editor contributions are
not accepted yet.
