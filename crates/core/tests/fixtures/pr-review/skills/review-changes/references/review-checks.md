# Review checks

Use the checks relevant to the change; do not report a checklist as findings.

- Trace input validation, boundary values, missing data and error paths.
- Check callers, wire formats, compatibility and changed return/error semantics.
- Check state ownership, stale asynchronous results, concurrent updates,
  cancellation and retry behavior. Retries must not duplicate side effects.
- Check persistence, migrations, destructive operations and recovery behavior.
- Check permissions and handling of untrusted content against the project's
  actual threat model. Do not propose removing required features.
- Check every supported execution mode through the shared feature entry point;
  flag duplicated orchestration or a mode that lacks the requested behavior.
- Check existing tests and whether a regression is actually demonstrated.
  Successful tests do not establish behavior they do not cover.
- Check roadmap/changelog claims against what the implementation delivers.

For each candidate finding, establish an example of a valid input or sequence
that fails after the change, and explain why the intended contract requires a
different result. Drop findings you cannot substantiate, or clearly present a
specific unresolved question rather than claiming a defect.
