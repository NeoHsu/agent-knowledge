# mnemark

Portable agent memory and workflow runbook system, exposed through the `mem`
CLI.

> [!NOTE]
> This `main` branch documents source version `0.9.0`. The `latest` installer
> follows the newest published GitHub release and can temporarily lag behind
> `main`; run `mem --version` and use the documentation from the matching Git
> tag when exact released behavior matters.

For agents working in this repository, read
[`docs/agent-reference.md`](docs/agent-reference.md) before making changes.
`AGENTS.md` is only a platform entrypoint; `docs/agent-reference.md` is the
canonical agent guidance.

`mnemark` is a Rust single-binary CLI for durable agent memory. It stores memory in a private/local SQLite knowledge store, maintains a rebuildable Tantivy search index, validates workflow runbooks, and packages portable artifacts and bundles.

## Why mnemark?

- Keep agent memory portable across platforms instead of tying it to one vendor.
- Save and query user preferences, project context, references, and workflow runbooks from the terminal.
- Use SQLite as the runtime source of truth and Tantivy for multilingual full-text search.
- Move reusable helper files with `manifest.toml`, `artifacts/`, and first-class store bundles.
- Give agents deterministic CLI operations while leaving judgment, retrospectives, and workflow execution to the agent.

## Install

Installer script for macOS / Linux:

```bash
base=https://github.com/NeoHsu/mnemark/releases/latest/download
curl --proto '=https' --tlsv1.2 -LsSf \
  "$base/mnemark-installer.sh" |
  sh
```

Windows PowerShell:

```powershell
$base = "https://github.com/NeoHsu/mnemark/releases/latest/download"
powershell -ExecutionPolicy Bypass -c `
  "irm $base/mnemark-installer.ps1 | iex"
```

Direct release downloads:

| Platform | Asset |
| --- | --- |
| Apple Silicon macOS | `mnemark-aarch64-apple-darwin.tar.xz` |
| Intel macOS | `mnemark-x86_64-apple-darwin.tar.xz` |
| ARM64 Linux | `mnemark-aarch64-unknown-linux-gnu.tar.xz` |
| x64 Linux | `mnemark-x86_64-unknown-linux-gnu.tar.xz` |
| x64 Windows | `mnemark-x86_64-pc-windows-msvc.zip` |

Checksums are published next to the release assets. See the
[latest release](https://github.com/NeoHsu/mnemark/releases/latest), then verify
which contract you installed:

```bash
mem --version
```

## Common workflows

```bash
mem init
mem config show
mem save --type feedback --name no_emoji --scope global --source manual --user-confirmed --tags '["style"]' --content "不要使用 emoji"
mem query "emoji"
mem query "name:no_emoji" --raw-query
mem setup claude-code
mem setup pi
mem save \
  --type workflow \
  --name release_runbook \
  --scope global \
  --source manual \
  --user-confirmed \
  --tags '["workflow:release","intent:release","risk:high"]' \
  --content-file templates/workflow.yaml
