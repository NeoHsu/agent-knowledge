# mnemark documentation

Use this index when the project homepage is not specific enough. Runnable
`mem` examples across repository Markdown are parsed against the real Clap
surface during tests.

## Sources and freshness

| Document class | Source of truth | Invalidated by | Verification |
| --- | --- | --- | --- |
| CLI examples and command reference | Clap args and command-effect classifier | Command, flag, default, conflict, or effect change | `mise run contract:check` |
| Versioned install and skill docs | Workspace version and compatibility manifest | Version or intended-tag change | `python3 scripts/check-skill-version.py` |
| Runtime, architecture, and agent guidance | Rust modules plus repository policy | Store, domain, setup, module, or safety-boundary change | Related Rust tests plus `cargo test -p mnemark --test doc_drift` |
| JSON contracts | `docs/schemas/*.schema.json` embedded by the CLI | Machine-output shape change | Schema discovery and fixture tests in `mise run contract:check` |
| Development and CI guidance | `mise.toml` and `.github/workflows/` | Task or workflow change | `mise run check:pr` and `scripts/check-workflows.sh` |
| Evaluation and performance evidence | Versioned fixtures and retained reports | Protocol, fixture, binary, or evidence-status change | Evaluator/checker command named in the document |
| Changelog and ADRs | Git/release history and accepted decisions | Release preparation or superseding architecture decision | Release metadata gate or new superseding ADR |

A clean checker proves form and selected contracts, not semantic completeness.
Changing a source listed above requires reviewing its routed documents in the
same change.

## Start here

| Guide | What it covers |
| --- | --- |
| [Getting Started](getting-started.md) | Verified installation, store initialization, first save/query, and agent setup |
| [Overview](overview.md) | System map, session lifecycle, save/query flow, workflows, and sync |
| [Compatibility](compatibility.md) | Supported platforms and stability rules for every public format |
| [Evaluation](evaluation.md) | Deterministic retrieval gate and captured cross-agent behavior traces |

## Operate memory safely

| Guide | What it covers |
| --- | --- |
| [Runtime Model](runtime-model.md) | Store discovery, config priority, command effects, and portable layout |
| [Workflows](workflows.md) | Workflow memories, artifacts, bundles, import/export, merge, and retrospectives |
| [Graph Memory](graph-memory.md) | Deterministic graph projection, semantic review, and non-RAG stance |
| [Security Policy](../SECURITY.md) | Threat model, controls, limitations, and private reporting |

## Automate against stable contracts

| Guide | What it covers |
| --- | --- |
| [JSON Contracts](json-schemas.md) | Bundled schemas, fixtures, error envelopes, and discovery commands |
| [CLI Surface](cli-surface.txt) | Generated command, positional, flag, default, and conflict snapshot |
| [Skill CLI Guide](../skills/mnemark/references/cli-guide.md) | Complete current command reference for agents |

## Maintain and release

| Guide | What it covers |
| --- | --- |
| [Agent Reference](agent-reference.md) | Canonical repository instructions and task-specific reading map |
| [Development](development.md) | Local checks, coverage, release smoke, and benchmark protocol |
| [Architecture](architecture.md) | Crate direction, write-domain, effects, atomic files, output, and test boundaries |
| [Production Operations](production.md) | Release qualification, recovery, upgrade, rollback, and incidents |
| [Performance](performance.md) | Retained baselines, comparison protocol, and capacity canary |
| [Architecture Decisions](adr/README.md) | Context and consequences for load-bearing design choices |
| [Contributing](../CONTRIBUTING.md) | Pull-request checks and public-contract responsibilities |
| [Changelog](../CHANGELOG.md) | User-visible changes by release |
