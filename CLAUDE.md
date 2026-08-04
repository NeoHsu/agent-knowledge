# CLAUDE.md

Read `docs/agent-reference.md` before making changes — it is the canonical agent guidance (safety rules, repo map, task routing, validation commands).

Repo-specific traps:

- Store discovery is runtime-only (`--home` -> `MNEMARK_HOME` -> user config -> `~/.mnemark`); source checkouts are never selected implicitly. Before any mutating `mem` command, run `mem config show` and verify the intended runtime store. Runtime data must never live in or be committed to this repo.
- Native dependencies use the platform C/C++ toolchain. If a parent shell injects Zig compiler variables, validate with `env -u CC -u CXX -u AR cargo test --workspace --locked`; Zig is not a repository prerequisite.
- When changing CLI behavior, update `skills/mnemark/references/cli-guide.md` and docs in the same change; command examples must stay copy-pastable and match the Clap args.
