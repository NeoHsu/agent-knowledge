# Performance Baseline

`mnemark` includes an isolated, deterministic scale benchmark for regression and
production-canary work:

```bash
scripts/build-release.sh
REPORT_FILE=/tmp/mnemark-benchmark.json scripts/benchmark-scale.sh
```

The benchmark generates mixed Prime-visible and reference memories, creates a
fresh store for every import sample, and measures the optimized release binary.
Import uses `--summary-only` so the measurement covers validation, persistence,
and indexing rather than retaining a per-item response report. Interactive
operations receive one warmup by default and run in separate processes while
retaining the operating system cache. Scale order is seed-randomized. Every
measured operation also runs a correctness assertion.

The CSV summary reports median, p95 (only when at least 20 samples exist),
minimum, maximum, peak RSS, and input/output sizes. The JSON report additionally
records:

- platform, Rust and Python versions;
- Git commit and dirty state;
- binary and benchmark-script SHA-256 hashes;
- seed, execution order, run counts, and cache model;
- database, Tantivy index, and bundle sizes;
- bundle snapshot, validation, hashing, archive, and install stage timings.

## Baseline publication policy

This `main` branch currently targets source version `0.7.0`, while the table
below remains the latest retained clean, published-release baseline. Do not
replace it with a dirty-tree or untagged development run. A 0.7 baseline should
be added only after the release commit is clean and the tagged platform binary
has run the same retained benchmark protocol; record the tag/commit, binary and
script hashes, platform, cache model, and sample counts. Development and 100k
capacity-canary reports may be discussed as observations but are not release
service-level objectives.

## 0.6.0 local baseline

Measured on 2026-07-13 with an Apple M2 Max (`arm64`), macOS 26.5.1, Rust
1.97.0, bundled SQLite, and the optimized release profile. Interactive values
are p50 / p95 across 20 samples; maintenance values are p50 across 5 fresh or
repeated samples as appropriate. The retained report uses the published v0.6.0
macOS arm64 artifact from clean release commit
`b84506e41544a68f27b2984d5e1f6ded70b756db`; its binary SHA-256 is
`a7abfb929d9e1e60c1069c7e65aaf711af9de69cf99580ad2ed982171404ba13`.

| N | Import | Query p50/p95 | Prime p50/p95 | Graph | Bundle |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 100 | 284 ms | 30 / 33 ms | 44 / 52 ms | 64 ms | 63 ms |
| 1,000 | 476 ms | 30 / 35 ms | 45 / 54 ms | 528 ms | 324 ms |
| 10,000 | 2.29 s | 50 / 58 ms | 76 / 81 ms | 5.29 s | 2.96 s |

These figures are a regression baseline, not a cross-machine service-level
objective. Query uses bounded adaptive over-fetch and deterministic reranking;
`[query].candidate_limit` defaults to 10,000 and may be set from 200 to 100,000
when a larger lexical candidate set is required. Prime applies ranking and
`LIMIT` in SQLite. Import uses 500-row chunk transactions with per-item
savepoints. Bundle export includes an online SQLite snapshot, full durable-field
secret scan, artifact validation, SHA-256, and gzip archive creation.

The earlier benchmark populated only `reference` memories, so its Prime numbers
did not exercise Prime retrieval and are invalid as a Prime scale baseline.

The old and corrected tables are not a controlled A/B comparison: the dataset,
sample count, warmup/cache protocol, correctness checks, and benchmark script
changed in addition to the binary. Their differences provide historical
context, not an exact percentage attributable to code changes. A controlled
speedup claim would require both binaries to run the same corrected dataset and
protocol in interleaved repetitions. The transaction and bundle-stage profiles
still independently identify the optimized bottlenecks without relying on the
cross-table ratio.

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
binary hash, commit, cache model, and correctness checks. The benchmark rejects
stale binaries whose version differs from `Cargo.toml`; use
`ALLOW_VERSION_MISMATCH=1` only for an intentional controlled A/B run.

A 100,000-memory capacity canary is available but is not yet a release baseline:

```bash
SCALES="100000" \
INTERACTIVE_RUNS=5 \
MAINTENANCE_RUNS=1 \
REPORT_FILE=/tmp/mnemark-100k.json \
scripts/benchmark-scale.sh
```

Do not infer a service-level objective from that canary. Review correctness,
peak RSS, database/index size, graph rebuild latency, and bundle stages before
raising the documented support envelope beyond 10,000 memories.
