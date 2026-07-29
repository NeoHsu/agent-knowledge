# Getting Started

This guide covers installing `mem`, initializing the active knowledge store,
saving the first memory, and installing the bundled mnemark skill. It describes
source version `0.9.0`; the `latest` installer can lag behind `main`, so verify
the installed version and use documentation from the matching Git tag when
necessary. For workflow runbooks, artifacts, bundles, and retrospectives, see
[Workflows](workflows.md).

## Install

Install `mem` from release assets instead of building from Rust source.

macOS / Linux:

```bash
base=https://github.com/NeoHsu/mnemark/releases/latest/download
curl --proto '=https' --tlsv1.2 -LsSfO "$base/mnemark-installer.sh"
curl --proto '=https' --tlsv1.2 -LsSfO "$base/mnemark-installer.sh.sha256"
if command -v sha256sum >/dev/null 2>&1; then
  sha256sum -c mnemark-installer.sh.sha256
else
  shasum -a 256 -c mnemark-installer.sh.sha256
fi
sh mnemark-installer.sh
```

Windows PowerShell:

```powershell
$base = "https://github.com/NeoHsu/mnemark/releases/latest/download"
Invoke-WebRequest "$base/mnemark-installer.ps1" -OutFile mnemark-installer.ps1
Invoke-WebRequest "$base/mnemark-installer.ps1.sha256" -OutFile mnemark-installer.ps1.sha256
$expected = ((Get-Content -Raw mnemark-installer.ps1.sha256).Trim() -split '\s+')[0]
$actual = (Get-FileHash -Algorithm SHA256 mnemark-installer.ps1).Hash.ToLowerInvariant()
if ($actual -ne $expected) { throw "installer checksum verification failed" }
& .\mnemark-installer.ps1
```

Direct release downloads are available on the [latest release page](https://github.com/NeoHsu/mnemark/releases/latest):

| Platform | Asset |
| --- | --- |
| Apple Silicon macOS | `mnemark-aarch64-apple-darwin.tar.xz` |
| Intel macOS | `mnemark-x86_64-apple-darwin.tar.xz` |
| ARM64 Linux | `mnemark-aarch64-unknown-linux-gnu.tar.xz` |
| x64 Linux | `mnemark-x86_64-unknown-linux-gnu.tar.xz` |
| x64 Windows | `mnemark-x86_64-pc-windows-msvc.zip` |

Checksums are published next to release assets. Releases also include a
CycloneDX 1.5 SBOM and GitHub build-provenance attestations. Confirm the
installed contract before continuing:

```bash
mem --version
```

## Initialize the store

After `mem` is on `PATH`:

```bash
mem init
mem config show
```

Runtime memory data is not stored in this source repository. See the
[Runtime Model](runtime-model.md) for active store discovery, config priority,
and runtime files.

## Save and query the first memory

```bash
mem save \
  --type feedback \
  --name no_emoji \
  --scope global \
  --source manual \
  --user-confirmed \
  --tags '["style"]' \
  --content "不要使用 emoji"
mem query "emoji"
mem query "name:no_emoji" --raw-query
```

Supported memory types are:

- `user`
- `feedback`
- `project`
- `reference`
- `preference`
- `workflow`

Query is read-only and no-touch by default. Use `--touch` only when access
telemetry is intentional. Secret-like values reject writes unless
`--redact-secrets` is explicit; manual provenance requires `--user-confirmed`.
The store and bundle formats are plaintext, so review the
[Security Policy](../SECURITY.md) before storing private data.

## Wire mnemark into your coding agents

One command per agent platform installs the user-level integration: a policy
block, the shared bundled skill where supported, and a session-start hook
running `mem prime` where the platform provides one:

```bash
mem setup list
mem setup claude-code
mem setup codex
mem setup pi
mem setup gemini-cli
mem setup opencode
mem doctor
```

Setup is user-level and never selects the current repository implicitly. The
bundled skill is installed once at `~/.agents/skills/mnemark`; Pi reads it
directly, while Claude Code and Codex use per-skill symlinks. `mem doctor`
verifies the policy, shared files, links, and session-start wiring. Platform
setup commands are idempotent and support `--dry-run`; `setup list` is already
read-only. Project knowledge is selected logically through memory scopes such
as `project:<owner>/<repo>`. See the capability matrix and explicit path
overrides in the
[CLI Guide](../skills/mnemark/references/cli-guide.md).

For multi-machine durability, make the store its own git repository and use
`mem sync`. Change to the `root` reported by `mem config show`; the default is
shown below:

```bash
cd ~/.mnemark
git init -b main
git remote add origin <private-repo-url>
mem sync --dry-run
mem sync          # local checkpoint + fetch/merge, no push
mem sync --push   # only after explicit approval
```

## Install the mnemark skill manually

`mem setup <platform>` installs the bundled skill into the shared Agent Skills
directory and links platform-specific skill paths when needed. Alternatively,
install from the repository with the open agent skills CLI:

```bash
npx skills add https://github.com/NeoHsu/mnemark/tree/v0.9.0 --skill mnemark
mem --json-errors contract --skill-version 0.9.0
```

Released skills and CLIs use exact SemVer lockstep. Proceed only when
`skill_compatibility.compatible` is `true`; a mismatch fails before store
configuration or memory data is read.

For a local checkout during development:

```bash
npx skills add ./skills/mnemark
```

Use `--global` to install for all projects, or `--agent <name>` when targeting
a specific supported agent. After installation, agents can save, query, audit,
merge, bundle, and run retrospectives through the local `mem` CLI.

## Next steps

| Need | Read |
| --- | --- |
| Complete command reference | [CLI Guide](../skills/mnemark/references/cli-guide.md) |
| Runtime store and portability | [Runtime Model](runtime-model.md) |
| Security boundaries and safe deployment | [`SECURITY.md`](../SECURITY.md) |
| Workflow runbooks, artifacts, bundles, retrospectives | [Workflows](workflows.md) |
| Repository development | [Development](development.md) |
| Production deployment, recovery, and rollback | [Production Operations](production.md) |
