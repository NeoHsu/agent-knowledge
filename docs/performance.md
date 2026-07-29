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

CI retains the JSON and CSV reports and applies
`scripts/benchmark-guardrails.json` through
`scripts/check-benchmark-regression.py`. These generous, cross-runner limits
catch catastrophic regressions and hangs; they are not performance claims.
For meaningful regression percentages, compare a candidate with a retained
report from the same platform and unchanged benchmark script:

```bash
python3 scripts/check-benchmark-regression.py \
  --report /tmp/candidate.json \
  --guardrails scripts/benchmark-guardrails.json \
  --baseline /tmp/baseline.json \
  --max-regression-percent 35
```

The checker rejects platform or benchmark-script mismatches by default. Override
those checks only for an explicitly reviewed protocol comparison.

## Release baseline: v0.8.0

This branch identifies source version `0.9.0`. The retained v0.8.0 release
baseline was captured on 2026-07-28 from clean tag
`v0.8.0`, commit `c7026b0ace895a404e327d8245565f67c3b4c265`. It used an
optimized local build of `mem` version `0.8.0` on an Apple M2 Max (`arm64`),
macOS 26.5.2, Rust 1.97.0, Python 3.14.6, and bundled SQLite.

Reproducibility identifiers:

- binary SHA-256: `390f2dd191378c49c7b1e4eafdeffec66b4f29763d4d4849f986a80a6685e70a`;
- benchmark script SHA-256: `dc951fc976d114dd365d0f9624bb00adec88eed336e0b954e2077b7402e904c9`;
- JSON report: [`benchmarks/v0.8.0-macos-arm64.json`](benchmarks/v0.8.0-macos-arm64.json)
  (SHA-256 `c441c67b56377f6215bd03487b00ef954c8d172a82edd0ae4b981a7ed2e50bf2`);
- CSV summary: [`benchmarks/v0.8.0-macos-arm64.csv`](benchmarks/v0.8.0-macos-arm64.csv)
  (SHA-256 `37dcb23277bcdc5fc961fa8ad61f4368a6dc1658c3e105627d913e1722d1bc86`).

The seed was 42 and the randomized execution order was 1,000, 100, then 10,000.
Interactive operations used one warmup and 20 measured processes, retaining the
operating-system cache. Import used three fresh initialized stores per scale.
Graph rebuild and bundle export used three repetitions against the same
populated store. Consequently, p95 is reported only for Query and Prime; the
three-sample maintenance figures are medians, not tail-latency claims.

| N | Import p50 | Query p50/p95 | Prime p50/p95 | Graph p50 | Bundle p50 |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 100 | 274 ms | 30.95 / 32.80 ms | 34.03 / 38.00 ms | 35.83 ms | 59.95 ms |
| 1,000 | 477 ms | 30.66 / 34.91 ms | 33.00 / 35.43 ms | 308.07 ms | 323.84 ms |
| 10,000 | 2.124 s | 44.96 / 54.04 ms | 54.77 / 68.23 ms | 3.203 s | 2.910 s |

The retained JSON is the source of truth for unrounded values, minimum/maximum,
peak RSS, database/index/bundle sizes, and bundle stage timings. These numbers
are a same-platform regression baseline, not a cross-machine service-level
objective or a claim about every workload.

### Post-refactor regression verification

After the behavior-preserving refactors, clean development commit
`0b5a6e4813cc41c4143d116ca876d07b11b39384` was benchmarked twice with the
same platform, script hash, seed, cache model, and run counts. Both reports
passed the portable guardrails and the 35% same-baseline catastrophic gate.
Differences above 10% were reviewed rather than treated as speedups: the same
candidate binary varied by 52.8% between its two three-sample 1,000-row import
medians, while all candidate medians still passed the gate. This confirms that
small-sample host variance is material and that the clean tagged report above,
not an untagged development run, remains the published baseline.

## Historical baseline: v0.6.0

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

## CI smoke and bounded development runs

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

## 100,000-memory capacity canary

A 100,000-memory capacity canary is available but is not a release baseline:

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
