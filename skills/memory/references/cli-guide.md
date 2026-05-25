# mem CLI Guide

## Write

```bash
bin/mem save --type feedback --name pr_small --scope "project:example/ot-product" --source agent --tags '["style:review","decision:pr-size"]' --content "PR 拆小逐個 review"
```

Confidence is inferred from source unless `--confidence` is provided:

- `manual`: high, protected
- `agent`: medium
- `daily_retro`: low
- `weekly_retro`: low

`save` strips common API keys, bearer tokens, and password/secret assignments before persistence.

Without `--force`, `save` returns `duplicate_found` for an exact name match and `similar_found` for high-overlap content. The caller should decide whether to skip, update, or supersede. With `--force`, an exact-name save updates the existing memory only if the incoming source is at least as trusted as the existing source.

## Query

```bash
bin/mem query "security review"
bin/mem query "部署流程" --scope auto
bin/mem query --type feedback
bin/mem query --tags "domain:security"
bin/mem query --sort time
bin/mem query --sort access-count
bin/mem query "deplpy" --fuzzy
bin/mem query "deploy" --semantic
```

Query updates `access_count` and `last_accessed_at`. Superseded memories are hidden unless `--include-superseded` is used.

`--fuzzy` searches across `name`, `description`, `content`, and `tags`. `--semantic` is a reserved interface and returns `unsupported` until an embedding backend is configured.

## Update and Lifecycle

```bash
bin/mem update no_emoji --content "不要在回覆中使用 emoji"
bin/mem update no_emoji --expected-version 2 --content "不要在回覆中使用 emoji"
bin/mem update no_emoji --add-tags '["style:output"]'
bin/mem supersede old_policy new_policy --expected-version 3 --content "新的政策內容"
bin/mem delete old_policy --expected-version 4
bin/mem delete old_policy --hard --force
```

Soft delete sets `valid_until`. Hard delete removes the row; protected memories require `--force`.
`--expected-version` returns `version_conflict` if the stored memory changed after the caller read it.

## Ambiguity

```bash
bin/mem ambiguity add --query "PR 策略" --memory-ids '["pr_small","pr_bundled"]' --context "scope unclear"
bin/mem ambiguity list --pending
bin/mem ambiguity resolve 1
```

## History and Maintenance

```bash
bin/mem history --recent
bin/mem history no_emoji
bin/mem stats
bin/mem audit
bin/mem audit --fix
bin/mem gc --days 90
bin/mem reindex
```

## Import and Export

```bash
bin/mem export --format json
bin/mem export --format markdown
bin/mem import memories.json
bin/mem import note.md --type reference
```

## Merge

```bash
bin/mem merge /path/to/theirs.db
bin/mem merge /path/to/theirs.db --prefer-trusted
```

Merge imports memories with new names, skips identical same-name memories, and records same-name content conflicts in `ambiguities` instead of overwriting automatically.
Lower-trust incoming same-name memories are rejected. `--prefer-trusted` lets a higher-trust incoming memory update a lower-trust local memory; equal-trust differences still become ambiguities.

## Release Build

```bash
scripts/build-release.sh
```

`bin/mem` runs `target/release/mem` when present and falls back to `cargo run` otherwise.
