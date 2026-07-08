# Weekly Retrospective

Goal: improve memory quality. Weekly retro reads `changelog`, `memory.db`, and `ambiguities`; it does not reread raw session logs.

## Steps

1. Run `mem retro weekly --limit 200`.
2. Run `mem export --format json --include-superseded` if full memory content is needed.
3. Run `mem ambiguity list --pending` when resolving conflicts.
4. Review `audit` and `stats` sections from the retro bundle.
5. Identify duplicate or near-duplicate memories.
6. Identify stale workflow steps, workflows with repeated failures, and repeated manual procedures that should become workflow memory.
7. Identify repeated helper code and classify it with `workflow-rules.md` Reusable Scripts.
8. Promote stable cross-project execution policy into a skill or reference document when useful.
9. Calibrate confidence:
   - frequently accessed low confidence can become medium after review
   - stale medium confidence can be downgraded or marked for cleanup
10. Resolve ambiguities when scope or newer evidence makes the answer clear.
11. Run `mem reconcile --scope <project-scope> --repo <checkout>` for each project scope touched this week; judge every flagged memory and fix it with `mem update`, `mem supersede`, or `mem delete`.
12. Run `mem audit --fix` if deterministic repairs are needed.
13. Offer sync. If approved, run `mem sync --dry-run`, then `mem sync --no-push` unless the user explicitly approves a remote push.

The promotion direction is:

```text
repeated facts/preferences -> memory
repeated project procedures -> workflow memory
project-specific reusable helpers -> project repo scripts/
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
