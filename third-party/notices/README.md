# Supplemental upstream notices

Some published crate archives omit their license texts. `index.json` maps exact
crate versions to upstream notices saved here, with the source URL pinned to the
commit recorded in that crate's `.cargo_vcs_info.json`. Identical texts share a
file. Winapi target import-library crates have no VCS metadata; their notices use
the parent winapi project's 0.3.9 release. Lucide's original icon license follows
the Git submodule revision pinned by lepticons 0.13.1.

The shared generator reads these files alongside license/copyright/notice files
from Cargo's resolved distributions. Builds never download upstream notices.
When updating dependencies, check for archives without notice files and refresh
exact-version entries from their upstream revision. A missing notice is explicitly
identified in the inventory; a declared license is not presented as its full text.
