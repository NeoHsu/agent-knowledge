# Getting Started

This guide covers installing `mem`, initializing the active knowledge store, saving the first memory, and installing the bundled mnemark skill. For workflow runbooks, artifacts, bundles, and retrospectives, see `docs/workflows.md`.

## Install

Install `mem` from release assets instead of building from Rust source.

macOS / Linux:

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/NeoHsu/mnemark/releases/latest/download/mnemark-installer.sh | sh
```

Windows PowerShell:

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/NeoHsu/mnemark/releases/latest/download/mnemark-installer.ps1 | iex"
```

Direct release downloads are available on the [latest release page](https://github.com/NeoHsu/mnemark/releases/latest):

| Platform | Asset |
| --- | --- |
| Apple Silicon macOS | `mnemark-aarch64-apple-darwin.tar.xz` |
| Intel macOS | `mnemark-x86_64-apple-darwin.tar.xz` |
| ARM64 Linux | `mnemark-aarch64-unknown-linux-gnu.tar.xz` |
| x64 Linux | `mnemark-x86_64-unknown-linux-gnu.tar.xz` |
| x64 Windows | `mnemark-x86_64-pc-windows-msvc.zip` |

Checksums are published next to release assets.

## Initialize the store

After `mem` is on `PATH`:

```bash
mem init
mem config show
```

Runtime memory data is not stored in this source repository. See `docs/runtime-model.md` for active store discovery, config priority, and runtime files.

## Save and query the first memory

```bash
mem save --type feedback --name no_emoji --scope global --source manual --tags '["style"]' --content "不要使用 emoji"
mem query "emoji"
mem query "name:no_emoji" --raw-query --no-touch
```

Supported memory types are:

- `user`
- `feedback`
- `project`
- `reference`
- `preference`
- `workflow`

`--no-touch` skips `access_count` and `last_accessed_at` updates, so it is safe for read-only agent context loading.

## Install the mnemark skill

This repository ships an mnemark agent skill at `skills/mnemark`. Install the `mem` CLI first, then install the skill with the open agent skills CLI:

```bash
npx skills add https://github.com/NeoHsu/mnemark/tree/main/skills/mnemark
```

For a local checkout during development:

```bash
npx skills add ./skills/mnemark
```

Use `--global` to install for all projects, or `--agent <name>` when targeting a specific supported agent. After installation, agents can save, query, audit, merge, bundle, and run retrospectives through the local `mem` CLI.

## Configure the coding agent entrypoint

Run this in the project root you want to configure. It prepends a memory policy to the coding agent entrypoint so user-requested saved memories go through mnemark instead of a platform-specific memory system.

```bash
mem setup agent-policy
```

Default target selection:

1. use `CLAUDE.md` when it already exists
2. otherwise create or update `AGENTS.md`

Use `--target` for a specific file and `--dry-run` to preview without writing:

```bash
mem setup agent-policy --target CLAUDE.md
mem setup agent-policy --target AGENTS.md --dry-run
```

The command is idempotent. If the `mnemark memory policy` block already exists, it will not insert a duplicate.

## Next steps

| Need | Read |
| --- | --- |
| Complete command reference | `skills/mnemark/references/cli-guide.md` |
| Runtime store and portability | `docs/runtime-model.md` |
| Workflow runbooks, artifacts, bundles, retrospectives | `docs/workflows.md` |
| Repository development | `docs/development.md` |
