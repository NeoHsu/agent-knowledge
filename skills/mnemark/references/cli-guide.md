# mem CLI Guide

## Setup

```bash
mem init
mem context --detect
mem config show
mem setup agent-policy
mem doctor
```

`init` creates the SQLite store and the Tantivy index inside the active knowledge home, idempotent on re-run. Runtime stores do not need `schema/memory-schema.sql`; the schema is embedded in the `mem` binary. `context --detect` returns the auto-detected scope (`global` or `project:<owner/repo>`) based on the current git remote; `--detect` is required. `config show` prints the active root, db/index paths, config paths, environment overrides, and effective command defaults.

CLI configuration uses TOML. User config lives at `~/.config/mnemark/config.toml` and can set `knowledge_home` plus command defaults. Store config lives at `<active-store-root>/config.toml` and can set store-local defaults; `$MNEMARK_HOME` is one way to choose that active store root. Active store discovery order is: explicit `--home`, current directory when it contains `schema/memory-schema.sql`, an executable-near repo root, `MNEMARK_HOME`, user-configured `knowledge_home`, then `~/.mnemark`. Command default priority is: CLI flags, user config, store config, then built-in defaults.

Before write commands, verify the target with `mem config show` or a dry-run. From a mnemark source checkout, use `--home <runtime-store>` unless the user explicitly intends that checkout to be the active store.

```toml
knowledge_home = "~/.mnemark"
default_scope = "auto"
default_limit = 20

[query]
default_scope = "auto"
default_limit = 20

[workflow]
default_scope = "auto"
default_limit = 20
```

## Setup helpers

```bash
mem setup agent-policy
mem setup agent-policy --target CLAUDE.md
mem setup agent-policy --target AGENTS.md --dry-run
mem setup list
mem setup claude-code
mem setup claude-code --dry-run
mem setup codex
mem setup gemini-cli
mem setup opencode
mem setup claude-code --base-dir /tmp/sandbox --no-hook
```

`setup agent-policy` prepends the mnemark memory policy (v2) to the coding-agent entrypoint in the current project. It prefers an existing `CLAUDE.md`, otherwise it creates or updates `AGENTS.md`. Use `--target <FILE>` to choose a specific file. The command is idempotent, upgrades the legacy v1 block in place, and does not duplicate the v2 block when it already exists.

`setup <platform>` wires mnemark into one coding agent for the whole user account. It installs three layers, each idempotent:

1. Policy: prepends the v2 policy block to the platform's global instructions file (`~/.claude/CLAUDE.md`, `~/.codex/AGENTS.md`, `~/.gemini/GEMINI.md`, or `~/.config/opencode/AGENTS.md`).
2. Skill: writes the bundled mnemark skill files (embedded in the binary, always version-matched) into the platform skill directory when the platform has one (`~/.claude/skills/mnemark`, `~/.codex/skills/mnemark`).
3. Session start: on Claude Code, adds a `SessionStart` hook running `mem prime` to `~/.claude/settings.json` while preserving all existing settings. Platforms without a hook mechanism rely on the policy block's "run `mem prime` at session start" instruction instead.

Use `--base-dir <DIR>` to target a different home directory, `--instructions`/`--skills-dir` to override paths for platform versions with different layouts, `--no-skill`/`--no-hook` to skip layers, and `--dry-run` to preview. `setup list` prints the capability matrix with resolved paths.

## Doctor

```bash
mem doctor
mem doctor --platform claude-code
mem doctor --base-dir /tmp/sandbox
```

`doctor` verifies the whole institution: binary version, runtime store presence, index freshness, store git versioning, and per-platform wiring (policy block version, skill files, session hook). Output is JSON with per-check `status` (`ok`, `warn`, `missing`, `error`) and a `fix` hint for anything not ok. `doctor` always resolves the runtime store (`--home`, `MNEMARK_HOME`, user config, `~/.mnemark`) and never treats a source checkout as the store.

## Session priming

```bash
mem prime
mem prime --scope auto --budget 4000
mem prime --per-section 5 --format json
```

`prime` emits one compact context block for agent session starts: user, feedback, preference, and project memories plus workflow names for the detected scope, followed by the save-before-finishing protocol. It is read-only (never touches access counters), fits output inside `--budget` characters by truncating and then dropping the lowest-priority entries, and primes workflows with their `goal` line only — load full runbooks with `mem workflow show`. Like `doctor`, it always targets the runtime store, so it is safe to run from any directory including a mnemark source checkout.

## Write

