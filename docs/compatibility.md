# Compatibility policy

| Surface | Source version 0.9 target |
| --- | --- |
| Deployment model | One operating-system user per active local/private store |
| Rust MSRV | 1.97 |
| Release platforms | macOS arm64/x86_64, Linux arm64/x86_64, Windows x86_64 |
| Store | SQLite schema v5; explicit backup-first migration |
| JSON errors | `contract_version: 1`; required fields stable within a minor release |
| Bundle | format v2 with per-file SHA-256 integrity manifest |
| Workflow | `schema_version: 1` |
| Graph export/ingest | `schema_version: 1` |
| Agent skill | Exact release lockstep with the `mem` CLI |
| Capacity baseline | Up to 10,000 memories; 100,000 is a reviewed canary only |

## Stability rules

Before 1.0, breaking machine-interface changes may occur only in a documented
minor release with changelog and migration guidance. Patch and minor releases
may add commands, optional fields, schema documents, error details, or enum
values. They must not silently remove required fields, make a read command
initialize or migrate a store, bypass an explicit global read-only gate, make
sync push by default, or reinterpret an
existing persisted schema without an explicit version transition.

Successful JSON shapes are versioned by the schema named by `mem schema list`.
Consumers should ignore additive object fields. `mem contract` remains
store-independent and reports the versions supported by the running binary.

## CLI and documentation

`docs/cli-surface.txt` snapshots every public command, positional, flag,
default, and conflict. Repository tests parse runnable `mem` examples against
Clap. A CLI change therefore requires regenerating the snapshot and updating
the skill/topic documentation in the same change.

## Agent skill lockstep

The released `skills/mnemark` package and CLI share exact SemVer. The
machine-readable `skills/mnemark/compatibility.json`, Cargo packages, lockfile,
SKILL.md gate, tag, and tag-pinned install docs must agree. CI verifies them.

Before store discovery or a memory operation, an installed skill runs once per
session:

```bash
mem --json-errors contract --skill-version 0.9.0
```

A mismatch exits 2 with `code: version_mismatch`, emits remediation without
reading configuration or store data, and never updates agent files without
approval. `mem setup <platform>` remains the preferred installation path
because the binary embeds and verifies its exact skill files.

## Persisted-data compatibility

Reads never migrate. Older stores require `mem migrate --dry-run` followed by
an explicitly approved `mem migrate`; newer stores are rejected. Bundle import
validates format, SQLite schema, integrity, hashes, secrets, and durable side
state before destination mutation. There is no automatic downgrade path: use a
verified pre-upgrade bundle or migration backup.
