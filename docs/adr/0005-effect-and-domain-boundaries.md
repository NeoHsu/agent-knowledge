# ADR 0005: Central effect policy and versioned write-domain boundary

- Status: Accepted
- Date: 2026-08-04

## Context

Lock routing, dry-run safety, read-only operation, JSON import normalization,
and ordinary saves all describe the same command and memory-write semantics.
Duplicating those decisions in CLI handlers or a second static catalog would
allow safety policy and persisted behavior to drift.

Multi-file setup and direct file writes also exposed partial-update windows:
an earlier policy or skill write could survive when a later hook update failed.

## Decision

- Keep one invocation-sensitive `CommandEffect` classifier and use it for lock
  routing, operation inspection, and global read-only enforcement.
- Convert CLI/import input through versioned wire types into a validated
  `SaveRequest`; only core domain persistence accepts that normalized type and
  returns a typed `SaveOutcome`.
- Use one atomic-file primitive for managed file replacement, and wrap platform
  setup targets in a snapshot/rollback transaction.
- Keep process concerns—Clap, file input, similarity orchestration, index
  completion, and stdout/stderr—in `mem-cli`; keep trust, validation,
  normalization, and row persistence in `mem-core`.

## Consequences

- A new command flag must update one effect classifier and its exhaustive tests.
- `--read-only` rejects all classified file, network, durable, and rebuildable
  effects before write locks or side effects.
- Import and direct saves share trust, secret, resource, workflow, provenance,
  and persistence behavior without constructing CLI arguments inside core.
- New incompatible write shapes require an explicit wire version/adapter.
- Setup failure restores managed policy, skill, symlink, and hook targets; an
  independently failing rollback is reported with the original error.
