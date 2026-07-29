# Pull request

## Summary

<!-- What user, agent, maintainer, or operator problem does this solve? -->

## Changes

-

## Validation

- [ ] `cargo fmt --all -- --check`
- [ ] `env -u CC -u CXX cargo clippy --workspace --locked --all-targets -- -D warnings`
- [ ] `env -u CC -u CXX cargo test --workspace --locked`
- [ ] `cargo audit --deny warnings`
- [ ] `cargo deny check`
- [ ] `python3 scripts/check-dependency-policy.py`
- [ ] `python3 scripts/check-source-hygiene.py`

## Public contract review

- [ ] CLI is unchanged, or `docs/cli-surface.txt`, topic docs, and the skill guide were updated.
- [ ] JSON is unchanged, or its schema and fixture were updated additively.
- [ ] Error code, exit code, and committed-write recovery semantics remain compatible.
- [ ] Store discovery and command effects remain explicit and correctly classified.
- [ ] Skill/Cargo/docs/tag versions remain exact according to `scripts/check-skill-version.py`.

## Security and operations

- [ ] No runtime store, private memory, secret, bundle, index, or release artifact is committed.
- [ ] New writes preserve backup, lock, secret, integrity, and rollback boundaries.
- [ ] Release changes preserve checksums, SBOM, attestations, native smoke/recovery, and benchmark gates.

## Documentation and release notes

- [ ] `CHANGELOG.md` was updated under Unreleased.
- [ ] User, operator, maintainer, and agent documentation was updated where applicable.
- [ ] Not-applicable checklist items are explained above.
