# Overview — How mnemark Works

Start here for the big picture. This page uses ASCII diagrams to show how the
pieces fit and how each usage scenario flows end to end. For command details see
[`skills/mnemark/references/cli-guide.md`](../skills/mnemark/references/cli-guide.md);
for store discovery and layout see [`runtime-model.md`](runtime-model.md); for
workflow/artifact/bundle/retro specifics see [`workflows.md`](workflows.md).

One idea underlies everything: **`mem` does deterministic operations; the agent
keeps the judgment.** The CLI saves, searches, validates, and versions. It never
executes a workflow, never decides what is durable, never overrides your
instructions.

## 1. System map

Four layers participate: you, the coding agent (wired by `mem setup`), the
`mem` binary, and the active knowledge store.

```text
                          YOU (developer)
                               |
                     "記住…" / "幫我存" / recall / retro
                               v
 +-------------------------------------------------------------------+
 |  CODING AGENT   Claude Code / Codex / pi / Gemini / opencode      |
 |                                                                   |
 |  `mem setup <platform>` wires up to 3 user-level layers:          |
 |    [1] policy block  -> contract first; process-read-only prime    |
 |    [2] session hook  -> same gates where native hooks exist        |
 |    [3] shared skill  -> supported platforms use                   |
 |          ~/.agents/skills/mnemark directly or through links       |
 +-------------------------------------------------------------------+
                               |
                               | shells out to
                               v
 +-------------------------------------------------------------------+
 |  mem   single Rust binary, schema embedded                        |
 |  save / query / prime / workflow / artifact / bundle / sync ...   |
 +-------------------------------------------------------------------+
                               |
                               | reads / writes the ACTIVE store
                               v
 +-------------------------------------------------------------------+
 |  ACTIVE KNOWLEDGE STORE    (~/.mnemark | MNEMARK_HOME | --home)   |
 |                                                                   |
 |    memory.db      <- source of truth: SQLite + changelog          |
 |    index/         <- rebuildable full-text: Tantivy  (mem reindex)|
 |    manifest.toml  <- artifact metadata                            |
 |    artifacts/     <- portable helper scripts / templates / ...    |
 |    config.toml    <- store-local defaults                         |
 |    .git/          <- optional private sync repository             |
 +-------------------------------------------------------------------+
```

Store discovery order is runtime-only for every command: `--home` -> `MNEMARK_HOME` -> user config `knowledge_home` -> `~/.mnemark`. A source checkout and executable parent are never selected implicitly.

## 2. Session lifecycle — the core loop

What a normal coding session looks like once mnemark is wired in.

```text
  session start
       |
       +-- context block already injected by guarded hook? -- yes --> use it
       |
       no
       v
  [ mem contract --skill-version <exact> ]
       |
       v
  [ mem --read-only prime ]  budget-capped
       |                     loads user / feedback / preference / project
       |                     plus workflow names
       v
  agent now holds durable "prior knowledge" in context
       |
       v
  +--> do the task <-------------------------------+
  |        |                                       |
  |        | need a recurring runbook?             |
  |        v                                       |
  |   mem workflow find "<intent>"                 |
  |   mem workflow show <name> --checklist         |
  |        |                                       |
  |        +---------------------------------------+
  |
  |  learned something durable? (correction / decision / procedure)
  v
  [ mem save ]  content shape: Trigger / Action / Why
       |
       v
  work unit complete
       |
       v
  offer sync; inspect [ mem sync --dry-run ] first
       |
       v
  [ mem sync ]  local checkpoint + optional fetch/merge (no push by default)
       |
       +--> mem sync --push only after explicit user approval
```

## 3. Inside `mem save`

Every memory save, including each valid import item, uses the same persistence
rules. Lint warnings do not block a save; validation and security errors do.

```text
  mem save --type … --scope … --tags … --content "…"
       |
       v
  secret gate across durable fields
       | detected + no --redact-secrets -> reject without persistence
       | explicit --redact-secrets      -> replace with [REDACTED]
       v
  provenance gate  (--source manual requires --user-confirmed)
       |
       v
  type/content validation  (workflow schema is strict unless explicitly bypassed)
       |
       v
  duplicate / similar check
       |  exact name  -> duplicate_found  -.
       |  high overlap -> similar_found    -+-> caller: skip / update / supersede
       |  unique                            |     (--force overwrites, subject to
       v                                     |      source-trust rule)
  write in one SQLite txn:  memory row + changelog row
       |
       v
  Tantivy index upsert
       | failure after DB commit -> mark index stale; SQLite remains authoritative
       | --json-errors -> index_stale_after_write + durable_write_committed=true
       v
  lint (non-blocking):  no_tags · content_long · relative_date_language · vague_name
                        · claims_outside_backticks
       |
       v
  result JSON { status: saved, id, version, warnings? }   -> caller fixes warnings
```

