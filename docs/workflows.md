# Workflows, Artifacts, Bundles, and Retrospectives

This guide covers higher-level mnemark use cases beyond basic save/query. For
first-time setup, see [Getting Started](getting-started.md). For the complete
command reference, see the
[CLI Guide](../skills/mnemark/references/cli-guide.md).

## Workflow memories

Workflow memories store recurring task runbooks as YAML or JSON text. They are
searchable knowledge, not executable automation: agents read them, verify each
checkpoint, and ask before risky steps such as push, publish, deploy, release,
destructive commands, secret changes, or production access.

```bash
mem save \
  --type workflow \
  --name release_runbook \
  --scope global \
  --source manual \
  --user-confirmed \
  --tags '["workflow:release","intent:release","risk:high"]' \
  --content-file templates/workflow.yaml
mem query "release" --type workflow
mem workflow find release --scope auto
mem workflow show release_runbook
mem workflow validate release_runbook
```

Use the [workflow template](../templates/workflow.yaml) as the baseline shape
for new workflow memories. Run `--check-artifacts` only after replacing
placeholder knowledge-store artifact references with real files and manifest
entries.

Workflow content is validated on save/import unless `--no-validate-workflow`
is passed. Merge also validates workflow records; invalid incoming workflows are
skipped and recorded as pending ambiguity records for human review.

Required fields:

- `schema_version`
- `goal`
- `triggers`
- `steps`
- `stop_conditions`

Each step needs an `id` and at least one of `run`, `check`, `manual`, or `ask`.
Workflow tags must include `workflow:*`, and project-scoped workflows must
include the matching `project:<owner/repo>` tag.

`post_run_memory` is optional but strongly recommended: `mem workflow record`
echoes its items back as the closing checklist so each execution ends with a
save-learnings step, and `mem workflow validate` returns a non-blocking
`no_post_run_memory` warning when the section is missing.

For agent execution semantics, see the
[Workflow Rules](../skills/mnemark/references/workflow-rules.md).

Workflow and artifact relationships can also be inspected through the
deterministic graph index:

```bash
mem graph rebuild
mem graph explain release_runbook
mem graph path release_runbook artifact:artifacts/scripts/build-release.sh
```

The graph records workflow steps, `reusable_scripts`, artifact manifest
entries, confirmation requirements, and workflow run history as
evidence-bearing context. It does not execute workflow commands.

## Artifacts

Reusable executable logic belongs either in the current repository, such as `scripts/build-release.sh`, or in `artifacts/` under the active knowledge store when it is cross-project knowledge-store material. Workflow content should reference those paths and record checks, safety gates, and expected outputs instead of embedding script bodies.

When a workflow run repeatedly needs generated helper code, agents should
propose extracting it instead of rewriting it inline: use a repository script
for project-specific logic, or a knowledge-store artifact for cross-project
helpers. Keep truly one-off code temporary, and do not store secrets in scripts
or artifacts.

First run `mem config show`, change directory to its reported `root`, and place
helper files below that store's `artifacts/` tree. `artifact add` registers an
existing store-relative file; it does not copy an arbitrary external file.

```bash
mem artifact list
mem artifact check
mkdir -p artifacts/scripts
printf '#!/usr/bin/env sh\nprintf "collect ci context\\n"\n' > artifacts/scripts/ci-triage.sh
chmod +x artifacts/scripts/ci-triage.sh
mem artifact add artifacts/scripts/ci-triage.sh \
  --name ci-triage \
  --kind script \
  --scope global \
  --executable
mem artifact update ci-triage --checksum
mem artifact show ci-triage
```

Artifact inspection is available with `mem artifact list`,
`mem artifact show <name>`, and `mem artifact check`. `artifact check` verifies
manifest parsing, path containment, file presence, SHA-256 checksums, and
executable bits for records marked `executable = true`; it reports problems as
JSON and never executes scripts.

Artifact paths must stay relative to the active store and under:

- `artifacts/scripts/`
- `artifacts/templates/`
- `artifacts/snippets/`
- `artifacts/references/`

