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

## Developer notes

### `--no-touch` flag

Queries with `--no-touch` skip the `access_count` update and do not acquire the write lock, making them safe for read-only agent polling.

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
