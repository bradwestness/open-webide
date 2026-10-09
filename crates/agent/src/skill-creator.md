# Create or improve a project skill

Follow this workflow with the user; this tool supplies guidance, not a completed
skill or evaluation result. Use the configured model and existing approved tools.

1. Capture intent from the conversation first: desired outcome, when to trigger,
   expected output, recurring steps, corrections, dependencies and success criteria.
   Ask only for missing decisions. Distinguish reusable workflows from project facts
   (memories). Read the original skill and relevant resources when improving one.
2. Draft a concise name and trigger description. Put what it does and specific
   usage contexts in the description. Put the actual workflow in instructions.
   Explain why important steps matter; avoid rigid rules and example overfitting.
   Preserve an existing name unless the user requests a rename.
3. Keep instructions lean, preferably under 500 lines. Move detailed references,
   templates and repeatable helper scripts into named resources. Link to resources
   explicitly and say when to load them through skill_read(id, resource). Follow
   next_offset to finish reading paged instructions and resources before using them.
   Resource text is not executed automatically; execution uses ordinary approved
   tools and the current workspace. Do not promise unavailable runtimes or tools.
4. Offer evaluation appropriate to the task. Objective outputs benefit from
   assertions; subjective outputs need user review. Let the user choose depth.
   Start with 2–3 realistic prompts and expected results; save reusable evaluation
   cases as a resource such as references/evals.json. Include edge cases.
5. For rigorous evaluation, compare the draft with no skill (new skill) or the
   previous version (improvement), using the same model and equivalent fresh
   context. If this runtime cannot isolate runs, ask the user to run the prompts
   in fresh sessions; clearly label same-session checks as weaker evidence.
   Never invent completed runs, scores, token counts or timings.
6. Show actual outputs, assertion evidence and user feedback together before
   revising. Compare pass rates and time/token usage only when measured. Look for
   flaky assertions and tests that pass regardless of the skill. Generalize from
   feedback, remove wasteful instructions and bundle repeated helper work.
7. Iterate with the user. Test trigger descriptions using realistic should-trigger
   prompts and adjacent near-misses that should not trigger. For larger test sets,
   reserve held-out cases so improvements are not selected on training results.
8. Save through skill_create or skill_update using the latest revision and normal
   mutation approvals. Never overwrite a conflicting edit. Use the Sessions Skills
   panel to review/edit the saved skill and export its SKILL.md/resources as ZIP.
   Report what was saved and which evaluations actually ran.

Source: Anthropic skill-creator, reviewed 2026-10-09:
https://github.com/anthropics/skills/blob/main/skills/skill-creator/SKILL.md
This is an adapted workflow for Open WebIDE, without a Claude CLI dependency.
