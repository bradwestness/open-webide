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

With multiple selections, Copy joins their source text in primary-selection order;
Cut removes those ranges in one undo step after writing the clipboard. Paste puts
one clipboard line into each selection when the line and selection counts match;
otherwise it repeats the complete text at every selection. Copy also includes compact
selection metadata: when it survives the clipboard and the cursor counts match,
each cursor receives its original fragment, including multiline and empty fragments.
Paste and match indentation rebases each fragment at its receiving line. Changed,
invalid or stripped metadata uses the plain-text behavior above. CRLF separators
between distributed plain-text lines are removed from their bodies. Clipboard
failures leave the source unchanged. Clipboard edits are disabled during an active input composition.

Use **Editing commands** or these shortcuts for multiple selections:

| Action | Shortcut |
| --- | --- |
| Select word, then next occurrence | Ctrl/Cmd+D |
| Select all occurrences | Ctrl/Cmd+Shift+L |
| Add cursor above/below | Ctrl/Cmd+Alt+Up/Down |
| Expand/shrink selection | Alt+Shift+Right/Left |
| Keep primary cursor | Escape |

Alt-click adds/removes a cursor. Alt+Shift click/drag selects a column from the
primary anchor, honoring tab stops and complete Unicode graphemes. Secondary
carets and selections use the same font metrics as the syntax paint, including
wrapped text; screen readers receive the cursor count. Arrow keys move all cursors
by grapheme or logical line; Ctrl/Alt+Left/Right move by word, and Shift extends
each selection. Home/End move to line boundaries, Ctrl/Cmd+Home/End to document
boundaries; on macOS Cmd+arrows use line/document boundaries. Vertical movement
retains the desired column across short lines. Multiple selection movement and
structural selection commands are bounded to files up to 2 MiB; Escape still
returns to the primary cursor in larger files.

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

The syntax/folding foundation uses shared incremental grammar providers for Rust,
TypeScript/TSX, JavaScript/JSX, Python, Java, C#, C++, PHP, Shell, C, Go, HTML and
CSS. Trees update with Unicode-safe byte edits. Fold descriptors cover blocks,
declarations and multiline comments/literals, with Python suite headers retained.
Cancellation and oversized files discard stale trees; unsupported languages use
lexical/indentation folding. Custom `SyntaxProvider` descriptors use the same
update and fold policy through `SyntaxDocument::with_provider`.

Frontend builds enable the core `editor-parser` feature. Backend WASI builds do not
need the parser or a WASI C SDK. A shared browser compiler adapter supplies portable
C headers; see `vendor/tree-sitter-language/PATCH.md`. The existing Rust grammar
patch remains compatible with that adapter.

HTML script/style bodies use independent JavaScript/CSS parsers and full-file
fold coordinates, including after Unicode/CRLF edits. Declared types select
supported bodies; JSON data scripts and unsupported style types stay in HTML.
Each provider can supply an injection selector returning validated source ranges.
The shared engine limits a document to 64 embedded bodies and discards every tree
on cancellation or an exceeded limit.

Paired typing/deletion and Enter use immutable parser contexts from the shared
per-document cache. Each cursor uses its own language, including JavaScript/CSS
in HTML, and template interpolation code remains editable while literal text is
protected. Incomplete input uses bounded lexical fallback; JavaScript template
interpolation also works during incomplete typing. Contexts verify their exact
source before a command can mutate history. Custom provider classifiers use the
same traversal, limits and fallback policy.

Comment/reindent commands, selection/navigation, richer highlighting and
worker/viewport rendering remain roadmap work; those commands continue to use
their documented bounded lexical structure.

The shared document now owns fold state independently of undo history. Commands
can collapse/expand at the caret, recursively or all, and reveal a navigation target.
The projection retains logical source row numbers and maps Unicode-safe byte offsets
and directional selections between visible text and the full source. Native edits
that cross omitted text explicitly require revealing it first. Edits open affected
folds and rebase unaffected headers; stale ranges stay invalid until refreshed from
the new source. Grouped undo/redo rebases the final transaction result once.

These primitives and the folding view use the shared editor facade in both modes.

## Folding

Supported code files show fold controls beside the logical line numbers for declarations,
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
Languages without a grammar use the shared language-aware lexer for bracket blocks and multiline
comments/literals, with indentation fallback when no parser is available. Adjacent
full-line comments can fold as a group. Indentation uses the configured tab width;
blank rows do not create blocks, and multiline literal contents do not contribute
fake indentation. YAML block scalars remain opaque.

Balanced, nested `region` / `endregion` markers in language comments also fold
(for example `// #region Name` / `// #endregion` or `# region Name` /
`# endregion`). Plain text supports `#region` / `#endregion`, and C/C++ supports
`#pragma region` / `#pragma endregion`. Unmatched markers remain ordinary text.
Parser ranges take precedence over indentation; all ranges retain one
control per header and cannot cross one another. Synchronous fallback work is
limited to 2 MiB, 100,000 lines and the lexer's bracket limit. Worker rendering
and real-device input verification remain roadmap work.

## Reading and navigation

Ctrl/Cmd+G opens **Go to line/column**; enter `line` or `line:column` and press
Enter. The editing menu and cursor-status footer open the same control. Coordinates
are one-based logical source lines and Unicode character columns; a tab is one
source character. Valid coordinates beyond the file clamp to its end. Invalid
input disables Go. Navigation reveals collapsed destinations and scrolls the
source caret into view, including long horizontal lines.

