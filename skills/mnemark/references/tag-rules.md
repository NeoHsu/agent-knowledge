# Tag Rules

Use tags to create lightweight, deterministic relationships. Tags should be machine-filterable and readable by agents.

## Format

Use `type:value` and lowercase values where possible.

- `person:alice`
- `project:example/ot-product`
- `domain:ot-security`
- `tool:cargo`
- `tool:github`
- `style:no-emoji`
- `decision:pr-size`
- `workflow:release`
- `intent:release`
- `risk:secret-leak`
- `risk:high`

## Extraction

- Prefer 2-6 tags per memory.
- Include a project tag for project-scoped memory.
- Include a domain tag for reusable technical knowledge.
- Include `style:*` for communication or coding preferences.
- Include `workflow:*` for workflow memories.
- Include `intent:*` for user intents that should retrieve a workflow.
- Include `risk:low`, `risk:medium`, or `risk:high` when a workflow contains side effects or safety checkpoints.
- Avoid vague tags such as `important`, `misc`, or `note`.

## Scope vs Tags

`scope` controls loading. Tags support filtering and review. A project memory should normally have both:

```json
{
  "scope": "project:example/ot-product",
  "tags": ["project:example/ot-product", "decision:deploy"]
}
```

Workflow memories must include at least one `workflow:*` tag. Project-scoped workflow memories should also include the exact matching project tag:

```json
{
  "scope": "project:example/ot-product",
  "tags": ["workflow:deploy", "intent:deploy", "project:example/ot-product", "risk:high"]
}
```
