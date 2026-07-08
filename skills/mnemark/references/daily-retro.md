# Daily Retrospective

Goal: extract durable knowledge missed during the day and keep `memory.db` current.

## Steps

1. Use the active platform's available conversation context or history.
2. Run `mem retro daily` to get active memories, recent changelog, pending ambiguities, stats, and audit context.
3. Compare available platform context with existing memory.
4. Save new facts with `source=daily_retro`.
5. Detect repeated manual procedures and suggest or save `type=workflow` memories when they are durable.
6. Notice repeated helper code and classify it with `workflow-rules.md` Reusable Scripts.
7. Update or supersede stale memories when evidence is clear.
8. Add ambiguity records when two memories conflict or scope is unclear.
9. Run `mem stats` and `mem audit`.
10. Report concise counts and unresolved questions.
11. Offer sync per `SKILL.md` Safety Gates.

## What to Save

- Repeated user preferences.
- Explicit corrections.
- Project decisions, deadlines, owners, constraints.
- Stable references to systems, repos, docs, or workflows.
- Repeated project procedures as `type=workflow` with `workflow:*` and `intent:*` tags.
- Artifact candidates: apply `workflow-rules.md` Reusable Scripts; one-off helpers stay local, project-specific helpers belong in repo `scripts/`, cross-project helpers belong in knowledge-store `artifacts/`, and ownership changes need user approval.

## What Not to Save

- Secrets or credentials.
- Raw logs.
- Temporary debugging state.
- Facts with no future utility.

## Report Shape

```text
Daily memory retro:
- added: 3
- updated: 1
- superseded: 1
- ambiguities: 1 pending
- suggested questions: ...
```
