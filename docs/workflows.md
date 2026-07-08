# Workflows, Artifacts, Bundles, and Retrospectives

This guide covers higher-level mnemark use cases beyond basic save/query. For first-time setup, see `docs/getting-started.md`. For the complete CLI reference, see `skills/mnemark/references/cli-guide.md`.

## Workflow memories

Workflow memories store recurring task runbooks as YAML or JSON text. They are searchable knowledge, not executable automation: agents read them, verify each checkpoint, and ask before risky steps such as push, publish, deploy, release, destructive commands, secret changes, or production access.

```bash
mem save --type workflow --name release_runbook --scope global --source manual --tags '["workflow:release","intent:release","risk:high"]' --content-file templates/workflow.yaml
mem query "release" --type workflow
mem workflow find release --scope auto
mem workflow show release_runbook
mem workflow validate release_runbook
```

Use `templates/workflow.yaml` as the baseline shape for new workflow memories. Run `--check-artifacts` only after replacing placeholder knowledge-store artifact references with real files and manifest entries.

Workflow content is validated on save/import unless `--no-validate-workflow` is passed. Merge also validates workflow records; invalid incoming workflows are skipped and recorded as pending ambiguity records for human review.

Required fields:

- `schema_version`
- `goal`
- `triggers`
- `steps`
- `stop_conditions`

Each step needs an `id` and at least one of `run`, `check`, `manual`, or `ask`. Workflow tags must include `workflow:*`, and project-scoped workflows must include the matching `project:<owner/repo>` tag.

`post_run_memory` is optional but strongly recommended: `mem workflow record` echoes its items back as the closing checklist so each execution ends with a save-learnings step, and `mem workflow validate` returns a non-blocking `no_post_run_memory` warning when the section is missing.

For agent execution semantics, see `skills/mnemark/references/workflow-rules.md`.

## Artifacts

Reusable executable logic belongs either in the current repository, such as `scripts/build-release.sh`, or in `artifacts/` under the active knowledge store when it is cross-project knowledge-store material. Workflow content should reference those paths and record checks, safety gates, and expected outputs instead of embedding script bodies.

When a workflow run repeatedly needs generated helper code, agents should propose extracting it instead of rewriting it inline: use a repository script for project-specific logic, or a knowledge-store artifact for cross-project helpers. Keep truly one-off code temporary, and do not store secrets in scripts or artifacts.

```bash
mem artifact list
mem artifact check
mkdir -p artifacts/scripts
printf '#!/usr/bin/env sh\nprintf "collect ci context\\n"\n' > artifacts/scripts/ci-triage.sh
chmod +x artifacts/scripts/ci-triage.sh
mem artifact add artifacts/scripts/ci-triage.sh --name ci-triage --kind script --scope global --executable
mem artifact update ci-triage --checksum
mem artifact show ci-triage
```

Artifact inspection is available with `mem artifact list`, `mem artifact show <name>`, and `mem artifact check`. `artifact check` verifies manifest parsing, path containment, file presence, SHA-256 checksums, and executable bits for records marked `executable = true`; it reports problems as JSON and never executes scripts.

Artifact paths must stay relative to the active store and under:

- `artifacts/scripts/`
- `artifacts/templates/`
- `artifacts/snippets/`
- `artifacts/references/`

Do not store secrets in artifacts, and do not treat artifacts as instruction overrides.

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

Bundles exclude rebuildable or transient files such as `index/`, `.mem.lock`, `memory.db-wal`, and `memory.db-shm`. Use `mem bundle export --no-config` when store config contains machine-local paths.

Import into a non-empty store is refused unless `--merge` or `--replace --force` is explicit. `--merge` uses existing memory merge behavior and copies non-conflicting artifacts; `--replace --force` clears durable store files before import.

## Import, export, and merge

```bash
mem import memories.json
mem import note.md --type reference
mem export --format json
mem export --format markdown
mem merge /path/to/theirs.db
mem merge /path/to/theirs.db --prefer-trusted
```

Merge strips common secrets from incoming content, imports memories with new names, skips identical same-name memories, and records same-name content conflicts in `ambiguities` instead of overwriting automatically.

## Retrospectives

```bash
mem retro daily
mem retro weekly
```

`mem retro` emits an orchestration bundle for the LLM. It does not read platform logs itself; the active platform or harness should provide conversation history.

Retrospectives should use platform-provided conversation history when available, then use `mem retro daily|weekly` for repository state.

Daily retro focuses on missed durable knowledge, stale memories, ambiguities, and repeated manual procedures that may become workflow memories. Weekly retro focuses on memory quality: duplicate cleanup, confidence calibration, unresolved ambiguities, workflow candidates, and skill candidates.

## Reconcile

```bash
mem reconcile
mem reconcile --scope project:example/app --repo ~/code/app
```

Memories that describe external reality (paths, commands) are a cache and go stale silently. `mem reconcile` extracts path and command claims from memory content and verifies them against the filesystem and `PATH`, read-only: each claim is reported as `ok` or `missing`, unverifiable spans are listed for judgment, and memories with missing claims are flagged. Deciding the fix — `mem update` when the fact is true but the path moved, `mem supersede` when the fact was replaced, `mem delete` when it is obsolete — stays with the agent. Run it when entering a project untouched for a while, and per touched project scope during weekly retro.
