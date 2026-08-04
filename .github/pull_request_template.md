# Pull request

## Summary

<!-- What user, agent, maintainer, or operator problem does this solve? -->

## Changes

-

## Validation

- [ ] `mise install`
- [ ] `mise run check:pr`
- [ ] Additional task-specific checks are listed below with results.

<!-- Search/graph/prime: mise run eval:retrieval -->
<!-- CLI/contracts: mise run contract:check -->
<!-- Workflow or shell changes: scripts/check-workflows.sh && shellcheck scripts/*.sh -->
<!-- Release candidate: RELEASE_TAG=v<version> scripts/check-release-readiness.sh -->

## Public contracts

- [ ] CLI is unchanged, or the generated surface, topic docs, and skill guide were updated.
- [ ] Machine JSON is unchanged, or its schema and representative fixture were updated additively.
- [ ] Error codes, exit codes, and committed-write recovery remain compatible.
- [ ] Store discovery and invocation effects remain explicit and centrally classified.
- [ ] CLI, Cargo, lockfile, skill, schemas, docs, changelog target, and intended tag are version-aligned.
- [ ] Retrieval-affecting changes passed `mise run eval:retrieval`, or retrieval is unchanged.
- [ ] Agent-policy changes updated behavior cases; synthetic traces are not presented as live evidence.

## Security and operations

- [ ] No runtime store, private memory, secret, bundle, index, or release artifact is committed.
- [ ] Mutating tests use an isolated `--home` or `TempDir` store.
- [ ] New writes preserve target verification, locking, secret checks, integrity, and rollback boundaries.
- [ ] Release changes preserve checksums, SBOM, attestations, native archive execution, recovery, and benchmark gates.

## Documentation and release notes

- [ ] The version-targeted Unreleased changelog section was updated.
- [ ] Affected user, operator, maintainer, agent, and skill docs were updated.
- [ ] Not-applicable checklist items are explained in the summary.
