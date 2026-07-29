# Changelog

All notable changes to mnemark are documented here.

## [Unreleased]

### Added

- Added bundled JSON Schema discovery, representative fixtures, stable
  Clap-derived operation IDs, exact parsed command-effect inspection, and a
  generated full CLI surface snapshot.
- Added exact CLI/skill version lockstep with a store-independent fail-closed
  compatibility gate and tag-pinned manual skill installation.
- Added cargo-deny policy, an 84% measured coverage floor, installer checksum
  sidecars, fail-closed archive verification in both generated installers, a
  CycloneDX 1.5 binary SBOM, and
  global artifact attestations.
- Added a task-oriented documentation hub, compatibility policy, architecture
  decision records, contribution guidance, and GitHub PR/issue templates.

### Changed

- JSON error envelopes now include the additive `retryable` field. Typed core
  failures map consistently to usage, compatibility, safety, not-found,
  conflict, and integrity codes while preserving committed-index recovery
  semantics and stable exit statuses.
- Installation documentation now verifies installer bytes before execution.

## [0.9.0] - 2026-07-29

### Added

- Added benchmark protocol v2 with sample-level interleaved binary comparisons,
  combined protocol hashes, median absolute deviation, and deterministic
  bootstrap confidence intervals.
- Added a release-build SHA-256 gate for the exact CC-CEDICT source archive
  embedded by Lindera.
- Added retained 10,000-memory interleaved optimization reports and a
  100,000-memory development canary covering latency, sizes, and peak RSS.
- Added the `index_stale_after_write` JSON error code and structured recovery
  details when a durable SQLite write commits before its Tantivy update fails,
  allowing automation to repair the index without blindly retrying the write.

### Changed

- Removed the duplicate Ubuntu MSRV lane from pull-request CI while retaining
  stable Linux and Rust 1.97 Linux/macOS/Windows coverage across the two
  workflows; direct `main` pushes still run both stable and MSRV lanes.
- Split deterministic graph materialization into rebuild orchestration,
  memory, workflow, artifact, and shared insertion modules without changing
  graph identities or traversal behavior.
- Split workflow command dispatch into focused listing, display, scaffold,
  validation, and run-recording handlers without changing CLI or JSON output.
- Split agent-platform setup into policy, platform, skill-link, and hook modules
  while preserving managed-file conflict checks and upgrade behavior.
- Split doctor diagnostics into store, platform, and report modules while
  preserving check order, status levels, and recovery guidance.
- Split maintenance commands into migration, runtime contract/config, history,
  stats, audit, garbage-collection, and rendering modules without changing SQL
  or output formats.

### Performance

- Reduced the Tantivy writer memory budget from 50 MB to 20 MB after
  interleaved 10,000-memory trials cut import peak RSS from about 210 MiB to
  about 119 MiB with no material throughput regression.
- Overlapped bundle hashing with archive compression after validation;
  the retained interleaved 10,000-memory comparison reduced median export
  latency by about 18%.

## [0.8.0] - 2026-07-17

### Added

- Added `mem contract` for store-independent machine-interface and persisted-
  format version discovery, plus versioned JSON error envelopes and contract
  regression tests.
- Added a clean-tree release-readiness gate, an isolated bundle recovery drill,
  portable benchmark guardrails, retained CI benchmark artifacts, and a
  production deployment/recovery/rollback guide.

### Changed

- Release smoke verification now restores and validates memory, workflow-run,
  artifact, graph, and local-sync state; release CI also gates bounded benchmark
  correctness on Linux.
- Workflow validation now rejects unsupported schema versions instead of only
  checking that `schema_version` is present.

### Fixed

- Native macOS release builds ignore the known inherited Mise Zig `CC`/`CXX`
  override that passes an incompatible architecture spelling to cc-rs.
- Doctor reports active rather than historical memory count, v4 policy upgrades
  handle a missing final newline, and scope detection recognizes GitHub's
  `ssh://git@github.com/...` remote form.

