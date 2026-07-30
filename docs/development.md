# Development

For agent-specific repo guidance, read `docs/agent-reference.md` first.

## Setup

`mise.toml` pins the local Rust MSRV toolchain and Zig C toolchain used in
restricted environments.

```bash
mise install
```

If the host has no `cc`, expose a `cc` shim on `PATH` that delegates to
`zig cc` before running Cargo. Do not hard-code a harness-specific shim path in
repository scripts.

## Run from source

```bash
cargo run -p mnemark --bin mem -- <args>
```

## Validate

The full local validation mirrors the stable CI lane:

```bash
cargo fmt --all -- --check
env -u CC -u CXX cargo clippy --workspace --locked --all-targets -- -D warnings
env -u CC -u CXX cargo test --workspace --locked
cargo audit --deny warnings
cargo deny check
python3 scripts/check-dependency-policy.py
python3 scripts/check-skill-version.py
python3 scripts/check-source-hygiene.py
```

The source-hygiene check rejects runtime `memory.db`, `index/`, lock/WAL/SHM,
and bundle-replacement backup state from this checkout without reading database
contents. Move required data to the intended private runtime store before
continuing; dirty-tree development overrides do not bypass this safety gate.

Install `cargo-audit` and `cargo-deny` locally when unavailable. Use
`env -u CC -u CXX` on macOS when inherited Zig compiler variables break native
dependency builds. `cargo deny check` enforces the accepted license, source,
wildcard, and advisory policy; the repository dependency script independently
requires every locked third-party package to come from crates.io and carry
license metadata. RustSec remains the vulnerability authority.

When changing workflows or shell scripts, also run:

```bash
actionlint .github/workflows/*.yml
shellcheck scripts/*.sh
```

The complete release-readiness gate combines those checks with metadata,
clean-tree, release-binary, recovery, and bounded benchmark validation:

```bash
scripts/check-release-readiness.sh
# development-only exercise of a dirty checkout
ALLOW_DIRTY=1 scripts/check-release-readiness.sh
```

Leave `ALLOW_DIRTY` unset when qualifying a release. The gate requires
`actionlint` and `shellcheck` by default; `REQUIRE_AUX_TOOLS=0` may skip a
missing auxiliary tool for bounded development only and cannot provide complete
release evidence.

CI tests both stable Rust and the declared Rust 1.97 MSRV. Pull requests use the
stable Linux lane in `ci.yml` plus the Rust 1.97 Linux/macOS/Windows release
verification matrix; direct `main` pushes run stable and MSRV lanes in `ci.yml`.
This avoids duplicating the same Ubuntu MSRV test on every pull request. The
stable lane also builds and smoke-tests the release binary and runs a bounded
benchmark correctness smoke. A separate Rust 1.97 coverage lane retains LCOV
and enforces an 84% line floor, set below the measured 84.36% baseline from
2026-07-29 so the gate prevents regression without claiming complete coverage.
Run it locally with `mise run coverage`.

## Release build and smoke test

```bash
scripts/build-release.sh
scripts/smoke-release.sh       # includes the isolated recovery drill
scripts/recovery-drill.sh      # may also be run independently
```

`build-release.sh` ignores only the known inherited `CC="zig cc"` /
`CXX="zig c++"` override on native macOS, where cc-rs receives an incompatible
architecture spelling; other explicitly selected toolchains are preserved.
After Cargo completes, `build-release.sh` verifies the exact upstream
CC-CEDICT source archive against the repository-pinned SHA-256 before accepting
the binary. This supplements Lindera's own download check and prevents a
release from silently embedding different dictionary input. The recovery drill
verifies a clean bundle restore, durable memory/workflow/artifact/graph state,
corruption rejection, migration preview, and local sync checkpoint without
network access.

Tag releases are gated by formatting, Clippy, tests, dependency audit, a
release-binary smoke/recovery test, and bounded benchmark guardrails on Linux.
The same test and smoke/recovery flow also runs on native macOS and Windows
runners. Published platform and global artifacts receive GitHub build-provenance
attestations. Releases include checksum sidecars for archives and installers,
fail-closed archive verification in both generated installers, and a verified
CycloneDX 1.5 binary SBOM. `scripts/verify-release-artifacts.py` validates the
assembled distribution.

Before creating a release tag:

1. Match the workspace version, `Cargo.lock`, skill compatibility manifest,
   tagged install docs, changelog, and intended tag; verify with
   `python3 scripts/check-skill-version.py --tag v<version>`.
2. Start from a clean working tree and run
   `RELEASE_TAG=v<version> scripts/check-release-readiness.sh`.
3. Retain the clean benchmark JSON/CSV emitted below `target/` and the CI
   benchmark artifact for performance claims.
4. Exercise the deployment-specific backup and rollback procedure from
   [`production.md`](production.md).
5. Push the release commit and wait for branch CI before creating the tag.

Creating or pushing a tag publishes through the release workflow and is an
external side effect. Do it only with explicit approval. `dist generate` does
not preserve repository hardening around SHA pins, dependency policy, installer
checksum verification, SBOM validation, or native verification; after
regeneration, reapply those changes and run `actionlint`.

## Scale benchmark

After a release build, run deterministic local acceptance benchmarks at 100,
1,000, and 10,000 memories:

