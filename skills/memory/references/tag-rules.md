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
- `risk:secret-leak`

## Extraction

- Prefer 2-6 tags per memory.
- Include a project tag for project-scoped memory.
- Include a domain tag for reusable technical knowledge.
- Include `style:*` for communication or coding preferences.
- Avoid vague tags such as `important`, `misc`, or `note`.

## Scope vs Tags

`scope` controls loading. Tags support filtering and review. A project memory should normally have both:

```json
{
  "scope": "project:example/ot-product",
  "tags": ["project:example/ot-product", "decision:deploy"]
}
```
