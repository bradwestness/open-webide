---
title: Open source software
description: The open-source projects and maintainers that make Open WebIDE possible.
---

# Open source software

Open WebIDE is built on the work of open-source maintainers and contributors.
Thank you to everyone who builds and maintains these projects.

Open WebIDE's own code is [MIT licensed](https://github.com/openwebide/openwebide/blob/main/LICENSE).
Third-party projects retain their own licenses. The inventory below links to
upstream projects and reports license declarations from their package metadata.
License texts and copyright notices are maintained in those projects' distributions.

The website generates the Rust inventory directly from the repository's manifests
and `Cargo.lock`, including direct, transitive and vendored crates, whenever it
builds. See the [project site guide](project-site.md) for the build command.

<!-- dependency-inventory -->

## Other components

- [Monaspace](https://github.com/githubnext/monaspace): bundled variable editor fonts
  (Argon, Neon, Xenon, Radon and Krypton), release v1.400. Copyright © 2023 GitHub;
  licensed under the [SIL Open Font License 1.1](https://github.com/openwebide/openwebide/blob/main/frontend/fonts/OFL.txt).
  The unmodified fonts and license are distributed together in `frontend/fonts/`.
- [SQLite](https://sqlite.org/): embedded database, bundled through `libsqlite3-sys`.
  SQLite is [in the public domain](https://sqlite.org/copyright.html).
## Development and runtime tools

We also thank [Rust](https://www.rust-lang.org/),
[Trunk](https://trunkrs.dev/), [Spin](https://github.com/spinframework/spin),
[Ollama](https://github.com/ollama/ollama) and
[llama.cpp](https://github.com/ggml-org/llama.cpp).
These provide the language toolchain, application build and hosting, and supported
model servers.
