# Getting Started

This guide covers installing `mem`, initializing one private store, saving and
querying the first memory, and wiring one coding agent.
It documents source version `0.10.0`. Verify `mem --version` and use the matching
Git tag and GitHub Release after its artifact workflow succeeds.

## Install a verified release

Prefer release assets over building from source.

### macOS and Linux

```bash
base=https://github.com/NeoHsu/mnemark/releases/latest/download
curl --proto '=https' --tlsv1.2 -LsSfO "$base/mnemark-installer.sh"
curl --proto '=https' --tlsv1.2 -LsSfO "$base/mnemark-installer.sh.sha256"
if command -v sha256sum >/dev/null; then
  sha256sum -c mnemark-installer.sh.sha256
else
  shasum -a 256 -c mnemark-installer.sh.sha256
fi
sh mnemark-installer.sh
mem --version
```

### Windows PowerShell

```powershell
$base = "https://github.com/NeoHsu/mnemark/releases/latest/download"
Invoke-WebRequest "$base/mnemark-installer.ps1" -OutFile mnemark-installer.ps1
Invoke-WebRequest "$base/mnemark-installer.ps1.sha256" -OutFile mnemark-installer.ps1.sha256
$expected = ((Get-Content -Raw mnemark-installer.ps1.sha256).Trim() -split '\s+')[0]
$actual = (Get-FileHash -Algorithm SHA256 mnemark-installer.ps1).Hash.ToLowerInvariant()
if ($actual -ne $expected) { throw "installer checksum verification failed" }
& .\mnemark-installer.ps1
mem --version
```

Direct archives are published for:

| Platform | Asset |
| --- | --- |
| Apple Silicon macOS | `mnemark-aarch64-apple-darwin.tar.xz` |
| Intel macOS | `mnemark-x86_64-apple-darwin.tar.xz` |
| ARM64 Linux | `mnemark-aarch64-unknown-linux-gnu.tar.xz` |
| x64 Linux | `mnemark-x86_64-unknown-linux-gnu.tar.xz` |
| x64 Windows | `mnemark-x86_64-pc-windows-msvc.zip` |

Release assets include checksum sidecars, a CycloneDX SBOM, and GitHub
build-provenance attestations. Verify the archive attestation when GitHub CLI is
available:

```bash
archive=mnemark-aarch64-apple-darwin.tar.xz
gh attestation verify "$archive" --repo NeoHsu/mnemark
```

## Initialize the intended store

Store discovery is `--home → MNEMARK_HOME → user config → ~/.mnemark`. A source
checkout is never selected implicitly. Inspect the resolved target before the
first write:

```bash
mem config show
mem init
mem doctor
```

Do not initialize a path until the reported root is the private location you
intend to use. Stores and bundles are plaintext; read
[Security](../SECURITY.md) before storing sensitive private material.

## Save and query the first memory

```bash
mem save \
  --type feedback \
  --name no_emoji \
  --scope global \
  --source manual \
  --user-confirmed \
  --tags '["style:no-emoji"]' \
  --content 'Trigger: user-facing replies. Action: do not use emoji. Why: explicit preference.'
mem --read-only query "emoji" --format compact
mem --read-only query "name:no_emoji" --raw-query
```

Supported memory types are `user`, `feedback`, `project`, `reference`,
`preference`, and `workflow`. Manual provenance requires `--user-confirmed`.
Secret-like values reject writes unless explicit destructive redaction is
approved.

## Wire one coding agent

Setup writes user-level policy and skill files; Claude Code setup may also edit
its session hook. Preview one platform before applying it:

```bash
mem setup list
mem setup pi --dry-run
mem setup pi
mem doctor --platform pi
```

Replace `pi` with one platform you actually use:

- `claude-code`
- `codex`
- `gemini-cli`
- `opencode`

Setup is idempotent. The shared skill is installed under
`~/.agents/skills/mnemark`; supported platform skill directories use that copy
directly or through a managed symlink. Gemini CLI and OpenCode receive policy
prose but do not expose a supported skill directory.

The installed policy and Claude Code hook enforce this session sequence unless
a delimited context block was already injected:

```bash
mem --json-errors contract --skill-version 0.10.0
mem --read-only prime
```

A compatibility or prime failure is reported once; agents continue with memory
unavailable and must not initialize, migrate, read, or write the store.

## Configure private Git sync

The store must be its own Git repository, never an enclosing source repository.
Change to the `root` reported by `mem config show`, then configure a private
remote:

```bash
cd ~/.mnemark
git init -b main
git remote add origin <private-repo-url>
mem sync --dry-run
mem sync
```

A normal sync creates a local checkpoint and fetches only when a remote exists.
Pass `--push` only after explicit approval.

## Manual skill installation

`mem setup <platform>` is preferred because it installs the exact embedded
skill. An equivalent exact-version manual install is:

```bash
npx skills add https://github.com/NeoHsu/mnemark/tree/v0.10.0 --skill mnemark
mem --json-errors contract --skill-version 0.10.0
```

For development from a local checkout:

```bash
npx skills add ./skills/mnemark
```

## Next steps

| Need | Read |
| --- | --- |
| System overview | [Overview](overview.md) |
| Store discovery and effects | [Runtime Model](runtime-model.md) |
| Workflow, artifacts, bundles, and retrospectives | [Workflows](workflows.md) |
| Graph retrieval | [Graph Memory](graph-memory.md) |
| Machine contracts | [Compatibility](compatibility.md) and [JSON Contracts](json-schemas.md) |
| Backup, recovery, and incidents | [Production Operations](production.md) |
| Repository development | [Development](development.md) |
