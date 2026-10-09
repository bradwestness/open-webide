# PR review

A first-party skills-only reference package. `review-changes` reviews working-tree
changes, branches or pull requests using the agent's existing tools. It does not
add a GitHub connection, MCP server, runtime or permission grant.

## Try the skill today

In OpenWebIDE, open **Sessions → Skills → Import folder** and select
`skills/review-changes`. Inspect the draft, save it and enable it for the project.
Ask the agent to review changes. The skill and its resource use the existing
project-skill format and importer in either workspace mode.

This manual import creates a project skill, not a plugin installation. Plugin
installation and updates are not implemented yet. The package is intentionally
absent from `marketplace.json` until a real commit-pinned release can be listed.

## Contribution

- `review-changes`: scope identification, evidence-based review and concise
  findings, with a supporting checklist loaded when needed.

Example prompts:

- “Review my uncommitted changes for bugs.”
- “Review this branch against main; focus on concurrency regressions.”
- “Review this PR and draft findings here.”

Requests to implement a feature or explain code should not trigger this skill.
Publishing comments or approvals is not part of the default review workflow.
