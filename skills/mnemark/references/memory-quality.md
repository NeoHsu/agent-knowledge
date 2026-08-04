# Memory Quality Rules

Memories are read by models of varying capability. Write every memory so the
least capable future model can apply it without re-deriving the reasoning that
produced it.

## The three-part shape

Every durable memory must make these three parts explicit:

1. **Trigger** — WHEN it applies, as a concrete condition.
2. **Action** — WHAT a future agent should do or verify.
3. **Why** — one line of rationale, provenance, date, or evidence.

For factual `project` or `reference` records, Action should explain how to use
or verify the fact; do not invent an imperative that the evidence does not
support.

Bad (fact without trigger or action):

```text
emoji 不好
```

Good:

```text
Trigger: 產出任何面向使用者的文字（回覆、commit message、文件）時。
Action: 不要使用 emoji。
Why: 使用者 2026-07-05 明確要求。
```

## Writing rules

- One fact per memory. Two facts means two memories; link them with shared tags instead.
- Testable: a reviewer must be able to answer "was this memory followed?" with yes or no.
- Include the exact command, path, or value — not a description of it. `cargo test --workspace --locked` beats「跑完整測試」.
- Convert relative dates ("last week", "目前") to absolute dates before saving.
- Record provenance in `--why` (who said it, in what context) so later retros can re-judge confidence.
- Names: short snake_case, stable across updates. Rename via `supersede`, not delete + save.

`mem save` mechanically lints for five of these — missing tags (`no_tags`), over-long content (`content_long`), relative-date language (`relative_date_language`), vague names (`vague_name`), and paths mentioned outside backticks (`claims_outside_backticks`) — and returns them as `warnings` in the save result (never blocking). Treat a returned warning as ground truth, not a reminder to re-derive: fix it with `mem update` per `SKILL.md` Save Workflow step 7 instead of re-checking the rule from prose.

Wrap every path and command the content asserts in backticks. `mem reconcile` extracts those backtick claims and verifies them against the filesystem, so a backticked claim keeps the memory mechanically checkable for staleness; a plain-text path may still be found heuristically but is second-class.

## Write moments

This batching rule applies to incidental memory candidates discovered while
performing another task. Mid-task, plain `mem prime` and ordinary `mem query`
load context while candidates are collected rather than saved: the final
outcome is not yet known, so early writes tend to be fragmentary.

It does not defer a task whose requested outcome is itself a store mutation,
such as an explicit remember request, import, migration, merge, artifact
change, or setup. Those operations follow `SKILL.md` Safety Gates at the point
they are requested.

Incidental memory writes happen at three moments, each with its own quality
gate:

| Moment | Why it is a good write point |
| --- | --- |
| End of a work unit | outcome is known; candidates can be written once, complete, in trigger/action/why shape |
| Retrospective (`mem retro daily\|weekly`) | bundle supplies changelog/audit/ambiguity context for dedupe and confidence calibration |
| Reconcile pass (`mem reconcile`) | fixes are grounded in a machine-verified report, not impressions |

Within incidental memory capture, two exceptions write immediately:

- The user explicitly asks to remember something (`記住`, "remember this") — user intent overrides batching.
- A task step proves an existing memory wrong — fix it on the spot with `update`/`supersede`/`delete`; a wrong memory misleads every session until corrected.

## Promotion ladder

Move knowledge up one level only when the threshold is met:

| Signal | Action |
| --- | --- |
| Durable fact or preference stated once | `mem save` as memory |
| Same manual procedure performed a 2nd time | propose a `type=workflow` memory (scaffold with `mem workflow new`) |
| Same helper script/template pasted, generated, or run a 2nd time | propose extraction; see `workflow-rules.md` Reusable Scripts |
| Workflow stable and used across 2+ projects | skill candidate — raise it in weekly retro |

## Mechanism over prose

When agents repeatedly violate a remembered rule, do not write a stronger reminder. Convert the rule into a mechanism: a validator, a template, a hook, or a CI check. Prose compliance degrades as model capability drops; mechanisms do not. Record the mechanism's location in the memory and mark the prose version superseded.

## Anti-bloat

- Before saving, `mem query` for an existing similar memory; prefer `update` or `supersede` over a new record.
- Delete or supersede memories proven wrong immediately. A wrong memory is worse than no memory, because agents trust the store.
- Deletion test: if removing this memory would change no future behavior, do not save it.
- Do not save what the repo already records (code structure, git history, CLAUDE.md content).
