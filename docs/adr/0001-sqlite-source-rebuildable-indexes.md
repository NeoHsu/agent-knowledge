# ADR 0001: SQLite source of truth and rebuildable indexes

- Status: Accepted
- Date: 2026-07-29

## Context

Agent memory needs transactional durable state, multilingual retrieval, and
relationship traversal. Treating every derived index as independently durable
would create multi-store commit and recovery ambiguity.

## Decision

SQLite owns memories, provenance, workflow runs, ambiguities, semantic edges,
and revisions. Tantivy and deterministic `graph_nodes`/`graph_edges` are
derived projections marked dirty and rebuilt explicitly or by documented
graph-dependent reads. A committed SQLite write followed by index failure is
reported as `index_stale_after_write`, never as an uncommitted mutation.

## Consequences

- Backup, migration, merge, and bundle integrity center on SQLite durable state.
- Search and graph indexes can be deleted and rebuilt without losing authored knowledge.
- Writes must preserve observable dirty state when projection updates fail.
- Durable semantic assertions remain distinct from disposable graph projection rows.
