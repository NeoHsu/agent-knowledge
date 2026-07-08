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

Three planes: you, the coding agent (wired by `mem setup`), the `mem` binary,
and the active knowledge store.

```text
                          YOU (developer)
                               |
                     "記住…" / "幫我存" / recall / retro
                               v
 +-------------------------------------------------------------------+
 |  CODING AGENT   Claude Code / Codex / Gemini CLI / opencode       |
 |                                                                   |
 |  `mem setup <platform>` wires 3 idempotent layers:                |
 |    [1] policy block  -> CLAUDE.md / AGENTS.md                      |
 |          "use mem, not built-in memory; run `mem prime` at start" |
 |    [2] session hook  -> SessionStart runs `mem prime`             |
 |    [3] skill files   -> skills/mnemark/ (SKILL.md + references)   |
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
 |    .git/          <- versioned and pushed by `mem sync`           |
 +-------------------------------------------------------------------+
```

Store discovery order (the runtime resolves the first that matches): `--home`
-> cwd with `schema/memory-schema.sql` -> executable-near repo -> `MNEMARK_HOME`
-> user config `knowledge_home` -> `~/.mnemark`. `prime`, `doctor`, and `sync`
skip the cwd/executable steps so a source checkout is never mistaken for a store.

## 2. Session lifecycle — the core loop

What a normal coding session looks like once mnemark is wired in.

```text
  session start
       |
       v
  [ mem prime ]  read-only, budget-capped
       |         loads user / feedback / preference / project + workflow names
       |         (skip if the SessionStart hook already injected the block)
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
  [ mem sync ]  commit + merge + push the store
```

## 3. Inside `mem save`

Every write runs the same pipeline. Warnings never block the save.

```text
  mem save --type … --scope … --tags … --content "…"
       |
       v
  strip secrets    (API keys, bearer tokens, password=/secret= assignments)
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
       |
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
        rank each section:  confidence  ->  access_count  ->  recency
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
with `mem workflow show`. Use `mem query … --no-touch` for read-only loads that
must not bump access counters.

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
  mem workflow record <name> --result success|failure --note "…"
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
 ~/.mnemark/.git  --- push --->      (origin)      <--- push ---  ~/.mnemark/.git
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

## 8. Usage scenarios at a glance

Each row is an end-to-end scenario mapped to its commands and the diagram above
that explains it.

```text
 SCENARIO                     COMMANDS (in order)                                   SEE
 --------                     -------------------                                   ---
 First-time wiring            mem init                                              #1
   (new machine / platform)   mem setup claude-code   (or codex/gemini-cli/…)
                              mem doctor              (verify all layers)

 Everyday coding session      mem prime  ->  work  ->  mem save  ->  mem sync       #2

 Save a preference/decision   mem save --type feedback --source manual …            #3
                              (fix any returned warnings with mem update)

 Recall for a task            mem query "<keywords>" --scope auto --format compact  #4
                              mem query "<intent>" --type workflow --scope auto

 Run a recurring procedure    mem workflow find "<intent>"                          #5
                              mem workflow show <name> --checklist
                              mem workflow record <name> --result success|failure

 Reusable helper script       repo-specific  -> scripts/*.sh                        #6
                              cross-project  -> mem artifact add … ; artifact check

 Daily / weekly retro         mem retro daily     (missed facts, stale, ambiguity)  #5,#6
                              mem retro weekly    (dedupe, confidence, candidates)

 Move a store between hosts    mem bundle export store.tgz                          #7
                              mem bundle import store.tgz --merge

 Sync across machines          mem sync            (git remote configured)          #7
                              mem ambiguity list --pending   (resolve conflicts)

 Health check / repair         mem doctor  ;  mem audit --fix  ;  mem reindex       #1,#3

 Stale-memory reconcile        mem reconcile --scope auto   (verify path/command    #1,#3
                              claims in memories against the filesystem, read-only)
```

## Where to go next

| Need | Read |
| --- | --- |
| Install, init, first save, wire agents | [`getting-started.md`](getting-started.md) |
| Store discovery, config priority, layout | [`runtime-model.md`](runtime-model.md) |
| Workflows, artifacts, bundles, retros | [`workflows.md`](workflows.md) |
| Every command and flag | [`cli-guide.md`](../skills/mnemark/references/cli-guide.md) |
| How agents execute runbooks safely | [`workflow-rules.md`](../skills/mnemark/references/workflow-rules.md) |
