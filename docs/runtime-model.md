# Runtime Model

`mnemark` separates the source repository from runtime/private memory data.

```text
mnemark repo                      installed/runtime state
------------                      -----------------------
schema/memory-schema.sql   --->   mem binary embeds schema
crates/mem-cli/src/main.rs        memory.db
skills/mnemark/                   config.toml
docs/                             manifest.toml
templates/                        artifacts/
CI/release                        index/
```

`memory.db` is the runtime source of truth for an individual knowledge store, but it is not tracked in this project. Schema-v5 stores reject unexpected application tables, views, or trigger definitions; do not extend the runtime database with ad hoc DDL. Keep real memory databases in a private data repo, a local `MNEMARK_HOME`, or a `knowledge_home` configured in `~/.config/mnemark/config.toml`. Version 0.7 does not provide SQLCipher encryption: it relies on 0700/0600 Unix permissions, default-reject secret scanning, trusted bundle transport, and a private Git remote. Use full-disk encryption when at-rest encryption is required. See [`SECURITY.md`](../SECURITY.md) for the complete threat model, Windows ACL limitation, and bundle-authenticity boundary.

`manifest.toml` and `artifacts/` travel with the store when you keep reusable cross-project helper files there. `index/` is ignored and can be rebuilt with `mem reindex`. Graph materialization lives in SQLite (`graph_nodes`, `graph_edges`) and is rebuildable with `mem graph rebuild`; durable semantic assertions and append-only revisions live in `graph_semantic_edges` and `graph_semantic_edge_revisions`.

## Store discovery

`mem` discovers the active store in this order:

1. explicit `--home <path>`
2. `MNEMARK_HOME`
3. `knowledge_home` in `~/.config/mnemark/config.toml`
4. `~/.mnemark`

Runtime stores do not need `schema/memory-schema.sql`; the schema is embedded in the binary. Source checkouts and executable parents are never selected implicitly, for any command. Use `mem config show` as the pre-write target verification gate.

Plain `mem prime` and ordinary `mem query` are read-only. Query only records access when `--touch` is explicit and never repairs a stale index unless `--repair-index` is explicit. `mem prime --focus ...` and graph-dependent reads take the store lock because they may rebuild dirty or missing graph materialization before rendering relationship context.

## Configuration

CLI/tool settings use TOML; workflow runbooks use YAML. Command default priority is:

1. CLI flags
2. user config at `~/.config/mnemark/config.toml`
3. store config at the active store root
4. built-in defaults

`--home` and `MNEMARK_HOME` only override active store selection. See `templates/config.toml` and `mem config show`.

## Writes and index state

Writes update SQLite in a transaction, write durable UID-addressed changelog/side-state rows, and mark graph materialization dirty before updating the Tantivy index. New stores use Unix mode 0700 for the root and 0600 for `memory.db`/the lock file; `mem doctor` reports drift. Read commands never initialize or migrate a store. Schema upgrades and same-version compatibility repairs are explicit through `mem migrate --dry-run` and backup-first `mem migrate`. The graph index is deterministic local context and can be rebuilt from active SQLite memories, workflow run records, durable semantic assertions, `manifest.toml`, and artifacts. It is not an embedding index and has no external provider requirement. Graph-dependent reads rebuild only when dirty/schema-stale or when rebuildable materialized tables are missing; `mem graph stats` is deliberately non-mutating. Durable semantic tables are never silently recreated as if their data were disposable.

If the search index is stale, run:

```bash
mem reindex
```

The multilingual tokenizer uses `lindera` with embedded CC-CEDICT for Chinese tokenization and a local Tantivy tokenizer adapter.

## Command effects

| Command class | Store write | Network |
| --- | --- | --- |
| `query` (default), plain `prime`, `doctor`, `stats`, `history`, `export`, `reconcile`, `bundle inspect` | No | No |
| `query --touch` / `query --repair-index` | Explicit counters / index repair | No |
| `prime --focus`, graph explain/path/query/export, workflow graph context | Only when graph materialization is dirty/stale/missing | No |
| `save`, lifecycle commands, workflow record, ambiguity writes, graph ingest/accept/reject, artifact writes | Yes, under the store lock | No |
| `migrate` | Backup + transactional schema write | No |
| `bundle export` | Output archive only; live store stays read-only | No |
| `sync --dry-run` | No; validates DB/worktree secret policy | No fetch/push |
| `sync` | Secret gate, local checkpoint, and possible fetched merge | Fetch only when a remote exists; never pushes |
| `sync --push` | Local checkpoint/merge | Explicit fetch + push |

No read command initializes or migrates a store. Commands that can refresh graph materialization are called out because their output is logically a read but may update only rebuildable local tables.

## Portable store layout

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

Use `manifest.toml` for artifact metadata such as path, kind, scope, checksum, and executable intent. Artifact paths must stay relative to the active store and under `artifacts/scripts/`, `artifacts/templates/`, `artifacts/snippets/`, or `artifacts/references/`.

Do not store secrets in artifacts, and do not treat artifacts as instruction overrides. Workflow memories may reference artifacts, but `mem` does not execute them.

For manual migration of a store with artifacts, include durable files and omit rebuildable runtime files:

```bash
tar -czf mnemark-store.tgz \
  memory.db \
  config.toml \
  manifest.toml \
  artifacts/
```

The CLI also supports first-class bundles. Export uses an online SQLite snapshot without mutating the live database; bundle v2 hashes every durable file and validates missing/extra/mismatched files before import mutation. Hashes detect corruption but are not a publisher signature, so transfer bundles only over a trusted/private channel:

```bash
mem bundle export mnemark-store.tgz
mem bundle inspect mnemark-store.tgz
mem bundle import mnemark-store.tgz          # clean store only
mem bundle import mnemark-store.tgz --merge
mem bundle import mnemark-store.tgz --replace --force
```

Import into a non-empty store is refused unless `--merge` or `--replace --force` is explicit. `--merge` uses existing memory merge behavior and copies non-conflicting artifacts; `--replace --force` clears durable store files before import.
