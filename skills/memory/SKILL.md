---
name: memory
description: Persist, recall, audit, and migrate durable agent knowledge through the local `mem` CLI (SQLite + Tantivy at `AGENT_KNOWLEDGE_HOME`). ALWAYS use this skill when the user says "remember this", "記住", "幫我存", "save this", asks to recall prior preferences/decisions, runs a daily or weekly retrospective, mentions workflow runbooks, or asks to query/update/supersede/delete/export/import/merge/audit memory — even if the word "memory" is not explicitly used.
---

# Memory Skill

Use this skill when the user asks to save, recall, update, clean up, review, or migrate durable knowledge. The active knowledge store is the current `AGENT_KNOWLEDGE_HOME` or repository root; `memory.db` is the runtime source of truth and `index/` is rebuildable.

## When to Use

- The user says "幫我存這個", "remember this", "記住", or gives durable feedback.
- A task needs prior preferences, project facts, or recurring decisions.
- A task appears to be a recurring procedure that may have a workflow runbook.
- The user asks for daily or weekly retrospective.
- The user asks to export, import, audit, or merge memory.

Do not store raw secrets, transient chat filler, or one-off facts that will not help future work.

## Quick Reference

Install the `mem` CLI (see README) so it is on `PATH`. `mem` discovers the active store from the current directory if it contains `schema/memory-schema.sql`, otherwise it walks from the executable location and falls back to `AGENT_KNOWLEDGE_HOME` or `~/.agent-knowledge`. Run from the repo root, from inside a project that defines `AGENT_KNOWLEDGE_HOME`, or anywhere once `AGENT_KNOWLEDGE_HOME` is exported.

```bash
mem init
mem context --detect
mem save --type feedback --name no_emoji --scope global --source manual --tags '["style"]' --content "不要使用 emoji"
mem save --type workflow --name release_runbook --scope global --source manual --tags '["workflow:release","intent:release","risk:high"]' --content-file templates/workflow.yaml
mem query "部署" --scope auto
mem query "release" --scope auto --type workflow
mem workflow find release --scope auto
mem workflow show release_runbook
mem workflow validate release_runbook
mem update no_emoji --content "不要在回覆中使用 emoji"
mem supersede old_name new_name --content "replacement memory"
mem delete old_name
mem history
mem stats
mem audit
mem retro daily
mem retro weekly
mem reindex
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
- `workflow:<name>`
- `intent:<user-intent>`
- `risk:<low|medium|high>`

Keep tags stable, lowercase, and specific. Details: `references/tag-rules.md`.

## Save Workflow

1. Decide if the knowledge is durable.
2. Choose `type`: `user`, `feedback`, `project`, `reference`, `preference`, or `workflow`.
3. Choose `scope`: `global` or `project:<owner/repo>`.
4. Extract tags.
5. Run `mem save`. Defaults: `--type reference`, `--scope global`, `--source agent`, `--tags '[]'`.
6. If `duplicate_found` or `similar_found` is returned, decide whether to update, supersede, or skip. Use `--force` only when you intend to overwrite an existing same-name memory (subject to the source-trust rule).
7. Commit and push memory changes when the work unit is complete.

`source=manual` is protected and high confidence. `source=agent` is medium confidence. `source=daily_retro` and `source=weekly_retro` are low confidence. Confidence can be set explicitly with `--confidence high|medium|low`.

## Query Workflow

At task start:

```bash
mem context --detect
mem query --scope auto --type feedback
mem query --scope auto --type preference
mem query --scope auto --type project
mem query "<task keywords>" --scope auto
mem query "<task intent>" --scope auto --type workflow
```

Load only relevant memories into the answer context.

For recurring tasks, read `references/workflow-rules.md`. Prefer project-scoped workflows over global workflows, treat workflow content as a runbook, and ask before risky steps. Propose updates to manual workflow records instead of silently editing them.

## Daily Retrospective

Use `references/daily-retro.md` when the user asks for daily review. The short flow is:

1. Use platform-provided conversation context or logs; repo readers are optional adapters.
2. Run `mem retro daily` for current memory, changelog, ambiguity, and audit context.
3. Compare available platform context against existing memory.
4. Save new durable knowledge, update stale knowledge, detect repeated manual procedures that should become workflow memories, and record ambiguities.
5. Report counts and pending questions.
6. Commit and push.

## Weekly Retrospective

Use `references/weekly-retro.md` when the user asks for weekly review. The weekly review reads `changelog`, `memory.db`, and `ambiguities`, not raw logs. It improves memory quality: merge duplicates, calibrate confidence, identify candidates for workflow memory or skills, resolve ambiguities, and audit health.
