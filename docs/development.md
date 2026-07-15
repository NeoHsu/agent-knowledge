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
python3 scripts/check-dependency-policy.py
```

Install `cargo-audit` locally when it is unavailable. Use
`env -u CC -u CXX` on macOS when inherited Zig compiler variables break native
dependency builds. The dependency policy check requires every locked
third-party package to come from crates.io and carry license metadata; RustSec
remains responsible for vulnerability advisories.

When changing workflows or shell scripts, also run:

```bash
actionlint .github/workflows/*.yml
shellcheck scripts/*.sh
```

CI tests both stable Rust and the declared Rust 1.97 MSRV. The stable lane also
builds and smoke-tests the release binary and runs a bounded benchmark
correctness smoke.

## Release build and smoke test

```bash
scripts/build-release.sh
scripts/smoke-release.sh
```

Tag releases are gated by formatting, Clippy, tests, dependency audit, and a
release-binary smoke test on Linux. The same test and smoke flow also runs on
native macOS and Windows runners. Published platform archives receive GitHub
build-provenance attestations.

Before creating a release tag:

1. Match the workspace version, `Cargo.lock`, changelog, and intended tag.
2. Start from a clean working tree and run the full validation above.
3. Run `scripts/build-release.sh` and `scripts/smoke-release.sh`; both reject a
   stale binary whose version differs from `Cargo.toml`.
4. Retain a clean benchmark JSON report for performance claims.
5. Push the release commit and wait for branch CI before creating the tag.

Creating or pushing a tag publishes through the release workflow and is an
external side effect. Do it only with explicit approval. `dist generate` does
not preserve repository hardening around SHA pins, dependency policy, or native
verification; after regeneration, reapply those changes and run `actionlint`.

## Scale benchmark

After a release build, run deterministic local acceptance benchmarks at 100,
1,000, and 10,000 memories:

```bash
scripts/benchmark-scale.sh
# bounded iteration while developing:
SCALES="100 1000" scripts/benchmark-scale.sh
```

The script uses isolated temporary stores and reports import, query, prime,
graph rebuild, and bundle export latency as CSV. It performs no network access
and deletes its stores on exit. It rejects a binary whose version differs from
`Cargo.toml`; set `ALLOW_VERSION_MISMATCH=1` only for an intentional controlled
comparison.

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
- `materialize.rs` — deterministic rebuild and source extraction;
- `health.rs` — graph audit and health reporting;
- `semantic.rs` and `semantic/` — shared validation/persistence plus ingest,
  merge, projection, and review operations.

Shared primitives belong in `ids` or `store`; semantic code must not depend
back on the deterministic materializer. Add new behavior to the narrowest
module instead of growing the façade.
