# Editor controls

Edit runs in Rust/WebAssembly and uses the same commands and file policies for
local and remote projects. The full editor roadmap is still in progress.

- Tab advances to the next indentation stop; Shift+Tab outdents selected lines.
- Enter retains indentation and uses the configured line ending, or the file's
  first line ending when no rule is set.
- Ctrl+Z / Cmd+Z undo; Ctrl+Shift+Z / Cmd+Shift+Z redo (Ctrl+Y also works).
- Ctrl+M toggles whether Tab indents or moves keyboard focus out of the editor.

The editor footer shows Spaces/Tabs, the indentation width and the tab width.
These are separate: an indentation step can be four columns while a hard tab
occupies three. Tabs fill as many complete tab stops as possible, then spaces fill
the remainder. Changing these controls affects the current document's commands and
how tabs are displayed. **Convert indentation** explicitly rewrites leading
whitespace while retaining its displayed column width. Conversion is one undo step;
it does not format the rest of the code. Widths are bounded to 1–16 columns.

Set user defaults in **Settings → Editor defaults**. Changes save immediately to the
user's database settings. Per-file footer overrides remain with the open document
for the current app lifetime; they do not change the project's `.editorconfig`.

## File rules

The editor discovers `.editorconfig` files from the file's parent directory up to
the project root. It stops at `root = true`; it does not read outside the workspace.
Closer files and later matching sections override earlier rules. `unset` removes
a property. When no rule applies, existing indentation is detected, then user
defaults fill in missing values.

Supported editing/save properties are `indent_style`, `indent_size` (including
`tab`), `tab_width`, `end_of_line` (`lf`/`crlf`), `insert_final_newline` and
`trim_trailing_whitespace`. Charset conversion is not performed; Edit requires
UTF-8. Glob sections support paths, wildcards, alternatives and integer ranges.

Without explicit save rules, line endings, trailing whitespace and the presence
or absence of a final newline are preserved. Configured cleanup is applied as one
undoable command before writing. Empty documents never gain a final newline.
Undoing cleanup after a successful save marks the buffer dirty again.

Unreadable, invalid or oversized configuration files produce a notice and fall
back to the remaining rules, detected indentation and defaults. Configuration
files are limited to 256 KiB. Saving `.editorconfig` refreshes the active file's
rules; switching files rediscovers rules. A pending discovery is discarded after
a project, folder, bridge or account change.
