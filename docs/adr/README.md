# Architecture decision records

ADRs record context, alternatives, decisions, and consequences. Current
behavior remains documented in user/developer guides; ADRs explain why the
load-bearing boundary exists.

| ADR | Status | Decision |
| --- | --- | --- |
| [0001](0001-sqlite-source-rebuildable-indexes.md) | Accepted | SQLite is durable truth; search and deterministic graph projections rebuild |
| [0002](0002-runtime-only-store-discovery.md) | Accepted | Runtime stores are selected explicitly and never inferred from source checkouts |
| [0003](0003-deterministic-non-rag-graph.md) | Accepted | Graph memory remains local, evidence-bearing, and provider-independent |
| [0004](0004-git-byte-transport-semantic-merge.md) | Accepted | Git transports store bytes while mem performs semantic database conflict merge |
