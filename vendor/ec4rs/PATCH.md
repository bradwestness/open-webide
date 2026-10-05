Vendored from the crates.io ec4rs 1.2.0 release (Apache-2.0).
Archive SHA-256: 3b31a881d38439026e3d5dd938ab20328d36e23caca8fd5981c42e4b677f5842

The only Rust source change is in src/glob/splitter.rs: the WASI adapter uses
OsStr::as_encoded_bytes instead of std::os::wasi::ffi::OsStrExt::as_bytes.
The old import requires the unstable wasip2 standard-library feature on our
stable wasm32-wasip2 backend target. Workspace paths are valid UTF-8, for which the two methods return the same bytes.
Native and browser parsing/glob behavior is unchanged.

Keep this patch until upstream releases a stable WASI-compatible version.
The editor uses ConfigParser over workspace-supplied UTF-8 bytes; it does not
call the crate's host-filesystem discovery functions.

The manifest declares the upstream doc_unstable cfg for Cargo check-cfg, so
modern stable builds do not emit an unexpected-cfg warning.
