# CLAUDE.md

Read `docs/agent-reference.md` before making changes — it is the canonical agent guidance (safety rules, repo map, task routing, validation commands).

Repo-specific traps:

- Never run `mem` commands with this repo as the working directory unless you pass `--home <store>`. This repo contains `schema/memory-schema.sql`, so store discovery would treat the repo itself as a runtime store and create `memory.db`/`index/` here. Runtime data must never be committed to this repo.
- Validate with `env -u CC -u CXX cargo test --workspace --locked` (mise sets `CC="zig cc"` which breaks some native builds).
- When changing CLI behavior, update `skills/mnemark/references/cli-guide.md` and docs in the same change; command examples must stay copy-pastable and match the Clap args.
