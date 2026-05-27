# mem CLI Guide

## Write

```bash
bin/mem save --type feedback --name pr_small --scope "project:example/ot-product" --source agent --tags '["style:review","decision:pr-size"]' --content "PR 拆小逐個 review"
bin/mem save --type workflow --name release_runbook --scope global --source manual --tags '["workflow:release","intent:release","risk:high"]' --content-file workflows/release.yaml
```

Confidence is inferred from source unless `--confidence` is provided:

- `manual`: high, protected
- `agent`: medium
- `daily_retro`: low
- `weekly_retro`: low

`save` accepts inline `--content` or `--content-file`. It strips common API keys, bearer tokens, and password/secret assignments before persistence.

Without `--force`, `save` returns `duplicate_found` for an exact name match and `similar_found` for high-overlap content. The caller should decide whether to skip, update, or supersede. With `--force`, an exact-name save updates the existing memory only if the incoming source is at least as trusted as the existing source.

Workflow memories are validated on save/import unless `--no-validate-workflow` is passed. Merge validates workflow records too; invalid incoming workflows are not imported automatically and are recorded as pending ambiguity records for human review. Required fields are `schema_version`, `goal`, `triggers`, `steps`, and `stop_conditions`; each step needs `id` plus one of `run`, `check`, `manual`, or `ask`. Use `templates/workflow.yaml` as the baseline template.

## Query

```bash
bin/mem query "security review"
bin/mem query "部署流程" --scope auto
bin/mem query --type feedback
bin/mem query "release" --type workflow --scope auto
bin/mem query --tags "domain:security"
bin/mem query --sort time
bin/mem query --sort access-count
bin/mem query "deplpy" --fuzzy
bin/mem query "name:pr_small" --raw-query
bin/mem query "security review" --no-touch
bin/mem query "deploy" --semantic
```

By default, query treats punctuation as literal text so searches like `project:owner/repo` do not require Tantivy syntax escaping. Use `--raw-query` when you intentionally want Tantivy query syntax such as `name:pr_small`.

Query updates `access_count` and `last_accessed_at`; use `--no-touch` for read-only context loading. Superseded memories are hidden unless `--include-superseded` is used.

`--tags` matches exact JSON-array membership, not substrings. `--fuzzy` searches across `name`, `description`, `content`, and `tags`. `--semantic` is a reserved interface and returns `unsupported` until an embedding backend is configured.

## Update and Lifecycle

```bash
bin/mem update no_emoji --content "不要在回覆中使用 emoji"
bin/mem update release_runbook --content-file workflows/release.yaml
bin/mem update no_emoji --expected-version 2 --content "不要在回覆中使用 emoji"
bin/mem update no_emoji --add-tags '["style:output"]'
bin/mem supersede old_policy new_policy --expected-version 3 --content "新的政策內容"
bin/mem delete old_policy --expected-version 4
bin/mem delete old_policy --hard --force
```

Soft delete sets `valid_until`. Hard delete removes the row; protected memories require `--force`.
`--expected-version` returns `version_conflict` if the stored memory changed after the caller read it.

## Workflow

```bash
bin/mem workflow list --scope auto
bin/mem workflow find release --scope auto
bin/mem workflow show release_runbook
bin/mem workflow validate release_runbook
```

Workflow helpers discover, display, and validate workflow memories. They never execute workflow commands. Agents execute runbook steps themselves, verify checkpoints, and ask before risky side effects.

## Ambiguity

```bash
bin/mem ambiguity add --query "PR 策略" --memory-ids '["pr_small","pr_bundled"]' --context "scope unclear"
bin/mem ambiguity list --pending
bin/mem ambiguity resolve 1 --note "project-specific preference"
bin/mem ambiguity resolve 1 --keep pr_small --soft-delete-others
```

`resolve --keep ... --soft-delete-others` soft-deletes non-protected alternatives referenced by the ambiguity and records skipped protected memories.
`ambiguity list` parses JSON fields such as `memory_ids`, structured merge-conflict `context`, and JSON `resolution` into JSON objects/arrays in the output.

## History and Maintenance

```bash
bin/mem history
bin/mem history no_emoji
bin/mem stats
bin/mem audit
bin/mem audit --fix
bin/mem gc --days 90
bin/mem reindex
bin/mem retro daily
bin/mem retro weekly --limit 200
```

`retro` emits an orchestration bundle for the LLM. It does not read platform logs itself; the active platform or harness should provide conversation history.

## Import and Export

```bash
bin/mem export --format json
bin/mem export --format markdown
bin/mem import memories.json
bin/mem import note.md --type reference
bin/mem import workflows.json --no-validate-workflow
```

`import` emits one summary JSON object:

```json
{
  "status": "import_complete",
  "total": 3,
  "counts": {
    "saved": 1,
    "duplicate_found": 1,
    "failed": 1
  },
  "results": []
}
```

JSON imports process an array of memory-like objects. Markdown or other files import as one `reference` memory unless `--type` is supplied.

## Merge

```bash
bin/mem merge /path/to/theirs.db
bin/mem merge /path/to/theirs.db --prefer-trusted
```

Merge strips common secrets from incoming content, imports memories with new names, skips identical same-name memories, and records same-name content conflicts in `ambiguities` instead of overwriting automatically. Merge conflict ambiguity records include a structured incoming snapshot in `context`.
Lower-trust incoming same-name memories are rejected. `--prefer-trusted` lets a higher-trust incoming memory update a lower-trust local memory; equal-trust differences still become ambiguities.

## Release Build

```bash
scripts/build-release.sh
```

`bin/mem` runs `target/release/mem` when present and falls back to `cargo run` otherwise.
