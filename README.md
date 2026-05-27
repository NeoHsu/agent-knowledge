# Agent Knowledge

Portable agent memory system. The repository owns the `mem` CLI, schema, skill instructions, deterministic session readers, and rebuildable Tantivy index logic. Runtime memory data lives in a local or private data checkout.

## Quick Start

```bash
scripts/build-release.sh
export PATH="$PWD/target/release:$PATH"

mem init
mem save --type feedback --name no_emoji --scope global --source manual --tags '["style"]' --content "不要使用 emoji"
mem save --type workflow --name release_runbook --scope global --source manual --tags '["workflow:release","intent:release","risk:high"]' --content-file workflows/release.yaml
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

`memory.db` is the runtime source of truth for an individual knowledge store, but it is not tracked in this project. Keep real memory databases in a private data repo or local `AGENT_KNOWLEDGE_HOME`. `index/` is ignored and can be rebuilt with `mem reindex`.

The multilingual tokenizer uses `lindera-tantivy` with embedded CC-CEDICT for Chinese tokenization. This pins Tantivy to 0.25 because `lindera-tantivy 2.0.0` is not yet compatible with Tantivy 0.26.

Session readers are optional adapters. Retrospectives should use platform-provided conversation history when available, then use `mem retro daily|weekly` for repository state.

## Development

`mise.toml` pins the local Rust MSRV toolchain and Zig C toolchain used in restricted environments. Run `mise install` when setting up a fresh checkout. If the host has no `cc`, expose a `cc` shim that delegates to `zig cc` before running Cargo; the OpenAB environment keeps that shim in `/home/node/bin`.

## Runtime Model

```text
agent-knowledge repo              runtime/private knowledge store
--------------------              -------------------------------
crates/mem-cli/src/main.rs        memory.db
schema/memory-schema.sql   --->   index/
skills/                           optional profile/config
readers/
docs/
CI/release
```

`mem` discovers the active store from the current directory when it contains `schema/memory-schema.sql`; otherwise it walks from the executable location and finally falls back to `AGENT_KNOWLEDGE_HOME` or `~/.agent-knowledge`.

Writes update SQLite in a transaction, write changelog rows, and then update the Tantivy index. If the index is stale, run `mem reindex`.

## Memory Types

Supported memory types are `user`, `feedback`, `project`, `reference`, `preference`, and `workflow`.

Workflow memories store recurring task runbooks as YAML or JSON text. They are searchable knowledge, not executable automation: agents read them, verify each checkpoint, and ask before risky steps such as push, publish, deploy, release, destructive commands, secret changes, or production access.

Use `templates/workflow.yaml` as the baseline shape for new workflow memories.

Workflow content is validated on save/import unless `--no-validate-workflow` is passed. Merge also validates workflow records; invalid incoming workflows are skipped and recorded as pending ambiguity records for human review. Required fields are `schema_version`, `goal`, `triggers`, `steps`, and `stop_conditions`; each step needs an `id` and at least one of `run`, `check`, `manual`, or `ask`. Workflow tags must include `workflow:*`, and project-scoped workflows must include the matching `project:<owner/repo>` tag.
