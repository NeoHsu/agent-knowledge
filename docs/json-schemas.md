# JSON contracts

The running binary exposes its public machine contracts without discovering or
initializing a store:

```bash
mem contract
mem schema list
mem schema print error-v1
mem operation list
mem operation inspect --store-exists -- query "release" --touch
```

Published schemas live under [`docs/schemas/`](schemas/) and are embedded in
`mem`. Every `*.schema.json` has a representative file under
[`docs/schemas/fixtures/`](schemas/fixtures/); tests discover and validate the
pairs automatically.

| Schema | Contract |
| --- | --- |
| `error-v1` | JSON stderr envelope emitted with `--json-errors` |
| `contract-v1` | Store-independent `mem contract` response |
| `schema-list-v1` | Bundled schema catalog |
| `operation-list-v1` | Stable Clap-derived leaf operation IDs |
| `operation-inspect-v1` | Exact store/file/network effects for one parsed invocation |
| `memory-list-v1` | JSON memory rows returned by query/export |
| `prime-v1` | Budgeted `prime --format json` response, including no-store status |
| `graph-export-v1` | Deterministic graph export |
| `bundle-manifest-v2` | `bundle.json` integrity manifest |
| `skill-compatibility-v1` | Exact CLI/skill release manifest |

For `memory-list-v1`, `tags` remains a JSON-encoded string array because that
is the established v1 `Memory` serialization. Parse the field as JSON; do not
split it on commas. Nullable lifecycle and content fields are represented
explicitly as `null`.

## Errors and exit codes

With `--json-errors`, stderr is one `error-v1` object. Required fields are
`status`, `contract_version`, `code`, `message`, `exit_code`, and `retryable`;
`details` is additive. Current stable classifications include:

| Exit | Code | Meaning |
| ---: | --- | --- |
| 2 | `cli_parse_error`, `usage`, `version_mismatch` | Invalid invocation or incompatible CLI/skill contract |
| 2 | `compatibility`, `safety_violation` | Unsupported persisted version or a rejected unsafe operation/input |
| 4 | `not_found` | A store, memory, schema, graph node, or artifact was not found |
| 1 | `conflict` | A reference is ambiguous or conflicts with durable state |
| 1 | `integrity` | SQLite, bundle, schema-object, or checksum validation failed |
| 1 | `index_stale_after_write` | Durable write committed but rebuildable indexing failed |
| 1 | `command_failed` | Unclassified non-retryable failure |

`index_stale_after_write` is not permission to repeat the mutation. Inspect the
durable result and run the returned recovery command first.

## Compatibility

Required fields remain compatible within a minor release and object fields may
be added. Consumers should ignore unknown fields and use control fields rather
than parsing human messages. A future breaking shape receives a new schema or
contract version and migration notes. See [Compatibility](compatibility.md).