```bash
scripts/benchmark-scale.sh
# bounded iteration while developing:
SCALES="100 1000" scripts/benchmark-scale.sh
```

The script uses isolated temporary stores and reports import, query, prime,
graph rebuild, and bundle export latency as CSV. Protocol v2 includes MAD and a
deterministic bootstrap interval for every median. It performs no network
access and deletes its stores on exit. The candidate binary must match
`Cargo.toml`; `ALLOW_VERSION_MISMATCH=1` is reserved for an intentional
candidate mismatch. CI checks the JSON report against
`scripts/benchmark-guardrails.json` and retains both JSON and CSV evidence.

For a controlled same-platform comparison, provide both binaries to one run so
samples are seed-interleaved instead of comparing two independent host windows:

```bash
REPORT_FILE=/tmp/candidate-v2.json \
BASELINE_REPORT_FILE=/tmp/baseline-v2.json \
BASELINE_MEM_BIN=/path/to/baseline/mem \
BASELINE_GIT_COMMIT=<baseline-commit> \
scripts/benchmark-scale.sh > /tmp/candidate-v2.csv

python3 scripts/check-benchmark-regression.py \
  --report /tmp/candidate-v2.json \
  --guardrails scripts/benchmark-guardrails.json \
  --baseline /tmp/baseline-v2.json \
  --max-regression-percent 35
```

The checker verifies the paired binary identities, run counts, protocol hash,
and exact sample schedule. Historical schema-v1 reports remain readable but are
not directly comparable to protocol v2.

The portable guardrails detect catastrophic regressions; they are deliberately
not cross-machine performance SLOs.

## Retrieval and agent behavior evaluation

Search, ranking, tokenizer, graph-query, and prime changes must run the
versioned retrieval fixture against the optimized binary:

```bash
mise run eval:retrieval
# equivalent after scripts/build-release.sh
python3 scripts/evaluate-retrieval.py --report target/retrieval-eval.json
```

The report records rankings, recall, reciprocal rank, plain/focused prime
coverage, graph evidence, fixture/binary hashes, and Git state. Stable CI and
the native release verification matrix retain the report as an artifact.
Synthetic scores are regression evidence, not a universal relevance claim.

Agent policy is evaluated from captured argv/approval/decision traces. Validate
the checked-in synthetic example with:

```bash
mise run eval:agent-reference
```

That command tests the evaluator only. Real platform evidence must identify the
platform, model, adapter, CLI, and skill versions and pass
`scripts/evaluate-agent-behavior.py --require-live`. See
[`evaluation.md`](evaluation.md) for the trace format, current evidence status,
and safety rules.

For an explicit capacity canary beyond the published 10,000-memory baseline:

```bash
SCALES="100000" \
INTERACTIVE_RUNS=5 \
MAINTENANCE_RUNS=1 \
REPORT_FILE=/tmp/mnemark-100k.json \
scripts/benchmark-scale.sh
```

The 100,000-memory run is not part of ordinary CI and must not be reported as a
supported baseline until its JSON report and correctness checks have been
reviewed.

## Developer notes

### Read-only query default

Queries are no-touch and lock-free by default, making them safe for agent
polling. `--touch` explicitly updates `access_count`/`last_accessed_at` and
acquires the write lock. A stale index produces an error unless the caller runs
`mem reindex` or explicitly passes `--repair-index`; the hidden `--no-touch`
flag remains only as a compatibility no-op.

### `serde_yaml_ng` alias

The original `serde_yaml` crate was abandoned; the workspace uses
`serde_yaml_ng` (the maintained fork) aliased as `serde_yaml` in `Cargo.toml`
for drop-in compatibility.

### Stale index tracking

The index stale state is tracked exclusively in the `metadata` table
(`index_dirty` key). There is no longer a `.stale` filesystem marker.

### Index schema versioning

Tantivy index artifacts carry `index/.mnemark-index-version`, owned by
`INDEX_SCHEMA_VERSION` in `crates/mem-core/src/search_index.rs`.

Bump it when Tantivy fields, field options, tokenizer behavior, token
normalization, indexed document content, or required ranking/filtering fields
change.

Do not bump it for query-time boosts, fuzzy query construction, SQLite-only
filtering, or CLI output changes.

### Bulk operations

`mem import` JSON arrays and `mem merge` use a single Tantivy `IndexWriter`
commit for all changes instead of N individual commits. Changed records are
fed to that writer in bounded batches.

### Graph module boundaries

`crates/mem-core/src/graph.rs` is a small public façade. Keep responsibilities
in the dedicated modules:

- `model.rs` — public graph types;
- `ids.rs` — stable node identifiers;
- `store.rs` — shared SQLite rows, metadata, and low-level writes;
- `query.rs` — explain, path, query, export, and candidates;
- `materialize/mod.rs` and `materialize/` — deterministic rebuild orchestration,
  memory/workflow/artifact extraction, and shared insertion helpers;
- `health.rs` — graph audit and health reporting;
- `semantic.rs` and `semantic/` — shared validation/persistence plus ingest,
  merge, projection, and review operations.

Shared primitives belong in `ids` or `store`; semantic code must not depend
back on the deterministic materializer. Add new behavior to the narrowest
module instead of growing the façade.