```bash
mem save --type feedback --name pr_small --scope "project:example/ot-product" --source agent --tags '["style:review","decision:pr-size"]' --content "PR 拆小逐個 review"
mem save --type workflow --name release_runbook --scope global --source manual --tags '["workflow:release","intent:release","risk:high"]' --content-file templates/workflow.yaml
mem save --type project --name q4_freeze --description "Mobile release cut" --tags '["project:example/ot-mobile"]' --content "..." --expires-at 2026-03-05T00:00:00Z --why "freeze for release branch"
```

Supported `--type` values: `user`, `feedback`, `project`, `reference` (default), `preference`, `workflow`. Defaults when omitted: `--scope global`, `--source agent`, `--tags '[]'`. Optional metadata fields are `--description`, `--why`, and `--expires-at` (RFC3339).

Confidence is inferred from source unless `--confidence` is provided:

- `manual`: high, protected
- `agent`: medium
- `daily_retro`: low
- `weekly_retro`: low

`save` accepts inline `--content` or `--content-file`. It strips common API keys, bearer tokens, and password/secret assignments before persistence.

Without `--force`, `save` returns `duplicate_found` for an exact name match and `similar_found` for high-overlap content. The caller should decide whether to skip, update, or supersede. With `--force`, an exact-name save updates the existing memory only if the incoming source is at least as trusted as the existing source.

Workflow memories are validated on save/import unless `--no-validate-workflow` is passed. Merge validates workflow records too; invalid incoming workflows are not imported automatically and are recorded as pending ambiguity records for human review. Required fields are `schema_version`, `goal`, `triggers`, `steps`, and `stop_conditions`; each step needs `id` plus one of `run`, `check`, `manual`, or `ask`. Use `templates/workflow.yaml` as the baseline template.

## Query

```bash
mem query "security review"
mem query "部署流程" --scope auto
mem query --type feedback
mem query "release" --type workflow --scope auto
mem query --tags "domain:security"
mem query --sort time
mem query --sort access-count
mem query "deplpy" --fuzzy
mem query "name:pr_small" --raw-query
mem query "security review" --no-touch
mem query --type feedback --format compact
mem query --type project --format table
```

By default, query treats punctuation as literal text so searches like `project:owner/repo` do not require Tantivy syntax escaping. Use `--raw-query` when you intentionally want Tantivy query syntax such as `name:pr_small`.

Query updates `access_count` and `last_accessed_at`; use `--no-touch` for read-only context loading. Superseded memories are hidden unless `--include-superseded` is used. `--format json|table|compact` controls output; prefer `compact` when loading results into an agent context window.

`--tags` matches exact JSON-array membership, not substrings. `--fuzzy` searches across `name`, `description`, `content`, and `tags`. `--semantic` is hidden and not implemented until an embedding backend is planned; manual use fails with an explicit error.

## Update and Lifecycle

```bash
mem update no_emoji --content "不要在回覆中使用 emoji"
mem update release_runbook --content-file templates/workflow.yaml
mem update no_emoji --expected-version 2 --content "不要在回覆中使用 emoji"
mem update no_emoji --add-tags '["style:output"]'
mem update no_emoji --description "preferred output style"
mem supersede old_policy new_policy --expected-version 3 --content "新的政策內容"
mem delete old_policy --expected-version 4
mem delete old_policy --hard --force
```

`update`, `supersede`, and `delete` accept `--source` (default `agent`) so the caller is recorded in the changelog. `update` also accepts `--description` and `--add-tags '["..."]'` (additive merge).
Soft delete sets `valid_until`. Hard delete removes the row; protected memories require `--force`.
`--expected-version` returns `version_conflict` if the stored memory changed after the caller read it.

## Workflow

```bash
mem workflow list --scope auto
mem workflow list --scope auto --limit 50 --include-superseded
mem workflow find release --scope auto
mem workflow find release --scope auto --limit 5
mem workflow show release_runbook
mem workflow show release_runbook --checklist
mem workflow validate release_runbook
mem workflow validate release_runbook --check-artifacts
mem workflow new triage_ci
mem workflow record release_runbook --result success --note "clean run"
mem workflow record release_runbook --result failure --note "push rejected at step publish"
```