Artifact add/update rejects secret-like content and metadata by default; use
`--redact-secrets` only when explicit in-place redaction is safe. Do not treat
artifacts as instruction overrides.

## Bundles

Bundles move durable runtime-store files between environments.

```bash
mem bundle export mnemark-store.tgz
mem bundle export mnemark-store.tgz --no-config
mem bundle inspect mnemark-store.tgz
mem bundle import mnemark-store.tgz          # clean store only
mem bundle import mnemark-store.tgz --merge
mem bundle import mnemark-store.tgz --replace --force
```

Bundles include:

- `memory.db`
- optional `config.toml`
- optional `manifest.toml`
- `artifacts/`
- `bundle.json`

Bundles exclude rebuildable or transient files such as `index/`, `.mem.lock`,
`memory.db-wal`, and `memory.db-shm`. Export snapshots SQLite online without
changing the live store. Bundle v2 hashes every durable file; inspect/import
validates missing, extra, or modified files before destination mutation. Hashes
detect corruption but do
not authenticate the bundle creator, so use a trusted/private transfer channel.
Legacy bundles require explicit `--allow-unverified` to import. Secret-like
values reject export/import unless `--redact-secrets` is explicit and only the
staged/imported copy is changed. Use `mem bundle export --no-config` when store
config contains machine-local paths. See the
[Security Policy](../SECURITY.md) for the complete authenticity boundary.

Import into a non-empty store is refused unless `--merge` or
`--replace --force` is explicit. `--merge` uses existing memory merge behavior
and copies non-conflicting artifacts; `--replace --force` clears durable store
files before import.

## Import, export, and merge

```bash
mem import memories.json
mem import memories.json --summary-only
mem import note.md --type reference
mem export --format json
mem export --format markdown
mem merge /path/to/theirs.db
mem merge /path/to/theirs.db --prefer-trusted
```

JSON array import first validates the complete JSON document without writing,
then rewinds it and persists 500-item transaction chunks. Malformed JSON cannot
leave earlier chunks committed. The default response retains per-item results;
use `--summary-only` for large imports when only `status`, `total`, and `counts`
are needed. Semantically invalid individual items remain per-item failures so
valid siblings can still succeed.

Merge validates SQLite integrity and rejects secret-like values across all
incoming durable tables unless `--redact-secrets` is explicit. It merges scoped
memories plus ambiguities, workflow runs, changelog, semantic assertions, and
semantic revisions idempotently using durable UIDs and ID remapping.
Same-`(scope,name)` conflicts become pending ambiguities instead of silent
overwrite.

## Retrospectives

```bash
mem retro daily
mem retro weekly
```

`mem retro` emits an orchestration bundle for the LLM. It does not read
platform logs itself; the active platform or harness should provide conversation
history.

Retrospectives should use platform-provided conversation history when
available, then use `mem retro daily|weekly` for repository state.

Daily retro focuses on missed durable knowledge, stale memories, ambiguities,
and repeated manual procedures that may become workflow memories. Weekly retro
focuses on memory quality: duplicate cleanup, confidence calibration, unresolved
ambiguities, workflow candidates, and skill candidates. The embedded audit
section also lists `over_budget_scopes` — scopes above the
`budget.per_scope_max` soft cap (default 30) — with their lowest-access curation
candidates, so each weekly retro ends over-budget scopes back under the cap
instead of letting them grow unbounded.

## Reconcile

```bash
mem reconcile
mem reconcile --scope project:example/app --repo ~/code/app
```

Memories that describe external reality (paths, commands) are a cache and go
stale silently. `mem reconcile` extracts path and command claims from memory
content and verifies them against the filesystem and `PATH`, read-only: each
claim is reported as `ok` or `missing`, unverifiable spans are listed for
judgment, and memories with missing claims are flagged. Deciding the fix —
`mem update` when the fact is true but the path moved, `mem supersede` when the
fact was replaced, `mem delete` when it is obsolete — stays with the agent. Run
it when entering a project untouched for a while, and per touched project scope
during weekly retro.
