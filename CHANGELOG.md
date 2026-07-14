# Changelog

All notable changes to mnemark are documented here.

## [Unreleased]

### Breaking changes

- Removed the former `setup agent-policy` subcommand. Agent setup is now exclusively user-level through `mem setup <platform>`; project knowledge remains logically scoped inside the active runtime store.

## [0.6.0] - 2026-07-11

### Breaking changes

- Store discovery is runtime-only: `--home`, `MNEMARK_HOME`, user config, then `~/.mnemark`. Source checkouts are never selected implicitly.
- Reads no longer initialize or migrate stores. Use explicit backup-first `mem migrate --dry-run` and `mem migrate`.
- `mem query` is no-touch by default; use `--touch` for access telemetry and `--repair-index` for explicit index repair.
- `mem sync` no longer pushes by default. Use `--push` only after explicit approval.
- `--source manual` requires `--user-confirmed`; unattested imported/merged claims are downgraded.
- Secret-like durable values reject writes by default. Explicit `--redact-secrets` replaces detected values with `[REDACTED]`.
- Memory names are unique within scope (`UNIQUE(scope, name)`); use `id:<memory-id>` for explicit ID resolution.

### Added

- Schema v5 store UUIDs, durable event UIDs, origin metadata, manual-confirmation provenance, strict RFC3339 lifecycle fields, Unix permission hardening, and explicit same-version repair for pre-release v5 stores.
- Explainable local graph memory with deterministic materialization, semantic edge review/revisions, scoped traversal, direction control, weighted path tie-breaking, audit, and focused priming.
- Deterministic query reranking across lexical relevance, source trust, confidence, scope specificity, and recency, with `--explain-score`.
- Bundle v2 online SQLite snapshots, streaming per-file SHA-256 manifests, archive/resource bounds, strict SQLite schema-object validation, and rollback-safe replacement.
- Agent-memory acceptance tests for read-only behavior, lifecycle isolation, scope isolation, secret leakage, durable merge idempotence, and skill packaging.
- Reproducible scale benchmarks with repeated p50/p95 measurements, peak RSS,
  artifact sizes, correctness assertions, binary/script hashes, cache metadata,
  and per-stage `bundle export --profile` timings.
- Configurable bounded query candidate retrieval through
  `[query].candidate_limit` (default 10,000; range 200-100,000).

### Fixed

- Expired, deleted, and superseded memories no longer leak into ordinary query,
  prime, workflow, retro, or graph recall.
- Query no longer repairs or recreates stale/missing indexes during read-only
  operation; writes rebuild incomplete indexes, and store/index symlinks are
  rejected.
- Secret validation now covers memory metadata, workflow runs, ambiguities,
  semantic graph state/revisions, artifacts, bundles, and merge side tables;
  artifact add/update/check rejects direct and intermediate symlinks before
  scanning, redaction, or hashing.
- Merge, bundle merge, and sync conflict resolution preserve and idempotently
  remap ambiguities, changelog events, workflow runs, semantic edges, and
  semantic revisions; sync checkpoints merged WAL, disables Git
  hooks/signing/prompts, validates and rolls back unsafe pulls, rebuilds the
  local index, and refuses residual bundle-replacement backups that could
  bypass secret scanning.
- Soft delete, supersede, ambiguity cleanup, and audit repair now increment
  retained memory versions for reliable optimistic concurrency.
- Prime text/JSON output now treats `--budget` as a hard character limit,
  including focused graph context, and reports when the fixed envelope cannot
  fit; ranked sections now apply ordering and `LIMIT` in SQLite instead of
  loading all matching memories.
- Query now pushes exact tag, scope, type, lifecycle, and expiry filters into
  Tantivy and uses adaptive bounded over-fetch before deterministic reranking.
- JSON import now reuses one connection, commits 500-row chunks with per-item
  savepoints, and updates graph/index dirty state once per chunk while
  preserving partial-success summaries.
- Bundle SQLite snapshots use larger cooperative backup steps, removing the
  fixed throttling that dominated export time without weakening snapshot
  consistency.
- Missing rebuildable graph tables are recreated and rebuilt without treating
  durable semantic tables as disposable.
- Release tags are gated by Linux/macOS/Windows tests and native binary smoke
  checks; release Actions are SHA-pinned and platform artifacts receive
  build-provenance attestations.
- The installed skill includes every referenced file, including
  `references/graph-rules.md`, and policy v5 reflects explicit migration,
  redaction, and push gates.
