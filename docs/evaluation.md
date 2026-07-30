# Retrieval and Agent Behavior Evaluation

mnemark separates two questions that are easy to blur:

1. **Does the deterministic retrieval stack return the intended evidence?**
2. **Does a real coding agent follow the mnemark safety and routing policy?**

The first question is gated in CI against the release binary. The second uses
captured action traces from an external agent harness. A hand-authored reference
trace validates the checker but is never presented as live-agent evidence.

## Retrieval quality gate

Run the checked-in synthetic corpus against an optimized binary:

```bash
scripts/build-release.sh
python3 scripts/evaluate-retrieval.py \
  --report target/retrieval-eval.json
```

The runner creates and deletes an isolated temporary store, saves only synthetic
memories, ingests one synthetic semantic edge, and performs no network access.
The versioned fixture is [`evals/retrieval-v1.json`](../evals/retrieval-v1.json).
It covers:

- exact lexical retrieval;
- Chinese and English mixed-language retrieval;
- fuzzy typo recovery;
- project-scope and source-trust ranking;
- plain priming;
- focused priming with connected graph evidence.

The report records the fixture and binary SHA-256 values, binary version,
platform, Git state, every returned ranking, and these bounded metrics:

| Metric | Meaning |
| --- | --- |
| Query case pass rate | Cases meeting recall, top-result, and forbidden-result assertions |
| Mean recall at K | Fraction of declared relevant memories found within each case limit |
| Mean reciprocal rank | How early the first relevant result appears |
| Prime case pass rate | Plain/focused prime cases containing the required sections or graph nodes |
| Graph evidence rate | Required relations returned with non-empty evidence |

The fixture currently requires every metric to equal `1.0`. These synthetic
scores prevent known behavior from regressing; they are not a claim that every
real query is useful. A ranking change should update implementation or fixture
expectations with a written relevance rationale, never merely lower a threshold
to make CI green.

## Agent behavior trace evaluation

[`evals/agent-behavior-v1.json`](../evals/agent-behavior-v1.json) defines
cross-agent scenarios for:

- session startup with and without an injected context block;
- explicit remember requests and end-of-work batching;
- missing-store recall;
- focused priming target preflight;
- sync push approval;
- risky workflow execution and run recording;
- secret-storage rejection;
- the negative case of ordinary Git sync.

The evaluator consumes captured actions, not prose explanations. Commands are
argv arrays so ordering and flags can be checked without parsing shell strings.
One response file represents one platform/model/adapter combination and must
contain exactly one trace for every checked-in case; omitted cases fail trace
coverage. A shortened response example has this shape:

```json
{
  "schema_version": 1,
  "fixture_sha256": "<sha256-of-evals/agent-behavior-v1.json>",
  "subject": {
    "kind": "live_agent",
    "platform": "pi",
    "model": "provider/model-version",
    "adapter": "harness-version-or-commit",
    "skill_version": "0.9.0",
    "cli_version": "0.9.0"
  },
  "traces": [
    {
      "case_id": "explicit-sync-push",
      "actions": [
        {"kind": "command", "argv": ["mem", "config", "show"]},
        {"kind": "command", "argv": ["mem", "sync", "--dry-run"]},
        {"kind": "approval", "approval": "sync_push"},
        {"kind": "command", "argv": ["mem", "sync", "--push"]}
      ]
    }
  ]
}
```

Score a captured matrix entry with:

```bash
python3 scripts/evaluate-agent-behavior.py \
  --responses /path/to/captured-agent-traces.json \
  --require-live \
  --report target/agent-behavior-eval.json
```

`--require-live` rejects synthetic traces. The checker also rejects a response
whose `fixture_sha256` does not identify the exact fixture being evaluated. The
harness must capture actual tool calls and approvals from the agent runtime; a
model-authored plan or manually edited transcript does not count as live
evidence. Use only synthetic prompts,
isolated stores, placeholder remotes, and sandboxed repositories. Never place
credentials, private memories, or production paths in a response file.

The checked-in
[`evals/agent-behavior-reference-v1.json`](../evals/agent-behavior-reference-v1.json)
is deliberately marked `kind: synthetic`. It demonstrates the trace format and
proves that every assertion can pass:

```bash
python3 scripts/evaluate-agent-behavior.py \
  --responses evals/agent-behavior-reference-v1.json \
  --report target/agent-behavior-reference-report.json
```

Passing that command validates the evaluator and fixture only. It is not
Claude Code, Codex, Pi, Gemini CLI, or OpenCode evidence.

## Evidence status

| Evidence | Current status |
| --- | --- |
| Retrieval fixture | Required in stable CI and native release verification |
| Synthetic agent trace | Checked by Python tests as evaluator self-test |
| Retained live agent matrix | Not yet published; each entry must pass with `--require-live` |

Until a live matrix is retained, documentation may claim that mnemark ships an
agent behavior **evaluation protocol**, not that every supported agent has
passed it.

## Changing retrieval or policy

- Search, tokenizer, ranking, graph-query, or prime changes must run the
  retrieval gate and review the complete report.
- Skill, setup-policy, approval, sync, or workflow-safety changes must update
  affected behavior cases and rerun the checker tests.
- Keep fixture data synthetic and deterministic.
- Add cases for a reproduced failure before changing thresholds or policy.
- Retain live reports with the exact platform, model, adapter, CLI, and skill
  versions so results can be reproduced and superseded honestly.
