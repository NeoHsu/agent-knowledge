# Security Policy and Threat Model

mnemark is a local-first CLI for private agent memory. Security reports should be evaluated against the boundaries below rather than assuming a hosted, multi-tenant service model.

## Supported versions

Security fixes target the latest published release and the current `main` branch. Older pre-1.0 releases may require upgrading the store through the documented backup-first migration flow.

## Protected assets

- durable memory and provenance in `memory.db`;
- workflow runs, ambiguity records, semantic graph assertions, and revisions;
- `config.toml`, `manifest.toml`, and files under `artifacts/`;
- private Git history and portable bundle archives.

The Tantivy `index/`, `.mem.lock`, WAL/SHM files, and materialized `graph_nodes`/`graph_edges` are local runtime state. The index and materialized graph can be rebuilt from durable data.

## Trust boundaries

mnemark treats the following as untrusted input:

- imported JSON, Markdown, workflow YAML, and semantic-edge JSON;
- incoming SQLite databases and pulled `memory.db` files;
- bundle archives, metadata, manifests, and artifact files;
- memory text returned to an agent;
- workflow and graph content, which is data rather than instruction authority.

The local operating-system account is trusted. A process with permission to modify the store, executable, or this account's Git configuration can bypass CLI checks. mnemark is not a sandbox for hostile local code.

## Implemented controls

- Runtime-only store discovery; source checkouts are never selected implicitly.
- Explicit initialization and backup-first migration; reads never initialize or migrate.
- SQLite integrity checks and an allowlist for application tables, views, and triggers.
- Exclusive/shared file locking around mutating and snapshot operations.
- Invocation-sensitive global read-only enforcement before write locks, file
  replacement, index repair, or network access.
- Atomic managed-file replacement plus grouped rollback for agent policy,
  skill, symlink, and hook setup.
- Fallible broken-pipe-safe output staged through a bounded in-memory/unnamed
  temporary-file spool, with terminal control/bidirectional-character escaping
  after secret redaction.
- Default-reject secret-pattern scanning across durable database fields, side-state, artifacts, bundles, merges, and sync worktrees.
- Manual-source attestation and source-trust checks during overwrite and merge.
- Resource limits for text, files, archives, graph payloads, paths, and collection sizes.
- Bundle path allowlisting, regular-file-only extraction, per-file SHA-256, and rollback-safe replacement.
- Sync isolation, disabled hooks/signing/prompts, semantic database conflict merge, pulled-state validation, and rollback.
- Unix store permissions hardened to directory mode `0700` and file mode `0600`.
- Release CI vulnerability audit, cargo-deny license/source policy, crates.io provenance/license-metadata checks, Git history/current-source secret scans, native tests and execution of the exact archived binaries, SHA-pinned Actions, a pinned SHA-256 gate for the CC-CEDICT build input, checksum-verifying installers, CycloneDX 1.5 SBOMs, and build-provenance attestations.
GitHub-hosted controls such as secret scanning, push protection, and private
vulnerability reporting are repository settings rather than binary guarantees.
Verify them in GitHub Settings before relying on them; the source-controlled
history/current-tree gitleaks gate remains independently reproducible.

## Explicit limitations

### No at-rest encryption

Source version 0.10 does not use SQLCipher and does not encrypt bundle archives. Protect runtime stores and bundles with full-disk encryption, an encrypted private volume, or an encrypted transport appropriate to the host. Do not place a store in a public repository or shared directory.

Adding database encryption requires a separately reviewed key-management design covering non-interactive agents, recovery, rotation, migration, bundle export, and multi-machine sync. It must not silently derive a key from weak local metadata.

### Integrity is not authenticity

Bundle SHA-256 values detect corruption or modification relative to `bundle.json`; they do not identify who created the bundle. Accept bundles only through a trusted/private channel. A future signature format needs explicit key identity, rotation, and trust-root semantics before it can be considered authoritative.

### Secret scanning is defense in depth

Pattern scanning catches common credentials and rejects or explicitly redacts them, but it is not a complete DLP system and cannot identify every proprietary token format. Do not intentionally store credentials. Redaction is destructive and must be explicitly requested.

### Platform permissions differ

Unix modes are enforced and checked. Source version 0.10 cannot prove an equivalent user-only ACL on Windows, and `mem doctor` reports that limitation. Restrict store and bundle locations to the current user with Windows ACL tooling.

### No workflow sandbox

`mem workflow` discovers, renders, validates, and records runbooks; it never executes commands. The calling agent remains responsible for repository instructions, user confirmation, and operating-system sandboxing.

## Safe operation

Follow the canonical deployment, backup, restore, upgrade, rollback, and
incident checklist in [`docs/production.md`](docs/production.md). This document
owns the threat model and limitations; the production guide owns operational
procedures.

## Control verification

A security-boundary or release-workflow change invalidates the implemented-control
list. Re-verify source-controlled controls with:

```bash
mise run security
python3 scripts/check-dependency-policy.py
scripts/check-workflows.sh
```

GitHub-hosted settings require a maintainer review in repository Settings; do
not infer them from source files alone.

## Reporting a vulnerability

Use GitHub's private [security-advisory form](https://github.com/NeoHsu/mnemark/security/advisories/new). Do not include real credentials, private memory content, or an unredacted store in a public issue. Provide a minimal synthetic reproducer, affected version, platform, expected boundary, and observed behavior.
