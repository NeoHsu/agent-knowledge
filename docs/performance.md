# Performance Baseline

`mnemark` includes an isolated, deterministic scale benchmark for regression and
production-canary work:

```bash
scripts/build-release.sh
REPORT_FILE=/tmp/mnemark-benchmark.json scripts/benchmark-scale.sh
```

The benchmark generates mixed Prime-visible and reference memories, creates a
fresh store for every import sample, and measures the optimized release binary.
Interactive operations receive one warmup by default and run in separate
processes while retaining the operating system cache. Scale order is
seed-randomized. Every measured operation also runs a correctness assertion.

The CSV summary reports median, p95 (only when at least 20 samples exist),
minimum, maximum, peak RSS, and input/output sizes. The JSON report additionally
records:

- platform, Rust and Python versions;
- Git commit and dirty state;
- binary and benchmark-script SHA-256 hashes;
- seed, execution order, run counts, and cache model;
- database, Tantivy index, and bundle sizes;
- bundle snapshot, validation, hashing, archive, and install stage timings.

## 0.6.0 local baseline

Measured on 2026-07-13 with an Apple M2 Max (`arm64`), macOS 26.5.1, Rust
1.97.0, bundled SQLite, and the optimized release profile. Interactive values
are p50 / p95 across 20 samples; maintenance values are p50 across 5 fresh or
repeated samples as appropriate. The retained report identifies clean commit
`a0e03472a43e79368d7c665a92f7bc00cb9cf281` and release-binary SHA-256
`76773041bcfbc86ac06d72169cf1e393f9d5be84e6ce13137426bcc3dbf59feb`.

| N | Import | Query p50/p95 | Prime p50/p95 | Graph | Bundle |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 100 | 416 ms | 19 / 24 ms | 29 / 34 ms | 41 ms | 46 ms |
| 1,000 | 442 ms | 21 / 24 ms | 30 / 33 ms | 363 ms | 244 ms |
| 10,000 | 1.66 s | 40 / 47 ms | 52 / 56 ms | 3.59 s | 2.27 s |

These figures are a regression baseline, not a cross-machine service-level
objective. Query uses bounded adaptive over-fetch and deterministic reranking;
`[query].candidate_limit` defaults to 10,000 and may be set from 200 to 100,000
when a larger lexical candidate set is required. Prime applies ranking and
`LIMIT` in SQLite. Import uses 500-row chunk transactions with per-item
savepoints. Bundle export includes an online SQLite snapshot, full durable-field
secret scan, artifact validation, SHA-256, and gzip archive creation.

The earlier benchmark populated only `reference` memories, so its Prime numbers
did not exercise Prime retrieval. It also measured the intentionally throttled
SQLite backup loop that previously dominated bundle export. Do not compare those
figures directly with this corrected mixed-data baseline.

For bounded development or CI runs:

```bash
SCALES="100 1000" \
INTERACTIVE_RUNS=2 \
MAINTENANCE_RUNS=1 \
WARMUP_RUNS=0 \
REPORT_FILE=/tmp/mnemark-benchmark.json \
scripts/benchmark-scale.sh
```

Run the default 20 interactive samples and 3-5 maintenance samples before
publishing performance claims. Keep the JSON report so results remain tied to a
binary hash, commit, cache model, and correctness checks.
