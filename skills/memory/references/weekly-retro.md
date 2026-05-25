# Weekly Retrospective

Goal: improve memory quality. Weekly retro reads `changelog`, `memory.db`, and `ambiguities`; it does not reread raw session logs.

## Steps

1. Run `bin/mem history --recent --limit 200`.
2. Run `bin/mem export --format json --include-superseded`.
3. Run `bin/mem ambiguity list --pending`.
4. Run `bin/mem stats` and `bin/mem audit`.
5. Identify duplicate or near-duplicate memories.
6. Promote repeated patterns into a skill or reference document when useful.
7. Calibrate confidence:
   - frequently accessed low confidence can become medium after review
   - stale medium confidence can be downgraded or marked for cleanup
8. Resolve ambiguities when scope or newer evidence makes the answer clear.
9. Run `bin/mem audit --fix` if only safe repairs are needed.
10. Commit and push.

## Output

Keep the report short:

```text
Weekly memory retro:
- merged: 2
- confidence changes: 3
- pending ambiguities: 1
- cleanup candidates: 4
- skill candidates: ...
```
