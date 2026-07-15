# Weekly Retrospective

Goal: improve memory quality. Weekly retro uses active memories, changelog,
ambiguities, workflow runs, pending graph edges, stats, and audit context from
the store; it does not reread raw session logs.

## Steps

1. Run `mem retro weekly --limit 200`.
2. Run `mem export --format json --include-superseded` if full memory content
   is needed.
3. Run `mem ambiguity list --pending` when resolving conflicts.
4. Review `audit` and `stats` sections from the retro bundle.
5. Identify duplicate or near-duplicate memories.
6. Identify stale workflow steps, workflows with repeated failures, and
   repeated manual procedures that should become workflow memory.
7. Identify repeated helper code and classify it with `workflow-rules.md`
   Reusable Scripts.
8. Propose stable cross-project execution policy as a skill candidate; do not
   create or edit a skill as a retrospective side effect.
9. Calibrate confidence:
   - frequently accessed low confidence can become medium after review
   - stale medium confidence can be downgraded or marked for cleanup
10. Resolve ambiguities when scope or newer evidence makes the answer clear.
11. Curate every `over_budget_scopes` entry down to `per_scope_max`: merge
    duplicates, supersede stale facts, and delete obsolete low-access memories.
    Raise `budget.per_scope_max` only when a scope genuinely needs more.
12. Run `mem reconcile --scope <project-scope> --repo <checkout>` for each
    project touched this week. Judge every flagged memory and fix it with
    `mem update`, `mem supersede`, or `mem delete`.
13. Run `mem audit --fix` only when deterministic repairs are needed and after
    confirming the active target per `SKILL.md` Safety Gates.
14. If the store changed, offer sync per `SKILL.md` Safety Gates.

The promotion direction is:

```text
repeated facts/preferences -> memory
repeated project procedures -> workflow memory
project-specific reusable helpers -> project repo scripts/
cross-project reusable helpers -> knowledge-store artifacts/
stable cross-project execution policy -> skill
```

Do not silently move scripts into or out of `artifacts/` under the active
knowledge store root. Recommend the ownership change, get user approval, then
add or update manifest entries with `mem artifact add` or
`mem artifact update --checksum`.

A skill candidate is an output for review, not authorization to modify the
skill collection. After approval, route a new skill to the environment's skill
creation workflow (`skill-creator` in this collection) and route an existing
skill audit/refactor to `skill-hygiene` when available.

## Output

Keep the report short:

```text
Weekly memory retro:
- review window: <YYYY-Www>
- merged: 2
- confidence changes: 3
- pending ambiguities: 1
- cleanup candidates: 4
- workflow candidates: ...
- artifact candidates: ...
- skill candidates: ...
```