## [0.7.0] - 2026-07-14

### Breaking changes

- Removed the former `setup agent-policy` subcommand. Agent setup is now
  exclusively user-level through `mem setup <platform>`; project knowledge
  remains logically scoped inside the active runtime store.

### Added

- Added the global `--json-errors` automation contract for machine-readable
  Clap and runtime failures, with secret-like values redacted from the JSON
  message.
- Added a repository security threat model, explicit encryption/authenticity
  boundaries, Windows ACL guidance, and automated weekly dependency updates.
- Added CJK and mixed-language query coverage plus bundle path-allowlist and
  credential-family regression tests.
- Added `mem import --summary-only` for bounded-memory large-import reporting
  without changing the default per-item result contract.

### Changed

- Unified ordinary saves and bulk imports behind one
  trust/provenance/version/changelog persistence pipeline.
- Added package metadata, stripped release symbols, and stale-binary version
  gates to smoke and benchmark scripts.
- Split graph health reporting into its own module.
- Decomposed the graph façade into model, query, materialization, identifier,
  and store modules, with semantic ingest, merge, projection, and review kept in
  separate units without changing the public graph API.
- Removed the dormant hidden `query --semantic` branch; lexical/fuzzy search
  and evidence-bearing graph retrieval remain the explicit non-RAG interfaces.

### Fixed

- Fuzzy queries now use the same multilingual analyzer as indexing instead of
  splitting only on whitespace, so Chinese and mixed-language typo searches
  use compatible terms.
- `mem doctor` now reports that automatic user-only ACL verification is
  unavailable on non-Unix platforms instead of silently omitting the permission
  boundary.

### Performance

- Graph materialization reuses cached SQLite node/edge upsert statements,
  reducing repeated SQL preparation during full rebuilds.
- JSON array import now performs a bounded-memory validation pass and then
  streams 500-item transaction chunks instead of retaining the full parsed
  array; malformed JSON is rejected before any chunk is written.
- Bulk index completion feeds 500-memory batches through one shared Tantivy
  writer/commit instead of cloning every changed document at once.

## [0.6.0] - 2026-07-13

### Breaking changes

- Store discovery is runtime-only: `--home`, `MNEMARK_HOME`, user config,
  then `~/.mnemark`. Source checkouts are never selected implicitly.
- Reads no longer initialize or migrate stores. Use explicit backup-first
  `mem migrate --dry-run` and `mem migrate`.
- `mem query` is no-touch by default; use `--touch` for access telemetry and
  `--repair-index` for explicit index repair.
- `mem sync` no longer pushes by default. Use `--push` only after explicit
  approval.
- `--source manual` requires `--user-confirmed`; unattested imported/merged
  claims are downgraded.
- Secret-like durable values reject writes by default. Explicit
  `--redact-secrets` replaces detected values with `[REDACTED]`.
- Memory names are unique within scope (`UNIQUE(scope, name)`); use
  `id:<memory-id>` for explicit ID resolution.

### Added

- Schema v5 store UUIDs, durable event UIDs, origin metadata,
  manual-confirmation provenance, strict RFC3339 lifecycle fields, Unix
  permission hardening, and explicit same-version repair for pre-release v5
  stores.
- Explainable local graph memory with deterministic materialization, semantic
  edge review/revisions, scoped traversal, direction control, weighted path
  tie-breaking, audit, and focused priming.
- Deterministic query reranking across lexical relevance, source trust,
  confidence, scope specificity, and recency, with `--explain-score`.
- Bundle v2 online SQLite snapshots, streaming per-file SHA-256 manifests,
  archive/resource bounds, strict SQLite schema-object validation, and
  rollback-safe replacement.
- Agent-memory acceptance tests for read-only behavior, lifecycle isolation,
  scope isolation, secret leakage, durable merge idempotence, and skill
  packaging.
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
