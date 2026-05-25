# Daily Retrospective

Goal: extract durable knowledge missed during the day and keep `memory.db` current.

## Steps

1. Read `config.yaml` and choose readers marked `detected: true`.
2. Run each reader for today's logs.
3. Run `bin/mem export --format json`.
4. Compare raw logs with existing memory.
5. Save new facts with `source=daily_retro`.
6. Update or supersede stale memories when evidence is clear.
7. Add ambiguity records when two memories conflict or scope is unclear.
8. Run `bin/mem stats` and `bin/mem audit`.
9. Report concise counts and unresolved questions.
10. Commit and push the repository.

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
