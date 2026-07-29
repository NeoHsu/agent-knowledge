# mem CLI Guide

This guide documents source version `0.9.0`. `mem setup <platform>` installs the
skill embedded in that binary, so the installed skill and CLI stay matched.
The `latest` release installer can lag behind source `main`; run `mem --version`
and use documentation from the matching Git tag when exact behavior matters.

## Contents

- Store setup: [Setup](#setup), [Machine-readable contract](#machine-readable-contract),
  [Setup helpers](#setup-helpers), [Doctor](#doctor),
  [Session priming](#session-priming)
- Memory: [Write](#write), [Query](#query),
  [Update and Lifecycle](#update-and-lifecycle), [Ambiguity](#ambiguity)
- Runbooks and portability: [Workflow](#workflow), [Artifacts](#artifacts),
  [Bundles](#bundles), [Reconcile](#reconcile)
- Relationships and transfer: [Graph](#graph),
  [Import and Export](#import-and-export), [Merge](#merge), [Sync](#sync)
- Operations: [History and Maintenance](#history-and-maintenance),
  [Install or Build](#install-or-build)

## Setup

```bash
mem config show
mem init
mem migrate --dry-run
mem migrate
mem context --detect
mem setup pi --dry-run
mem setup pi
mem doctor
```

`init` explicitly creates a new SQLite store and Tantivy index. It never
migrates an older store. Use `migrate --dry-run`, then `migrate`, for an
explicit backup-first schema upgrade or same-version compatibility repair.
Read commands reject missing or outdated stores instead of initializing or
migrating them. Runtime stores do not need `schema/memory-schema.sql`; the
schema is embedded in the `mem` binary. `context --detect` returns the
auto-detected scope (`global` or `project:<owner/repo>`) based on the current
Git remote; `--detect` is required. `config show` prints the active root,
database/index paths, config paths, environment overrides, and effective
command defaults.

CLI configuration uses TOML. User config lives at
`~/.config/mnemark/config.toml` and can set `knowledge_home` plus command
defaults. Store config lives at `<active-store-root>/config.toml` and can set
store-local defaults. Active store discovery order is explicit `--home`,
`MNEMARK_HOME`, user-configured `knowledge_home`, then `~/.mnemark`. Source
checkouts are never selected implicitly. Command default priority is CLI flags,
user config, store config, then built-in defaults.

Before store-changing commands, verify the target with `mem config show`. Before
agent wiring, append `--dry-run` to the exact planned setup command;
`config show` does not verify policy, skill, or hook paths. From a mnemark
source checkout, use `--home <runtime-store>` unless the user explicitly
intends that checkout to be the active store.

All commands accept the global `--json-errors` flag. Successful output is
unchanged; Clap parse failures and runtime failures become one JSON object on
stderr with stable `status`, `contract_version`, `code`, `message`, and
`exit_code` fields. Without the flag, errors remain human-readable. A mutating
command whose SQLite transaction committed before its Tantivy update failed
uses `code: "index_stale_after_write"` and adds a `details` object with
`durable_write_committed: true`, the affected operation, and `mem reindex`
recovery guidance. Do not blindly retry that write; inspect the returned ids or
query SQLite-backed state, repair the index, then decide whether another
mutation is needed. `mem contract` lists known error codes and optional fields.

```bash
mem --json-errors query "release safety" --scope auto
```

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

## Machine-readable contract

```bash
mem contract
```

`contract` is store-independent and remains available even when user/store
configuration is missing or malformed. It reports the CLI output contract
version, required JSON-error fields, and current store, bundle, workflow, graph,
and benchmark-report schema versions. Automation should pin a compatible
`mem --version`, inspect this response, and tolerate additive object fields.
Required fields remain compatible within a minor release; before 1.0, a
breaking machine-interface change requires a documented minor release.

## Setup helpers

Run the exact dry-run before configuring each selected platform. Supported
platform names are `claude-code`, `codex`, `pi`, `gemini-cli`, and `opencode`.
Repeat the preview/apply pair with the selected name.

```bash
mem setup list
mem setup claude-code --dry-run
mem setup claude-code
mem setup codex --dry-run
mem setup pi --dry-run
mem setup gemini-cli --dry-run
mem setup opencode --dry-run
mem setup claude-code \
  --base-dir /tmp/sandbox \
  --no-hook \
  --dry-run
```

`setup <platform>` manages up to three idempotent user-level layers:

1. Policy: prepends the v5 block to `~/.claude/CLAUDE.md`,
   `~/.codex/AGENTS.md`, `~/.pi/agent/AGENTS.md`, `~/.gemini/GEMINI.md`, or
   `~/.config/opencode/AGENTS.md`. An unedited v1/v2/v3/v4 policy is upgraded in
   place; a changed managed block reports `drifted` instead of being
   overwritten.
2. Skill: writes the bundled, version-matched skill once to
   `~/.agents/skills/mnemark`. Pi reads it directly; Claude Code and Codex
   receive per-skill symlinks. Gemini CLI and OpenCode have no configured skill
   directory and rely on policy prose. A legacy platform copy is migrated only
   when it contains managed mnemark files; unmanaged files report `conflict`
   and are never deleted.
3. Session start: Claude Code receives a `SessionStart` hook in
   `~/.claude/settings.json`. Other platforms rely on the policy's visible
   `mem prime` instruction.

Setup has no project mode and never selects the current repository implicitly.
Project-specific recall remains a logical memory scope selected by commands
such as `mem prime --scope auto` inside the shared active store. Explicit path
overrides may still target a sandbox or custom user installation root.

Use `--base-dir <DIR>` to target a different user home, `--instructions` to
override the policy path, `--skills-dir` to override the platform link parent,
or `--shared-skills-dir` to override the shared Agent Skills parent.
`--no-skill`/`--no-hook` skip layers, and `--dry-run` previews changes.
`setup list` prints the shared source, platform link mode, and resolved paths.

## Doctor

```bash
mem doctor
mem doctor --platform claude-code
mem doctor --base-dir /tmp/sandbox
```

`doctor` verifies binary/schema compatibility, SQLite `quick_check`, durable
store identity, Unix permissions, graph health, index freshness, store Git
versioning, managed policy content, shared skill files, platform links, and
session hooks. Output is JSON with per-check `status` (`ok`, `warn`, `missing`,
`error`) and a `fix` hint for anything not ok. It always resolves the runtime
store (`--home`, `MNEMARK_HOME`, user config, `~/.mnemark`) and never treats a
source checkout as the store.

## Session priming

```bash
mem prime
mem prime --scope auto --budget 4000
mem prime --focus "release safety"
mem prime --per-section 5 --format json
```

`prime` emits one compact context block for agent session starts: user,
feedback,
preference, and project memories plus workflow names for the detected scope,
followed by the save-before-finishing protocol. It is read-only unless `--focus`
is supplied; focused priming refreshes graph and stale Tantivy state when
needed, then adds budget-capped graph neighborhood context for the task focus.
Text and JSON
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
mem save \
  --type feedback \
  --name pr_small \
  --scope "project:example/ot-product" \
  --source agent \
  --tags '["style:review","decision:pr-size"]' \
  --content "PR 拆小逐個 review"
mem workflow new release_runbook
mem save \
  --type workflow \
  --name release_runbook \
  --scope global \
  --source manual \
  --user-confirmed \
  --tags '["workflow:release","intent:release","risk:high"]' \
  --content-file release_runbook.yaml
mem save \
  --type project \
  --name q4_freeze \
  --description "Mobile release cut" \
  --tags '["project:example/ot-mobile"]' \
  --content "..." \
  --expires-at 2026-03-05T00:00:00Z \
  --why "freeze for release branch"
```

Supported `--type` values are `user`, `feedback`, `project`, `reference`
(default), `preference`, and `workflow`. Defaults are `--scope global`,
`--source agent`, and `--tags '[]'`. Optional metadata fields are
`--description`, `--why`, and strict-RFC3339 `--expires-at`. Resource gates cap
names and scopes at 256 bytes, descriptions/why/tags at 64 KiB, content at
1 MiB, and tags at 100 entries.

Confidence is inferred from source unless `--confidence` is provided:

- `manual`: high, protected
- `agent`: medium
- `daily_retro`: low
- `weekly_retro`: low

`save` accepts inline `--content` or `--content-file`. Secret-like values in
names, descriptions, content, tags, scopes, provenance, and other durable
fields reject the write by default. Use `--redact-secrets` only when replacing
the detected value with `[REDACTED]` is explicitly intended. `--source manual`
requires `--user-confirmed`; imported or merged manual claims without durable
attestation are downgraded to agent trust.

Without `--force`, `save` returns `duplicate_found` for an exact name match and
`similar_found` for high-overlap content. The caller should decide whether to
skip, update, or supersede. With `--force`, an exact-name save updates the
existing memory only if the incoming source is at least as trusted as the
existing source.

Workflow memories are validated on save/import unless
`--no-validate-workflow` is passed. Merge validates them too; invalid incoming
workflows become pending ambiguity records instead of being imported. Required
fields are `schema_version`, `goal`, `triggers`, `steps`, and `stop_conditions`;
each step needs `id` plus one of `run`, `check`, `manual`, or `ask`. Scaffold
runbooks with `mem workflow new <name>`; its baseline template is embedded in
the binary.

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

By default, query treats punctuation as literal text, so searches like
`project:owner/repo` do not require Tantivy syntax escaping. Use `--raw-query`
only for intentional Tantivy syntax such as `name:pr_small`.

Query is read-only and no-touch by default. Use `--touch` only for intentional
access telemetry; it acquires the write lock. A stale Tantivy index causes an
actionable error instead of hidden repair: run `mem reindex` or opt in with
`--repair-index`. Relevance sorting combines normalized lexical score, source
trust, confidence, scope specificity, and recency; `--explain-score` exposes
every component. Superseded and inactive memories are hidden unless requested.
`--format json|table|compact` controls output; prefer `compact` for agent
context.

`--tags` matches exact JSON-array membership, not substrings. `--fuzzy`
searches across `name`, `description`, `content`, and `tags`. Query has no
embedding mode: Tantivy owns lexical/fuzzy retrieval, while `mem graph query`
and focused priming provide evidence-bearing relationship context without a
hidden provider call.

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

`update`, `supersede`, and `delete` accept `--source`; manual source requires
`--user-confirmed`. `update` can change type, destination scope, confidence,
description, content, expiry, source, and tags (`--set-tags`, `--add-tags`,
`--remove-tags`), with clear flags for nullable fields. Bare names resolve
within `--scope`; use `id:<memory-id>` to force an id when a scoped name
collides with another memory's id.

Soft delete sets `valid_until`. Expired, deleted, and superseded memories are
excluded from ordinary query/prime/workflow/graph recall. Hard delete removes
the row; protected memories require `--force`.
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
mem workflow record release_runbook \
  --result failure \
  --note "push rejected at step publish"
```

`workflow find <intent>` searches by positional intent. Run notes reject
secret-like values unless `--redact-secrets` is explicit; manual source requires
`--user-confirmed`. Show and validate accept a workflow name or memory id.

`workflow show --checklist` renders an ordered execution checklist, flags
`confirm: true` steps, and appends the run-record step. Prefer it over raw JSON
before execution. `workflow show --with-graph-context` refreshes the graph when
needed and includes related workflow, artifact, policy, and run edges.

`workflow new <name>` scaffolds embedded baseline YAML (default `<name>.yaml`;
`--output` and `--force` are available). `workflow record` writes run telemetry;
record every run so retrospectives can identify stale or repeatedly failing
runbooks from evidence. Its response returns `post_run_memory` items, or
`post_run_memory_missing`; process that closing checklist before ending work.
Validation emits a non-blocking `no_post_run_memory` warning when needed.

Workflow helpers never execute runbook commands. Agents execute steps, verify
checkpoints, and ask before risky side effects. Keep project-specific executable
logic in version-controlled repository files and cross-project helpers under
the store's `artifacts/` tree. `workflow validate --check-artifacts` checks
`owner: knowledge_store` entries against manifest paths, files, checksums, and
executable bits. Replace placeholders first. Artifact paths used by
`steps.run` must also appear in `reusable_scripts`.

## Artifacts

Run `mem config show` first and change directory to its reported `root` before
the filesystem commands below. `artifact add` registers an existing path under
that active store; it does not copy an external file into the store.

```bash
mem artifact list
mem artifact show ci-triage
mem artifact show scripts.ci-triage
mem artifact check
mkdir -p artifacts/scripts
printf '#!/usr/bin/env sh\nprintf "collect ci context\\n"\n' \
  > artifacts/scripts/ci-triage.sh
chmod +x artifacts/scripts/ci-triage.sh
mem artifact add artifacts/scripts/ci-triage.sh \
  --name ci-triage \
  --kind script \
  --scope global \
  --description "Collect CI failure context" \
  --executable
mem artifact update ci-triage --checksum
mem artifact remove ci-triage
mem artifact remove ci-triage --delete-file
```

Artifact commands inspect `manifest.toml` and files under `artifacts/` in the
active store root. Select it with `--home`, `MNEMARK_HOME`, user config, or the
default `~/.mnemark`; source repositories are never discovered as stores.
`artifact list` prints manifest entries as JSON. `artifact show <name>` accepts
a unique short name or a qualified name such as `scripts.ci-triage`.
`artifact check` verifies manifest parsing, path containment, file presence,
SHA-256 checksums, and required executable bits. It reports unsafe paths,
missing files, invalid/mismatched checksums, invalid scopes, and non-executable
scripts as JSON. Artifact commands never execute scripts.

`artifact add` derives the manifest name from the file stem unless `--name` is
provided, requires a normalized path to an existing regular file under the
active store, and rejects symlinks in the file or any intermediate path before
secret scanning, redaction, or hashing. It rejects secret-like content/metadata
unless `--redact-secrets` is explicit, computes `sha256:<hex>`, and writes
deterministic TOML. Existing name or path conflicts require `--force`.
`artifact update <name> --checksum` applies the same path and secret gates
before
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

Bundles include `memory.db`, optional `config.toml`, optional `manifest.toml`,
`artifacts/`, and `bundle.json`. Stores and bundles are plaintext; use
disk/volume encryption where needed and transfer archives only through a trusted
channel. Export uses SQLite's online backup API without checkpointing or
mutating the live store.

Bundle v2 records a SHA-256 for every imported file. Inspect/import reject
missing, extra, or mismatched files and unexpected SQLite tables, views, or
triggers before destination mutation. Hashes detect modification but do not
authenticate the bundle publisher. Legacy bundles without a complete hash
manifest require explicit `--allow-unverified`. Secret-like durable values
reject
export/import by default; `--redact-secrets` changes only the staged/imported
copy.

Bundles exclude `index/`, `.mem.lock`, `memory.db-wal`, and `memory.db-shm`.
Use `bundle export --no-config` when config has machine-local paths.
`bundle inspect` lists entries and metadata without importing. Default import
requires a clean store and rebuilds the index. A non-empty store requires
`--merge` or `--replace --force`. Merge copies only non-conflicting regular
artifact files; symlinks and special files are rejected. Replacement takes an
online rollback snapshot and restores it if installation fails.

## Ambiguity

```bash
mem ambiguity add \
  --query "PR 策略" \
  --memory-ids '["pr_small","pr_bundled"]' \
  --context "scope unclear"
mem ambiguity list --pending
mem ambiguity resolve 1 --note "project-specific preference"
mem ambiguity resolve 1 --keep pr_small --soft-delete-others
```

Ambiguity queries, context, and resolution notes reject secret-like values
unless `--redact-secrets` is explicit. `resolve --keep ...
--soft-delete-others` soft-deletes non-protected alternatives and records
skipped protected memories. `ambiguity list` parses `memory_ids`, structured
merge-conflict `context`, and `resolution` as JSON values in the output.

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

`history` accepts an optional positional `name`,
`--action <save|update|supersede|delete|...>`, and `--limit` (default 20).
`gc --days`
purges soft-deleted memories older than the given window.

`audit` reports broken supersede links, expired-but-active records, stale and
cleanup candidates, and `over_budget_scopes`. Each over-budget scope includes
low-access curation candidates, excluding protected manual memories. Curate by
merging, superseding, or deleting rather than raising the cap by default.

`retro` emits an orchestration bundle for the LLM. It does not read platform
logs; the platform or harness must provide conversation history.
`retro daily|weekly --limit` controls bundle size (default 50).

## Reconcile

```bash
mem reconcile
mem reconcile --scope project:example/app --repo ~/code/app
mem reconcile --type project
```

`reconcile` treats memories as a cache of external reality. It extracts path
and command claims from backtick spans first, then bare path-like tokens as a
fallback. Paths are checked relative to `--repo` (default: current directory;
`<placeholder>` segments match any entry), and commands use `PATH` lookup.

The command is read-only: it never executes claims, edits memories, or updates
access counters. Default `--scope auto` checks global plus the detected project;
an explicit scope checks only that scope. Workflow memories are skipped unless
`--type workflow` is passed because workflow validation owns runbook checks.
The report marks claims `ok` or `missing`, lists `unverifiable` spans, and
leaves update/supersede/delete judgment to the agent.

## Graph

```bash
mem graph rebuild
mem graph stats
mem graph explain release_runbook --scope auto
mem graph path release_runbook \
  artifact:artifacts/scripts/build-release.sh \
  --direction any
mem graph path release_policy release_runbook \
  --max-depth 4 \
  --confidence inferred \
  --direction outgoing \
  --format compact
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

`graph` materializes an explainable relationship index in SQLite tables:
`graph_nodes`, `graph_edges`, durable `graph_semantic_edges`, and append-only
`graph_semantic_edge_revisions`. It is graph-first context retrieval, not
embedding RAG: no vector database, provider, or hidden LLM call is required.
Graph-dependent commands rebuild only when dirty or schema-stale. `graph stats`
never rebuilds implicitly, so `dirty` remains observable.

Deterministic rebuild creates memory, tag, scope, type, source, artifact,
claim, workflow step/run, and concept nodes. Relations include `has_tag`,
`in_scope`, `has_type`, `from_source`, `superseded_by`, `mentions_path`,
`mentions_command`, `references_artifact`, `has_workflow_step`,
`step_uses_artifact`, `requires_confirmation`, `recorded_run`, and `has_result`.

Active, unexpired memories are normal inputs. Superseded rows remain only for
lineage; deleted/expired memories and expired semantic edges are excluded.
Administrative `has_type`, `in_scope`, and `from_source` edges remain visible
but are not path/query bridges unless `--include-metadata` is explicit.

`graph explain <reference>` accepts graph ids such as `memory:<id>`,
`tag:<tag>`, and `artifact:<path>`; `id:<memory-id>`; or a scoped memory name.
It returns active direct neighbors with relation, confidence, status, evidence,
and direction. `--depth` accepts 0 or 1. Explain/path default to `--scope auto`
(global plus detected project); use `--scope all` only intentionally.

`graph path <from> <to>` finds a minimum-hop active path, then breaks equal-hop
ties globally by cumulative relation weight, confidence, and stable identity.
`--direction any|outgoing|incoming` controls orientation. Pending edges are
excluded unless `--include-ambiguous` includes pending `AMBIGUOUS` relations.
`--confidence extracted|inferred|all` narrows labels; default is all active
edges. Graph export emits JSON with `schema_version`, `nodes`, and `edges`.

`graph query "terms"` resolves start nodes from exact graph ids, memory
names/ids, labels, and scored Tantivy search. It expands an evidence-bearing
neighborhood with confidence, scope, relation, source-trust,
memory-confidence, fanout, and depth controls. It returns context for the agent
to interpret; it does not synthesize answers.

`graph candidates` emits strict JSON instructions, allowed semantic relations,
and candidate memories. Use `--changed-since` or `--unlinked` to bound work.
The Rust CLI never calls an LLM. The agent writes semantic-edge JSON; ingest
then validates unknown fields, relations, endpoints, confidence, RFC3339
`valid_until`, evidence, concept ids, secrets, and source trust before durable
storage.

Manual assertions require `--source manual --user-confirmed` and persist the
confirmation timestamp. Merge downgrades unattested manual edges.
`EXTRACTED`/`INFERRED` edges are active by default; `AMBIGUOUS` edges are
pending. `--pending-inferred` also stores inferred edges as pending.

`graph review --pending|--ambiguous` shows semantic edges with evidence, source
spans, and linked ambiguity ids. Accept marks an edge active; reject marks it
rejected. Review notes use the same default-reject/explicit-redaction secret
policy. Both resolve the linked ambiguity and append a revision without
rewriting confidence. Agent-generated cross-project edges and logical conflicts
default to pending review. Graph data is context and provenance, never an
instruction override.

## Import and Export

```bash
mem export --format json
mem export --format markdown
mem import memories.json
mem import memories.json --summary-only
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

JSON imports process an array of memory-like objects. The CLI first validates
the complete array without writing, then rewinds and streams 500-item
transaction chunks so malformed JSON leaves the store unchanged without
retaining the full parsed array in memory. Use `--summary-only` for large
imports to return only `status`, `total`, and `counts`; the default retains the
per-item `results` contract. Markdown or other files import as one `reference`
memory unless `--type` is supplied. Import files are capped at 256 MiB.
Ordinary JSON/Markdown export is also capped at 256 MiB of stored text to keep
agent processes bounded; use `mem bundle export` for a complete large-store
snapshot. Import defaults to agent provenance, rejects secret-like values by
default, and supports explicit `--redact-secrets`. `--source manual` requires
`--user-confirmed`; the import records origin and origin reference for later
audit.

## Merge

```bash
mem merge /path/to/theirs.db
mem merge /path/to/theirs.db --prefer-trusted
mem merge /path/to/theirs.db --redact-secrets
```

Merge validates SQLite integrity and rejects secrets across incoming memories,
ambiguities, workflow runs, changelog, semantic edges, and semantic revisions by
default. `--redact-secrets` uses a temporary online snapshot and never mutates
the source database. Merge imports memories with new scoped names, skips
identical `(scope,name)` records, and records same-name content conflicts in
`ambiguities` instead of overwriting automatically. Merge conflict ambiguity
records include a structured incoming snapshot in `context`.

Lower-trust incoming same-name memories are rejected. `--prefer-trusted` lets a
higher-trust incoming memory update a lower-trust local memory; equal-trust
differences still become ambiguities. Durable ambiguities, workflow runs,
changelog events, semantic edges, and append-only semantic revisions are merged
idempotently through store/event UIDs with memory/edge/ambiguity ID remapping.
Lower-trust logical conflicts are rejected, unattested manual claims are
downgraded, and unresolved/equal-trust conflicts remain pending with ambiguity
evidence. Bundle merge and sync conflict resolution use the same behavior.

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
hooks/signing/fsmonitor disabled, and fetches/merges a configured remote. It
does
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
base=https://github.com/NeoHsu/mnemark/releases/latest/download
curl --proto '=https' --tlsv1.2 -LsSf \
  "$base/mnemark-installer.sh" |
  sh
```

```powershell
# Windows PowerShell
$base = "https://github.com/NeoHsu/mnemark/releases/latest/download"
powershell -ExecutionPolicy Bypass -c `
  "irm $base/mnemark-installer.ps1 | iex"
```

Manual archives are available on the
[latest release page](https://github.com/NeoHsu/mnemark/releases/latest).
The latest published version may lag behind this source guide; verify the
installed contract before using the examples above:

```bash
mem --version
```

From a mnemark repository checkout, release verification uses the repository's
own scripts:

```bash
scripts/build-release.sh
scripts/smoke-release.sh
```

After building, `scripts/smoke-release.sh` copies the release binary into an
isolated install directory and verifies that `mem init`, `config show`,
save/query/reindex/export all work against a runtime-only store with no schema
file. For source-only development, expose `target/release/mem` on `PATH` or
follow `docs/development.md` from the matching repository tag. All examples in
this guide assume `mem` is on `PATH`.
