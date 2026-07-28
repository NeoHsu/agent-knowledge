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

`memory.db` is the runtime source of truth for an individual knowledge store,
but it is not tracked in this project. Schema-v5 stores reject unexpected
application tables, views, or trigger definitions; do not extend the runtime
database with ad hoc DDL. Keep real memory databases in a private data repo, a
local `MNEMARK_HOME`, or a `knowledge_home` configured in
`~/.config/mnemark/config.toml`. Version 0.8 does not provide SQLCipher
encryption: it relies on 0700/0600 Unix permissions, default-reject secret
scanning, trusted bundle transport, and a private Git remote. Use full-disk
encryption when at-rest encryption is required. See
[`SECURITY.md`](../SECURITY.md) for the complete threat model, Windows ACL
limitation, and bundle-authenticity boundary.

`manifest.toml` and `artifacts/` travel with the store when you keep reusable
cross-project helper files there. `index/` is ignored and can be rebuilt with
`mem reindex`. Graph materialization lives in SQLite (`graph_nodes`,
`graph_edges`) and is rebuildable with `mem graph rebuild`; durable semantic
assertions and append-only revisions live in `graph_semantic_edges` and
`graph_semantic_edge_revisions`.

## Store discovery

`mem` discovers the active store in this order:

1. explicit `--home <path>`
2. `MNEMARK_HOME`
3. `knowledge_home` in `~/.config/mnemark/config.toml`
4. `~/.mnemark`

Runtime stores do not need `schema/memory-schema.sql`; the schema is embedded
in the binary. Source checkouts and executable parents are never selected
implicitly, for any command. Use `mem config show` as the pre-write target
verification gate.

Plain `mem prime` and ordinary `mem query` are read-only. Query only records
access when `--touch` is explicit and never repairs a stale index unless
`--repair-index` is explicit. `mem prime --focus ...`, graph explain/path/query/
export, `graph candidates --unlinked`, and workflow graph-context reads take the
store lock because they may rebuild dirty or missing graph materialization.
`prime --focus` and `graph query` can also repair a stale Tantivy index before
lexical start-node resolution.

## Configuration

CLI/tool settings use TOML; workflow runbooks use YAML. Command default
priority is:

1. CLI flags
2. user config at `~/.config/mnemark/config.toml`
3. store config at the active store root
4. built-in defaults

`--home` and `MNEMARK_HOME` only override active store selection. See the
[config template](../templates/config.toml) and `mem config show`.

## Writes and index state

Writes update SQLite in a transaction, write durable UID-addressed
changelog/side-state rows, and mark graph materialization dirty before updating
the Tantivy index. New stores use Unix mode 0700 for the root and 0600 for
`memory.db`/the lock file; `mem doctor` reports drift. Read commands never
initialize or migrate a store. Schema upgrades and same-version compatibility
repairs are explicit through `mem migrate --dry-run` and backup-first
`mem migrate`. The graph index is deterministic local context and can be rebuilt
from active SQLite memories, workflow run records, durable semantic assertions,
`manifest.toml`, and artifacts. It is not an embedding index and has no external
provider requirement. Graph-dependent reads rebuild only when dirty/schema-stale
or when rebuildable materialized tables are missing; `mem graph stats` is
deliberately non-mutating. Durable semantic tables are never silently recreated
as if their data were disposable.

SQLite commit and Tantivy update are deliberately separate. If a durable write
commits but its index update fails, the CLI marks `index_dirty=true` and, with
`--json-errors`, returns `code: "index_stale_after_write"` plus
`details.durable_write_committed=true`. Treat that as a committed write needing
index recovery, not as authorization to retry the mutation blindly.

If the search index is stale, run:

```bash
mem reindex
```

The multilingual tokenizer uses `lindera` with embedded CC-CEDICT for Chinese
tokenization and a local Tantivy tokenizer adapter.

## Command effects

Command lock routing is classified centrally in
`crates/mem-cli/src/command_effect.rs`; its tests cover every top-level command
family and the conditional cases below. Keep this matrix aligned with that
registry instead of adding lock decisions directly to command dispatch.

