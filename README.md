# mnemark

**Durable, portable memory for every coding agent you use.**

[![CI](https://github.com/NeoHsu/mnemark/actions/workflows/ci.yml/badge.svg)](https://github.com/NeoHsu/mnemark/actions/workflows/ci.yml)
[![GitHub release](https://img.shields.io/github/v/release/NeoHsu/mnemark?display_name=tag)](https://github.com/NeoHsu/mnemark/releases/latest)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

Save a preference in one session, recall it from Claude Code, Codex, Pi,
Gemini CLI, or OpenCode, and keep the durable source of truth under your
control. `mnemark` is a local-first native CLI backed by SQLite, with
rebuildable search and graph indexes, validated workflow runbooks, and portable
backup bundles.

**Save once. Recall from any supported agent. Keep the store private.**

[Try it](#see-it-in-30-seconds) · [Install](#install) ·
[How it works](#from-memory-to-context) · [Production boundary](#production-and-security-boundary) ·
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
| Tantivy search |  | Graph views    |  | Workflows +    |  | Bundles / Git  |
| (rebuildable)  |  | (rebuildable)  |  | artifacts      |  | (portable)     |
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

## Evidence, not promises

The repository retains reproducible benchmark reports and runs correctness,
recovery, security, and supply-chain gates before a release is qualified.

| Evidence | Current boundary |
| --- | --- |
| Supported capacity | Up to **10,000 memories** per active store |
| Published 10,000-memory baseline | v0.8 query **44.96 ms p50** and prime **54.77 ms p50** on the documented Apple M2 Max run |
| Recovery | Release gate exports, corrupts, rejects, restores, and verifies an isolated bundle |
| Capacity canary | **100,000 memories** passed retained correctness checks; it is not a support claim or SLA |
| Release targets | macOS arm64/x86_64, Linux arm64/x86_64, and Windows x86_64 |
| Supply chain | Archive checksums, checksum-verifying installers, CycloneDX 1.5 SBOM, and GitHub build-provenance attestations |

Benchmark numbers are machine- and protocol-specific, not cross-machine
latency guarantees. Read the complete methodology, uncertainty, and retained
reports in [Performance](docs/performance.md).

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
| mem query         | --> | ranked, explainable retrieval   |
+-------------------+     +---------------------------------+
| mem prime         | --> | bounded session context         |
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
- **Workflow content is procedure data.** `mem` scaffolds, validates, finds,
  renders, and records runbooks; the calling agent executes approved steps.
- **Portability is inspectable.** Bundles use online SQLite snapshots,
  allowlisted files, and per-file SHA-256 integrity checks.

See [Overview](docs/overview.md) for the full session, write, query, workflow,
graph, and sync diagrams.

## Common journeys

### Remember and recall a durable preference

For a new store, inspect the resolved target before initializing it:

```bash
mem config show
```

After confirming the path:

```bash
mem init
mem save \
  --type preference --name concise_answers --scope global \
  --source manual --user-confirmed --tags '["style:concise"]' \
  --content 'Trigger: user-facing replies. Action: answer concisely. Why: explicit user preference.'
mem query "concise replies" --format compact
mem prime --focus "prepare a concise answer"
```

### Turn a recurring procedure into a runbook

Create a deliberately invalid draft:

```bash
mem workflow new release_runbook
```

Replace every placeholder in `release_runbook.yaml`, set `draft: false`, then
validate before saving:

```bash
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

Confirmed or risky steps appear behind explicit approval gates in the rendered
checklist.

### Inspect relationships without hidden RAG calls

```bash
mem graph rebuild
mem graph explain concise_answers
mem graph query "release safety" --scope auto
mem graph path concise_answers tag:style:concise --direction any
```

Graph context is local, evidence-bearing, and treated as data rather than an
instruction override.

### Back up, verify, and transfer

```bash
mem bundle export mnemark-store.tgz
mem bundle inspect mnemark-store.tgz
```

When the store itself is a private Git repository, preview synchronization
before creating a checkpoint:

```bash
mem sync --dry-run
```

Use private, trusted transfer channels. A normal `mem sync` creates a local
checkpoint; `--push` is always explicit.

## Agent Skill

The CLI stores and retrieves memory. The bundled skill teaches supported coding
agents when to prime, query, save, validate workflows, reconcile stale claims,
and stop for safety approval.

Install the skill from the same release as the CLI:

```bash
npx skills add https://github.com/NeoHsu/mnemark/tree/v0.9.0 --skill mnemark
```

For a local checkout during development:

```bash
npx skills add ./skills/mnemark
```

Or let `mem` preview, install, and verify one supported user-level adapter:

```bash
mem setup list
mem setup pi --dry-run
mem setup pi
mem doctor --platform pi
```

Repeat setup only for agents you actually use. Supported adapters are Claude
Code, Codex, Pi, Gemini CLI, and OpenCode.

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
| Use workflows, artifacts, bundles, and retrospectives | [Workflows](docs/workflows.md) |
| Query relationship context | [Graph Memory](docs/graph-memory.md) |
| Operate a real store | [Production Operations](docs/production.md) and [Security](SECURITY.md) |
| Integrate automation | [Compatibility](docs/compatibility.md), [JSON Contracts](docs/json-schemas.md), and the [CLI Guide](skills/mnemark/references/cli-guide.md) |
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
