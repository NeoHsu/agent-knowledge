# Daily Retrospective

Goal: extract durable knowledge missed during the day and keep `memory.db` current.

## Steps

1. Use the active platform's available conversation context or history. Repository readers are optional adapters for platforms that cannot expose logs directly.
2. Run `mem retro daily` to get active memories, recent changelog, pending ambiguities, stats, and audit context.
3. Compare available platform context with existing memory.
4. Save new facts with `source=daily_retro`.
5. Detect repeated manual procedures and suggest or save `type=workflow` memories when they are durable.
6. Update or supersede stale memories when evidence is clear.
7. Add ambiguity records when two memories conflict or scope is unclear.
8. Run `mem stats` and `mem audit`.
9. Report concise counts and unresolved questions.
10. Commit and push the repository.

## What to Save

- Repeated user preferences.
- Explicit corrections.
- Project decisions, deadlines, owners, constraints.
- Stable references to systems, repos, docs, or workflows.
- Repeated project procedures as `type=workflow` with `workflow:*` and `intent:*` tags.

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
