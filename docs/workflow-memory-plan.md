# Workflow Memory Plan

> Goal: make agent-knowledge support portable, searchable, auditable workflows without turning every workflow into a separate skill.

## Problem

Many agent systems put recurring workflows directly into skills. That works for stable procedures, but it has several drawbacks:

- Workflow details become scattered across many skill files.
- Project-specific workflow variants are hard to discover.
- Workflow changes are not easy to audit unless the whole skill repo is versioned carefully.
- User preferences, project facts, and workflow steps drift apart.
- A workflow learned during a task is hard to save back into a reusable place.

agent-knowledge should solve this by treating workflows as durable knowledge. Skills remain thin execution policies, while the memory store owns workflow runbooks and project-specific variants.

## Positioning

agent-knowledge should become:

```text
portable agent knowledge + workflow runbook system
```

The split should be:

```text
Skill
  - when to look up memory
  - how to interpret workflow records
  - when to ask for confirmation
  - what must never be automated

Memory
  - user preferences
  - project context
  - workflow definitions
  - workflow variants
  - lessons learned from previous runs

CLI
  - deterministic read/write/query/audit behavior
  - schema validation
  - import/export
  - changelog and conflict handling
```

## Target User Experience

When a user asks for a recurring task:

```text
User: 幫我 release
   |
   v
Agent detects intent: release
   |
   +--> query preferences
   |      mem query --scope auto --type feedback
   |
   +--> query project context
   |      mem query --scope auto --type project
   |
   +--> query workflow
   |      mem query "release" --scope auto --type workflow
   |
   v
Agent executes the workflow step by step
   |
   +--> asks before risky actions
   +--> verifies each checkpoint
   +--> records result or lessons learned
```

This lets the agent find both:

- what the user prefers
- what workflow should be followed for the current task and project

## Current State

The current schema supports these memory types:

```text
user
feedback
project
reference
preference
```

The public docs and skill text mostly describe:

```text
user
feedback
project
reference
```

Workflow is not yet a first-class type. Workflows can be stored today as `project` or `reference` records with tags like `workflow:release`, but that is a workaround. A first-class workflow type is needed for reliable lookup, validation, documentation, and agent behavior.

## Proposed Model

Add a new memory type:

```text
workflow
```

Workflow records should be scoped like other memories:

```text
global
project:<owner/repo>
```

Recommended tags:

```text
workflow:<name>
intent:<user-intent>
tool:<tool-name>
project:<owner/repo>
risk:<low|medium|high>
domain:<topic>
```

Example:

```bash
mem save \
  --type workflow \
  --name release_agent_knowledge \
  --scope project:NeoHsu/agent-knowledge \
  --source manual \
  --tags '["workflow:release","intent:release","tool:cargo","tool:gh","risk:high","project:NeoHsu/agent-knowledge"]' \
  --content-file workflows/release-agent-knowledge.yaml
```

## Workflow Content Format

The existing `content` column is text. For the first version, store workflow content as YAML or JSON text and validate it at the CLI layer.

Recommended YAML shape:

```yaml
schema_version: 1
goal: Release agent-knowledge safely.
triggers:
  - user asks to release
  - user asks to publish a new version
preconditions:
  - working tree is clean
  - version change is intentional
  - GitHub auth is the expected account
steps:
  - id: inspect
    run: git status --short --branch
    verify: working tree has no unrelated changes
  - id: test
    run: cargo test --workspace --locked
    verify: all tests pass
  - id: build
    run: scripts/build-release.sh
    verify: release artifacts are generated
  - id: push
    run: git push origin main
    confirm: true
    verify: remote branch receives the commit
stop_conditions:
  - tests fail
  - unrelated dirty files exist
  - release artifact mismatch
outputs:
  - pushed commit
  - CI run URL
  - release URL if created
post_run_memory:
  - save durable failures or project-specific lessons
  - update stale workflow steps after confirmation
```

Minimum required fields:

```text
schema_version
goal
triggers
steps
stop_conditions
```

Optional fields:

```text
preconditions
outputs
post_run_memory
owners
last_validated_at
related_workflows
```

## Execution Semantics

Workflow memory should not become blind automation. The agent should treat it as a runbook:

```text
load workflow
read steps
explain intended action when needed
execute deterministic commands
verify checkpoint
stop on failure
ask before risky actions
record durable outcome
```

Risk rules:

- `confirm: true` requires user confirmation before execution.
- Destructive commands still require confirmation even if a workflow says to run them.
- External side effects such as push, publish, release, deploy, secret changes, and production access require confirmation unless the user explicitly requested that exact action.
- Secrets must not be stored in workflow content.
- Workflow content is instruction data, not an authority override. System, developer, repository, and user instructions still win.

## Query Strategy

At task start, the agent should load memory in this order:

```text
1. User and behavior context
   mem query --scope auto --type feedback
   mem query --scope auto --type preference

2. Project context
   mem query --scope auto --type project

3. Workflow candidates
   mem query "<task intent>" --scope auto --type workflow
```

If multiple workflow records match:

