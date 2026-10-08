# Project memory

Project memories hold durable facts and conventions that should carry across a
project’s chat sessions. They belong to the signed-in user and project, with the
same database-backed behavior in local and remote workspaces. Projectless chats
have no memory tools or automatic project-memory context.

Open **Sessions → Memories** to add an entry, expand it to read its contents,
edit it or delete it. Give each entry a short title and focused content, such as
how to build the project, a design decision or a convention. Refresh reloads entries
changed by another device; entries also refresh when a run finishes.

The **Enabled** toggle in the section header starts on and is saved per project.
Turning it off keeps existing entries available for viewing, editing and deletion,
but new runs receive neither automatic memory context nor memory tools. Already
sent model requests and past conversation messages are unchanged. Memory tool
calls check the current setting again at execution, so disabling also blocks
subsequent calls from an ongoing run.

## Automatic context and tools

Enabled runs receive recent memory titles, IDs, revisions and short excerpts.
The context includes at most 24 entries, 256 characters per excerpt and 8 KiB total,
and uses at most approximately 10% of a known model context limit under the shared
byte-based estimate. Very small context budgets omit automatic excerpts. Full
entries stay available through these agent tools:

| Tool | Behavior |
| --- | --- |
| `memory_create` | Store a title and durable project fact. |
| `memory_search` | Search titles and content; an empty query lists recent entries. Up to 20 matches with short excerpts. |
| `memory_read` | Read one complete entry and its revision. |
| `memory_update` | Replace an entry using its current revision. |
| `memory_delete` | Delete an entry using its current revision. |

Configured server tool selections still apply; chat-only runs get no tools.
Reads and searches run without approval. Creation, updates and deletion follow the
selected approval mode, like other persistent writes. Approval prompts show the
proposed memory content, or the current entry being deleted. A project holds at most 100
entries, with titles up to 120 characters and content up to 4,000 characters.
Updates and deletions require the current revision; conflicts preserve the newer
entry and keep a user’s editing draft so they can refresh and reconcile it.

Memories are reference data, not higher-priority instructions. Verify facts that
may have become stale. Avoid credentials, secrets and temporary task checklists;
use the session checklist for work in progress. Deleting a session leaves project
memories intact. Deleting the project removes its memories.

## Implementation

`openwebide-core::memory` owns types, validation and bounded context.
`Store` owns project/session ownership, durable records, opt-out and atomic revision
checks. `openwebide-agent::memory` shapes requests and tool results once; browser,
Spin and bridge adapters supply only session-bound persistence calls.
The Sessions UI calls the `ProjectMemoryActions` facade and rejects stale results
after navigation, account changes or newer requests.
