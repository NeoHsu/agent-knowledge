# Weekly Retrospective

Goal: improve memory quality. Weekly retro reads `changelog`, `memory.db`, and `ambiguities`; it does not reread raw session logs.

## Steps

1. Run `mem retro weekly --limit 200`.
2. Run `mem export --format json --include-superseded` if full memory content is needed.
3. Run `mem ambiguity list --pending` when resolving conflicts.
4. Review `audit` and `stats` sections from the retro bundle.
5. Identify duplicate or near-duplicate memories.
6. Identify stale workflow steps, workflows with repeated failures, and repeated manual procedures that should become workflow memory.
7. Identify repeated ad hoc scripts, shell snippets, templates, or references that should become repository scripts or knowledge-store artifacts.
8. Promote stable cross-project execution policy into a skill or reference document when useful.
9. Calibrate confidence:
   - frequently accessed low confidence can become medium after review
   - stale medium confidence can be downgraded or marked for cleanup
10. Resolve ambiguities when scope or newer evidence makes the answer clear.
11. Run `mem audit --fix` if deterministic repairs are needed.
12. Run `mem sync` to commit and push the store.

The promotion direction is:

```text
repeated facts/preferences -> memory
repeated project procedures -> workflow memory
project-specific reusable scripts -> project repo scripts/
cross-project reusable helpers -> knowledge-store artifacts/
stable cross-project execution policy -> skill
```

Do not silently move scripts into or out of `artifacts/` under the active knowledge store root. Recommend the ownership change, get user approval, then add or update manifest entries with `mem artifact add` or `mem artifact update --checksum`.

## Output

Keep the report short:

```text
Weekly memory retro:
- merged: 2
- confidence changes: 3
- pending ambiguities: 1
- cleanup candidates: 4
- workflow candidates: ...
- artifact candidates: ...
- skill candidates: ...
```