Source sets confidence and trust: `manual` = high/protected, `agent` = medium,
`daily_retro` / `weekly_retro` = low.

## 4. Inside `mem prime` / `mem query` — scoping and budget

```text
  --scope auto
       |
       +-- global -----------------------------+
       +-- project:<owner/repo> ---------------+   (detected from git remote)
                                               |
                                               v
        rank each section: project scope -> source trust -> confidence -> telemetry -> recency
                                               |
        section priority:  user > feedback > preference > project > workflow
                                               |
                                               v
        fit into --budget chars:  truncate entries, then drop lowest-priority
                                               |
                                               v
                                   compact context block
```

Workflows prime with their `goal` line only; load the full runbook on demand
with `mem workflow show`. `mem prime --focus "<task>"` additionally ensures the
local graph is current and adds budget-capped, scored graph neighborhood context
for task starts that need relationship context. Focused and graph-dependent
reads may update only rebuildable graph/index state. Ordinary query is lock-free
and no-touch by default; `--touch` is the explicit telemetry-writing mode.
Query relevance combines lexical score, source trust, confidence, scope
specificity, and recency; inspect it with `--explain-score`.

## 5. Workflow lifecycle — data, not automation

A workflow memory is a validated YAML runbook. `mem` finds, shows, and validates
it; **the agent executes the steps** and stops at the safety gates.

```text
  recurring task
       |
       v
  mem workflow find "<intent>"    prefer: project scope > high confidence > tag match
       |
       v
  mem workflow show <name> --checklist
       |
       v
  AGENT runs steps itself  (mem never runs them)
       |
       |   step has confirm:true, OR push / publish / deploy / secret change?
       |            |
       |            +--> ASK the user first
       |
       |   check fails / unrelated dirty tree / missing auth / unsafe state?
       |            |
       |            +--> STOP  (stop_conditions)
       v
  record exactly one outcome:
    mem workflow record <name> --result success --note "…"
    mem workflow record <name> --result failure --note "…"
       |
       v
  feeds  mem retro daily|weekly   (spot repeatedly failing or stale runbooks)
```

## 6. Promotion ladder — moving knowledge up only when earned

```text
  stated once         performed 2nd time      pasted 2nd time        stable across 2+ projects
  durable fact  --->    workflow memory  --->   repo scripts/    --->   skill
  / preference           (runbook)              or artifacts/          (raise in weekly retro)
      |                     |                       |                       |
  mem save             mem workflow new        scripts/*.sh /          skills/<name>/
  (feedback/…)         (type=workflow)         artifacts/scripts/*
```

Rule of thumb: prose that agents keep violating should become a *mechanism*
(validator / template / hook / CI check), not a louder reminder.

## 7. Multi-machine sync — git moves bytes, `mem` resolves meaning

```text
   Machine A                    remote (private git)                  Machine B
 ~/.mnemark/.git  -- --push -->      (origin)      <-- --push --  ~/.mnemark/.git
        ^                               |  |                              ^
        |                               v  v                              |
   mem sync  --- fetch / merge ---------+  +--------- fetch / merge --- mem sync
        |
        |  both machines changed memory.db?
        v
   binary conflict  ->  keep local db, merge the remote copy via `mem merge` logic
        |
        v
   same-name content conflicts  ->  pending ambiguity records  (no rows lost)
        |
        v
   mem ambiguity list --pending   ;   mem ambiguity resolve <id> …
```

Without a configured remote, `mem sync` commits locally and reports
`local_only`. It refuses to commit into an enclosing repo — the store root must
be its own git repository.

## Next steps

Use the task-oriented [documentation index](README.md) for installation,
runtime, workflow, graph, compatibility, production, and development guides.
The complete command surface for agents is the
[CLI guide](../skills/mnemark/references/cli-guide.md).

Changes to store discovery, setup policy/hooks, save normalization, prime/query,
workflow rendering, or sync semantics invalidate the corresponding diagram.
Run `mise run contract:check` and the affected integration-test module before
updating this overview.
