# Production Operations

This runbook applies to mnemark's local-first production profile. It does not
qualify mnemark as a hosted or multi-tenant service. Security controls and
limitations are authoritative in [`SECURITY.md`](../SECURITY.md); runtime store
discovery and command effects are authoritative in
[`runtime-model.md`](runtime-model.md).

## Supported production profile

- one operating-system user per active store;
- store outside source checkouts on a private, access-controlled volume;
- SQLite as durable truth, with rebuildable Tantivy and graph projections;
- private, trusted Git remotes and bundle channels;
- up to 10,000 memories per supported store;
- full-disk or private-volume encryption when confidentiality matters.

Unix installs enforce store mode `0700` and database/lock mode `0600`. Windows
builds cannot automatically prove an equivalent current-user-only ACL; apply
and verify one with platform tooling before storing sensitive material.

Stores and bundles are plaintext. Bundle hashes detect corruption, not
publisher identity.

## Release qualification

A source revision is not qualified merely because local checks pass. The exact
commit must have a unique version, a dated changelog release heading, complete
local gate evidence, and successful remote artifact jobs.

From a clean worktree, with `<version>` replaced by the workspace version:

```bash
python3 scripts/check-release-metadata.py --release-tag v<version>
RELEASE_TAG=v<version> scripts/check-release-readiness.sh
```

The gate rejects reuse of a tag that points to another commit. It verifies:

- workspace, lockfile, skill, schema, documentation, changelog, and tag
  alignment;
- clean Git state;
- formatting, Clippy, tests, RustSec, cargo-deny, dependency provenance, secret
  scans, and workflow checks;
- release build, binary size, retrieval fixtures, smoke tests, and recovery;
- bounded 100/1,000-memory correctness and catastrophic-regression guardrails.

A bounded development exercise may use:

```bash
ALLOW_DIRTY=1 scripts/check-release-readiness.sh
```

That override cannot qualify a release. Neither can `REQUIRE_AUX_TOOLS=0` or
`RUN_BENCHMARK=0`.

Before publication, retain remote macOS, Linux, and Windows archive execution,
checksums, installers, SBOM, and provenance evidence for the exact commit.
Publishing, pushing, tagging, deploying, and replacing a production store remain
explicit operator actions.

## Deployment checklist

1. Install a platform artifact only after checksum and provenance verification.
2. Run `mem --version` and use documentation from the matching tag.
3. Inspect the store target:

   ```bash
   mem config show
   ```

4. Initialize only a new intended path, then verify it:

   ```bash
   mem init
   mem doctor
   ```

5. Keep the store private; configure a private Git remote only if required.
6. Run `mem sync --dry-run` before every sync and never push without approval.
7. Create and inspect a recovery bundle before migration or material operational
   changes.
8. Run `mem audit`, `mem artifact check`, and representative query/prime checks.

## Backup and restore

Never copy a live `memory.db` directly. Create an online SQLite snapshot bundle:

```bash
mem bundle export mnemark-store.tgz
mem bundle inspect mnemark-store.tgz
```

Restore into a fresh isolated directory first:

```bash
restore_home="$(mktemp -d "${TMPDIR:-/tmp}/mnemark-restore.XXXXXX")"
trap 'rm -rf -- "$restore_home"' EXIT
mem --home "$restore_home" bundle import mnemark-store.tgz
mem --home "$restore_home" doctor
mem --home "$restore_home" --read-only query "restore verification" --format compact
mem --home "$restore_home" --read-only artifact check
```

The repository recovery drill exercises memory, workflow-run, artifact, graph,
checksum-corruption, migration-preview, and local-sync assertions without
network access:

```bash
scripts/build-release.sh
scripts/recovery-drill.sh
```

## Upgrade and rollback

1. Stop concurrent agent writes and run `mem config show`.
2. Export and inspect a bundle.
3. Preview migration:

   ```bash
   mem migrate --dry-run
   ```

4. If required, retain the reported backup and run `mem migrate` after approval.
5. Run `mem doctor`, `mem audit`, a representative query, focused prime when
   graph context is used, and `mem artifact check`.
6. On failure, stop writes. Restore the pre-upgrade bundle or migration backup
   into an isolated path, verify it, then deliberately switch the configured
   root.

There is no automatic downgrade path.

## Incident checklist

- **Index stale or missing:** preserve `memory.db`, then run `mem reindex`.
- **Graph dirty:** inspect `mem graph stats`, then run `mem graph rebuild`.
- **Unexpected schema object or integrity failure:** stop writes and restore a
  trusted backup; do not bypass validation.
- **Rejected sync pull:** retain the error and pre-pull checkout; do not force
  Git state over semantic rollback.
- **Bundle checksum failure:** reject and reacquire the archive through a trusted
  channel.
- **Suspected secret exposure:** remove access, rotate the external credential at
  its issuer, then remove or explicitly redact affected durable records and Git
  history.