mem workflow find release --scope auto
mem workflow validate release_runbook
mem artifact check
mem bundle export mnemark-store.tgz
mem retro daily
```

`mem setup <platform>` wires the selected agent at user level. It has no
project mode and never selects the current repository implicitly. Project
memories remain logically isolated by `project:<owner>/<repo>` scope inside the
active runtime store. Automation can add the global `--json-errors` flag to
receive stable JSON error envelopes on stderr without changing successful
output.

## Documentation

| File | Description |
| --- | --- |
| [Overview](docs/overview.md) | Big-picture ASCII diagrams: system map, session lifecycle, save/query flows, workflow lifecycle, sync, and all usage scenarios |
| [Getting Started](docs/getting-started.md) | Install, initialize, first save/query, and mnemark skill install |
| [Workflows](docs/workflows.md) | Workflow memories, artifacts, bundles, import/export, merge, and retrospectives |
| [Runtime Model](docs/runtime-model.md) | Store discovery, config priority, runtime files, artifact layout, and index behavior |
| [Graph Memory](docs/graph-memory.md) | Deterministic local graph index, explain/path/export commands, and non-RAG stance |
| [mnemark Skill CLI Guide](skills/mnemark/references/cli-guide.md) | Complete current `mem` CLI command reference |
| [Workflow Rules](skills/mnemark/references/workflow-rules.md) | How agents should interpret and safely execute workflow memory runbooks |
| [Agent Reference](docs/agent-reference.md) | Canonical instructions for agents changing this repo: safety rules, repo map, task routing, validation |
| [Development](docs/development.md) | Local setup, source commands, validation, release smoke tests, developer notes |
| [Production Operations](docs/production.md) | Qualified deployment profile, release gate, recovery, upgrade, rollback, and incidents |
| [Security](SECURITY.md) | Threat model, implemented controls, explicit limitations, and reporting guidance |
| [Performance](docs/performance.md) | Published release baseline, regression protocol, and capacity-canary rules |
| [Changelog](CHANGELOG.md) | Breaking changes, features, and fixes by release |

## Agent Skill

Install the bundled AI agent skill to enable `mem`-aware assistance: durable
memory save/query, workflow runbook lookup, retrospective flows,
merge/audit/bundle commands, and safety rules.

```bash
npx skills add https://github.com/NeoHsu/mnemark/tree/main/skills/mnemark
```

For a local checkout during development:

```bash
npx skills add ./skills/mnemark
```

## Feature map

### Memory and retrieval

- **Memory lifecycle:** save, query, update, supersede, expire, and delete.
  Use `mem save --type <type> --name <name> --content "<text>"`,
  `mem query "<terms>"`, `mem update <name> --content "<text>"`,
  `mem supersede <old> <new> --content "<replacement>"`, and
  `mem delete <name>`. Save/update accept RFC3339 `--expires-at`; update also
  supports `--clear-expires-at`.
- **Memory types:** `user`, `feedback`, `project`, `reference`, `preference`, and
  validated `workflow` memories.
- **Search:** multilingual Tantivy retrieval, fuzzy/raw syntax, deterministic
  trust-aware reranking, explicit telemetry, and score explanations through
  `mem query "<terms>" --fuzzy`, `--raw-query`, `--touch`, and
  `--explain-score`.
- **Scope and history:** global/project-aware selection with
  `mem context --detect` and `--scope auto`; changelog and usage summaries with
  `mem history` and `mem stats`.
- **Health and reconciliation:** `mem audit`, `mem audit --fix`, `mem gc`, and
  `mem reconcile --scope auto`.

### Graph, workflows, and artifacts

- **Graph:** rebuild, explain, traverse, query, export, curate, and review with
  `mem graph rebuild`, `mem graph stats`, `mem graph explain <reference>`,
  `mem graph path <from> <to>`, `mem graph query "<terms>"`,
  `mem graph export`, `mem graph candidates`, `mem graph ingest <file>`,
  `mem graph review`, `mem graph accept <edge-id>`, and
  `mem graph reject <edge-id>`. Focused priming uses
  `mem prime --focus "<task>"`.
- **Workflows:** scaffold, find, show, validate, and record recurring runbooks
  with `mem workflow new <name>`, `mem workflow find "<intent>"`,
  `mem workflow show <reference>`, `mem workflow validate <reference>`, and
  `mem workflow record <reference> --result success`.
- **Artifacts:** portable helper metadata and safety checks through
  `mem artifact list`, `mem artifact show <name>`, `mem artifact check`,
  `mem artifact add <path> --kind script`,
  `mem artifact update <name> --checksum`, and
  `mem artifact remove <name>`; metadata lives in `manifest.toml`.

### Portability and integrations

- **Machine contract:** `mem contract` reports the versioned JSON-error and
  persisted-format contracts without reading or initializing a store.
- **Bundles:** `mem bundle export <file>`, `mem bundle inspect <file>`, and
  `mem bundle import <file>`.
- **Migration and transfer:** `mem migrate`,
  `mem import <file> --summary-only`, `mem export`, and `mem merge <db>`.
- **Retrospectives:** `mem retro daily` and `mem retro weekly`.
- **Agent setup:** `mem setup claude-code`, `mem setup codex`, `mem setup pi`,
  `mem setup gemini-cli`, and `mem setup opencode` install user-level policy,
  shared skills, and supported startup adapters.
- **Agent skill:** versioned source under `skills/mnemark`, installed once at
  `~/.agents/skills/mnemark` and linked where needed.

## Runtime Store

`memory.db` is the runtime source of truth for an individual knowledge store,
but it is not tracked in this project. Keep real memory databases in a private
data repo, a local `MNEMARK_HOME`, or a `knowledge_home` configured in
`~/.config/mnemark/config.toml`.

Runtime stores can include `config.toml`, `manifest.toml`, `artifacts/`, and a
rebuildable `index/`. See [Runtime Model](docs/runtime-model.md) for details.

## Security at a glance

Runtime stores and bundle archives are not encrypted by mnemark. Bundle hashes
detect corruption but do not authenticate the sender. Keep stores and bundles
private, use full-disk or volume encryption when confidentiality matters, and
accept bundles only through a trusted channel. See [Security](SECURITY.md) for
the complete threat model and platform limitations.

## Development

```bash
mise install
cargo fmt --all -- --check
env -u CC -u CXX cargo clippy --workspace --locked --all-targets -- -D warnings
env -u CC -u CXX cargo test --workspace --locked
cargo audit --deny warnings
python3 scripts/check-dependency-policy.py
```

Install `cargo-audit` before the full local validation when it is not already
available. See [Development](docs/development.md) for MSRV, release smoke, shell
workflow checks, and benchmark validation.

For source-only CLI runs:

```bash
cargo run -p mnemark --bin mem -- <args>
```

Release build and smoke test:

```bash
scripts/build-release.sh
scripts/smoke-release.sh
```
