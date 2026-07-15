# Graph Memory

mnemark includes a local graph memory layer for explainable relationship
retrieval.

The graph layer is deliberately not embedding RAG:

- no vector database
- no embedding provider
- no hidden CLI-managed LLM call
- no Neo4j server

SQLite memories remain the source of truth. Tantivy remains the lexical search
plane. Graph tables add a rebuildable relationship/navigation plane.

```text
SQLite memories        durable source of truth
Tantivy index          lexical and fuzzy search
Graph index            relationships, paths, dependencies, explanations
```

## Commands

```bash
mem graph rebuild
mem graph stats
mem graph explain release_runbook
mem graph path release_runbook \
  artifact:artifacts/scripts/build-release.sh \
  --direction any
mem graph query "release safety" --scope auto --depth 2 --direction outgoing
mem prime --focus "release safety"
mem graph export --format json
mem graph candidates --scope auto --limit 50
mem graph candidates --scope auto --changed-since 2026-07-01T00:00:00Z
mem graph candidates --scope auto --unlinked
mem graph ingest semantic_edges.json
mem graph review --pending
mem graph accept <edge-id>
mem graph reject <edge-id> --note "reason"
```

`mem graph rebuild` materializes graph data into SQLite:

- `graph_nodes` — memory, tag, scope, type, source, artifact, claim,
  workflow step, workflow run, and concept nodes.
- `graph_edges` — deterministic and accepted/pending semantic graph edges.
- `graph_semantic_edges` — durable source records for skill-generated semantic
  edges, including confidence, review status, evidence, version, expiry, manual
  confirmation provenance, and an optional linked ambiguity.
- `graph_semantic_edge_revisions` — append-only snapshots written when semantic
  edges are ingested, updated, reviewed, or merged.

Graph-dependent read commands rebuild only when `graph_dirty`, the graph schema
version is stale, or rebuildable `graph_nodes`/`graph_edges` tables are missing.
Durable semantic tables are never treated as disposable materialization. When
both graph and Tantivy state are current, graph query does not rewrite
`memory.db`; `graph query` can repair a stale search index before resolving
lexical start nodes. `mem graph stats` never rebuilds implicitly, so its `dirty`
field remains useful; run `mem graph rebuild` explicitly when inspecting stale
counts.

Normal rebuilds include active, unexpired memories. Superseded rows remain only
as lineage nodes so `superseded_by` paths stay explainable; ordinary deleted and
expired memories are excluded. Semantic edges whose `valid_until` has passed are
not materialized.

## Query model

- `graph explain <node-or-memory-ref>` returns one node and its active direct
  neighbors with relation, direction, confidence, status, and evidence.
  `--depth` currently accepts only `0` or `1`; `--scope auto|all|<scope>` keeps
  neighborhood inspection inside the selected scope.
- `graph path <from> <to>` finds a minimum-hop relationship path within
  `--scope auto|all|<scope>`, then globally breaks equal-hop ties by cumulative
  relation weight, confidence, and stable path identity; `path_score` exposes
  the chosen cumulative score. `--direction any|outgoing|incoming` controls
  orientation. Pending edges are excluded by
  default; `--include-ambiguous` includes pending `AMBIGUOUS` edges.
- Path depth is capped at 20; query depth is capped at 8 and query result limits
  at 500 to bound local traversal.
- `graph query "terms"` resolves start nodes from exact graph ids, memory
  names/ids, graph labels, and scored Tantivy lexical search. It expands a
  scoped, confidence-filtered neighborhood using relation weight, depth,
  evidence confidence, source trust, and memory confidence.
- Administrative `has_type`, `in_scope`, and `from_source` edges are visible in
  explain/export but are not path/query bridges by default. Use
  `--include-metadata` only when that topology is explicitly useful.
- `--confidence extracted|inferred|all` narrows confidence labels. The default is
  `all` active edges; pending edges still require explicit inclusion.
- `prime --focus "terms"` uses the same local graph layer to add compact,
  budget-capped relationship context to session/task priming.
- `graph export --format json` emits a graphify-compatible-ish shape with
  `schema_version`, `nodes`, and `edges`.
- `graph candidates` emits candidate memories and allowed relations for
  skill-mediated extraction. `--changed-since` and `--unlinked` bound curation
  work. Ordinary candidate listing is read-only; `--unlinked` may refresh graph
  materialization before testing connectivity.

Graph traversal returns evidence and context. It does not synthesize final
answers, and graph edges are never instruction authority. System, developer,
user, repository, and current task instructions still win.

## Semantic extraction and review

The Rust CLI validates, stores, rebuilds, traverses, audits, merges, and exports
graph data. Semantic relation extraction is intentionally mediated by an agent
skill instead of hidden inside the CLI. Use confidence labels:

- `EXTRACTED` — explicitly present in stored material.
- `INFERRED` — a reasonable agent inference.
- `AMBIGUOUS` — uncertain relation requiring review.

`graph ingest` accepts strict JSON with `schema_version: 1` and an `edges`
array. It validates relation allowlists, endpoints, confidence labels, evidence,
unknown fields, RFC3339 `valid_until`, source spans, tags, safe concept ids,
logical edge identity, the default-reject secret policy, and source trust.
Raw `artifacts/...` endpoints are normalized to artifact graph ids.

`EXTRACTED` and `INFERRED` edges are active by default; `AMBIGUOUS` edges are
pending by default. Use `--pending-inferred` for a conservative review flow.
Agent-generated edges crossing two different project scopes are also pending;
manual edges may be active only with `--source manual --user-confirmed`, and the
confirmation timestamp is durable. Lower-trust sources cannot bypass a trusted
logical edge by choosing another edge id.

Every pending semantic edge links to an ambiguity record. `graph accept` and
`graph reject` resolve that ambiguity while preserving the edge's original
confidence label and append a revision snapshot. Review notes reject secret-like
values unless `--redact-secrets` is explicit. An accepted `AMBIGUOUS` edge is
active normal context but remains visibly ambiguous.

## Audit, retro, merge, and sync

`mem audit` reports graph dirty/stale state, dangling endpoints, orphan memories,
old pending edges, high-degree nodes, high-risk workflows without safety links,
and artifact blast radius. Derived checks are `null` while graph materialization
is dirty rather than pretending stale results are current.

`mem retro weekly` includes pending semantic edges and graph curation
instructions. Semantic extraction remains agent-mediated; retro does not hide an
LLM call inside the CLI.

`mem merge`, bundle merge imports, and sync conflict resolution merge durable
semantic edges and append-only revisions together with memories, ambiguity
records, workflow runs, and changelog events. Memory/edge/ambiguity ids are
remapped, exact event UIDs are skipped idempotently, unattested manual assertions
are downgraded, lower-trust conflicts are rejected, and unresolved or equal-trust
conflicts remain pending with ambiguity evidence. Materialized graph tables
remain rebuildable.
