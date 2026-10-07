# Layout comparison sources

Run these sources through `../measure-editor-layout.py`, not as a separate build
tree. The wrapper creates a temporary Cargo workspace, applies the repository's
current lint policy and WASM binding versions, and forces `CARGO_TARGET_DIR` to the
repository's `target/`. Its source directory is removed on completion.

The supplied TTF is embedded for the native and browser experiments. No font or
candidate dependency is added to the application. `Cargo.lock` freezes candidate
dependencies; update it deliberately when changing the candidate or the project's
WASM binding versions. The browser comparison checks that measurements run and
records geometry differences; it does not assert renderer parity.

See [the measurements and reproduction command](../../docs/editor-performance.md#cold-layout-candidate-check).
