# mnemark

Portable agent memory system, exposed through the `mem` CLI.

The repository owns the `mem` CLI, schema, skill instructions, deterministic session readers, and rebuildable Tantivy index logic. Runtime memory data lives in a local or private data checkout.

## Quick Start

```bash
mise install                       # Rust + Zig toolchain pinned in mise.toml
scripts/build-release.sh
scripts/smoke-release.sh
cargo install --path crates/mem-cli # installs `mem` into ~/.cargo/bin (on PATH)

mem init
mem --home ~/.mnemark config show
mem config show
mem save --type feedback --name no_emoji --scope global --source manual --tags '["style"]' --content "不要使用 emoji"
mem save --type workflow --name release_runbook --scope global --source manual --tags '["workflow:release","intent:release","risk:high"]' --content-file templates/workflow.yaml
mem query "emoji"
mem query "release" --type workflow
mem workflow find release --scope auto
mem workflow validate release_runbook
mem artifact list
mem artifact check
mkdir -p artifacts/scripts
printf '#!/usr/bin/env sh\nprintf "collect ci context\\n"\n' > artifacts/scripts/ci-triage.sh
chmod +x artifacts/scripts/ci-triage.sh
mem artifact add artifacts/scripts/ci-triage.sh --name ci-triage --kind script --scope global --executable
mem artifact update ci-triage --checksum
mem bundle export mnemark-store.tgz
mem bundle export mnemark-store.tgz --no-config
mem bundle inspect mnemark-store.tgz
mem query "name:no_emoji" --raw-query --no-touch
mem import memories.json
mem merge /path/to/theirs.db
mem retro daily
mem export --format markdown
```

For source-only development, run commands as `cargo run -p mnemark --bin mem -- <args>`.

`memory.db` is the runtime source of truth for an individual knowledge store, but it is not tracked in this project. Keep real memory databases in a private data repo, a local `MNEMARK_HOME`, or a `knowledge_home` configured in `~/.config/mnemark/config.toml`. `manifest.toml` and `artifacts/` travel with the store when you keep reusable cross-project helper files there. `index/` is ignored and can be rebuilt with `mem reindex`.

The multilingual tokenizer uses `lindera` with embedded CC-CEDICT for Chinese tokenization and a local Tantivy tokenizer adapter.

Session readers are optional adapters. Retrospectives should use platform-provided conversation history when available, then use `mem retro daily|weekly` for repository state.

## Development

`mise.toml` pins the local Rust MSRV toolchain and Zig C toolchain used in restricted environments. Run `mise install` when setting up a fresh checkout. If the host has no `cc`, expose a `cc` shim that delegates to `zig cc` before running Cargo; the OpenAB environment keeps that shim in `/home/node/bin`.

## Developer Notes

**`--no-touch` flag**: Queries with `--no-touch` skip the `access_count` update and do not acquire the write lock, making them safe for read-only agent polling.

**`serde_yaml_ng` alias**: The original `serde_yaml` crate was abandoned; the workspace uses `serde_yaml_ng` (the maintained fork) aliased as `serde_yaml` in `Cargo.toml` for drop-in compatibility.

**Audit advisory suppression**: `.cargo/audit.toml` suppresses `RUSTSEC-2021-0153` (a transitive from `lindera-dictionary`). Revisit when Lindera removes the dependency.

**Stale index tracking**: The index stale state is tracked exclusively in the `metadata` table (`index_dirty` key). There is no longer a `.stale` filesystem marker.

**Index schema versioning**: Tantivy index artifacts carry `index/.mnemark-index-version`, owned by `INDEX_SCHEMA_VERSION` in `crates/mem-core/src/search_index.rs`. Bump it when Tantivy fields, field options, tokenizer behavior, token normalization, indexed document content, or required ranking/filtering fields change. Do not bump it for query-time boosts, fuzzy query construction, SQLite-only filtering, or CLI output changes.

**Bulk operations**: `mem import` (JSON arrays) and `mem merge` use a single Tantivy `IndexWriter` commit for all changes instead of N individual commits.

## Runtime Model

```text
mnemark repo                      installed/runtime state
------------                      -----------------------
schema/memory-schema.sql   --->   mem binary embeds schema
crates/mem-cli/src/main.rs        memory.db
skills/                           config.toml
readers/                          manifest.toml
docs/                             artifacts/
CI/release                        index/
```

