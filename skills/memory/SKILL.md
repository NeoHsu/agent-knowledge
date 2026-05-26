# Memory Skill

Use this skill when the user asks to save, recall, update, clean up, review, or migrate durable knowledge. The active knowledge store is the current `AGENT_KNOWLEDGE_HOME` or repository root; `memory.db` is the runtime source of truth and `index/` is rebuildable.

## When to Use

- The user says "幫我存這個", "remember this", "記住", or gives durable feedback.
- A task needs prior preferences, project facts, or recurring decisions.
- The user asks for daily or weekly retrospective.
- The user asks to export, import, audit, or merge memory.

Do not store raw secrets, transient chat filler, or one-off facts that will not help future work.

## Quick Reference

Run commands from the repository root:

```bash
bin/mem save --type feedback --name no_emoji --scope global --source manual --tags '["style"]' --content "不要使用 emoji"
bin/mem query "部署" --scope auto
bin/mem update no_emoji --content "不要在回覆中使用 emoji"
bin/mem supersede old_name new_name --content "replacement memory"
bin/mem delete old_name
bin/mem history --recent
bin/mem stats
bin/mem audit
bin/mem retro daily
bin/mem retro weekly
bin/mem reindex
```

Full CLI details: `references/cli-guide.md`.

## Tag Extraction

Prefer typed tags in `type:value` form:

- `person:<name>`
- `project:<owner/repo>`
- `domain:<topic>`
- `tool:<name>`
- `style:<preference>`
- `decision:<topic>`

Keep tags stable, lowercase, and specific. Details: `references/tag-rules.md`.

## Save Workflow

1. Decide if the knowledge is durable.
2. Choose `type`: `user`, `feedback`, `project`, or `reference`.
3. Choose `scope`: `global` or `project:<owner/repo>`.
4. Extract tags.
5. Run `bin/mem save`.
6. If duplicate/similar/conflict is returned, decide whether to update, supersede, or skip.
7. Commit and push memory changes when the work unit is complete.

`source=manual` is protected and high confidence. `source=agent` is medium confidence. Retrospective sources are low confidence.

## Query Workflow

At task start:

```bash
bin/mem context --detect
bin/mem query --scope auto --type feedback
bin/mem query "<task keywords>" --scope auto
```

Load only relevant memories into the answer context.

## Daily Retrospective

Use `references/daily-retro.md` when the user asks for daily review. The short flow is:

1. Use platform-provided conversation context or logs; repo readers are optional adapters.
2. Run `bin/mem retro daily` for current memory, changelog, ambiguity, and audit context.
3. Compare available platform context against existing memory.
4. Save new durable knowledge, update stale knowledge, and record ambiguities.
5. Report counts and pending questions.
6. Commit and push.

## Weekly Retrospective

Use `references/weekly-retro.md` when the user asks for weekly review. The weekly review reads `changelog`, `memory.db`, and `ambiguities`, not raw logs. It improves memory quality: merge duplicates, calibrate confidence, identify candidates for skills, resolve ambiguities, and audit health.
