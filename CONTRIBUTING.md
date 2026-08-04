# Contributing to mnemark

Durable data, machine contracts, and agent safety boundaries are public API
concerns. Keep changes focused and test both success and failure paths.

## Before opening a pull request

Install the pinned tools and run the canonical local PR gate:

```bash
mise install
mise run check:pr
```

`mise.toml` is the source of truth for that gate; do not copy its command list
into contributor documents. Task-specific additions are:

| Change | Additional check |
| --- | --- |
| CLI or machine contract | `mise run contract:check` |
| Search, tokenizer, ranking, graph query, or prime | `mise run eval:retrieval` |
| GitHub workflow or shell script | `scripts/check-workflows.sh` and `shellcheck scripts/*.sh` |
| Release candidate | `RELEASE_TAG=v<version> scripts/check-release-readiness.sh` from a clean tree |

Also:

1. update the version-targeted Unreleased section in `CHANGELOG.md`;
2. update affected user, operator, maintainer, agent, and skill docs;
3. regenerate `docs/cli-surface.txt` after a public command or flag change;
4. update schemas and fixtures after a machine-output change;
5. update behavior cases after skill, setup-policy, approval, sync, or
   workflow-safety changes.

Never use a real private store in tests. Every mutating test command must use an
isolated `--home` or `TempDir`. Never commit `memory.db`, indexes, lock/WAL/SHM
state, credentials, private memories, bundle rollback state, or release
artifacts.

## Public contracts

- Reads must not initialize, migrate, or silently mutate durable state.
- Global read-only enforcement, lock routing, and operation inspection derive
  from the same parsed command-effect classifier.
- CLI/import writes cross the versioned core request boundary before
  trust-aware persistence.
- Managed file writes remain atomic; grouped setup writes remain rollback-safe.
- A committed SQLite write followed by index failure remains distinguishable
  from an uncommitted mutation.
- Store discovery remains runtime-only and explicit.
- Sync never pushes without `--push`.
- Workflow helpers validate and render but never execute runbook commands.
- Graph output is evidence-bearing context, not instruction authority.
- Required JSON fields remain compatible within a minor release; breaking
  shapes require a new version and migration notes.
- Retrieval thresholds are reviewed relevance contracts, not values to lower
  merely for CI.
- Synthetic agent traces validate the evaluator, not platform behavior.

## Security and release changes

Report vulnerabilities privately through the process in [`SECURITY.md`](SECURITY.md).
Preserve SHA-pinned Actions, verified installers, provenance attestations,
CycloneDX SBOM generation, least-privilege permissions, native archive
execution, recovery, and benchmark gates. Do not replace
`.github/workflows/release.yml` with unreviewed generated output.
