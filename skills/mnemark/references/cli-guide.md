# mem CLI Guide

## Setup

```bash
mem init
mem migrate --dry-run
mem migrate
mem context --detect
mem config show
mem setup pi
mem doctor
```

`init` explicitly creates a new SQLite store and Tantivy index. It never migrates an older store. Use `migrate --dry-run`, then `migrate`, for an explicit backup-first schema upgrade or same-version compatibility repair. Read commands reject missing or outdated stores instead of initializing or migrating them. Runtime stores do not need `schema/memory-schema.sql`; the schema is embedded in the `mem` binary. `context --detect` returns the auto-detected scope (`global` or `project:<owner/repo>`) based on the current git remote; `--detect` is required. `config show` prints the active root, db/index paths, config paths, environment overrides, and effective command defaults.

CLI configuration uses TOML. User config lives at `~/.config/mnemark/config.toml` and can set `knowledge_home` plus command defaults. Store config lives at `<active-store-root>/config.toml` and can set store-local defaults. Active store discovery order is: explicit `--home`, `MNEMARK_HOME`, user-configured `knowledge_home`, then `~/.mnemark`. Source checkouts are never selected implicitly. Command default priority is: CLI flags, user config, store config, then built-in defaults.

Before write commands, verify the target with `mem config show` or a dry-run. From a mnemark source checkout, use `--home <runtime-store>` unless the user explicitly intends that checkout to be the active store.

```toml
knowledge_home = "~/.mnemark"
default_scope = "auto"
default_limit = 20

[query]
default_scope = "auto"
default_limit = 20
candidate_limit = 10000

[workflow]
default_scope = "auto"
default_limit = 20

[budget]
per_scope_max = 30
```

`query.candidate_limit` bounds lexical candidates loaded for deterministic
reranking (default 10,000; valid range 200-100,000). It must be at least as
large as `query --limit`. `budget.per_scope_max` is a soft cap on active
memories per scope (default 30; 0 disables). Exceeding it never blocks saves;
`mem audit` and retro bundles flag the scope for curation instead.

## Setup helpers

```bash
mem setup list
mem setup claude-code
mem setup claude-code --dry-run
mem setup codex
mem setup pi
mem setup gemini-cli
mem setup opencode
mem setup claude-code --base-dir /tmp/sandbox --no-hook
```

`setup <platform>` wires mnemark into one coding agent for the whole user account. It installs three layers, each idempotent:

1. Policy: prepends the v5 block to `~/.claude/CLAUDE.md`, `~/.codex/AGENTS.md`, `~/.pi/agent/AGENTS.md`, `~/.gemini/GEMINI.md`, or `~/.config/opencode/AGENTS.md`. An unedited v1/v2/v3/v4 policy is upgraded in place; a changed managed block reports `drifted` instead of being overwritten.
2. Skill: writes the bundled, version-matched skill once to `~/.agents/skills/mnemark`. Pi reads it directly; Claude Code and Codex receive per-skill symlinks. A legacy platform copy is migrated only when it contains managed mnemark files; unmanaged files report `conflict` and are never deleted.
3. Session start: Claude Code receives a `SessionStart` hook in `~/.claude/settings.json`. Other platforms rely on the policy's visible `mem prime` instruction.

Setup has no project mode and never selects the current repository implicitly. Project-specific recall remains a logical memory scope selected by commands such as `mem prime --scope auto` inside the shared active store. Explicit path overrides may still target a sandbox or custom user installation root.

Use `--base-dir <DIR>` to target a different user home, `--instructions` to override the policy path, `--skills-dir` to override the platform link parent, or `--shared-skills-dir` to override the shared Agent Skills parent. `--no-skill`/`--no-hook` skip layers, and `--dry-run` previews changes. `setup list` prints the shared source, platform link mode, and resolved paths.

## Doctor

```bash
mem doctor
mem doctor --platform claude-code
mem doctor --base-dir /tmp/sandbox
```

