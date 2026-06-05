# Runtime Model

`mnemark` separates the source repository from runtime/private memory data.

```text
mnemark repo                      installed/runtime state
------------                      -----------------------
schema/memory-schema.sql   --->   mem binary embeds schema
crates/mem-cli/src/main.rs        memory.db
skills/mnemark/                   config.toml
readers/                          manifest.toml
docs/                             artifacts/
CI/release                        index/
```

`memory.db` is the runtime source of truth for an individual knowledge store, but it is not tracked in this project. Keep real memory databases in a private data repo, a local `MNEMARK_HOME`, or a `knowledge_home` configured in `~/.config/mnemark/config.toml`.

`manifest.toml` and `artifacts/` travel with the store when you keep reusable cross-project helper files there. `index/` is ignored and can be rebuilt with `mem reindex`.

## Store discovery

`mem` discovers the active store in this order:

1. explicit `--home <path>`
2. current directory with `schema/memory-schema.sql`
3. a parent of the executable with `schema/memory-schema.sql`
4. `MNEMARK_HOME`
5. `knowledge_home` in `~/.config/mnemark/config.toml`
6. `~/.mnemark`

Runtime stores do not need `schema/memory-schema.sql`; the schema is embedded in the binary.

## Configuration

CLI/tool settings use TOML; workflow runbooks use YAML. Command default priority is:

1. CLI flags
2. user config at `~/.config/mnemark/config.toml`
3. store config at the active store root
4. built-in defaults

`--home` and `MNEMARK_HOME` only override active store selection. See `templates/config.toml` and `mem config show`.

## Writes and index state

Writes update SQLite in a transaction, write changelog rows, and then update the Tantivy index. If the index is stale, run:

```bash
mem reindex
```

The multilingual tokenizer uses `lindera` with embedded CC-CEDICT for Chinese tokenization and a local Tantivy tokenizer adapter.

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

The CLI also supports first-class bundles:

```bash
mem bundle export mnemark-store.tgz
mem bundle inspect mnemark-store.tgz
mem bundle import mnemark-store.tgz          # clean store only
mem bundle import mnemark-store.tgz --merge
mem bundle import mnemark-store.tgz --replace --force
```

Import into a non-empty store is refused unless `--merge` or `--replace --force` is explicit. `--merge` uses existing memory merge behavior and copies non-conflicting artifacts; `--replace --force` clears durable store files before import.