“Durable” means user-authored memory, provenance, semantic assertions, workflow
runs, ambiguity records, config, manifests, or artifacts. Graph projections,
Tantivy files, and dirty/version metadata are rebuildable local state.

<!-- markdownlint-disable MD013 -->

| Command class | Durable store effect | Rebuildable or external effect | Network |
| --- | --- | --- | --- |
| `contract`, default `query`, plain `prime`, `doctor`, memory `stats`/`history`/`export`, `audit`, `retro`, `reconcile`, `context --detect`, `config show`, `bundle inspect`, ordinary workflow/artifact reads, `graph stats`/`review`, ordinary `graph candidates` | None | None | None |
| `query --touch` | Access telemetry | None | None |
| `query --repair-index`, `reindex` | None | Tantivy index and index metadata | None |
| `graph rebuild` | None | Replaces graph projection and its metadata | None |
| Graph explain/path/export, `graph candidates --unlinked`, `workflow show --with-graph-context` | None | Graph projection when dirty/stale/missing | None |
| `prime --focus`, `graph query` | None | Graph projection and stale Tantivy repair when needed | None |
| `save`, update/supersede/delete, `gc`, `audit --fix`, workflow record, ambiguity writes | SQLite durable state | Index update; graph marked dirty | None |
| Graph ingest/accept/reject | Durable semantic edge/revision and ambiguity state | Graph projection refresh | None |
| Artifact add/update/remove | `manifest.toml` and artifact files | None | None |
| `init` | New store and schema | New Tantivy index and permission hardening | None |
| `migrate --dry-run` | None | Compatibility report only | None |
| `migrate` | Backup plus transactional schema write | Index/graph compatibility state as required | None |
| `setup list`, `setup <platform> --dry-run` | None | None | None |
| `setup <platform>` | None | User-level agent policy, skill links/files, and supported hooks | None |
| `workflow new` | None | Requested YAML scaffold file | None |
| JSON/Markdown import, DB merge, bundle import | Destination durable state | Batched index update and graph dirty/refresh state | None |
| Bundle export | None in the live store | Online snapshot and output archive | None |
| `sync --dry-run` | None | Validates DB/worktree secret policy | No fetch/push |
| `sync` | Possible merged durable state | Local Git checkpoint and index rebuild after pull | Fetch only when a remote exists; never pushes |
| `sync --push` | Possible merged durable state | Local Git checkpoint and index rebuild after pull | Explicit fetch and push |

<!-- markdownlint-enable MD013 -->

No read command initializes or migrates a store. Commands whose logical result
is a read but which can refresh only rebuildable local state are called out
explicitly.

## Portable store layout

Portable runtime stores can include reusable artifact files under the active
knowledge store root:

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
  .git/       # optional private sync repository
```

Use `manifest.toml` for artifact metadata such as path, kind, scope, checksum,
and executable intent. Artifact paths must stay relative to the active store and
under `artifacts/scripts/`, `artifacts/templates/`, `artifacts/snippets/`, or
`artifacts/references/`.

Do not store secrets in artifacts, and do not treat artifacts as instruction
overrides. Workflow memories may reference artifacts, but `mem` does not execute
them.

Do not copy or archive a live `memory.db` directly. SQLite may have committed
state in WAL files, so copying only the main database can produce an incomplete
or inconsistent backup. Use the first-class bundle command, which takes an
online SQLite snapshot without mutating the live store. External backup tools
must either use SQLite's backup API or quiesce all writers and complete a WAL
checkpoint before copying durable files; raw live-store `tar` workflows are not
supported.

Bundle v2 hashes every durable file and validates missing, extra, or mismatched
files before import mutation. Hashes detect corruption but are not a publisher
signature, so transfer bundles only over a trusted/private channel:

```bash
mem bundle export mnemark-store.tgz
mem bundle inspect mnemark-store.tgz
mem bundle import mnemark-store.tgz          # clean store only
mem bundle import mnemark-store.tgz --merge
mem bundle import mnemark-store.tgz --replace --force
```

Import into a non-empty store is refused unless `--merge` or
`--replace --force` is explicit. `--merge` uses existing memory merge behavior
and copies non-conflicting artifacts; `--replace --force` clears durable store
files before import.