`workflow find <intent>` searches by intent string (positional). `workflow show <reference>` and `workflow validate <reference>` accept either the workflow name or its memory id. `workflow show --checklist` renders the runbook as an ordered execution checklist (checkbox per step, `confirm: true` steps flagged, run-record step appended) — prefer it over raw JSON when you are about to execute. `workflow new <name>` scaffolds a YAML file from the embedded baseline template (default `<name>.yaml`, `--output`/`--force` available) so new runbooks start structurally valid. `workflow record <reference> --result success|failure --note "<one line>"` logs one execution into the store's `workflow_runs` telemetry; record every run — `mem retro daily|weekly` bundles per-workflow run stats so retros can spot repeatedly failing or stale runbooks with data instead of impressions. The record response echoes the runbook's `post_run_memory` items (or `post_run_memory_missing` when the section is absent): treat that list as the closing checklist and process it before ending the work unit, so learning is part of the pipeline rather than a habit. `workflow validate` warns (`no_post_run_memory`, non-blocking) when a runbook lacks the section. Workflow helpers discover, display, and validate workflow memories; they never execute workflow commands. Agents execute runbook steps themselves, verify checkpoints, and ask before risky side effects. Reference reusable scripts or artifacts by path in workflow content; keep executable script bodies in version-controlled repository files for project-specific logic or in `artifacts/` under the active knowledge store for cross-project helpers. `workflow validate --check-artifacts` additionally checks `owner: knowledge_store` entries against `manifest.toml`, path containment, required file presence, checksums, and executable bits. Use it after placeholder artifact references have been replaced with real files and manifest entries. Artifact paths used in `steps.run` must be declared in `reusable_scripts`.

## Artifacts

```bash
mem artifact list
mem artifact show ci-triage
mem artifact show scripts.ci-triage
mem artifact check
mkdir -p artifacts/scripts
printf '#!/usr/bin/env sh\nprintf "collect ci context\\n"\n' > artifacts/scripts/ci-triage.sh
chmod +x artifacts/scripts/ci-triage.sh
mem artifact add artifacts/scripts/ci-triage.sh --name ci-triage --kind script --scope global --description "Collect CI failure context" --executable
mem artifact update ci-triage --checksum
mem artifact remove ci-triage
mem artifact remove ci-triage --delete-file
```

Artifact commands inspect `manifest.toml` and files under `artifacts/` in the active knowledge store root. `$MNEMARK_HOME` is one way to choose that root, but `--home`, repository discovery, or user config may choose a different active store. `artifact list` prints manifest entries as JSON. `artifact show <name>` accepts a short name when it is unique or a qualified name such as `scripts.ci-triage`. `artifact check` verifies manifest parsing, path containment, file presence, SHA-256 checksums, and executable bits for records marked `executable = true`; it reports missing files, checksum mismatches, unsafe paths, invalid checksums, invalid scopes, and non-executable scripts as JSON. Artifact commands never execute scripts.

`artifact add` derives the manifest name from the file stem unless `--name` is provided, requires the artifact file to already exist under the active store, computes `sha256:<hex>`, and writes deterministic TOML. Existing name or path conflicts require `--force`. `artifact update <name> --checksum` refreshes the checksum after manual file edits. `artifact remove <name>` removes the manifest entry only; deleting the file requires `--delete-file`.

## Bundles

```bash
mem bundle export mnemark-store.tgz
mem bundle export mnemark-store.tgz --no-config
mem bundle inspect mnemark-store.tgz
mem bundle import mnemark-store.tgz          # clean store only
mem bundle import mnemark-store.tgz --merge
mem bundle import mnemark-store.tgz --replace --force
```

Bundles include durable portable store files: `memory.db`, optional `config.toml`, optional `manifest.toml`, `artifacts/`, and `bundle.json`. They exclude rebuildable or transient files such as `index/`, `.mem.lock`, `memory.db-wal`, and `memory.db-shm`. Use `bundle export --no-config` when store config contains machine-local paths. `bundle inspect` lists archive entries and `bundle.json` metadata without importing. `bundle import` initializes a clean store by default and runs `mem reindex` behavior after import. Import into a non-empty store is refused unless `--merge` or `--replace --force` is explicit. `--merge` imports memories through existing merge logic and copies non-conflicting artifacts; `--replace --force` clears durable store files before import.

## Ambiguity

```bash
mem ambiguity add --query "PR 策略" --memory-ids '["pr_small","pr_bundled"]' --context "scope unclear"
mem ambiguity list --pending
mem ambiguity resolve 1 --note "project-specific preference"
mem ambiguity resolve 1 --keep pr_small --soft-delete-others
```

`resolve --keep ... --soft-delete-others` soft-deletes non-protected alternatives referenced by the ambiguity and records skipped protected memories.
`ambiguity list` parses JSON fields such as `memory_ids`, structured merge-conflict `context`, and JSON `resolution` into JSON objects/arrays in the output.

