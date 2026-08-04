# Development

Read [`agent-reference.md`](agent-reference.md) before changing this repository.
It owns safety and task-routing instructions; this guide owns tools, CI topology,
and implementation notes.

## Setup and source runs

`mise.toml` pins Rust, Python, linters, security tools, and Cargo utilities.
Native SQLite, compression, and TLS dependencies use Xcode Command Line Tools on
macOS, Clang or GCC on Linux, and MSVC Build Tools on Windows. Zig is not a
repository prerequisite.

```bash
mise install
cargo run -p mnemark --bin mem -- --help
```

Do not use a real private store. A mutating source run must pass an isolated
`--home` path.

## Validation tasks

Run the canonical pull-request gate:

```bash
mise run check:pr
```

Use `mise tasks` to inspect its exact current commands. Focused entry points are:

| Task | Purpose |
| --- | --- |
| `mise run check:fast` | Formatting, Clippy, and focused contract tests |
| `mise run contract:check` | CLI surface, schemas, effects, docs, and skill lockstep |
| `mise run test:nextest` | Workspace tests plus doctests |
| `mise run coverage` | LCOV generation and the 86% line floor |
| `mise run eval:retrieval` | Release build plus deterministic retrieval fixture |
| `mise run eval:agent-reference` | Synthetic agent-trace checker self-test |
| `mise run size:check` | 50 MiB release binary budget |
| `mise run production-check` | Complete clean-tree release gate |

Workflow or shell changes additionally run:

```bash
scripts/check-workflows.sh
shellcheck scripts/*.sh
```

The source-hygiene gate rejects runtime databases, indexes, lock/WAL/SHM files,
and bundle-replacement state without reading database content. Dirty-tree
release overrides do not bypass it.

## CI topology

The source-controlled workflows are configured to run:

- stable Linux checks with sccache and nextest;
- Rust 1.97.1 MSRV verification;
- Rust 1.97.1 Linux/macOS/Windows native release verification;
- an 86% line-coverage floor with retained LCOV;
- release build, binary-size, smoke, recovery, retrieval, and bounded benchmark
  checks;
- Git history/current-source secret scans, Python lint, dependency policy,
  unused-dependency checks, and workflow security audit;
- cargo-dist PR upload jobs that build real archives/installers and execute the
  host-compatible archived binary without publishing.

Workflow configuration is not proof that an unpushed commit passed. Evidence
must identify the exact commit and artifact run.

## Release build and qualification

```bash
scripts/build-release.sh
python3 scripts/check-binary-size.py
scripts/smoke-release.sh
```

`build-release.sh` ignores only inherited `CC="zig cc"`, `CXX="zig c++"`, or
`AR="zig ar"` on native macOS. It verifies the pinned CC-CEDICT input after
Cargo completes. The recovery smoke validates bundle restore, durable
memory/workflow/artifact/graph state, corruption rejection, migration preview,
and a network-free local sync checkpoint.

A release candidate also requires a unique version, a dated changelog heading,
a clean tree, and the complete gate:

```bash
RELEASE_TAG=v<version> scripts/check-release-readiness.sh
```

`ALLOW_DIRTY=1`, `REQUIRE_AUX_TOOLS=0`, and `RUN_BENCHMARK=0` are bounded
development options and cannot qualify a release. Tagging, pushing, and
publishing require explicit approval. See
[Production Operations](production.md) for deployment and rollback evidence.

## Benchmarks and evaluation

The scale benchmark uses isolated stores and records binary, protocol, commit,
platform, sample schedule, correctness, latency, RSS, and artifact sizes:

```bash
scripts/benchmark-scale.sh
```

Use `SCALES="100 1000"` only for bounded development. Published performance
claims require the full protocol and retained JSON described in
[Performance](performance.md).

Retrieval-affecting changes run:

```bash
mise run eval:retrieval
```

Agent-policy changes update the versioned action-trace cases and run:

```bash
mise run eval:agent-reference
```

Synthetic traces validate the evaluator only. Live evidence must identify the
platform, model, adapter, CLI, skill, fixture hash, and actual tool calls, then
pass `scripts/evaluate-agent-behavior.py --require-live`. See
[Evaluation](evaluation.md).

## Implementation notes

### Read-only query default

Ordinary query is no-touch and lock-free. `--touch` writes access telemetry;
`--repair-index` explicitly permits rebuildable index repair. The hidden
`--no-touch` flag is only a compatibility no-op.

### YAML dependency

The workspace aliases the maintained `serde_yaml_ng` fork as `serde_yaml` in
`Cargo.toml`.

### Index state and schema version

Index stale state lives only in SQLite metadata under `index_dirty`; there is no
`.stale` file. Tantivy artifacts carry `index/.mnemark-index-version`, owned by
`INDEX_SCHEMA_VERSION` in `crates/mem-core/src/search_index.rs`.

Bump it when indexed fields, field options, tokenizer behavior, normalization,
indexed content, or required ranking/filtering fields change. Do not bump it for
query-time boosts, fuzzy construction, SQLite-only filtering, or CLI output.

### Write-domain boundary

CLI/import adapters construct versioned wire requests in
`crates/mem-core/src/memory_domain.rs`. Validation and normalization produce a
`SaveRequest`; only that type reaches trust-aware persistence and returns a
`SaveOutcome`. File input, similarity orchestration, index completion, and
output rendering remain in `mem-cli`.

### Test topology

`mem-cli` disables automatic per-file integration targets.
`tests/integration.rs` links acceptance modules through one shared `TempDir`
harness; `doc_drift` remains an explicit separate target. Filter focused runs,
for example:

```bash
cargo test -p mnemark --test integration setup::
```

### Bulk operations

JSON import and DB merge feed changed records to one Tantivy writer in bounded
batches instead of committing once per row.

### Workflow modules

`crates/mem-core/src/workflow.rs` is the façade for schema validation and intent
ranking. Keep artifact-reference checks in `workflow/artifacts.rs` and
fail-closed rendering in `workflow/checklist.rs`. Core never executes workflow
commands.

### Graph modules

`crates/mem-core/src/graph.rs` is the façade:

- `model.rs` — public graph types;
- `ids.rs` — stable node identifiers;
- `store.rs` — shared SQLite rows and metadata;
- `query/` — resolution, traversal, path, export, and candidates;
- `materialize/` — deterministic rebuild and extraction;
- `health.rs` — audit and health reports;
- `semantic.rs` and `semantic/` — durable validation, ingest, merge, projection,
  and review.

Shared primitives belong in `ids` or `store`; semantic code must not depend back
on deterministic materialization. Add behavior to the narrowest module instead
of growing a façade.
