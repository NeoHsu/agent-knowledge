# Daily Retrospective

Goal: extract durable knowledge missed during the day and keep `memory.db` current.

## Steps

1. Use the active platform's available conversation context or history. Repository readers are optional adapters for platforms that cannot expose logs directly.
2. Run `bin/mem retro daily` to get active memories, recent changelog, pending ambiguities, stats, and audit context.
3. Compare available platform context with existing memory.
4. Save new facts with `source=daily_retro`.
5. Update or supersede stale memories when evidence is clear.
6. Add ambiguity records when two memories conflict or scope is unclear.
7. Run `bin/mem stats` and `bin/mem audit`.
8. Report concise counts and unresolved questions.
9. Commit and push the repository.

## What to Save

- Repeated user preferences.
- Explicit corrections.
- Project decisions, deadlines, owners, constraints.
- Stable references to systems, repos, docs, or workflows.

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
