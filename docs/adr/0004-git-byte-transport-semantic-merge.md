# ADR 0004: Git byte transport and semantic database merge

- Status: Accepted
- Date: 2026-07-29

## Context

Git can version and transfer a private store, but SQLite is binary and ordinary
file conflict resolution cannot preserve memory scope, provenance, ambiguity,
semantic revisions, or trust ordering.

## Decision

`mem sync` checkpoints and commits locally, fetches only when a remote exists,
and never pushes without `--push`. When both sides changed `memory.db`, Git
transports the remote copy while `mem merge` performs semantic, UID-aware,
trust-aware reconciliation. Unsafe pulled state rolls back before use.

## Consequences

- Same-name differences become explicit ambiguities instead of silent loss.
- Hooks, signing, prompts, and unsafe worktree files are disabled or rejected.
- Sync remains a local checkpoint by default and network push is auditable.
- Git history alone is not the semantic conflict-resolution mechanism.
