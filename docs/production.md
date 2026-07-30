# Production Operations

This guide defines the production boundary for mnemark's local-first CLI model
and the evidence required before publishing or deploying a release. It does not
turn mnemark into a hosted or multi-tenant service.

## Supported production profile

The qualified profile is one operating-system user per active store, with the
store outside the source checkout and on a private, access-controlled volume.
SQLite is the durable source of truth; Tantivy and materialized graph tables are
rebuildable. Git remotes and bundle transfer channels must be private and
trusted.

macOS and Linux enforce store directory mode `0700` and database/lock mode
`0600`. Windows builds are tested, but automatic current-user-only ACL
verification is not yet available; `mem doctor` reports that limitation. Apply
and verify a user-only ACL with platform tooling before using sensitive data on
Windows.

The retained release baseline covers up to 10,000 memories. Larger stores are a
capacity canary until a clean benchmark report is reviewed and the documented
support boundary is raised.

Stores and bundles are plaintext. Use full-disk or private-volume encryption
when confidentiality matters. Bundle SHA-256 values detect corruption but do
not authenticate the publisher; accept bundles only through a trusted channel.

## Release qualification

Run the complete gate from a clean worktree:

```bash
scripts/check-release-readiness.sh
```

The gate verifies:

- workspace, lockfile, changelog, documentation, exact CLI/skill manifest, and
  intended release-tag version alignment, including refusal to reuse a tag
  from another commit;
- a clean Git worktree;
- formatting, Clippy, all tests, RustSec audit, cargo-deny license/source/ban
  policy, and independent dependency provenance metadata;
- release build version, deterministic retrieval-quality cases,
  installed-binary smoke checks, and a recovery drill;
- bounded 100/1,000-memory benchmark correctness and portable catastrophic-
  regression guardrails.

It writes retained local benchmark evidence below `target/`. A dirty development
checkout can exercise the same pipeline without qualifying a release:

```bash
ALLOW_DIRTY=1 scripts/check-release-readiness.sh
```

Use that override only while developing. For a clean qualification the gate
derives `v<workspace-version>`; set `RELEASE_TAG` explicitly when the intended
tag uses the accepted unprefixed form or needs to be made visible in logs:

```bash
RELEASE_TAG=v0.9.0 scripts/check-release-readiness.sh
```

The gate requires local `shellcheck` and `actionlint` by default.
`REQUIRE_AUX_TOOLS=0` or `RUN_BENCHMARK=0` is available for a bounded debugging
iteration, but a run that skips either check does not provide complete release
evidence.

## Machine-readable compatibility

Automation should pin a compatible `mem` version and inspect the supported
contracts before operating a store:

```bash
mem contract
mem --version
```

`mem contract` does not read or initialize a store. It reports the CLI output
contract, published schema names, and current store, bundle, workflow, graph,
and benchmark-report schema versions. `mem schema list|print` exposes the exact
bundled JSON Schemas; `mem operation inspect` reports parsed command effects.
JSON error envelopes carry `contract_version`; required fields remain stable
within a minor release, while additive fields are allowed. Before 1.0, breaking
machine-interface changes require a documented minor release.

## Deployment checklist

1. Install a platform release asset only after verifying its published
   checksum; verify build provenance and inspect the CycloneDX 1.5 SBOM when
   qualifying a deployment.
2. Run `mem --version` and use documentation from the matching Git tag.
3. Resolve the intended store before creating or changing it:

   ```bash
   mem config show
   ```

4. Create a new store only when intended, then verify it:

   ```bash
   mem init
   mem doctor
   ```

5. Keep the store private, configure a private Git remote only if sync is
   required, and use `mem sync --dry-run` before every sync.
6. Create and inspect a recovery bundle before migration or significant
   operational changes.
7. Run `mem audit`, `mem artifact check`, and a query/prime smoke appropriate to
   the deployment.

## Backup and recovery

Do not copy a live `memory.db` directly. Create an online snapshot bundle:

```bash
mem bundle export mnemark-store.tgz
mem bundle inspect mnemark-store.tgz
```

Restore into an isolated empty target first:

```bash
mem --home /tmp/mnemark-restore bundle import mnemark-store.tgz
mem --home /tmp/mnemark-restore doctor
mem --home /tmp/mnemark-restore query --format compact
mem --home /tmp/mnemark-restore artifact check
```

The repository recovery drill performs this flow with memory, workflow-run,
artifact, graph, checksum-corruption, migration-dry-run, and local-sync
assertions:

```bash
scripts/build-release.sh
scripts/recovery-drill.sh
```

The drill uses temporary stores, performs no network access, and deletes its
fixtures on exit.

## Upgrade and rollback

1. Stop concurrent agent writes and run `mem config show`.
2. Export and inspect a bundle.
3. Preview schema work:

   ```bash
   mem migrate --dry-run
   ```

4. If migration is required, retain the reported backup and run `mem migrate`.
5. Run `mem doctor`, `mem audit`, a representative query, focused prime when
   graph context is used, and `mem artifact check`.
6. On failure, stop using the store. Restore the pre-upgrade bundle or the
   migration backup into an isolated path, verify it, then switch the configured
   store root back deliberately.

There is no automatic downgrade path. Use export/import or restore a backup
created by the older compatible release.

## Incident checklist

- **Index stale or missing:** preserve `memory.db`, then run `mem reindex`.
- **Graph dirty:** inspect `mem graph stats`, then run `mem graph rebuild`.
- **Unexpected schema object or integrity failure:** stop writes and restore a
  trusted backup; do not bypass validation.
- **Rejected sync pull:** retain the reported error and pre-pull checkout; do
  not force Git state over the semantic rollback.
- **Bundle checksum failure:** reject the archive and reacquire it through the
  trusted channel.
- **Suspected secret exposure:** remove access to the store/bundle, rotate the
  external credential at its issuer, then remove or explicitly redact affected
  durable records and Git history.

Do not publish, push, deploy, or replace a production store solely because a
local gate succeeded. Those remain explicit operator actions.
