# Agent Knowledge

Portable agent memory system. The repository owns profile files, a SQLite memory database, a rebuildable Tantivy index, deterministic session readers, and the `mem` CLI.

## Quick Start

```bash
bin/mem init
bin/mem save --type feedback --name no_emoji --scope global --source manual --tags '["style"]' --content "不要使用 emoji"
bin/mem query "emoji"
bin/mem merge /path/to/theirs.db
scripts/build-release.sh
bin/mem export --format markdown
```

`memory.db` is the source of truth and should be committed. `index/` is ignored and can be rebuilt with `bin/mem reindex`.

The current multilingual tokenizer is Tantivy n-gram. `lindera-tantivy` is not enabled because its latest release depends on Tantivy 0.25 while this project uses Tantivy 0.26 to keep the transitive `lru` advisory fixed.
