# File and folder menus

Right-click a file or folder, hold it for half a second on touch devices, or focus
it and press Shift+F10/the context-menu key. The inline **…** button opens the same
menu. Escape or an outside click dismisses it; arrow keys navigate the menu.

Create, rename, move, copy the project-relative path, or delete an entry. Move
accepts a destination relative to the project; its parent folder must exist.
Delete requires confirmation. Save or discard affected unsaved editor changes
before rename, move, stage or ignore; resolve pending edit reviews first.

Git actions follow the file/folder's index and working-tree status. Ignore appends
an anchored literal rule to `.gitignore`, preserving existing rules and newline
style. Revert restores HEAD in both the index and working tree, including a
rename's original paths, after confirmation. Untracked files remain; a conflicting
untracked replacement blocks Revert. Unstage also works before the first commit.
Git requires an available execution bridge and, for local projects, a matching
host folder. Filesystem and chat actions remain available without Git.

Explain, Summarize and Review changes append a path-aware prompt to the chat draft.
They open Chat without sending the prompt automatically.

Both workspace modes use the same mutation policy. Moves copy and verify before
removing the original; they are not atomic. A failed operation reports whether
an incomplete destination or both locations remain. Moves are limited to 10 MiB
per file, 32 MiB total and 10,000 entries, and reject symbolic-link paths and
Git/runtime internals. Larger moves should use the host filesystem tools.

Existing **…** menus on sessions, servers, system prompts, chat messages and panel
headers also support right-click, long press and the keyboard context-menu keys.
Text inputs retain their native browser editing menus.
