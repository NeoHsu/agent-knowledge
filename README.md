# mnemark

**Portable, local-first memory for coding agents.**

[![CI](https://github.com/NeoHsu/mnemark/actions/workflows/ci.yml/badge.svg)](https://github.com/NeoHsu/mnemark/actions/workflows/ci.yml)
[![GitHub release](https://img.shields.io/github/v/release/NeoHsu/mnemark?display_name=tag)](https://github.com/NeoHsu/mnemark/releases/latest)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

Save preferences, project decisions, corrections, and trusted procedures once;
recall them from Claude Code, Codex, Pi, Gemini CLI, or OpenCode without using a
hosted memory service. SQLite is durable truth. Search and graph projections are
local and rebuildable. Stored content is prior data, never instruction
authority.

> [!IMPORTANT]
> This branch documents source version `0.10.0`. Use released binaries only
> from the matching Git tag and GitHub Release after its workflow completes
> successfully. Run `mem --version`; the CLI, bundled skill, and documentation
> must remain in exact release lockstep.

[Quick start](#quick-start) · [Capabilities](#capabilities) ·
[Evidence](#evidence-and-support-boundary) · [Documentation](#documentation)

## Quick start

### Try an isolated store

This complete save/query/prime example never touches an existing store:

```bash
demo_store="$(mktemp -d "${TMPDIR:-/tmp}/mnemark-demo.XXXXXX")"
trap 'rm -rf -- "$demo_store"' EXIT

mem --home "$demo_store" init
mem --home "$demo_store" save \
  --type preference --name concise_answers --scope global \
  --source manual --user-confirmed --tags '["style:concise"]' \
  --content 'Trigger: user-facing replies. Action: answer concisely. Why: explicit preference.'
mem --home "$demo_store" query "concise replies" --format compact
mem --home "$demo_store" --read-only prime
```

For a real store, inspect the resolved target before initialization:

```bash
mem config show
mem init
```

### Install

With [mise](https://mise.jdx.dev/):

```bash
mise use --global --pin github:NeoHsu/mnemark@latest
mem --version
```

For checksum-verified shell and PowerShell installers, direct release archives,
SBOMs, and provenance verification, follow
[Getting Started](docs/getting-started.md).

### Wire one agent

Preview one platform before writing user-level policy, skill, or hook files:

```bash
mem setup list
mem setup pi --dry-run
mem setup pi
mem doctor --platform pi
```

Replace `pi` with `claude-code`, `codex`, `gemini-cli`, or `opencode` only for a
platform you use. Agent policy and Claude Code's session hook enforce the same
session sequence: exact compatibility check, process-level read-only priming,
and fail-closed continuation without memory when either gate fails.

## Why mnemark

| Without mnemark | With mnemark |
| --- | --- |
| Knowledge is tied to one agent platform | One private store serves supported agents |
| Session context disappears | Bounded priming restores explicit prior data |
| Search returns isolated text matches | Focused recall can include evidence-bearing relationships |
| Procedures remain in old chats | Validated workflow memories are searchable runbooks |
| Backups and transfer are opaque | SQLite snapshots, bundles, checksums, and private Git are operator-controlled |
| Automation guesses side effects | Versioned schemas and parsed operation effects are discoverable |

The agent keeps judgment and the user keeps authority. `mem` performs
deterministic storage, retrieval, validation, and versioning; it never executes
workflow commands or silently promotes stored text into instructions.

## System boundary

```text
supported coding agents
          |
          v
       mem CLI
          |
          v
 SQLite durable source
    |          |          |
    v          v          v
 Tantivy    graph      workflows,
 search     projection artifacts, bundles
(rebuildable) (rebuildable)
```

Runtime-store discovery is always:

```text
--home -> MNEMARK_HOME -> user config knowledge_home -> ~/.mnemark
```

Source checkouts and executable parents are never selected implicitly. Reads do
not initialize or migrate a store.

## Everyday agent loop

Unless a session hook already injected a delimited mnemark context block, the
first two memory invocations are:

```bash
mem --json-errors contract --skill-version 0.10.0
mem --read-only prime
```

Then:

1. do the task with the store structurally read-only;
2. query only when primed context is insufficient;
3. load a workflow for a recurring procedure;
4. at work-unit completion, save confirmed corrections, decisions, and reusable
   procedures as `Trigger / Action / Why`;
5. preview synchronization with `mem sync --dry-run`;
6. never pass `--push` without explicit approval.

The exact agent policy, target checks, conditional-effect rules, and mutation
boundaries live in the bundled skill and are installed from the same release as
the CLI.

## Capabilities

### Deterministic recall

Tantivy provides lexical and fuzzy retrieval. SQLite applies scope, lifecycle,
trust, confidence, and deterministic reranking. Focused priming may add bounded
relationship context from the local graph projection.

```bash
mem --read-only query "release safety" --scope auto --format compact
mem operation inspect --store-exists -- prime --focus "release safety"
mem prime --focus "release safety"
mem graph path release_policy release_runbook --format compact
```

Inspect conditionally mutating invocations before use. Global `--read-only`
blocks durable, rebuildable, output-file, and network effects before mutation.

See [Graph Memory](docs/graph-memory.md) and
[Runtime Model](docs/runtime-model.md).

### Workflow runbooks and artifacts

Workflow memories are validated procedure data. `mem` finds, validates, renders,
and records them; the calling agent executes approved steps.

```bash
mem workflow find release --scope auto
mem workflow show release_runbook --checklist
mem workflow validate release_runbook --check-artifacts --repo .
```

Repository-specific helpers stay under repository `scripts/`. Cross-project
helpers may be registered as private-store artifacts. Validation checks paths,
regular files, checksums, and executable intent without running helpers.

See [Workflows](docs/workflows.md).

### Curation and reconciliation

```bash
mem retro daily
mem retro weekly --limit 200
mem --read-only reconcile --scope auto
```

Retrospectives emit read-only review bundles; they do not schedule themselves or
read platform logs. Reconciliation checks remembered filesystem and command
claims without executing them.

### Backup, transfer, and sync

```bash
mem bundle export mnemark-store.tgz
mem bundle inspect mnemark-store.tgz
mem sync --dry-run
mem sync
```

Sync checkpoints the store locally and may fetch a configured private remote;
network push remains impossible unless `--push` is explicit. Same-name semantic
conflicts become pending ambiguities rather than silent overwrites.

Stores and bundles are plaintext. Use private storage, trusted transfer
channels, and disk or volume encryption where confidentiality matters.

## Stable machine contracts

Automation does not need to scrape help text:

```bash
mem contract
mem schema list
mem operation list
mem operation inspect -- query "release notes"
mem --read-only query "release notes"
```

- `mem contract` is store-independent.
- `mem schema list|print` exposes bundled JSON Schemas.
- `mem operation list|inspect` returns stable leaf operation IDs and exact
  store/file/network effects.
- `--read-only` or `MNEMARK_READ_ONLY=true` enforces the parsed effect decision.
- `--max-bytes` or `MNEMARK_MAX_BYTES` rejects oversized stdout before partial
  publication.
- `--json-errors` emits versioned typed error envelopes.

See [Compatibility](docs/compatibility.md) and
[JSON Contracts](docs/json-schemas.md).

## Agent skill

The CLI and bundled skill use exact release lockstep. For an exact-version
manual install:

```bash
npx skills add https://github.com/NeoHsu/mnemark/tree/v0.10.0 --skill mnemark
mem --json-errors contract --skill-version 0.10.0
```

For development from a local checkout:

```bash
npx skills add ./skills/mnemark
```

`mem setup <platform>` remains preferred because the binary embeds and verifies
its exact skill package.

## Evidence and support boundary

| Evidence | Boundary |
| --- | --- |
| Published release | `v0.9.0` points to its own release commit; it is not evidence for this `main` revision |
| Supported store | One operating-system user, private store, up to 10,000 memories |
| Published latency baseline | Retained `v0.8.0` Apple M2 Max report; not a cross-machine SLO |
| Current source gates | Formatting, tests, security, retrieval, recovery, binary-size, and benchmark checks are configured locally and in CI |
| Agent behavior | Synthetic fixtures validate the evaluator; no retained live cross-agent matrix is published |
| Capacity canary | Retained 100,000-memory correctness run; not a support claim |
| Artifact matrix | Five configured targets across macOS, Linux, and Windows |
| Supply chain | Archive/installer checksums, native archive execution, CycloneDX SBOM, and GitHub provenance configuration |

A local pass does not establish qualification for an unpushed commit. Release
evidence belongs to the exact commit and artifacts that produced it. See
[Evaluation](docs/evaluation.md), [Performance](docs/performance.md), and
[Production Operations](docs/production.md).

## Security boundary

The qualified deployment profile is deliberately narrow:

- one operating-system user per private store;
- explicit backup-first migrations and no automatic downgrade;
- private, access-controlled storage and trusted Git/bundle channels;
- host-level encrypted storage for confidential memory.

Bundle hashes detect corruption, not publisher identity. Secret scanning is
defense in depth, not DLP. Unix permissions are enforced as `0700`/`0600`;
Windows user-only ACL verification must be performed with platform tooling.

Read [Security](SECURITY.md) before operating a real store.

## Documentation

| Goal | Start here |
| --- | --- |
| Install and save the first memory | [Getting Started](docs/getting-started.md) |
| Understand the system | [Overview](docs/overview.md) |
| Inspect discovery, effects, and layout | [Runtime Model](docs/runtime-model.md) |
| Use workflows, artifacts, bundles, and retrospectives | [Workflows](docs/workflows.md) |
| Understand relationship-aware recall | [Graph Memory](docs/graph-memory.md) |
| Integrate automation | [Compatibility](docs/compatibility.md) and [JSON Contracts](docs/json-schemas.md) |
| Operate or recover a store | [Production Operations](docs/production.md) and [Security](SECURITY.md) |
| Evaluate retrieval or agent policy | [Evaluation](docs/evaluation.md) and [Performance](docs/performance.md) |
| Change the repository | [Agent Reference](docs/agent-reference.md), [Development](docs/development.md), and [Contributing](CONTRIBUTING.md) |
| Understand architectural decisions | [Architecture](docs/architecture.md) and [ADRs](docs/adr/README.md) |

The complete task-oriented index is [docs/README.md](docs/README.md).

## Development

Agents must read [`docs/agent-reference.md`](docs/agent-reference.md) before
changing this repository.

```bash
mise install
mise run check:pr
```

Do not use a real memory store in tests. See [Development](docs/development.md),
[Contributing](CONTRIBUTING.md), and [Changelog](CHANGELOG.md).
