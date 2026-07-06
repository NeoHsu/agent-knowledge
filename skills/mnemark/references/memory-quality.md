# Memory Quality Rules

Memories are read by models of varying capability. Write every memory so the least capable future model can apply it mechanically, without re-deriving the reasoning that produced it.

## The three-part shape

Every `feedback`, `preference`, and decision-type memory content must state:

1. **Trigger** — WHEN it applies, as a concrete condition.
2. **Action** — WHAT to do, as an imperative instruction.
3. **Why** — one line of rationale, with date or evidence.

Bad (fact without trigger or action):

```text
emoji 不好
```

Good:

```text
產出任何面向使用者的文字（回覆、commit message、文件）時，不要使用 emoji。
原因：使用者 2026-07-05 明確要求。
```

## Writing rules

- One fact per memory. Two facts means two memories; link them with shared tags instead.
- Testable: a reviewer must be able to answer "was this memory followed?" with yes or no.
- Include the exact command, path, or value — not a description of it. `env -u CC -u CXX cargo test --workspace --locked` beats「用正確的環境變數跑測試」.
- Convert relative dates ("last week", "目前") to absolute dates before saving.
- Record provenance in `--why` (who said it, in what context) so later retros can re-judge confidence.
- Names: short snake_case, stable across updates. Rename via `supersede`, not delete + save.

## Promotion ladder

Move knowledge up one level only when the threshold is met:

| Signal | Action |
| --- | --- |
| Durable fact or preference stated once | `mem save` as memory |
| Same manual procedure performed a 2nd time | propose a `type=workflow` memory (use `templates/workflow.yaml` shape) |
| Same helper script/template pasted a 2nd time | propose a repo `scripts/` file (project-specific) or knowledge-store artifact (cross-project) |
| Workflow stable and used across 2+ projects | skill candidate — raise it in weekly retro |

## Mechanism over prose

When agents repeatedly violate a remembered rule, do not write a stronger reminder. Convert the rule into a mechanism: a validator, a template, a hook, or a CI check. Prose compliance degrades as model capability drops; mechanisms do not. Record the mechanism's location in the memory and mark the prose version superseded.

## Anti-bloat

- Before saving, `mem query` for an existing similar memory; prefer `update` or `supersede` over a new record.
- Delete or supersede memories proven wrong immediately. A wrong memory is worse than no memory, because agents trust the store.
- Deletion test: if removing this memory would change no future behavior, do not save it.
- Do not save what the repo already records (code structure, git history, CLAUDE.md content).
