# Agent skills

Skills hold reusable workflows for a project. They belong to the signed-in user
and project, live in the database and carry across sessions and devices. Local
and remote projects have the same controls and agent tools. Projectless chats do
not receive project skills.

Open **Sessions → Skills**. Use **+** to write a skill, expand an entry to read it,
or choose **Edit**, **Delete** or **Export ZIP**. Give it a lowercase kebab-case
name, a description explaining what it does and when to use it, and instructions.
Add named supporting references, helper scripts or assets as needed. Resources
stay in the database; scripts are never executed by importing or reading a skill.

The project **Enabled/Disabled** control keeps saved skills available for
administration while disabling their agent tools and automatic catalog. An
individual skill's **Enable this skill** checkbox controls discovery and loading
of that workflow. Changes are checked at tool execution as well as at the next
run. Existing requests and conversation history are unaffected. Entries refresh
automatically and after runs finish.

## Import and export

**Import file or archive** accepts a Markdown skill file with YAML frontmatter,
or a ZIP, `.skill` (ZIP), tar, tar.gz or tgz archive. **Import folder** reads a
folder containing one `SKILL.md` and its supporting files. A manifest looks like:

```markdown
---
name: review-build
description: Review build failures when asked to validate project changes.
---
Read references/checks.md, then run the appropriate approved checks.
```

Archives can have files at their root or inside a single skill folder. Additional
YAML frontmatter such as license, compatibility and metadata is preserved.
Text resources remain editable; binary assets retain their original bytes through
base64 storage. All imports open an editing draft for review. **Save skill** adds
it to the database; Cancel leaves existing skills alone. Importing an existing
name reports a conflict rather than replacing it.

**Export ZIP** downloads `<name>.zip` containing `<name>/SKILL.md` plus the skill's
supporting resources. The ZIP can be imported again or used by a compatible skill
consumer. Export includes portable skill content; the project's enabled setting
is an Open WebIDE preference rather than portable frontmatter.

Decoding runs in the app, without requiring a bridge or extracting files to the
host. Imports reject traversal paths, links, duplicate files, multiple manifests
and files outside the skill folder. Archives are limited to 1 MiB, and a saved
skill to 128 KiB of content including metadata and resources. Each project holds
up to 100 skills; each skill has at most 16 resources. Names are limited to 64
characters, descriptions to 1,024, and instructions and individual resource
contents to 32,768 characters (binary limits apply to their base64 representation).
Unsupported formats or oversized imports show an error without saving a partial
skill.

## Agent discovery and tools

Runs receive a bounded catalog of enabled names and trigger descriptions, up to
8 KiB and approximately 10% of the model's context under the shared byte estimate,
reduced further when tool schemas and project instructions leave less room.
Instructions and resources are loaded only on demand. If the catalog omits a
skill, the agent can discover it with `skill_list`.

| Tool | Behavior |
| --- | --- |
| `skill_list` | List up to 20 enabled summaries per page, optionally matching names or descriptions; follow `next_offset`. |
| `skill_read` | Read a page of instructions, revision and resource names, or one named resource; follow `next_offset`. |
| `skill_create` | Save a reusable skill and its resources. |
| `skill_update` | Update a skill using its current revision; omitted resources, metadata and enabled preference are preserved. |
| `skill_delete` | Remove a skill using its current revision. |
| `skill_creator` | Load the guided workflow for creating or improving a skill from a goal or conversation. |

Read windows default to 4,000 characters, with a maximum of 8,000. The agent must
finish reading all instruction pages before following a skill; binary resources
are returned as base64 pages for reconstruction. Catalog descriptions are
shortened to 256 characters; the full description is available on read.

Reads and discovery run without approval. Saving, editing and
deleting follow the run's normal approval mode and show the proposed content.
Configured server tool selections still apply. Skills cannot expand tool
permissions or override user instructions. Stale revisions report a conflict;
the editing draft is retained so the user can reconcile the newer version.
Deleting a session leaves skills intact; deleting its project removes them.

## Creating and improving a skill

Ask the agent to turn a recurring workflow or the current conversation into a
skill. Install the Skill Authoring plugin from Plugins; its `skill-authoring`
skill supplies an adaptation of
[Anthropic's skill-creator workflow](https://github.com/anthropics/skills/blob/main/skills/skill-creator/SKILL.md),
reviewed on October 9, 2026, using your configured Ollama or llama.cpp model.
It captures intent, drafts concise instructions and resources, offers realistic
test prompts and baseline comparisons, collects user feedback and iterates.
Trigger evaluation includes related prompts that should not activate the skill.

The skill provides guidance; it does not secretly launch benchmarks or save a
skill. The agent continues with the user and existing tools. Rigorous evaluation
requires comparable fresh sessions; when isolation is unavailable, the workflow
asks the user to run the prompts in fresh sessions and labels weaker checks.
Only actual outputs and measured results should be reported. Saving uses the
same revision checks and approvals as other skill mutations.

## Implementation

`openwebide-core::skills` owns validation, catalog budgeting and portable formats.
`Store` owns persistence, session/project ownership, opt-out and atomic revision
checks. `openwebide-agent::skills::SkillTools` implements discovery, loading,
mutation previews and creator guidance once. Browser, Spin and bridge adapters
supply session-bound database calls. Sessions uses `ProjectSkillActions` for all
administration, import and export actions; it rejects responses from older
requests, projects, accounts and sessions. Archive decoding is shared Rust code
compiled to WebAssembly, with thin file-picker and download primitives.