## History and Maintenance

```bash
mem history
mem history no_emoji
mem history --action save --limit 50
mem stats
mem audit
mem audit --fix
mem gc --days 90
mem reindex
mem retro daily
mem retro daily --limit 100
mem retro weekly --limit 200
```

`history` accepts an optional positional `name` plus `--action <save|update|supersede|delete|...>` and `--limit` (default 20). `gc --days` purges soft-deleted memories older than the given window.
`retro` emits an orchestration bundle for the LLM. It does not read platform logs itself; the active platform or harness should provide conversation history. `retro daily|weekly --limit` controls the bundle size (default 50).

## Reconcile

```bash
mem reconcile
mem reconcile --scope project:example/app --repo ~/code/app
mem reconcile --type project
```

`reconcile` treats memories as a cache of external reality and checks that cache mechanically: it extracts path and command claims from memory content (backtick code spans first, bare path-like tokens as fallback) and verifies each against the filesystem — paths via existence checks relative to `--repo` (default: current directory; `<placeholder>` segments match any entry), commands via `PATH` lookup. It is read-only: it never executes commands, edits memories, or updates access counters. `--scope auto` (default) checks global plus the detected project scope; an explicit `--scope` checks exactly that scope. Workflow memories are skipped unless `--type workflow` is passed because `mem workflow validate` owns runbook checking. The JSON report marks each claim `ok` or `missing`, lists `unverifiable` spans, and flags memories with missing claims; deciding whether a flagged memory needs `mem update`, `mem supersede`, or `mem delete` stays with the agent.

## Import and Export

```bash
mem export --format json
mem export --format markdown
mem import memories.json
mem import note.md --type reference
mem import workflows.json --no-validate-workflow
```

`import` emits one summary JSON object:

```json
{
  "status": "import_complete",
  "total": 3,
  "counts": {
    "saved": 1,
    "duplicate_found": 1,
    "failed": 1
  },
  "results": [
    {
      "index": 0,
      "status": "saved",
      "result": {"status": "saved", "id": "example", "version": 1}
    }
  ]
}
```

JSON imports process an array of memory-like objects. Markdown or other files import as one `reference` memory unless `--type` is supplied.

## Merge

```bash
mem merge /path/to/theirs.db
mem merge /path/to/theirs.db --prefer-trusted
```

Merge strips common secrets from incoming content, imports memories with new names, skips identical same-name memories, and records same-name content conflicts in `ambiguities` instead of overwriting automatically. Merge conflict ambiguity records include a structured incoming snapshot in `context`.
Lower-trust incoming same-name memories are rejected. `--prefer-trusted` lets a higher-trust incoming memory update a lower-trust local memory; equal-trust differences still become ambiguities.

## Sync

```bash
mem sync --dry-run
mem sync --no-push
mem sync
mem sync --remote origin --message "weekly retro updates"
```

`sync` moves the runtime store through its own git repository: it checkpoints the SQLite WAL, maintains `.gitignore` for rebuildable files, commits local changes, fetches and merges the remote, and pushes. Agents should start with `mem sync --dry-run` and should not push unless the user explicitly asked to sync/push or approved the dry-run result; use `mem sync --no-push` for approved local-only checkpoints. Git moves bytes; `mem` resolves meaning — when both machines changed `memory.db`, the binary conflict is resolved by keeping the local database and merging the remote copy through the same logic as `mem merge`, so same-name conflicts become pending ambiguity records instead of lost rows. `sync` requires the store root to be its own git repository (it refuses to commit into an enclosing repo), stops on conflicts outside `memory.db`, and reports every action as JSON. Without a configured remote it commits locally and reports `local_only`.

## Install or Build

Install release binaries instead of building from Rust source:

```bash
# macOS / Linux
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/NeoHsu/mnemark/releases/latest/download/mnemark-installer.sh | sh

# Windows PowerShell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/NeoHsu/mnemark/releases/latest/download/mnemark-installer.ps1 | iex"
```

Manual archives are available on the [latest release page](https://github.com/NeoHsu/mnemark/releases/latest).

For repository development or release verification:

```bash
scripts/build-release.sh
scripts/smoke-release.sh
```

After building, `scripts/smoke-release.sh` copies the release binary into an isolated install directory and verifies that `mem init`, `config show`, save/query/reindex/export all work against a runtime-only store with no schema file. For source-only development, expose `target/release/mem` on `PATH` or run via Cargo as documented in `docs/development.md`. All examples in this guide assume `mem` is on `PATH`.
