# Daily Retrospective

Goal: extract durable knowledge missed during the day and keep `memory.db`
current.

## Steps

1. Identify the review date/time window and the available platform conversation
   source. Never imply that `mem retro` contains chat logs.
2. If platform context is unavailable, mark it unavailable and limit the review
   to store maintenance; report conversation-derived checks as skipped rather
   than reconstructing them.
3. Run `mem retro daily` to get active memories, recent changelog, pending
   ambiguities, workflow runs, stats, and audit context.
4. Compare only the available platform context with existing memory.
5. Save confirmed durable facts with `source=daily_retro`.
6. Detect repeated manual procedures and propose `type=workflow` candidates.
   Save one only after the user approves creating the runbook.
7. Notice repeated helper code and classify it with `workflow-rules.md` Reusable
   Scripts; propose ownership changes instead of moving files silently.
8. Update or supersede stale memories when evidence is clear.
9. Add ambiguity records when two memories conflict or scope is unclear.
10. Run `mem stats` and `mem audit`.
11. Report concise counts, skipped checks, candidates, and unresolved questions.
12. If the store changed, offer sync per `SKILL.md` Safety Gates.

## What to Save or Propose

- Repeated user preferences.
- Explicit corrections.
- Project decisions, deadlines, owners, constraints.
- Stable references to systems, repos, docs, or workflows.
- User-approved `type=workflow` memories for repeated project procedures, with
  `workflow:*` and `intent:*` tags.
- Artifact candidates: apply the ownership rules in `workflow-rules.md`, report
  the proposed owner, and wait for approval before moving anything.

## What Not to Save

- Secrets or credentials.
- Raw logs.
- Temporary debugging state.
- Facts with no future utility.

## Report Shape

```text
Daily memory retro:
- window: <YYYY-MM-DD>
- conversation context: available | unavailable
- added: 3
- updated: 1
- superseded: 1
- ambiguities: 1 pending
- workflow/artifact candidates: ...
- skipped checks: ...
- suggested questions: ...
```
