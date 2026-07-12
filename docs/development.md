# Development

For agent-specific repo guidance, read `docs/agent-reference.md` first.

## Setup

`mise.toml` pins the local Rust MSRV toolchain and Zig C toolchain used in restricted environments.

```bash
mise install
```

If the host has no `cc`, expose a `cc` shim that delegates to `zig cc` before running Cargo; the OpenAB environment keeps that shim in `/home/node/bin`.

## Run from source

```bash
cargo run -p mnemark --bin mem -- <args>
```

## Validate

```bash
env -u CC -u CXX cargo test --workspace --locked
```

Use `env -u CC -u CXX` on macOS when inherited Zig compiler variables break native dependency builds.

## Release build and smoke test

```bash
scripts/build-release.sh
scripts/smoke-release.sh
```

Tag releases are gated by formatting, Clippy, tests, dependency audit, and a
release-binary smoke test on Linux. The same test and smoke flow also runs on
native macOS and Windows runners. Published platform archives receive GitHub
build-provenance attestations.

## Scale benchmark

After a release build, run deterministic local acceptance benchmarks at 100, 1,000, and 10,000 memories:

```bash
scripts/benchmark-scale.sh
# bounded iteration while developing:
SCALES="100 1000" scripts/benchmark-scale.sh
```

The script uses isolated temporary stores and reports import, query, prime, graph rebuild, and bundle export latency as CSV. It performs no network access and deletes its stores on exit.

## Developer notes

### Read-only query default

Queries are no-touch and lock-free by default, making them safe for agent polling. `--touch` explicitly updates `access_count`/`last_accessed_at` and acquires the write lock. A stale index produces an error unless the caller runs `mem reindex` or explicitly passes `--repair-index`; the hidden `--no-touch` flag remains only as a compatibility no-op.

### `serde_yaml_ng` alias

The original `serde_yaml` crate was abandoned; the workspace uses `serde_yaml_ng` (the maintained fork) aliased as `serde_yaml` in `Cargo.toml` for drop-in compatibility.

### Stale index tracking

The index stale state is tracked exclusively in the `metadata` table (`index_dirty` key). There is no longer a `.stale` filesystem marker.

### Index schema versioning

Tantivy index artifacts carry `index/.mnemark-index-version`, owned by `INDEX_SCHEMA_VERSION` in `crates/mem-core/src/search_index.rs`.

Bump it when Tantivy fields, field options, tokenizer behavior, token normalization, indexed document content, or required ranking/filtering fields change.

Do not bump it for query-time boosts, fuzzy query construction, SQLite-only filtering, or CLI output changes.

### Bulk operations

`mem import` JSON arrays and `mem merge` use a single Tantivy `IndexWriter` commit for all changes instead of N individual commits.