Ctrl/Cmd+Shift+\ jumps to the matching bracket beside the caret. Comments and
literals remain opaque. The active line and matching brackets are highlighted,
and the footer shows line, column and selected character count. Indentation
guides use visual tab stops and complete indentation steps, independently of
source columns; blank lines continue the common surrounding indentation.
These controls also work in read-only files and share the same source-coordinate
facade in both modes. Horizontal scrolling remains the default. **Settings → Editor defaults** offers
**Word wrap** and **Show whitespace**, saved with the existing user-scoped database
preferences. These reading options affect Edit without changing file content.
Wrapping keeps one gutter number per logical source line and measures fold-row
heights after resizing; navigation uses the rendered caret position. Whitespace
markers show spaces, tabs and line endings while retaining their original text
nodes and source offsets. The paint adapter normalizes CRLF to match the native
textarea; the document and saved file retain their original separators.

## Find and replace

Ctrl/Cmd+F opens Find; Ctrl+H or Cmd+Alt+F opens Replace. Enter and Shift+Enter
move to the next/previous match. Search starts case-sensitive; **Match case**,
**Whole word** and **Regex** are explicit options with visible match counts.
Whole-word boundaries include Unicode letters and combining marks. Regex mode
uses [Rust regex syntax](https://docs.rs/regex/1.13.1/regex/): look-around and backreferences in the pattern are not
supported, and invalid patterns display an error. Line anchors understand LF/CRLF.

Select source text before opening Find to enable **In selection**. Search keeps
full-source context, so the selection edges do not create artificial word or line
boundaries. Replacement adjusts that scope; ordinary edits or switching files
clear it. Find reveals folded matches and works in numbered diff views.

**Replace next** changes the current match; **Replace all** changes all matches
in scope in one undo step. Enter in Replace performs next; Ctrl/Cmd+Enter performs
all. Regex replacement accepts `$1`, `${name}` and `$$` for a literal dollar;
literal replacement leaves dollar signs unchanged. Pending review, read-only
files and diff views disable replacement. Invalid patterns/scopes and size errors
leave text and history intact.

Search is bounded to 2 MiB source files, 64 KiB patterns and 100,000 matches;
replacement output is limited to 32 MiB. Empty search input has no matches.
Zero-width regex matches advance at Unicode boundaries and replacement is one
finite transaction. Large-file worker/viewport support remains roadmap work.

## File buffers

Opening another file retains unsaved text for the current app lifetime; it no
longer asks to discard the previous file. Returning restores its document history,
selection, folds and scroll position. Clean files are read again to pick up disk
changes; unsaved files keep their text. Save writes only the selected file.

Filesystem actions protect unsaved buffers even when another file is selected.
Confirmed delete/revert clears affected buffers. Loading temporarily disables
editing, and late reads cannot replace newer input or another account/project.
The file tab strip keeps opening order, shows unsaved indicators, and supports
Left/Right/Home/End navigation. Closing an unsaved file asks before discarding it;
Cancel leaves it open. Closing the selected file chooses an adjacent tab. A close
confirmation cannot discard newer edits or files from another account/project.
Database draft/reload recovery is still on the roadmap; refreshing currently loses
these in-memory buffers.

## Recovery contract (integration in progress)

The shared Rust recovery format and `/api/projects/{id}/editor-recovery` GET/PUT
endpoints persist tab order, selection, per-file scroll/read-only state and document
snapshots in user-scoped database settings. PUT requires the revision returned by
GET; a stale revision or changed project root returns 409. Closing every tab retains
a revision rather than deleting it, so an older window cannot resurrect its drafts.
Project deletion removes its recovery setting. Ordinary settings reads exclude
recovery bodies, and ordinary settings writes cannot bypass revision checks.

Snapshots capture committed text rather than an active IME preview. The saved
baseline, selections and collapsed folds restore with the draft; its edits become
one undoable recovery transaction. Paths, Unicode offsets, folds, scroll values,
format versions and bounds are validated in shared code. Text encoding keeps
control-heavy files within the same wire bounds as ordinary files. Limits are
64 tabs and 128 MiB of text/metadata, with each document using the editor's existing
32 MiB text limit. Failed writes preserve the preceding stored recovery.

The frontend backend facade now exposes typed recovery requests and collects
coherent active/hidden snapshots with the original saved baselines. HTTP 409 is
kept distinct from transport errors; recovery replies and save acknowledgements
are validated. Old-session REST responses cannot expire a newly active session.
Client autosave, hydration, workspace permissions, disk-change/conflict feedback
and retry behavior still need integration. The app currently keeps buffers in
memory; refreshing still loses them.

Recovery hydration primitives prepare all documents before publishing any state,
restore ordered tabs and active or hidden buffers, and preserve unrelated workspace
state. Guards reject a changed project, reset, pending read, document revision,
selection, draft, or composition. Old media URLs are returned to the caller for
revocation. Shared disk reconciliation preserves conflicting/missing drafts, detects
an already completed write, and refreshes clean files. Optional disk reads distinguish
missing ancestors from permission/transport errors through the same Workspace
primitive used by rewind. The automatic scheduler, hydration wiring and conflict UI
remain pending; these primitives alone do not restore files on refresh.
