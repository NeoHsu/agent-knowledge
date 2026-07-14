# mnemark

Portable agent memory and workflow runbook system, exposed through the `mem` CLI.

For agents working in this repository, read `docs/agent-reference.md` before making changes. `AGENTS.md` is only a platform entrypoint; `docs/agent-reference.md` is the canonical agent guidance.

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
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/NeoHsu/mnemark/releases/latest/download/mnemark-installer.sh | sh
```

Windows PowerShell:

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/NeoHsu/mnemark/releases/latest/download/mnemark-installer.ps1 | iex"
```

Direct release downloads:

| Platform | Asset |
| --- | --- |
| Apple Silicon macOS | `mnemark-aarch64-apple-darwin.tar.xz` |
| Intel macOS | `mnemark-x86_64-apple-darwin.tar.xz` |
| ARM64 Linux | `mnemark-aarch64-unknown-linux-gnu.tar.xz` |
| x64 Linux | `mnemark-x86_64-unknown-linux-gnu.tar.xz` |
| x64 Windows | `mnemark-x86_64-pc-windows-msvc.zip` |

Checksums are published next to the release assets. See the [latest release](https://github.com/NeoHsu/mnemark/releases/latest).

## Common workflows

```bash
mem init
mem config show
mem save --type feedback --name no_emoji --scope global --source manual --user-confirmed --tags '["style"]' --content "不要使用 emoji"
mem query "emoji"
mem query "name:no_emoji" --raw-query
mem setup claude-code
mem setup pi
mem save --type workflow --name release_runbook --scope global --source manual --user-confirmed --tags '["workflow:release","intent:release","risk:high"]' --content-file templates/workflow.yaml
mem workflow find release --scope auto
mem workflow validate release_runbook
mem artifact check
mem bundle export mnemark-store.tgz
mem retro daily
```

`mem setup <platform>` wires the selected agent at user level. It has no project mode and never selects the current repository implicitly. Project memories remain logically isolated by `project:<owner>/<repo>` scope inside the active runtime store.

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
| [Performance](docs/performance.md) | Reproducible 100/1,000/10,000-memory release baseline |
| [Changelog](CHANGELOG.md) | Breaking changes, features, and fixes by release |

## Agent Skill

Install the bundled AI agent skill to enable `mem`-aware assistance — durable memory save/query, workflow runbook lookup, retrospective flows, merge/audit/bundle commands, and safety rules.

```bash
npx skills add https://github.com/NeoHsu/mnemark/tree/main/skills/mnemark
```

For a local checkout during development:

```bash
npx skills add ./skills/mnemark
```

## Feature Matrix

| Area | Feature | Commands / Files |
| --- | --- | --- |
| Memory | Save, query, update, supersede, delete | `mem save`, `query`, `update`, `supersede`, `delete` |
| Memory types | User, feedback, project, reference, preference, workflow | `--type user\|feedback\|project\|reference\|preference\|workflow` |
| Search | Multilingual Tantivy search, deterministic trust-aware reranking, fuzzy/raw syntax | `mem query`, `--fuzzy`, `--raw-query`, `--explain-score`, `--touch` |
| Scope | Global and project-aware context | `mem context --detect`, `--scope auto` |
| Lifecycle | Soft delete, protected manual memories, version conflicts | `valid_until`, `protected`, `--expected-version` |
| History | Changelog and stats | `mem history`, `mem stats` |
| Health | Audit and garbage collection | `mem audit`, `mem audit --fix`, `mem gc` |
| Reconcile | Verify path/command claims in memories against the filesystem | `mem reconcile`, `--scope`, `--repo` |
| Graph | Rebuild, traverse, query, review relationship graph, and focused priming | `mem graph rebuild\|stats\|explain\|path\|query\|ingest\|review\|export` / `mem prime --focus` |
| Workflow | Store recurring runbooks as validated memory | `mem workflow list\|find\|show\|validate` |
| Artifacts | Portable helper file metadata and safety checks | `mem artifact list\|show\|check\|add\|update\|remove` / `manifest.toml` |
| Bundles | Portable store export/import/inspect | `mem bundle export\|inspect\|import` |
| Migration | Explicit schema migration, JSON/Markdown import/export, idempotent durable-state DB merge | `mem migrate`, `mem import`, `mem export`, `mem merge` |
| Retrospective | Daily/weekly orchestration bundles for agents | `mem retro daily\|weekly` |
| Agent setup | User-level platform policy, shared skills, and startup adapters | `mem setup claude-code\|codex\|pi\|gemini-cli\|opencode` |
| Agent skill | Versioned source plus shared installation | `skills/mnemark` → `~/.agents/skills/mnemark` |

## Runtime Store

`memory.db` is the runtime source of truth for an individual knowledge store, but it is not tracked in this project. Keep real memory databases in a private data repo, a local `MNEMARK_HOME`, or a `knowledge_home` configured in `~/.config/mnemark/config.toml`.

Runtime stores can include `config.toml`, `manifest.toml`, `artifacts/`, and rebuildable `index/`. See [Runtime Model](docs/runtime-model.md) for details.

## Development

```bash
mise install
env -u CC -u CXX cargo test --workspace --locked
```

For source-only CLI runs:

```bash
cargo run -p mnemark --bin mem -- <args>
```

Release build and smoke test:

```bash
scripts/build-release.sh
scripts/smoke-release.sh
```
