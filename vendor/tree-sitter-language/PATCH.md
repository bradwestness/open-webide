# Shared browser grammar compatibility

Source: crates.io tree-sitter-language 0.1.8, MIT. The Rust language API is unchanged.

The browser compiler adapter supplies these headers to all grammar C builds.
Header additions expose wchar_t, C11 static_assert and ASCII isdigit required by
the C++ and Bash scanners. They retain the existing upstream runtime contracts.

DEP_TREE_SITTER_LANGUAGE_WASM_SRC points to empty legacy translation units:
released C/C#/CSS build scripts still compile stdio/stdlib/string from that path,
while Tree-sitter 0.27 supplies the actual routines. The original compatibility
files deliberately fail compilation for the retired 0.26 implementation.
This patch requires the pinned Tree-sitter 0.27 runtime; it does not implement
or replace allocator, string, character or IO runtime behavior.

Remove when released grammars and upstream headers support the same build.
Packaging excludes Cargo.lock and registry metadata.
