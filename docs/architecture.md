# Architecture

mnemark is a two-crate Rust workspace. `mem-cli` owns process concerns; `mem-core`
owns reusable memory, persistence, validation, search, graph, workflow, and
artifact semantics.

## Dependency direction

```text
Clap / environment / files / stdout
              │
              ▼
          mem-cli adapters
              │  versioned wire requests
              ▼
     mem-core validation + domain
              │
              ▼
 SQLite durable truth ──► rebuildable Tantivy + graph projections
```

`mem-core` never depends on `mem-cli`. CLI modules may orchestrate locks,
transactions, similarity checks, index completion, and output rendering, but
trust, provenance, validation, and row-persistence decisions belong in core.

## Input boundary

Memory writes follow **wire → validate → normalize → domain**:

1. Clap or JSON import adapters read process-level inputs.
2. `SaveRequestV1` and `ImportWireV1` make the accepted shape and version
   explicit.
3. `validate_and_normalize` resolves scope, enforces manual attestation,
   redacts or rejects secrets, validates limits/tags/workflows, and returns a
   `SaveRequest`.
4. Only `SaveRequest` reaches `persist_save`; `SaveOutcome` preserves the
   stable saved/updated/duplicate/rejected result shapes.

Additive wire metadata can remain compatible, while a new semantic shape needs
a new version and an explicit adapter.

## Effect and lock boundary

`crates/mem-cli/src/command_effect.rs` is the single classifier for store
access, durable/rebuildable/output-file writes, and network effects. Dispatch
uses it for lock routing and for `--read-only` enforcement before any write lock
or mutation. Operation inspection serializes the same decision, preventing a
second policy catalog from drifting.

## File replacement boundary

`mem_core::atomic_file` stages same-directory writes, flushes and syncs bytes,
preserves existing permissions, and installs with platform-aware replacement.
Manifest, setup, workflow scaffold, index marker, redaction, and sync metadata
writes use this primitive. Agent setup additionally snapshots every managed
policy/skill/hook target and rolls the group back if a later step fails.

## Output boundary

CLI output is written through fallible locked writers. Broken stdout pipes are
a successful early consumer exit. `--max-bytes` rejects oversized rendered
stdout before partial output. Error text is secret-redacted and escapes
control/bidirectional characters before terminal rendering. Machine interfaces
remain versioned by the contract and JSON schemas.

## Test topology

Rust unit tests live beside domain code. CLI acceptance coverage is linked into
one `integration` target with a shared `TempDir` harness; `doc_drift` remains a
separate target because it owns CLI-surface regeneration. Property tests cover
byte-preserving atomic replacement, while deterministic integration tests cover
store, import, graph, workflow, setup rollback, and error contracts.

See the [runtime model](runtime-model.md) for command effects and durable state,
and the [ADRs](adr/README.md) for the decisions behind these boundaries.
