# Weekly Retrospective

Goal: improve memory quality. Weekly retro reads `changelog`, `memory.db`, and `ambiguities`; it does not reread raw session logs.

## Steps

1. Run `mem retro weekly --limit 200`.
2. Run `mem export --format json --include-superseded` if full memory content is needed.
3. Run `mem ambiguity list --pending` when resolving conflicts.
4. Review `audit` and `stats` sections from the retro bundle.
5. Identify duplicate or near-duplicate memories.
6. Identify stale workflow steps, workflows with repeated failures, and repeated manual procedures that should become workflow memory.
7. Promote stable cross-project execution policy into a skill or reference document when useful.
8. Calibrate confidence:
   - frequently accessed low confidence can become medium after review
   - stale medium confidence can be downgraded or marked for cleanup
9. Resolve ambiguities when scope or newer evidence makes the answer clear.
10. Run `mem audit --fix` if deterministic repairs are needed.
11. Commit and push.

The promotion direction is:

```text
repeated facts/preferences -> memory
repeated project procedures -> workflow memory
stable cross-project execution policy -> skill
```

## Output

Keep the report short:

```text
Weekly memory retro:
- merged: 2
- confidence changes: 3
- pending ambiguities: 1
- cleanup candidates: 4
- workflow candidates: ...
- skill candidates: ...
```
