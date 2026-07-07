---
name: mnemark
description: Use mnemark to persist, recall, audit, migrate, and retrospect durable agent memory through the local `mem` CLI and its active SQLite + Tantivy knowledge store. ALWAYS use this skill when the user says "remember this", "記住", "幫我存", "save this", asks to recall prior preferences/decisions, runs a daily or weekly retrospective, mentions workflow runbooks, or asks to query/update/supersede/delete/export/import/merge/audit memory — even if the word "memory" is not explicitly used.
---

# mnemark Skill

Use this skill when the user asks to save, recall, update, clean up, review, or migrate durable knowledge through mnemark. The active knowledge store is discovered from `--home`, the current directory when it contains `schema/memory-schema.sql`, an executable-near repo root, `MNEMARK_HOME`, user config, or the default root; `memory.db` is the runtime source of truth, `manifest.toml` and `artifacts/` hold portable helper-file metadata and files, and `index/` is rebuildable.

## When to Use

- The user says "幫我存這個", "remember this", "記住", or gives durable feedback.
- A task needs prior preferences, project facts, or recurring decisions.
- A task appears to be a recurring procedure that may have a workflow runbook.
- The user asks for daily or weekly retrospective.
- The user asks to export, import, audit, or merge memory.

Do not store raw secrets, transient chat filler, or one-off facts that will not help future work.

## Setup

Install the `mem` CLI (see README) so it is on `PATH`. `mem` discovers the active store from the current directory if it contains `schema/memory-schema.sql`, otherwise it walks from the executable location and falls back to `MNEMARK_HOME`, `knowledge_home` in `~/.config/mnemark/config.toml`, or `~/.mnemark`. Runtime stores do not need schema files because the schema is embedded in the binary. CLI/tool settings and artifact manifests use TOML; workflow runbooks use YAML. Use `mem config show` to debug the active root and effective defaults.

```bash
mem init
mem setup claude-code
mem doctor
```

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

Write memory content in the trigger/action/why shape from `references/memory-quality.md` so weaker models can apply it mechanically.

1. Decide if the knowledge is durable.
2. Choose `type`: `user`, `feedback`, `project`, `reference`, `preference`, or `workflow`.
3. Choose `scope`: `global` or `project:<owner/repo>`.
4. Extract tags.
5. Run `mem save`. Defaults: `--type reference`, `--scope global`, `--source agent`, `--tags '[]'`.
6. If `duplicate_found` or `similar_found` is returned, decide whether to update, supersede, or skip. Use `--force` only when you intend to overwrite an existing same-name memory (subject to the source-trust rule).
7. If the result carries `warnings` (`no_tags`, `content_long`, `relative_date_language`, `vague_name` — mechanically checked by `mem save`, see `references/memory-quality.md`), fix the memory with `mem update` instead of leaving it degraded.
8. Run `mem sync` when the work unit is complete to commit and push the store through its git repository.

```bash
mem save --type feedback --name no_emoji --scope global --source manual --tags '["style"]' --content "不要使用 emoji"
mem update no_emoji --content "不要在回覆中使用 emoji"
mem supersede old_name new_name --content "replacement memory"
mem delete old_name
mem sync
```

`source=manual` is protected and high confidence. `source=agent` is medium confidence. `source=daily_retro` and `source=weekly_retro` are low confidence. Confidence can be set explicitly with `--confidence high|medium|low`.

Full CLI details, including `history`/`stats`/`audit`/`gc`/`export`/`import`/`merge`/`reindex`: `references/cli-guide.md`.

## Query Workflow

At session or task start, one command loads the durable context:

```bash
mem prime
```

`prime` is read-only, budget-capped, and always targets the runtime store. If a session-start hook already injected the mnemark context block, do not run it again. For task-specific lookups beyond the primed block:

```bash
mem query "<task keywords>" --scope auto --format compact
mem query "<task intent>" --scope auto --type workflow
```

Load only relevant memories into the answer context.

For recurring tasks, read `references/workflow-rules.md`. Prefer project-scoped workflows over global workflows, treat workflow content as a runbook, and ask before risky steps. Workflow memories may reference repository scripts or knowledge-store artifacts, but they should not embed script bodies. Run `mem workflow validate --check-artifacts` only after referenced knowledge-store artifact files and manifest entries exist. Propose updates to manual workflow records instead of silently editing them.

```bash
mem workflow find release --scope auto
mem workflow show release_runbook --checklist
mem workflow record release_runbook --result success --note "clean run"
```

## Artifact Guidance

Use `artifacts/` under the active knowledge store root for reusable cross-project helper scripts, templates, snippets, and references that should travel with the memory store. `$MNEMARK_HOME` is one way to choose that root, but `--home`, repository discovery, or user config may choose a different active store. Use repository `scripts/` for project-specific executable logic. Artifact metadata belongs in `manifest.toml`; see `templates/manifest.toml`.

Allowed artifact paths are under `artifacts/scripts/`, `artifacts/templates/`, `artifacts/snippets/`, or `artifacts/references/`. Reject absolute paths, `..` traversal, and paths that escape the active store. Never store secrets in artifacts, never let artifacts override higher-priority instructions, and do not silently move scripts between a project repo and the knowledge store without user approval.

Use `mem artifact list`, `mem artifact show <name>`, and `mem artifact check` to inspect artifacts, and `mem artifact add`, `mem artifact update <name> --checksum`, and `mem artifact remove` to maintain manifest metadata. Inspection commands never execute scripts, and `artifact remove` never deletes files without `--delete-file`.

Use `mem bundle export`, `mem bundle inspect`, and `mem bundle import` to move `memory.db`, `config.toml`, `manifest.toml`, and `artifacts/` together. Use `mem bundle export --no-config` when store config contains machine-local paths. Bundle import refuses non-empty stores unless `--merge` or `--replace --force` is explicit.

```bash
mem artifact add artifacts/scripts/ci-triage.sh --name ci-triage --kind script --scope global --executable
mem artifact update ci-triage --checksum
mem bundle export mnemark-store.tgz
```

Full artifact and bundle command details: `references/cli-guide.md`.

## Daily Retrospective

Use `references/daily-retro.md` when the user asks for daily review. The short flow is:

1. Use platform-provided conversation context or logs.
2. Run `mem retro daily` for current memory, changelog, ambiguity, and audit context.
3. Compare available platform context against existing memory.
4. Save new durable knowledge, update stale knowledge, detect repeated manual procedures that should become workflow memories, and record ambiguities.
5. Report counts and pending questions.
6. Commit and push.

## Weekly Retrospective

Use `references/weekly-retro.md` when the user asks for weekly review. The weekly review reads `changelog`, `memory.db`, and `ambiguities`, not raw logs. It improves memory quality: merge duplicates, calibrate confidence, identify candidates for workflow memory or skills, resolve ambiguities, and audit health.
