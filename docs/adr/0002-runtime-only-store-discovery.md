# ADR 0002: Runtime-only store discovery

- Status: Accepted
- Date: 2026-07-29

## Context

A source checkout contains schema, tests, and release assets but must not
silently become a private memory store. Agents frequently run commands from
repository roots, so current-directory inference risks committing runtime data
or mutating the wrong target.

## Decision

Store precedence is explicit `--home`, `MNEMARK_HOME`, user configuration, then
`~/.mnemark`. Source checkouts and executable parents are never candidates.
Reads never initialize or migrate. `mem config show` is the pre-write target
verification mechanism.

## Consequences

- Repository location does not affect memory isolation.
- Tests and manual source runs must use isolated `--home` paths.
- First use requires an explicit `mem init` after target review.
- Agent setup is user-level; project separation is logical scope inside the active store.
