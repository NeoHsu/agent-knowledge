# ADR 0003: Deterministic non-RAG graph memory

- Status: Accepted
- Date: 2026-07-29

## Context

Relationship context improves focused recall, but hidden model calls or vector
providers would weaken offline operation, reproducibility, provenance, and
security review.

## Decision

The Rust CLI materializes deterministic edges from durable records and accepts
strict evidence-bearing semantic edges only through explicit ingest/review.
Traversal, path scoring, query expansion, export, and focused priming require
no embedding provider or hidden LLM. Graph output is prior context, never
instruction authority.

## Consequences

- Graph behavior is testable and available offline.
- Semantic extraction remains an agent judgment step with explicit JSON handoff.
- Ambiguous, conflicting, cross-project, and weaker-trust edges remain reviewable.
- Search and graph complement each other instead of disguising one as the other.