`doctor` verifies the whole installation: binary/schema compatibility, SQLite `quick_check`, durable store identity, Unix store permissions, graph health, index freshness, store git versioning, exact managed policy content, the shared skill files, platform skill links, and session hooks. Output is JSON with per-check `status` (`ok`, `warn`, `missing`, `error`) and a `fix` hint for anything not ok. `doctor` always resolves the runtime store (`--home`, `MNEMARK_HOME`, user config, `~/.mnemark`) and never treats a source checkout as the store.

## Session priming

```bash
mem prime
mem prime --scope auto --budget 4000
mem prime --focus "release safety"
mem prime --per-section 5 --format json
```

`prime` emits one compact context block for agent session starts: user, feedback,
preference, and project memories plus workflow names for the detected scope,
followed by the save-before-finishing protocol. It is read-only unless `--focus`
is supplied; focused priming rebuilds the local graph index, then adds
budget-capped graph neighborhood context for the task focus. Text and JSON
output, including focused graph context, never exceed `--budget`; entries are
truncated and then dropped by priority. A budget too small for the fixed
protocol/envelope fails with the required minimum instead of silently
exceeding the requested limit. Normal section content never touches access
counters, and workflows prime with
their `goal` line only — load full runbooks with `mem workflow show`. Like
`doctor`, it always targets the runtime store, so it is safe to run from any
directory including a mnemark source checkout.

## Write

```bash
mem save --type feedback --name pr_small --scope "project:example/ot-product" --source agent --tags '["style:review","decision:pr-size"]' --content "PR 拆小逐個 review"
mem workflow new release_runbook
mem save --type workflow --name release_runbook --scope global --source manual --user-confirmed --tags '["workflow:release","intent:release","risk:high"]' --content-file release_runbook.yaml
mem save --type project --name q4_freeze --description "Mobile release cut" --tags '["project:example/ot-mobile"]' --content "..." --expires-at 2026-03-05T00:00:00Z --why "freeze for release branch"
```

Supported `--type` values: `user`, `feedback`, `project`, `reference` (default), `preference`, `workflow`. Defaults when omitted: `--scope global`, `--source agent`, `--tags '[]'`. Optional metadata fields are `--description`, `--why`, and `--expires-at` (strict RFC3339). Resource gates cap names at 256 bytes, descriptions/why/tags at 64 KiB, content at 1 MiB, scope at 256 bytes, and tags at 100 entries.

Confidence is inferred from source unless `--confidence` is provided:

- `manual`: high, protected
- `agent`: medium
- `daily_retro`: low
- `weekly_retro`: low

`save` accepts inline `--content` or `--content-file`. Secret-like values in names, descriptions, content, tags, scopes, provenance, and other durable fields reject the write by default. Use `--redact-secrets` only when replacing the detected value with `[REDACTED]` is explicitly intended. `--source manual` requires `--user-confirmed`; imported or merged manual claims without durable attestation are downgraded to agent trust.

Without `--force`, `save` returns `duplicate_found` for an exact name match and `similar_found` for high-overlap content. The caller should decide whether to skip, update, or supersede. With `--force`, an exact-name save updates the existing memory only if the incoming source is at least as trusted as the existing source.

