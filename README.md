# Agent Knowledge

Portable agent memory system. The repository owns the `mem` CLI, schema, skill instructions, deterministic session readers, and rebuildable Tantivy index logic. Runtime memory data lives in a local or private data checkout.

## Quick Start

```bash
mise install                       # Rust + Zig toolchain pinned in mise.toml
scripts/build-release.sh
scripts/smoke-release.sh
cargo install --path crates/mem-cli # installs `mem` into ~/.cargo/bin (on PATH)

mem init
mem --home ~/.agent-knowledge config show
mem config show
mem save --type feedback --name no_emoji --scope global --source manual --tags '["style"]' --content "不要使用 emoji"
mem save --type workflow --name release_runbook --scope global --source manual --tags '["workflow:release","intent:release","risk:high"]' --content-file templates/workflow.yaml
mem query "emoji"
mem query "release" --type workflow
mem workflow find release --scope auto
mem workflow validate release_runbook
mem query "name:no_emoji" --raw-query --no-touch
mem import memories.json
mem merge /path/to/theirs.db
mem retro daily
mem export --format markdown
```

For source-only development, run commands as `cargo run -p agent-knowledge --bin mem -- <args>`.

`memory.db` is the runtime source of truth for an individual knowledge store, but it is not tracked in this project. Keep real memory databases in a private data repo, a local `AGENT_KNOWLEDGE_HOME`, or a `knowledge_home` configured in `~/.config/agent-knowledge/config.toml`. `index/` is ignored and can be rebuilt with `mem reindex`.

The multilingual tokenizer uses `lindera` with embedded CC-CEDICT for Chinese tokenization and a local Tantivy tokenizer adapter.

Session readers are optional adapters. Retrospectives should use platform-provided conversation history when available, then use `mem retro daily|weekly` for repository state.

## Development

`mise.toml` pins the local Rust MSRV toolchain and Zig C toolchain used in restricted environments. Run `mise install` when setting up a fresh checkout. If the host has no `cc`, expose a `cc` shim that delegates to `zig cc` before running Cargo; the OpenAB environment keeps that shim in `/home/node/bin`.

## Developer Notes

**`--no-touch` flag**: Queries with `--no-touch` skip the `access_count` update and do not acquire the write lock, making them safe for read-only agent polling.

**`serde_yaml_ng` alias**: The original `serde_yaml` crate was abandoned; the workspace uses `serde_yaml_ng` (the maintained fork) aliased as `serde_yaml` in `Cargo.toml` for drop-in compatibility.

**Audit advisory suppression**: `.cargo/audit.toml` suppresses `RUSTSEC-2021-0153` (a transitive from `lindera-dictionary`). Revisit when Lindera removes the dependency.

**Stale index tracking**: The index stale state is tracked exclusively in the `metadata` table (`index_dirty` key). There is no longer a `.stale` filesystem marker.

**Index schema versioning**: Tantivy index artifacts carry `index/.agent-knowledge-index-version`, owned by `INDEX_SCHEMA_VERSION` in `crates/mem-core/src/search_index.rs`. Bump it when Tantivy fields, field options, tokenizer behavior, token normalization, indexed document content, or required ranking/filtering fields change. Do not bump it for query-time boosts, fuzzy query construction, SQLite-only filtering, or CLI output changes.

**Bulk operations**: `mem import` (JSON arrays) and `mem merge` use a single Tantivy `IndexWriter` commit for all changes instead of N individual commits.

## Runtime Model

```text
agent-knowledge repo              installed/runtime state
--------------------              -----------------------
schema/memory-schema.sql   --->   mem binary embeds schema
crates/mem-cli/src/main.rs        memory.db
skills/                           index/
readers/                          config.toml
docs/                             optional profile/config
CI/release
```

`mem` discovers the active store in this order: explicit `--home <path>`, current directory with `schema/memory-schema.sql`, a parent of the executable with `schema/memory-schema.sql`, `AGENT_KNOWLEDGE_HOME`, `knowledge_home` in `~/.config/agent-knowledge/config.toml`, then `~/.agent-knowledge`. Runtime stores do not need `schema/memory-schema.sql`; the schema is embedded in the binary.

CLI/tool settings use TOML; workflow runbooks use YAML. Command default priority is: CLI flags, user config at `~/.config/agent-knowledge/config.toml`, store config at the active store root, then built-in defaults. `--home` and `AGENT_KNOWLEDGE_HOME` only override active store selection. See `templates/config.toml` and `mem config show`.

Writes update SQLite in a transaction, write changelog rows, and then update the Tantivy index. If the index is stale, run `mem reindex`.

## Memory Types

Supported memory types are `user`, `feedback`, `project`, `reference`, `preference`, and `workflow`.

Workflow memories store recurring task runbooks as YAML or JSON text. They are searchable knowledge, not executable automation: agents read them, verify each checkpoint, and ask before risky steps such as push, publish, deploy, release, destructive commands, secret changes, or production access.

Use `templates/workflow.yaml` as the baseline shape for new workflow memories.

Workflow content is validated on save/import unless `--no-validate-workflow` is passed. Merge also validates workflow records; invalid incoming workflows are skipped and recorded as pending ambiguity records for human review. Required fields are `schema_version`, `goal`, `triggers`, `steps`, and `stop_conditions`; each step needs an `id` and at least one of `run`, `check`, `manual`, or `ask`. Workflow tags must include `workflow:*`, and project-scoped workflows must include the matching `project:<owner/repo>` tag.
