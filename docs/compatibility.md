# Compatibility Policy

This policy describes source and release version `0.10.0`. Version alignment
alone does not qualify a release; published-release evidence applies only to
the exact `v0.10.0` commit after its artifact workflow succeeds.

| Surface | Version 0.10 contract |
| --- | --- |
| Deployment model | One operating-system user per active local/private store |
| Rust MSRV | 1.97.1 |
| Source-build C toolchain | Platform-native Clang/GCC/MSVC; Zig is not required |
| Release targets | macOS arm64/x86_64, Linux arm64/x86_64, Windows x86_64 |
| Store | SQLite schema v5; explicit backup-first migration |
| JSON errors | `contract_version: 1`; required fields stable within a minor release |
| Bundle | Format v2 with per-file SHA-256 integrity manifest |
| Workflow | `schema_version: 1` |
| Graph export/ingest | `schema_version: 1` |
| Agent skill | Exact release lockstep with the `mem` CLI |
| Capacity | Up to 10,000 memories; 100,000 remains a development canary |

## Stability rules

Before 1.0, a breaking machine-interface change requires a documented minor
release with migration guidance. Patch and minor releases may add commands,
optional fields, schema documents, error details, or enum values. They must not:

- remove required fields silently;
- make a read initialize or migrate a store;
- bypass global read-only enforcement;
- make sync push by default;
- reinterpret persisted data without an explicit version transition.

Consumers must ignore additive object fields and use machine control fields
instead of parsing human messages. `mem contract` is store-independent and
reports the versions supported by the running binary.

## CLI and documentation

`docs/cli-surface.txt` is generated from Clap and records every public command,
positional, flag, default, and conflict. Repository tests parse runnable `mem`
examples against the same Clap surface. A CLI change therefore updates the
snapshot, topic docs, and bundled skill together.

Regenerate the surface with:

```bash
mise run contract:update
```

Verify all documentation contracts with:

```bash
mise run contract:check
```

## Agent skill lockstep

The CLI, Cargo packages, lockfile, bundled skill, compatibility manifest,
schema fixture, behavior traces, tagged install docs, and release tag share
exact SemVer.

Unless a session hook already injected context after running the same checks, an
installed skill starts with:

```bash
mem --json-errors contract --skill-version 0.10.0
mem --read-only prime
```

A mismatch exits 2 with `code: version_mismatch` before configuration or store
data is read. `mem setup <platform>` is preferred because the binary embeds the
matching skill and installs policy version 6 plus guarded session wiring.

## Persisted-data compatibility

Reads never migrate. Older stores require `mem migrate --dry-run` followed by an
explicitly approved `mem migrate`; newer stores are rejected. Bundle import
validates format, SQLite schema, integrity, hashes, secrets, and durable side
state before destination mutation.

There is no automatic downgrade. Restore a verified pre-upgrade bundle or
migration backup created by the older compatible release.
