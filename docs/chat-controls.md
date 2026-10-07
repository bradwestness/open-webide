# Chat commands and goals

These controls work the same in local and remote projects, and in projectless chats.
Host command and Git capabilities still depend on the execution bridge.

## Discover commands

Type `/` in the composer to see commands with argument hints. Up and Down select
an entry; Tab or Enter completes a partial command name. Enter sends an exact
command. Clicking an entry also completes it. Escape dismisses the suggestions
without changing the draft. Image attachments keep the prompt on the normal send
path rather than executing a slash command.

Use `/help` for commands and shortcuts, or `/help goal` to search the command list.
`/model` lists models; `/tokens` and `/context` show context accounting.

## Follow activity

Consecutive tool calls and finished reasoning-only messages share an expandable
activity summary. The step count includes those reasoning messages; the duration
sums recorded tool execution time, excluding approval waits. Expand the group to
inspect individual reasoning and tool results. Approvals and failures force the
group open, so neither is hidden behind a collapsed summary. Running tools and their
activity summary use the same animated spinner as thinking blocks; approval waits,
completed tools and cancelled runs stop the animation. Replies remain visible.

## Compact saved context

When the run is idle, send `/compact` to summarize its saved conversation with
the existing compaction engine. The summary becomes the context for later turns;
original messages remain in the database and visible history. Explicit compaction
works even when automatic compaction is disabled. It uses the configured fast
model when suitable, with primary-model fallback and the same budget checks.

The model needs a configured or detected context limit. A failed summary leaves
original history intact. Stop cancels the operation. If another window sends a
message during summarization, the stale summary is rejected; retry when idle.

## Work toward a goal

Send `/goal <objective>` to save an objective in this session and start an agent
run. A session is created if needed. Use `/goal start <objective>` when the text
would otherwise be interpreted as a control command.

The goal panel shows the objective alongside the agent's existing plan and progress.
The usual tools, permissions and Stop control apply. An agent reply leaves the goal
open for review; it does not automatically mark the objective successful or launch
another run.

| Command | Action |
| --- | --- |
| `/goal` or `/goal status` | Show the saved objective and state. |
| `/goal pause` | Save the paused state and stop this goal's current run. |
| `/goal resume` | Continue from saved history with the same objective. |
| `/goal complete` | Confirm completion after reviewing the result. |

The panel offers Continue, Pause and Mark complete buttons. Continue preserves an
unsent draft. Complete the current goal before starting another one. Goals survive
reloads and device changes through session-scoped database storage. Returning to
an active goal shows review/continue controls; recovery does not replay tools or
start work automatically. A forked conversation starts without the source session's
goal. Concurrent changes from another window require a refresh before updating.
