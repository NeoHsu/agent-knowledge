# mem CLI Guide

## Setup

```bash
mem init
mem context --detect
mem config show
```

`init` creates the SQLite store and the Tantivy index inside the active knowledge home, idempotent on re-run. Runtime stores do not need `schema/memory-schema.sql`; the schema is embedded in the `mem` binary. `context --detect` returns the auto-detected scope (`global` or `project:<owner/repo>`) based on the current git remote; `--detect` is required. `config show` prints the active root, db/index paths, config paths, environment overrides, and effective command defaults.

CLI configuration uses TOML. User config lives at `~/.config/agent-knowledge/config.toml` and can set `knowledge_home` plus command defaults. Store config lives at `$AGENT_KNOWLEDGE_HOME/config.toml` and can set store-local defaults. Active store discovery order is: current repo root, executable-near repo root, `AGENT_KNOWLEDGE_HOME`, user-configured `knowledge_home`, then `~/.agent-knowledge`. Command default priority is: CLI flags, user config, store config, then built-in defaults.

```toml
knowledge_home = "~/.agent-knowledge"
default_scope = "auto"
default_limit = 20

[query]
default_scope = "auto"
default_limit = 20

[workflow]
default_scope = "auto"
default_limit = 20
```

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
mem query "deploy" --semantic
```

By default, query treats punctuation as literal text so searches like `project:owner/repo` do not require Tantivy syntax escaping. Use `--raw-query` when you intentionally want Tantivy query syntax such as `name:pr_small`.

Query updates `access_count` and `last_accessed_at`; use `--no-touch` for read-only context loading. Superseded memories are hidden unless `--include-superseded` is used.

`--tags` matches exact JSON-array membership, not substrings. `--fuzzy` searches across `name`, `description`, `content`, and `tags`. `--semantic` is a reserved interface and returns `unsupported` until an embedding backend is configured.

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
mem workflow validate release_runbook
```

`workflow find <intent>` searches by intent string (positional). `workflow show <reference>` and `workflow validate <reference>` accept either the workflow name or its memory id. Workflow helpers discover, display, and validate workflow memories; they never execute workflow commands. Agents execute runbook steps themselves, verify checkpoints, and ask before risky side effects. Reference reusable scripts by path in workflow content; keep executable script bodies in version-controlled repository files to avoid drift.

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
  "results": []
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

## Release Build

```bash
scripts/build-release.sh
scripts/smoke-release.sh
```

After building, `scripts/smoke-release.sh` copies the release binary into an isolated install directory and verifies that `mem init`, `config show`, save/query/reindex/export all work against a runtime-only store with no schema file. Install or expose `target/release/mem` on `PATH` (for example via `cargo install --path crates/mem-cli`, or by adding `target/release` to `PATH`). All examples in this guide assume `mem` is on `PATH`.
