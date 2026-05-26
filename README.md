# Agent Knowledge

Portable agent memory system. The repository owns the `mem` CLI, schema, skill instructions, deterministic session readers, and rebuildable Tantivy index logic. Runtime memory data lives in a local or private data checkout.

## Quick Start

```bash
bin/mem init
bin/mem save --type feedback --name no_emoji --scope global --source manual --tags '["style"]' --content "不要使用 emoji"
bin/mem query "emoji"
bin/mem merge /path/to/theirs.db
bin/mem retro daily
scripts/build-release.sh
bin/mem export --format markdown
```

`memory.db` is the runtime source of truth for an individual knowledge store, but it is not tracked in this project. Keep real memory databases in a private data repo or local `AGENT_KNOWLEDGE_HOME`. `index/` is ignored and can be rebuilt with `bin/mem reindex`.

The multilingual tokenizer uses `lindera-tantivy` with embedded CC-CEDICT for Chinese tokenization. This pins Tantivy to 0.25 because `lindera-tantivy 2.0.0` is not yet compatible with Tantivy 0.26.

Session readers are optional adapters. Retrospectives should use platform-provided conversation history when available, then use `bin/mem retro daily|weekly` for repository state.
