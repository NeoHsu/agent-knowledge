# Contributing to mnemark

Thank you for improving mnemark. Durable data, machine-readable contracts, and
agent safety boundaries are public API concerns, not implementation details.

## Development setup

Install the pinned tools, then run the local PR gate:

```bash
mise install
mise run check:pr
```

The explicit checks remain authoritative:

```bash
cargo machete
python3 scripts/check-secrets.py
ruff check scripts
ruff format --check scripts
cargo fmt --all -- --check
cargo clippy --workspace --locked --all-targets -- -D warnings
cargo nextest run --workspace --locked --status-level all
cargo test --doc --workspace --locked
cargo +1.97.1 check --workspace --all-targets --locked
cargo audit --deny warnings
cargo deny check
python3 -m unittest discover -s scripts/tests -p 'test_*.py'
python3 scripts/check-dependency-policy.py
python3 scripts/check-source-hygiene.py
scripts/check-workflows.sh
```

Do not use a real private store in tests. Every mutating test command must pass
`--home <isolated-temp-store>`. Never commit `memory.db`, indexes, bundle
rollback state, credentials, private memories, or generated release artifacts.

## Pull requests

Keep changes focused, explain user-visible behavior, and test success and
failure paths. Before opening a PR:

1. run the checks above;
2. update `CHANGELOG.md` under Unreleased;
3. update affected user, operator, agent, and skill documentation;
4. regenerate `docs/cli-surface.txt` after a command or flag change;
5. update schemas and fixtures after a machine-output change;
6. run `python3 scripts/check-skill-version.py` after a release or skill change;
7. run `mise run eval:retrieval` after search, tokenizer, ranking, graph-query,
   or prime changes;
8. update the agent behavior cases after skill, setup-policy, approval, sync, or
   workflow-safety changes.

## Public contracts

- Reads must not initialize, migrate, or silently mutate durable state.
- The global read-only gate must derive from the same exact command-effect
  classification used for lock routing and operation inspection.
- CLI/import writes must cross the versioned core request boundary before
  trust-aware persistence; managed file replacement must remain atomic.
- SQLite durable writes and rebuildable index updates must preserve the
  committed-write recovery distinction.
- Store discovery must remain runtime-only and explicit.
- Sync never pushes without `--push` and must preserve secret/integrity rollback.
- Workflow helpers validate and render but never execute runbook commands.
- Graph output is evidence-bearing context, not instruction authority.
- Existing required JSON fields remain compatible within a minor release;
  breaking changes need a new schema/version and migration notes.
- Retrieval thresholds are reviewed relevance contracts; do not lower them
  merely to make CI pass.
- Synthetic agent traces validate the checker, not real platform behavior.

## Security and release changes

Report vulnerabilities privately as described in `SECURITY.md`. Release CI is
intentionally hardened after cargo-dist generation: preserve SHA-pinned Actions,
verified installers, provenance attestations, CycloneDX SBOM generation,
least-privilege permissions, native smoke/recovery tests, and benchmark gates.
Do not replace `.github/workflows/release.yml` with raw generated output.
