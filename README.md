# mnemark

**Give every coding agent the same memory—and keep it yours.**

*Memory that grows more useful, not just larger.*

[![CI](https://github.com/NeoHsu/mnemark/actions/workflows/ci.yml/badge.svg)](https://github.com/NeoHsu/mnemark/actions/workflows/ci.yml)
[![GitHub release](https://img.shields.io/github/v/release/NeoHsu/mnemark?display_name=tag)](https://github.com/NeoHsu/mnemark/releases/latest)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

Your coding agent may change. Your preferences, project decisions, release
lessons, and trusted procedures should not disappear with it.

`mnemark` is a local-first memory layer for Claude Code, Codex, Pi, Gemini CLI,
and OpenCode. Save knowledge once and recall it from any supported agent.

As you use it, related facts can become explainable context, repeated procedures
can be promoted into safe runbooks, and explicit retrospectives surface what to
keep, correct, or retire. Every durable change remains explicit, reviewable,
and under your control.

> Local-first · SQLite source of truth · No required hosted memory service

[Try it](#see-it-in-30-seconds) · [Install](#install) ·
[Product highlights](#product-highlights) · [Evidence](#evidence-not-promises) ·
[Evaluation](docs/evaluation.md) · [How it works](#from-memory-to-context) ·
[Documentation](#documentation-map)

```text
+----------------------------------------------------------------------------+
|                          SUPPORTED CODING AGENTS                           |
|          Claude Code     Codex     Pi     Gemini CLI     OpenCode          |
+--------------------------------------+-------------------------------------+
                                       |
                                       v
                             +---------+---------+
                             |      mem CLI      |
                             +---------+---------+
                                       |
                                       v
                         +-------------+-------------+
                         |   SQLite durable source   |
                         +-------------+-------------+
                                       |
         +-------------------+---------+---------+-------------------+
         |                   |                   |                   |
         v                   v                   v                   v
+----------------+  +----------------+  +----------------+  +----------------+
| Tantivy search |  | Relationship   |  | Workflows +    |  | Bundles / Git  |
| (rebuildable)  |  | graph context  |  | artifacts      |  | (portable)     |
+----------------+  +----------------+  +----------------+  +----------------+
```

## Why portable memory instead of platform-native memory?

Coding agents are interchangeable; their built-in memory usually is not.
Preferences, project decisions, recovery procedures, and hard-won lessons can
be trapped in one product, one machine, or one conversation.

| Without mnemark | With mnemark |
| --- | --- |
| Memory is tied to an agent platform | One private store serves every supported agent |
| Important context disappears between sessions | `mem prime` loads a bounded, explicit context block |
| Search stops at matching text | Focused recall can add connected policies, workflows, artifacts, and past runs with relationship evidence |
| Memory accumulates until it becomes stale and noisy | Daily and weekly retrospectives turn history, audit, runs, ambiguities, and graph review into a deliberate curation loop |
| Procedures live as prose in old chats | Validated workflow runbooks are searchable and auditable |
| Backups are platform-specific or opaque | SQLite, bundles, checksums, and private Git remain operator-controlled |
| Automation guesses command behavior | Versioned schemas, typed errors, and operation effects are discoverable |

`mnemark` does not replace agent judgment and does not make stored text an
instruction authority. It gives agents a deterministic memory boundary while
the current user request and higher-priority instructions continue to win.

## See it in 30 seconds

Already have `mem` installed? Use an isolated temporary store to exercise the
complete save → recall → focused-prime loop without touching a real store:

```bash
demo_store="$(mktemp -d "${TMPDIR:-/tmp}/mnemark-demo.XXXXXX")"
trap 'rm -rf -- "$demo_store"' EXIT

mem --home "$demo_store" init
mem --home "$demo_store" save \
  --type preference --name concise_answers --scope global \
  --source manual --user-confirmed --tags '["style:concise"]' \
  --content 'Trigger: user-facing replies. Action: answer concisely. Why: explicit demo preference.'
mem --home "$demo_store" query "concise replies" --format compact
mem --home "$demo_store" prime --focus "prepare a concise answer"
```

The query result is immediately human-readable:

```text
concise_answers [preference] scope=global confidence=high tags=style:concise
  Trigger: user-facing replies. Action: answer concisely. Why: explicit demo preference.
```

`prime` wraps the same durable fact in a delimited context block and labels it
as prior data rather than instruction authority. For a real store, first run
`mem config show` and initialize only the path you intend to use.

## One memory lifecycle, not a pile of notes

Most memory tools stop after storing and searching text. mnemark carries useful
knowledge through its full lifecycle:

```text
+----------+   +---------+   +---------+   +---------+   +---------+
| REMEMBER |-->| CONNECT |-->| OPERATE |-->| IMPROVE |-->|  MOVE   |
+----------+   +---------+   +---------+   +---------+   +---------+
 save/query     graph         workflows     retro         bundles
 prime          evidence      artifacts     reconcile     sync/merge
```

Remember a durable fact. Connect it to the policies, tools, and outcomes around
it. Turn repeated work into a safe runbook. Curate stale knowledge instead of
letting it accumulate. Move the store without silently overwriting meaning.

## Install

> [!NOTE]
> This `main` branch documents source version `0.9.0`. The `latest` installer
> follows the newest published GitHub release and can temporarily lag behind
> `main`. Run `mem --version` and use documentation from the matching Git tag
> when exact released behavior matters.

### mise

If you already use [mise](https://mise.jdx.dev/), install the latest published
release globally and pin the resolved version:

```bash
mise use --global --pin github:NeoHsu/mnemark@latest
mem --version
```

### Verified installer for macOS and Linux

Download the installer and its checksum before execution:

```bash
base=https://github.com/NeoHsu/mnemark/releases/latest/download
curl --proto '=https' --tlsv1.2 -LsSfO "$base/mnemark-installer.sh"
curl --proto '=https' --tlsv1.2 -LsSfO "$base/mnemark-installer.sh.sha256"
if command -v sha256sum >/dev/null 2>&1; then
  sha256sum -c mnemark-installer.sh.sha256
else
  shasum -a 256 -c mnemark-installer.sh.sha256
fi
sh mnemark-installer.sh
mem --version
```

### Windows PowerShell

```powershell
$base = "https://github.com/NeoHsu/mnemark/releases/latest/download"
Invoke-WebRequest "$base/mnemark-installer.ps1" -OutFile mnemark-installer.ps1
Invoke-WebRequest "$base/mnemark-installer.ps1.sha256" -OutFile mnemark-installer.ps1.sha256
$expected = ((Get-Content -Raw mnemark-installer.ps1.sha256).Trim() -split '\s+')[0]
$actual = (Get-FileHash -Algorithm SHA256 mnemark-installer.ps1).Hash.ToLowerInvariant()
if ($actual -ne $expected) { throw "installer checksum verification failed" }
& .\mnemark-installer.ps1
mem --version
```

### Direct release downloads

| Platform | Asset |
| --- | --- |
| Apple Silicon macOS | `mnemark-aarch64-apple-darwin.tar.xz` |
| Intel macOS | `mnemark-x86_64-apple-darwin.tar.xz` |
| ARM64 Linux | `mnemark-aarch64-unknown-linux-gnu.tar.xz` |
| x64 Linux | `mnemark-x86_64-unknown-linux-gnu.tar.xz` |
| x64 Windows | `mnemark-x86_64-pc-windows-msvc.zip` |

Checksums, the SBOM, and provenance attestations are published with each
release. After downloading an archive, verify its GitHub attestation when the
GitHub CLI is available:

```bash
archive=mnemark-aarch64-apple-darwin.tar.xz
gh attestation verify "$archive" --repo NeoHsu/mnemark
```

### Initialize and wire one agent

For a new store, inspect the resolved target and initialize only after the path
is the one you intend to use:

```bash
mem config show
mem init
```

Then preview and install one user-level agent adapter. Replace `pi` with
`claude-code`, `codex`, `gemini-cli`, or `opencode` when appropriate:

```bash
mem setup list
mem setup pi --dry-run
mem setup pi
mem doctor --platform pi
```

Repeat setup only for agents you actually use. The store remains shared; agent
setup installs policy, skill, and supported hook layers without creating a
separate per-agent memory database.

## Product highlights

### Recall the whole decision, not just matching text

Keyword search answers **“Which memories mention this task?”** Relationship-aware
recall answers **“What else is connected to those memories, and why?”**

In an isolated release example, a lexical query for `external side effect`
returns only `release_policy`. Graph expansion follows extracted relationships
and also recovers the connected runbook:

```text
Graph context for external side effect:
- memory:release_policy [memory] score=1.000
- tag:policy:release [tag] score=0.560
- memory:release_runbook [memory] score=0.216
edges:
- memory:release_policy --has_tag [EXTRACTED]--> tag:policy:release
- memory:release_runbook --has_tag [EXTRACTED]--> tag:policy:release
```

Use focused priming for the normal task-start experience. Inspect graph
commands only when the relationship itself matters:

```bash
mem prime --focus "release 0.9.1"
mem graph query "release safety" --scope auto --format compact
mem graph explain release_runbook --scope auto
mem graph path release_policy release_runbook --format compact
```

The graph is local and evidence-bearing. Deterministic links come from stored
tags, paths, workflow steps, artifacts, supersession, and run records; semantic
links must pass confidence, trust, scope, and review gates. No vector database
or hidden provider call is required, and graph context never becomes
instruction authority.

### Turn recurring work into a safe runbook

Procedures should not remain buried in old chats. Workflow memories turn them
into validated runbooks that agents can find, render, and record without giving
the CLI permission to execute them.

```bash
mem workflow new release_runbook
# Replace placeholders, set draft: false, then validate before saving.
mem workflow validate --file release_runbook.yaml
mem save \
  --type workflow --name release_runbook --scope global \
  --source manual --user-confirmed \
  --tags '["workflow:release","intent:release","risk:high"]' \
  --content-file release_runbook.yaml
mem workflow find release --scope auto
mem workflow show release_runbook --checklist
mem workflow record release_runbook --result success --note "clean run"
```

The default scaffold is deliberately invalid until authored. The rendered
checklist orders preflight, checks, approvals, actions, and verification; risky
steps stop for explicit confirmation, and success and failure are recorded
separately.

Project-specific helpers stay in repository `scripts/`. Cross-project helpers
can travel with the private store as artifacts. Validation checks path
containment, regular files, checksums, and executable bits without running the
scripts:

```bash
mem workflow validate release_runbook --check-artifacts --repo .
mem artifact check
```

### Keep memory useful instead of letting it pile up

Durable memory eventually becomes stale, duplicated, or contradictory.
`mem retro` emits a bounded, read-only review bundle so an agent and user can
decide what to keep, correct, retire, or promote.

```bash
mem retro daily
mem retro weekly --limit 200
mem reconcile --scope auto --repo .
```

Daily review can compare available platform conversation context with recent
memory evidence. Weekly review surfaces duplicate or stale memories, repeated
workflow failures, unresolved ambiguities, scope budgets, and pending graph
relationships. `reconcile` checks remembered paths and commands against current
external reality without executing them.

These commands do not schedule themselves or mutate memories. Run them
explicitly, ask a configured agent, or invoke them from an external harness.
`mnemark` has no built-in scheduler and does not read platform logs.

### Move memory without losing meaning

A store should be portable without turning conflicts into silent data loss.
Bundles use an online SQLite snapshot and verify every durable file before
import. Private Git sync moves bytes; mnemark merges scoped memories, workflow
runs, changelog events, and relationship assertions by durable identity.
Same-name content conflicts become pending ambiguities instead of overwrites.

```bash
mem bundle export mnemark-store.tgz
mem bundle inspect mnemark-store.tgz
mem sync --dry-run
mem sync
mem ambiguity list --pending
```

A normal `mem sync` creates a local checkpoint and fetches only when a remote
exists. It never pushes unless `--push` is explicit. Stores and bundles remain
plaintext, so use private storage, trusted transfer channels, and disk or volume
encryption where confidentiality matters.

## Evidence, not promises

The repository retains reproducible benchmark reports and runs correctness,
recovery, security, and supply-chain gates before a release is qualified.

| Evidence | Current boundary |
| --- | --- |
| Supported capacity | Up to **10,000 memories** per active store |
| Published 10,000-memory baseline | v0.8 query **44.96 ms p50** and prime **54.77 ms p50** on the documented Apple M2 Max run |
| Recovery | Release gate exports, corrupts, rejects, restores, and verifies an isolated bundle |
| Retrieval quality | Versioned lexical, fuzzy, multilingual, trust/scope, plain-prime, and graph-context cases gate the release binary |
| Agent behavior | A versioned cross-agent trace protocol checks routing, target preflight, approval order, and fail-closed decisions; a retained live matrix is still pending |
| Capacity canary | **100,000 memories** passed retained correctness checks; it is not a support claim or SLA |
| Release targets | macOS arm64/x86_64, Linux arm64/x86_64, and Windows x86_64 |
| Supply chain | Archive checksums, checksum-verifying installers, CycloneDX 1.5 SBOM, and GitHub build-provenance attestations |

Benchmark numbers are machine- and protocol-specific, not cross-machine
latency guarantees. Synthetic retrieval scores prevent known behavior from
regressing but do not prove usefulness for every real task, and synthetic agent
traces are evaluator self-tests rather than live-agent evidence. Read
[Evaluation](docs/evaluation.md) and the complete performance methodology in
[Performance](docs/performance.md).

## From memory to context

```text
WRITE PATH

+----------------------------+
| mem save / update / import |
+-------------+--------------+
              |
              v
+----------------------------+
| validation + secret gates  |
+-------------+--------------+
              |
              v
+----------------------------+
| SQLite transaction         |
+-------------+--------------+
              |
              +---> changelog + workflow runs       [durable]
              +---> Tantivy lexical index           [rebuildable]
              +---> deterministic graph views       [rebuildable]

READ + PORTABILITY

+-------------------+     +---------------------------------+
| mem query         | --> | ranked matching memories        |
+-------------------+     +---------------------------------+
| mem prime         | --> | bounded session context         |
+-------------------+     +---------------------------------+
| mem prime --focus | --> | context + connected evidence    |
+-------------------+     +---------------------------------+
| mem retro         | --> | reviewable quality bundle       |
+-------------------+     +---------------------------------+
| mem bundle export | --> | portable online snapshot        |
+-------------------+     +---------------------------------+
```

The architecture keeps responsibilities explicit:

- **SQLite is durable truth.** Search indexes and materialized graph tables can
  be rebuilt.
- **Reads do not initialize or migrate.** Store creation and migration are
  deliberate operator actions.
- **Search is deterministic and multilingual.** Tantivy retrieves candidates;
  SQLite filters and trust-aware reranking keep the result explainable.
- **Relationship context is local and evidence-bearing.** Focused recall starts
  from lexical matches, traverses bounded graph relationships, and retains
  confidence and provenance without a vector database or hidden provider call.
- **Memory quality is deliberate.** Daily and weekly retrospectives emit
  read-only review bundles; the agent and user decide what to keep, correct,
  retire, or promote.
- **Workflow content is procedure data.** `mem` scaffolds, validates, finds,
  renders, and records runbooks; the calling agent executes approved steps.
- **Portability is inspectable.** Bundles use online SQLite snapshots,
  allowlisted files, and per-file SHA-256 integrity checks.

See [Overview](docs/overview.md) for the full session, write, query, workflow,
graph, and sync diagrams.

## The everyday loop

Once mnemark is wired into an agent, the normal flow stays small:

```text
session start
     |
     v
 mem prime  --->  do the task  --->  save durable learning
                       |                       |
                       v                       v
             query or load a runbook     mem sync --dry-run
```

- Start with `mem prime`; use `--focus` when relationship context will help.
- Query only when the primed context is not enough.
- Find and render a workflow when the task is recurring.
- Save confirmed corrections, decisions, and reusable lessons at the end of the
  work unit.
- Preview synchronization before creating a local checkpoint, and never push
  without explicit approval.

The CLI handles deterministic storage, validation, retrieval, and versioning.
The agent keeps the judgment and the user keeps authority.

## Agent Skill

The CLI stores and retrieves memory. The bundled skill teaches supported coding
agents when to prime, query, save, run daily or weekly memory curation, validate
workflows, reconcile stale claims, and stop for safety approval.

Install the skill from the same release as the CLI:

```bash
npx skills add https://github.com/NeoHsu/mnemark/tree/v0.9.0 --skill mnemark
```

For a local checkout during development:

```bash
npx skills add ./skills/mnemark
```

The [installation flow](#initialize-and-wire-one-agent) shows how
`mem setup <platform>` previews, installs, and verifies the supported
user-level policy, skill, and hook layers.

The CLI and skill use exact release lockstep. Automation can fail closed before
store discovery with:

```bash
mem --json-errors contract --skill-version 0.9.0
```

## Stable machine contracts

Agents and automation do not need to scrape help text or guess side effects:

```bash
mem contract
mem schema list
mem operation list
mem operation inspect -- query "release notes"
```

- `mem contract` reports CLI, store, workflow, graph, bundle, and schema
  versions without reading or creating a store.
- `mem schema list|print` exposes the bundled JSON Schemas.
- `mem operation list|inspect` exposes stable leaf operation IDs and classified
  store, filesystem, and network effects.
- `--json-errors` emits versioned typed error envelopes while successful output
  remains unchanged.

See [Compatibility](docs/compatibility.md) and
[JSON Contracts](docs/json-schemas.md) for the public stability policy.

## Production and security boundary

The qualified production profile is intentionally narrow:

- one operating-system user per active private store;
- up to 10,000 memories per supported store;
- private, access-controlled storage and trusted Git/bundle channels;
- explicit backup-first migration and no automatic downgrade;
- full-disk or private-volume encryption when confidentiality matters.

> [!IMPORTANT]
> Stores and bundles are plaintext. Bundle hashes detect corruption but do not
> authenticate the publisher. Secret scanning is defense in depth, not a DLP
> system. Windows builds are tested, but user-only ACLs must currently be
> verified with platform tooling.

Run `mem doctor` after installation, migration, or machine changes. Before
publishing or deploying a release, run the clean-worktree qualification gate
and retain its recovery and benchmark evidence:

```bash
scripts/check-release-readiness.sh
```

Read [Production Operations](docs/production.md) for backup, restore, upgrade,
rollback, and incident procedures, and [Security](SECURITY.md) for the full
threat model and reporting path.

## Documentation map

| Goal | Start here |
| --- | --- |
| Understand the model | [Overview](docs/overview.md) and [Runtime Model](docs/runtime-model.md) |
| Install and save the first memory | [Getting Started](docs/getting-started.md) |
| Use workflows and portable artifacts | [Product highlights](#turn-recurring-work-into-a-safe-runbook) and [Workflows](docs/workflows.md) |
| Understand relationship-aware recall | [Product highlights](#recall-the-whole-decision-not-just-matching-text) and [Graph Memory](docs/graph-memory.md) |
| Curate memory quality | [Product highlights](#keep-memory-useful-instead-of-letting-it-pile-up), [Daily Retro](skills/mnemark/references/daily-retro.md), and [Weekly Retro](skills/mnemark/references/weekly-retro.md) |
| Back up, transfer, merge, or sync | [Product highlights](#move-memory-without-losing-meaning), [Workflows](docs/workflows.md), and [Runtime Model](docs/runtime-model.md) |
| Operate a real store | [Production Operations](docs/production.md) and [Security](SECURITY.md) |
| Integrate automation | [Compatibility](docs/compatibility.md), [JSON Contracts](docs/json-schemas.md), and the [CLI Guide](skills/mnemark/references/cli-guide.md) |
| Evaluate retrieval or agent policy | [Retrieval and Agent Behavior Evaluation](docs/evaluation.md) |
| Change the repository | [Agent Reference](docs/agent-reference.md), [Development](docs/development.md), and [Architecture Decisions](docs/adr/README.md) |

The complete task-oriented index lives in the
[Documentation Hub](docs/README.md).

## Development

Agents changing this repository must read
[`docs/agent-reference.md`](docs/agent-reference.md) first. `AGENTS.md` is only
a platform entrypoint; the agent reference is canonical.

```bash
mise install
cargo fmt --all -- --check
env -u CC -u CXX cargo clippy --workspace --locked --all-targets -- -D warnings
env -u CC -u CXX cargo test --workspace --locked
cargo audit --deny warnings
cargo deny check
python3 scripts/check-dependency-policy.py
```

For source-only CLI runs:

```bash
cargo run -p mnemark --bin mem -- --help
```

For release qualification:

```bash
scripts/check-release-readiness.sh
```

See [Development](docs/development.md) for MSRV, coverage, release smoke,
workflow lint, and benchmark commands. Contributions are covered by
[CONTRIBUTING.md](CONTRIBUTING.md); notable changes are tracked in
[CHANGELOG.md](CHANGELOG.md).
