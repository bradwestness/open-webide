# Editor controls

Edit runs in Rust/WebAssembly and uses the same commands and file policies for
local and remote projects. The full editor roadmap is still in progress.

- Tab advances to the next indentation stop; Shift+Tab outdents selected lines.
- Enter retains indentation and uses the configured line ending, or the file's
  first line ending when no rule is set. Supported code languages indent after an
  opening bracket; Python also indents after a code colon. Enter inside an empty
  bracket pair puts the closer on its own line. Strings and comments are opaque.
- Typing a closing bracket on an otherwise empty indented line aligns it with its
  opener. Brackets and language-supported quotes auto-close in code, wrap selected
  text, and skip an existing closer. Backspace between an empty pair removes both.
  Rust apostrophes are inserted without auto-closing so lifetimes remain ordinary
  typing; selected text can still be wrapped in character quotes. Comments and
  strings retain ordinary typing behavior.
- Ctrl+Z / Cmd+Z undo; Ctrl+Shift+Z / Cmd+Shift+Z redo (Ctrl+Y also works).
- Ctrl+M toggles whether Tab indents or moves keyboard focus out of the editor.

The editing menu provides line and comment commands and **Reindent selected
lines**. Keyboard equivalents:

| Command | Shortcut |
| --- | --- |
| Move selected lines | Alt+Up / Alt+Down |
| Duplicate lines above/below | Alt+Shift+Up / Alt+Shift+Down |
| Duplicate selected text (or current line) | Ctrl/Cmd+Shift+D |
| Delete selected lines | Ctrl/Cmd+Shift+K |
| Insert an indented line above/below | Ctrl/Cmd+Shift+Enter / Ctrl/Cmd+Enter |
| Toggle line/block comments | Ctrl/Cmd+/ / Ctrl/Cmd+Shift+/ |
| Paste and match indentation | Ctrl/Cmd+Shift+V |

Normal paste preserves clipboard whitespace. Matching indentation is explicit:
remove the snippet's common leading indentation, preserve relative indentation,
and rebase subsequent lines to the receiving line. Whitespace-only paste is kept;
configured line endings apply to inserted text. Paste and reindent each form one
undo step. Reindent aligns selected bracket-delimited blocks, leaves multiline
string contents untouched and preserves Python's existing block depth; it is not
a language formatter. Unsupported comment/reindent actions appear disabled.

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

Structural commands currently use bounded lexical analysis: files over 2 MiB or
65,536 bracket tokens fall back to ordinary indentation and typing. JavaScript
template strings remain opaque, including their interpolations. Parser-backed
editing commands and large-file benchmarks remain roadmap work.

The edit view retains each file’s caret, selection direction and horizontal/vertical
scroll position within its project while the app is open, including when switching
to a diff view and back. New files start at the beginning; changing accounts clears
these positions. Reload persistence is part of the remaining draft-recovery work.

The syntax/folding foundation now includes a shared incremental Rust parser. Its
previous tree is updated with Unicode-safe byte edits; fold candidates cover code
blocks, declarations and multiline comments/literals. Cancellation, parser work
limits and oversized files discard stale results. Other languages and
worker/viewport work remain under development;
the existing editing commands continue to use their documented lexical structure.

Frontend builds enable the core `editor-parser` feature. Backend WASI builds do not
need the parser or a WASI C SDK. The Rust grammar’s small build-script patch uses
upstream WASM headers; see `vendor/tree-sitter-rust/PATCH.md`.

The shared document now owns fold state independently of undo history. Commands
can collapse/expand at the caret, recursively or all, and reveal a navigation target.
The projection retains logical source row numbers and maps Unicode-safe byte offsets
and directional selections between visible text and the full source. Native edits
that cross omitted text explicitly require revealing it first. Edits open affected
folds and rebase unaffected headers; stale ranges stay invalid until refreshed from
the new source. Grouped undo/redo rebases the final transaction result once.

These primitives and the folding view use the shared editor facade in both modes.

## Folding

Rust files show fold controls beside the logical line numbers for declarations,
blocks and multiline comments/literals. Click a control to collapse or expand;
the editing menu also offers cursor, recursive and all-document commands.
Ctrl/Cmd+Alt+[ folds at the cursor and Ctrl/Cmd+Alt+] unfolds; add Shift for
recursive commands. Fold state belongs to the document, including across view
and file switches during the current app lifetime.

Hidden lines remain in the source and copied selections. Syntax highlighting
retains the full file's context, and Find reveals a hidden match. Native editing,
paste, cut and composition reveal the selected source lines before the browser
changes them; disjoint folds remain collapsed. Commands and undo/redo rebase the
remaining anchors and restore the source caret through the updated projection.
Input-only browser events replay their change against the complete source.
Unchanged projected values and selections stay under the native input method's
control during composition.
Other files use the shared language-aware lexer for bracket blocks and multiline
comments/literals, with indentation fallback when no parser is available. Adjacent
full-line comments can fold as a group. Indentation uses the configured tab width;
blank rows do not create blocks, and multiline literal contents do not contribute
fake indentation. YAML block scalars remain opaque.

Balanced, nested `region` / `endregion` markers in language comments also fold
(for example `// #region Name` / `// #endregion` or `# region Name` /
`# endregion`). Plain text supports `#region` / `#endregion`, and C/C++ supports
`#pragma region` / `#pragma endregion`. Unmatched markers remain ordinary text.
Rust parser ranges take precedence over indentation; all ranges retain one
control per header and cannot cross one another. Synchronous fallback work is
limited to 2 MiB, 100,000 lines and the lexer's bracket limit. Richer language
parsers, worker rendering and real-device input verification remain roadmap work.