`mem` discovers the active store in this order: explicit `--home <path>`, current directory with `schema/memory-schema.sql`, a parent of the executable with `schema/memory-schema.sql`, `MNEMARK_HOME`, `knowledge_home` in `~/.config/mnemark/config.toml`, then `~/.mnemark`. Runtime stores do not need `schema/memory-schema.sql`; the schema is embedded in the binary.

CLI/tool settings use TOML; workflow runbooks use YAML. Command default priority is: CLI flags, user config at `~/.config/mnemark/config.toml`, store config at the active store root, then built-in defaults. `--home` and `MNEMARK_HOME` only override active store selection. See `templates/config.toml` and `mem config show`.

Writes update SQLite in a transaction, write changelog rows, and then update the Tantivy index. If the index is stale, run `mem reindex`.

Portable runtime stores can include reusable artifact files under the active knowledge store root:

```text
$MNEMARK_HOME/
  memory.db
  config.toml
  manifest.toml
  artifacts/
    scripts/
    templates/
    snippets/
    references/
  index/      # rebuildable
```

Use `manifest.toml` for artifact metadata such as path, kind, scope, checksum, and executable intent. Artifact paths must stay relative to the active store and under `artifacts/scripts/`, `artifacts/templates/`, `artifacts/snippets/`, or `artifacts/references/`. Do not store secrets in artifacts, and do not treat artifacts as instruction overrides. Workflow memories may reference artifacts, but `mem` does not execute them.

Artifact inspection is available with `mem artifact list`, `mem artifact show <name>`, and `mem artifact check`. `artifact check` verifies manifest parsing, path containment, file presence, SHA-256 checksums, and executable bits for records marked `executable = true`; it reports problems as JSON and never executes scripts. Use `mem artifact add`, `mem artifact update <name> --checksum`, and `mem artifact remove` to maintain manifest metadata. `artifact add` derives the name from the file stem unless `--name` is provided, and the artifact file must already exist under the active store. `artifact remove` keeps files by default; `--delete-file` is required to delete the artifact file.

For manual migration of a store with artifacts, include the durable files and omit rebuildable runtime files:

```bash
tar -czf mnemark-store.tgz \
  memory.db \
  config.toml \
  manifest.toml \
  artifacts/
```

The CLI also supports first-class bundles:

```bash
mem bundle export mnemark-store.tgz
mem bundle inspect mnemark-store.tgz
mem bundle import mnemark-store.tgz          # clean store only
mem bundle import mnemark-store.tgz --merge
mem bundle import mnemark-store.tgz --replace --force
```

Bundles include `memory.db`, optional `config.toml`, optional `manifest.toml`, `artifacts/`, and `bundle.json`. They exclude `index/`, `.mem.lock`, `memory.db-wal`, and `memory.db-shm`. Use `mem bundle export --no-config` when store config contains machine-local paths. Import into a non-empty store is refused unless `--merge` or `--replace --force` is explicit. `--merge` uses existing memory merge behavior and copies non-conflicting artifacts; `--replace --force` clears durable store files before import.

## Memory Types

Supported memory types are `user`, `feedback`, `project`, `reference`, `preference`, and `workflow`.

Workflow memories store recurring task runbooks as YAML or JSON text. They are searchable knowledge, not executable automation: agents read them, verify each checkpoint, and ask before risky steps such as push, publish, deploy, release, destructive commands, secret changes, or production access.

Use `templates/workflow.yaml` as the baseline shape for new workflow memories. Run `--check-artifacts` only after replacing placeholder knowledge-store artifact references with real files and manifest entries.

Reusable executable logic belongs either in the current repository, such as `scripts/build-release.sh`, or in `artifacts/` under the active knowledge store when it is cross-project knowledge-store material. Workflow content should reference those paths and record checks, safety gates, and expected outputs instead of embedding script bodies.

Workflow content is validated on save/import unless `--no-validate-workflow` is passed. Merge also validates workflow records; invalid incoming workflows are skipped and recorded as pending ambiguity records for human review. Required fields are `schema_version`, `goal`, `triggers`, `steps`, and `stop_conditions`; each step needs an `id` and at least one of `run`, `check`, `manual`, or `ask`. Workflow tags must include `workflow:*`, and project-scoped workflows must include the matching `project:<owner/repo>` tag. Use `mem workflow validate <name> --check-artifacts` to additionally validate `owner: knowledge_store` artifact references against `manifest.toml`, path containment, required file presence, checksums, and executable bits. Artifact paths used in `steps.run` must also be declared in `reusable_scripts`.