Workflow memories are validated on save/import unless `--no-validate-workflow` is passed. Merge validates workflow records too; invalid incoming workflows are not imported automatically and are recorded as pending ambiguity records for human review. Required fields are `schema_version`, `goal`, `triggers`, `steps`, and `stop_conditions`; each step needs `id` plus one of `run`, `check`, `manual`, or `ask`. Scaffold new runbooks with `mem workflow new <name>` — the baseline template is embedded in the binary, so it works everywhere `mem` does.

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
mem query "security review" --touch
mem query "security review" --explain-score
mem query --type feedback --format compact
mem query --type project --format table
```

By default, query treats punctuation as literal text so searches like `project:owner/repo` do not require Tantivy syntax escaping. Use `--raw-query` when you intentionally want Tantivy query syntax such as `name:pr_small`.

Query is read-only and no-touch by default. Use `--touch` only when access telemetry is intentionally desired; it acquires the write lock. A stale Tantivy index causes an actionable error instead of hidden repair—run `mem reindex` or opt in with `--repair-index`. Relevance sorting deterministically combines normalized lexical score, source trust, confidence, scope specificity, and recency; `--explain-score` exposes every component in JSON. Superseded and inactive memories are hidden unless explicitly requested. `--format json|table|compact` controls output; prefer `compact` when loading results into an agent context window.

`--tags` matches exact JSON-array membership, not substrings. `--fuzzy` searches across `name`, `description`, `content`, and `tags`. `--semantic` is hidden and not implemented until an embedding backend is planned; manual use fails with an explicit error.

## Update and Lifecycle

```bash
mem update no_emoji --content "不要在回覆中使用 emoji"
mem update release_runbook --content-file release_runbook.yaml
mem update no_emoji --expected-version 2 --content "不要在回覆中使用 emoji"
mem update no_emoji --add-tags '["style:output"]'
mem update no_emoji --description "preferred output style"
mem supersede old_policy new_policy --expected-version 3 --content "新的政策內容"
mem delete old_policy --expected-version 4
mem delete old_policy --hard --force
```

`update`, `supersede`, and `delete` accept `--source`; manual source requires `--user-confirmed`. `update` can change type, destination scope, confidence, description, content, expiry, source, and tags (`--set-tags`, `--add-tags`, `--remove-tags`), with explicit clear flags for nullable fields. Bare names resolve within `--scope`; use `id:<memory-id>` to force an id when a scoped name collides with another memory's id.
Soft delete sets `valid_until`. Expired, deleted, and superseded memories are excluded from ordinary query/prime/workflow/graph recall. Hard delete removes the row; protected memories require `--force`.
Every lifecycle or semantic mutation of a retained memory row increments
`version`, including soft delete, superseding the old row, ambiguity soft-delete
resolution, and `audit --fix` lifecycle/link repair. Access telemetry does not
change the semantic version. `--expected-version` therefore returns
`version_conflict` whenever any semantic change happened after the caller read
the memory.

## Workflow

```bash
mem workflow list --scope auto
mem workflow list --scope auto --limit 50 --include-superseded
mem workflow find release --scope auto
mem workflow find release --scope auto --limit 5
mem workflow show release_runbook
mem workflow show release_runbook --checklist
mem workflow show release_runbook --with-graph-context
mem workflow validate release_runbook
mem workflow validate release_runbook --check-artifacts
mem workflow new triage_ci
mem workflow record release_runbook --result success --note "clean run"
mem workflow record release_runbook --result failure --note "push rejected at step publish"
```

`workflow find <intent>` searches by intent string (positional). Workflow run notes reject secret-like values unless `--redact-secrets` is explicit; `workflow record --source manual` requires `--user-confirmed`. `workflow show <reference>` and `workflow validate <reference>` accept either the workflow name or its memory id. `workflow show --checklist` renders the runbook as an ordered execution checklist (checkbox per step, `confirm: true` steps flagged, run-record step appended) — prefer it over raw JSON when you are about to execute. `workflow show --with-graph-context` rebuilds the graph and includes related workflow/artifact/policy/run edges next to the workflow. `workflow new <name>` scaffolds a YAML file from the embedded baseline template (default `<name>.yaml`, `--output`/`--force` available) so new runbooks start structurally valid. `workflow record <reference> --result success|failure --note "<one line>"` logs one execution into the store's `workflow_runs` telemetry; record every run — `mem retro daily|weekly` bundles per-workflow run stats so retros can spot repeatedly failing or stale runbooks with data instead of impressions. The record response echoes the runbook's `post_run_memory` items (or `post_run_memory_missing` when the section is absent): treat that list as the closing checklist and process it before ending the work unit, so learning is part of the pipeline rather than a habit. `workflow validate` warns (`no_post_run_memory`, non-blocking) when a runbook lacks the section. Workflow helpers discover, display, and validate workflow memories; they never execute workflow commands. Agents execute runbook steps themselves, verify checkpoints, and ask before risky side effects. Reference reusable scripts or artifacts by path in workflow content; keep executable script bodies in version-controlled repository files for project-specific logic or in `artifacts/` under the active knowledge store for cross-project helpers. `workflow validate --check-artifacts` additionally checks `owner: knowledge_store` entries against `manifest.toml`, path containment, required file presence, checksums, and executable bits. Use it after placeholder artifact references have been replaced with real files and manifest entries. Artifact paths used in `steps.run` must be declared in `reusable_scripts`.

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

Artifact commands inspect `manifest.toml` and files under `artifacts/` in the active knowledge store root. Select that root with `--home`, `MNEMARK_HOME`, user config, or the default `~/.mnemark`; source repositories are never discovered as stores. `artifact list` prints manifest entries as JSON. `artifact show <name>` accepts a short name when it is unique or a qualified name such as `scripts.ci-triage`. `artifact check` verifies manifest parsing, path containment, file presence, SHA-256 checksums, and executable bits for records marked `executable = true`; it reports missing files, checksum mismatches, unsafe paths, invalid checksums, invalid scopes, and non-executable scripts as JSON. Artifact commands never execute scripts.

`artifact add` derives the manifest name from the file stem unless `--name` is
provided, requires a normalized path to an existing regular file under the
active store, and rejects symlinks in the file or any intermediate path before
secret scanning, redaction, or hashing. It rejects secret-like content/metadata
unless `--redact-secrets` is explicit, computes `sha256:<hex>`, and writes
deterministic TOML. Existing name or path conflicts require `--force`.
`artifact update <name> --checksum` applies the same path and secret gates before
refreshing the checksum after manual file edits. `artifact remove <name>`
removes the manifest entry only; deleting the file requires `--delete-file`.

## Bundles

```bash
mem bundle export mnemark-store.tgz
mem bundle export mnemark-store.tgz --no-config
mem bundle export redacted-store.tgz --redact-secrets
mem bundle inspect mnemark-store.tgz
mem bundle import mnemark-store.tgz          # clean store only
mem bundle import mnemark-store.tgz --merge
mem bundle import mnemark-store.tgz --replace --force
mem bundle import legacy-with-secrets.tgz --redact-secrets
mem bundle import trusted-v1.tgz --allow-unverified
```

Bundles include durable portable store files: `memory.db`, optional `config.toml`, optional `manifest.toml`, `artifacts/`, and `bundle.json`. Export uses SQLite's online backup API, so it does not checkpoint or mutate the live store. Bundle v2 records a SHA-256 for every imported file; inspect/import reject missing, extra, or mismatched files and unexpected SQLite tables, views, or trigger definitions before destination mutation. Legacy bundles without a complete hash manifest require explicit `--allow-unverified` for import. Secret-like durable values reject export/import by default; `--redact-secrets` modifies only the staged/imported copy. Bundles exclude rebuildable or transient files such as `index/`, `.mem.lock`, `memory.db-wal`, and `memory.db-shm`. Use `bundle export --no-config` when store config contains machine-local paths. `bundle inspect` lists archive entries and `bundle.json` metadata without importing. `bundle import` initializes a clean store by default and runs `mem reindex` behavior after import. Import into a non-empty store is refused unless `--merge` or `--replace --force` is explicit. `--merge` imports memories through existing merge logic and copies non-conflicting regular artifact files; symlinks and special files are rejected. `--replace --force` takes an online rollback snapshot before clearing durable files and restores the previous store if replacement fails.

## Ambiguity

```bash
mem ambiguity add --query "PR 策略" --memory-ids '["pr_small","pr_bundled"]' --context "scope unclear"
mem ambiguity list --pending
mem ambiguity resolve 1 --note "project-specific preference"
mem ambiguity resolve 1 --keep pr_small --soft-delete-others
```

Ambiguity queries, context, and resolution notes reject secret-like values unless `--redact-secrets` is explicit. `resolve --keep ... --soft-delete-others` soft-deletes non-protected alternatives referenced by the ambiguity and records skipped protected memories.
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
`audit` reports broken supersede links, expired-but-active memories, stale and cleanup candidates, and `over_budget_scopes`: scopes holding more active memories than `budget.per_scope_max`, each with its lowest-access curation candidates (protected manual memories excluded). Curate over-budget scopes down to the cap — merge, supersede, or delete — rather than raising the cap by default.
`retro` emits an orchestration bundle for the LLM. It does not read platform logs itself; the active platform or harness should provide conversation history. `retro daily|weekly --limit` controls the bundle size (default 50).

## Reconcile

```bash
mem reconcile
mem reconcile --scope project:example/app --repo ~/code/app
mem reconcile --type project
```

`reconcile` treats memories as a cache of external reality and checks that cache mechanically: it extracts path and command claims from memory content (backtick code spans first, bare path-like tokens as fallback) and verifies each against the filesystem — paths via existence checks relative to `--repo` (default: current directory; `<placeholder>` segments match any entry), commands via `PATH` lookup. It is read-only: it never executes commands, edits memories, or updates access counters. `--scope auto` (default) checks global plus the detected project scope; an explicit `--scope` checks exactly that scope. Workflow memories are skipped unless `--type workflow` is passed because `mem workflow validate` owns runbook checking. The JSON report marks each claim `ok` or `missing`, lists `unverifiable` spans, and flags memories with missing claims; deciding whether a flagged memory needs `mem update`, `mem supersede`, or `mem delete` stays with the agent.

## Graph

```bash
mem graph rebuild
mem graph stats
mem graph explain release_runbook --scope auto
mem graph path release_runbook artifact:artifacts/scripts/build-release.sh --direction any
mem graph path release_policy release_runbook --max-depth 4 --confidence inferred --direction outgoing --format compact
mem graph query "release safety" --scope auto --depth 2 --direction any
mem graph export --format json
mem graph candidates --scope auto --limit 50
mem graph candidates --scope auto --changed-since 2026-07-01T00:00:00Z
mem graph candidates --scope auto --unlinked
mem graph ingest semantic_edges.json
mem graph review --pending
mem graph accept sem_abc123
mem graph reject sem_abc123 --note "not useful"
```

`graph` materializes an explainable local relationship index in SQLite tables (`graph_nodes`, `graph_edges`, durable `graph_semantic_edges`, and append-only `graph_semantic_edge_revisions`). It is graph-first context retrieval, not embedding RAG: no vector database, provider, or hidden LLM call is required. Graph-dependent commands rebuild only when graph state is dirty or schema-stale; `graph stats` never rebuilds implicitly, so `dirty` is observable.

Deterministic rebuild creates memory, tag, scope, type, source, artifact, claim, workflow step, workflow run, and deterministic concept nodes. Edges include `has_tag`, `in_scope`, `has_type`, `from_source`, `superseded_by`, `mentions_path`, `mentions_command`, `references_artifact`, `has_workflow_step`, `step_uses_artifact`, `requires_confirmation`, `recorded_run`, and `has_result`. Active, unexpired memories are normal graph inputs; superseded rows remain only for lineage, while deleted/expired memories and expired semantic edges are excluded. Administrative `has_type`, `in_scope`, and `from_source` edges remain visible but are not path/query bridges unless `--include-metadata` is explicit.

`graph explain <reference>` accepts a graph node id (`memory:<id>`, `tag:<tag>`, `artifact:<path>`), `id:<memory-id>`, or a scoped memory name, then returns active direct neighbors with relation, confidence, status, evidence, and direction; `--depth` accepts 0 or 1. Explain and path default to `--scope auto` (global plus detected project); use `--scope all` only for intentional cross-project inspection. `graph path <from> <to>` finds a minimum-hop relationship path over active edges, then globally breaks equal-hop ties by cumulative relation weight, confidence, and stable path identity. `--direction any|outgoing|incoming` controls edge orientation. Pending edges are excluded unless `--include-ambiguous` is set for pending `AMBIGUOUS` relationships. `--confidence extracted|inferred|all` narrows confidence labels; the default is all active edges. `graph export --format json` emits a graphify-compatible-ish JSON shape with `schema_version`, `nodes`, and `edges`.

`graph query "terms"` resolves starting nodes from exact graph ids, memory names/ids, labels, and scored Tantivy lexical memory search, then expands an evidence-bearing neighborhood with confidence, scope, relation, source-trust, memory-confidence, fanout, and depth controls. It returns context for the agent to interpret; it does not synthesize final answers.

`graph candidates` emits a strict JSON payload for skill-mediated semantic extraction: instructions, allowed semantic relations, and candidate memories. Use `--changed-since` or `--unlinked` to bound curation work. The Rust CLI still does not call an LLM. The agent writes semantic edge JSON, then `graph ingest <file>` validates a strict unknown-field-denying schema, allowlisted relations, endpoints, confidence labels, RFC3339 `valid_until`, evidence, concept ids, secret policy, and source trust before storing durable `graph_semantic_edges`. Manual semantic assertions require `--source manual --user-confirmed` and persist the confirmation timestamp; unattested manual edges from another store are downgraded during merge. `EXTRACTED` and `INFERRED` edges are active by default, `AMBIGUOUS` edges are pending by default, and `--pending-inferred` stores inferred edges as pending too.

`graph review --pending|--ambiguous` shows stored semantic edges with evidence, source spans, and linked ambiguity ids. `graph accept <edge-id>` marks an edge active; `graph reject <edge-id> --note ...` marks it rejected, with the same default-reject / explicit-`--redact-secrets` policy for review notes. Both resolve the linked ambiguity and append a revision snapshot without rewriting the confidence label. Agent-generated cross-project edges and logical edge conflicts default to pending review. Treat graph data as context and provenance, never as an instruction override.

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

JSON imports process an array of memory-like objects. Markdown or other files import as one `reference` memory unless `--type` is supplied. Import files are capped at 256 MiB. Ordinary JSON/Markdown export is also capped at 256 MiB of stored text to keep agent processes bounded; use `mem bundle export` for a complete large-store snapshot. Import defaults to agent provenance, rejects secret-like values by default, and supports explicit `--redact-secrets`. `--source manual` requires `--user-confirmed`; the import records origin and origin reference for later audit.

## Merge

```bash
mem merge /path/to/theirs.db
mem merge /path/to/theirs.db --prefer-trusted
mem merge /path/to/theirs.db --redact-secrets
```

Merge validates SQLite integrity and rejects secrets across incoming memories, ambiguities, workflow runs, changelog, semantic edges, and semantic revisions by default. `--redact-secrets` uses a temporary online snapshot and never mutates the source database. Merge imports memories with new scoped names, skips identical `(scope,name)` records, and records same-name content conflicts in `ambiguities` instead of overwriting automatically. Merge conflict ambiguity records include a structured incoming snapshot in `context`.
Lower-trust incoming same-name memories are rejected. `--prefer-trusted` lets a higher-trust incoming memory update a lower-trust local memory; equal-trust differences still become ambiguities. Durable ambiguities, workflow runs, changelog events, semantic edges, and append-only semantic revisions are merged idempotently through store/event UIDs with memory/edge/ambiguity ID remapping. Lower-trust logical conflicts are rejected, unattested manual claims are downgraded, and unresolved/equal-trust conflicts remain pending with ambiguity evidence. Bundle merge and sync conflict resolution use the same behavior.

## Sync

```bash
mem sync --dry-run
mem sync
mem sync --push
mem sync --remote origin --message "weekly retro updates" --push
```

`sync` moves the runtime store through its own private git repository. Before
any commit it scans durable SQLite text and every regular worktree file outside
rebuildable/SQLite paths, rejecting secrets, non-UTF-8 files that cannot be
scanned safely, symlinks, and special files. A residual
`.bundle-replace-backup-*` directory stops sync with a recovery hint and is
always ignored by Git, preventing rollback data from bypassing validation. It
then checkpoints the SQLite WAL for a real checkpoint, maintains `.gitignore`
for rebuildable files, commits local changes with repository
hooks/signing/fsmonitor disabled, and fetches/merges a configured remote. It does
not push by default; `--push` is an explicit network side effect requiring user
approval. `mem sync --dry-run` is lock-free and does not checkpoint, commit,
fetch, merge, or push. Git moves bytes; `mem` resolves meaning — when both
machines changed `memory.db`, the binary conflict is resolved by keeping the
local database and merging the remote copy with `mem merge`, so same-name
conflicts become pending ambiguity records instead of lost
rows. Pulled state is integrity/secret checked before use, unsafe pulls roll
back to the pre-pull checkout, merged WAL is checkpointed before commit, and
the local search index is rebuilt after a successful pull. `sync` requires the
store root to be its own git repository (it refuses to commit into an enclosing
repo), stops on conflicts outside `memory.db`, and reports every action as JSON.
Without a configured remote it commits locally and reports `local_only`.

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