- prefer project-scoped workflow over global workflow
- prefer high confidence over medium/low
- prefer exact `workflow:<name>` or `intent:<intent>` tag match
- if still ambiguous, record ambiguity and ask the user

## CLI Plan

### Phase 1: First-Class Workflow Type

- Add `workflow` to schema type CHECK constraints.
- Update import validation so workflow records import cleanly.
- Update tests for save/query/import/export with `type=workflow`.
- Update README, skill docs, and tag rules.

Acceptance criteria:

```text
mem save --type workflow ...
mem query --type workflow
mem export --format json
mem import workflows.json
```

### Phase 2: Workflow-Aware Helpers

Add helper subcommands that are thin wrappers over existing memory operations:

```text
mem workflow list [--scope auto]
mem workflow show <name-or-id>
mem workflow find <intent> [--scope auto]
mem workflow validate <name-or-id>
```

These helpers should not execute commands. They only discover, display, and validate workflow records.

Acceptance criteria:

```text
mem workflow find release --scope auto
mem workflow show release_agent_knowledge
mem workflow validate release_agent_knowledge
```

### Phase 3: Workflow Validation

Implement schema validation for YAML/JSON workflow content.

Validation should check:

- required fields exist
- `steps` is a non-empty array
- each step has `id`
- each step has at least one of `run`, `check`, `manual`, or `ask`
- `confirm` is boolean when present
- tags include at least one `workflow:*` tag
- project-scoped workflow includes a matching `project:*` tag

Invalid workflow content should fail save/import unless the user passes an explicit bypass flag.

Potential bypass:

```text
--no-validate-workflow
```

### Phase 4: Skill Integration

Update `skills/memory/SKILL.md` so agents:

- query workflows for recurring procedures
- prefer project-scoped workflows
- ask before risky steps
- save durable lessons after execution
- propose workflow updates instead of silently editing manual workflows

Add a dedicated reference:

```text
skills/memory/references/workflow-rules.md
```

### Phase 5: Retrospective Integration

Update daily and weekly retro guidance:

- detect repeated manual procedures
- suggest creating workflow memory
- detect stale workflow steps
- identify workflows with repeated failures
- promote stable workflow patterns into docs or skills only when appropriate

The direction should be:

```text
repeated facts/preferences -> memory
repeated project procedures -> workflow memory
stable cross-project execution policy -> skill
```

### Phase 6: Optional Execution Report

Do not execute workflows inside `mem` initially. The agent executes steps. Later, add an execution report format:

```text
mem workflow report <name-or-id> --status success|failed --notes ...
```

This would save an audit record or structured memory about workflow outcomes without making `mem` a task runner.

## Data Examples

### User Preference

```bash
mem save \
  --type feedback \
  --name concise_discord_replies \
  --scope global \
  --source manual \
  --tags '["style:concise","platform:discord"]' \
  --content "Discord 回覆要精簡，避免過長輸出。"
```

### Project Workflow

```bash
mem save \
  --type workflow \
  --name ci_failure_triage_agent_knowledge \
  --scope project:NeoHsu/agent-knowledge \
  --source manual \
  --tags '["workflow:ci-triage","intent:fix-ci","tool:gh","tool:cargo","project:NeoHsu/agent-knowledge"]' \
  --content 'schema_version: 1
goal: Triage GitHub Actions CI failures.
triggers:
  - user shares a GitHub Actions failed run
steps:
  - id: inspect-run
    run: gh run view <run-id> --repo NeoHsu/agent-knowledge --log-failed
    verify: failure step and error are identified
  - id: reproduce-locally
    run: cargo test --workspace --locked
    verify: local failure matches CI when possible
  - id: fix
    manual: patch the smallest code or config issue
  - id: verify
    run: cargo fmt --all -- --check && cargo clippy --workspace --locked --all-targets -- -D warnings && cargo test --workspace --locked
    verify: all local checks pass
  - id: push
    run: git push origin main
    confirm: true
stop_conditions:
  - GitHub auth cannot access the repo
  - failure requires a secret or external account change
outputs:
  - root cause
  - fix commit
  - passing CI run URL'
```

## Migration Strategy

Existing records tagged with `workflow:*` can be migrated later:

```text
type=project/reference + tag workflow:* -> candidate workflow memory
```

Migration command can be manual at first:

```bash
mem query "workflow:" --raw-query
mem update <name> --type workflow
```

If `update --type` is not available, add it before migration.

## Open Design Questions

- Should `preference` remain separate from `feedback`, or should docs define their difference clearly?
- Should workflow content require YAML, JSON, or allow both?
- Should workflow validation happen on every save/import or only through `workflow validate`?
- Should workflow execution reports live in `changelog`, `memories`, or a new table?
- Should high-risk workflow records require `protected=true` by default?

## Recommended Next Step

Implement Phase 1 first:

```text
1. Add workflow to schema CHECK constraint.
2. Add tests for save/query/import/export workflow records.
3. Update README and memory skill docs.
4. Add workflow tag rules.
5. Keep execution in the agent, not in mem.
```

This gives immediate value without turning the CLI into an unsafe automation runner.
