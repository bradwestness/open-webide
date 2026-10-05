# Browser WASM grammar build

Source: crates.io `tree-sitter-rust` 0.24.2, MIT (see LICENSE),
https://github.com/tree-sitter/tree-sitter-rust.

The generated parser, scanner and Rust API are unchanged. The only code patch adds
`DEP_TREE_SITTER_LANGUAGE_WASM_HEADERS` to the C include path for
`wasm32-unknown-*`. These headers come from `tree-sitter-language` 0.1.8, the same
compatibility headers used by the released Tree-sitter runtime. Native builds
retain their existing include paths. Remove this patch when an upstream release
handles the WASM headers itself.

Packaging excludes Cargo.lock and registry metadata.
