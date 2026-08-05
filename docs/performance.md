# Performance and Capacity Evidence

This page documents the benchmark protocol, the published release baseline, and
the retained capacity canary for source version `0.10.1`. Benchmark evidence is
bound to the exact binary, commit, protocol hash, platform, and report; it is not
a cross-machine service-level objective.

## Benchmark protocol

Build an optimized binary and run an isolated deterministic benchmark:

```bash
scripts/build-release.sh
REPORT_FILE=/tmp/mnemark-benchmark.json scripts/benchmark-scale.sh
```

The benchmark creates temporary stores, imports mixed Prime-visible and
reference memories, and measures the optimized binary. Every operation includes
a correctness assertion. Stores are removed on exit and no network access is
required.

Protocol v2 records:

- platform, Rust and Python versions;
- Git commit and dirty state;
- binary and combined protocol SHA-256 hashes;
- seed, execution order, exact sample schedule, run counts, and cache model;
- median, median absolute deviation, deterministic bootstrap interval, and p95
  when at least 20 samples exist;
- peak RSS and database, index, bundle, input, and output sizes;
- bundle snapshot, validation, hashing, archive, and install-stage timings.

Bundle hashing overlaps gzip archive creation after validation. `hash_ms` and
`archive_ms` are concurrent wall-clock observations and must not be added to
infer total latency.

Portable guardrails in `scripts/benchmark-guardrails.json` catch hangs and
catastrophic regressions; they are intentionally too broad for speedup claims.
For a meaningful percentage comparison, execute both binaries in one seeded,
interleaved run:

```bash
REPORT_FILE=/tmp/candidate-v2.json \
BASELINE_REPORT_FILE=/tmp/baseline-v2.json \
BASELINE_CSV_FILE=/tmp/baseline-v2.csv \
BASELINE_MEM_BIN=/path/to/baseline/mem \
BASELINE_GIT_COMMIT=<baseline-commit> \
scripts/benchmark-scale.sh > /tmp/candidate-v2.csv

python3 scripts/check-benchmark-regression.py \
  --report /tmp/candidate-v2.json \
  --guardrails scripts/benchmark-guardrails.json \
  --baseline /tmp/baseline-v2.json \
  --max-regression-percent 35
```

The checker requires matching platform, protocol hash, run counts, peer binary
identity, and sample schedule. Schema-v1 reports remain historical data but are
not directly comparable with protocol v2.

## Published release baseline: v0.8.0

The retained published baseline was captured on 2026-07-28 from clean tag
`v0.8.0`, commit `c7026b0ace895a404e327d8245565f67c3b4c265`, on an Apple
M2 Max (`arm64`) with macOS 26.5.2, Rust 1.97.0, Python 3.14.6, and bundled
SQLite.

Reproducibility identifiers:

- binary SHA-256:
  `390f2dd191378c49c7b1e4eafdeffec66b4f29763d4d4849f986a80a6685e70a`;
- benchmark script SHA-256:
  `dc951fc976d114dd365d0f9624bb00adec88eed336e0b954e2077b7402e904c9`;
- [JSON report](benchmarks/v0.8.0-macos-arm64.json), SHA-256
  `c441c67b56377f6215bd03487b00ef954c8d172a82edd0ae4b981a7ed2e50bf2`;
- [CSV summary](benchmarks/v0.8.0-macos-arm64.csv), SHA-256
  `37dcb23277bcdc5fc961fa8ad61f4368a6dc1658c3e105627d913e1722d1bc86`.

The seed was 42 and execution order was 1,000, 100, then 10,000. Query and Prime
used one warmup and 20 measured processes. Import used three fresh stores per
scale; graph rebuild and bundle export used three repetitions.

| N | Import p50 | Query p50/p95 | Prime p50/p95 | Graph p50 | Bundle p50 |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 100 | 274 ms | 30.95 / 32.80 ms | 34.03 / 38.00 ms | 35.83 ms | 59.95 ms |
| 1,000 | 477 ms | 30.66 / 34.91 ms | 33.00 / 35.43 ms | 308.07 ms | 323.84 ms |
| 10,000 | 2.124 s | 44.96 / 54.04 ms | 54.77 / 68.23 ms | 3.203 s | 2.910 s |

The JSON report is authoritative for unrounded values, uncertainty, peak RSS,
and artifact sizes. The 10,000-memory row supports the documented capacity
boundary; it does not guarantee latency on another machine or workload.

## Excluded development comparison

The repository retains v0.9 development reports under `docs/benchmarks/`, but
the pre-optimization report names source commit
`2e2be697cd92da1735984632263717044678e7e4`, which is not reachable from the
repository or public GitHub commit API as observed on 2026-08-04. Those reports
remain historical artifacts but are excluded from published speedup and source
reproducibility claims.

This exclusion does not affect the published v0.8.0 baseline or the independent
100,000-memory capacity canary below.

## Bounded CI and development runs

Use fewer samples only for correctness-oriented iteration:

```bash
SCALES="100 1000" \
INTERACTIVE_RUNS=2 \
MAINTENANCE_RUNS=1 \
WARMUP_RUNS=0 \
REPORT_FILE=/tmp/mnemark-benchmark.json \
scripts/benchmark-scale.sh
```

Before publishing a performance claim, run the default 20 interactive and five
maintenance samples and retain the JSON report. `ALLOW_VERSION_MISMATCH=1` is
reserved for an intentional candidate-version experiment and must not qualify a
release.

## 100,000-memory capacity canary

The development canary is not a supported release baseline:

```bash
SCALES="100000" \
INTERACTIVE_RUNS=5 \
MAINTENANCE_RUNS=1 \
REPORT_FILE=/tmp/mnemark-100k.json \
scripts/benchmark-scale.sh

python3 scripts/check-benchmark-regression.py \
  --report /tmp/mnemark-100k.json \
  --guardrails scripts/benchmark-guardrails.json
```

A clean development build from commit
`d67823742da2a650b31cce81fdcf498dce615f86` ran on 2026-07-29 on the same
Apple M2 Max class of machine with Rust 1.97.0 and Python 3.14.6. All operation
correctness assertions and broad capacity guardrails passed.

| Operation | Samples | Median | Median 95% interval | Peak RSS |
| --- | ---: | ---: | ---: | ---: |
| Import | 1 | 18.86 s | — | 276.34 MiB |
| Query | 5 | 32.74 ms | 31.32–35.85 ms | 46.03 MiB |
| Prime | 5 | 160.04 ms | 158.46–164.77 ms | 12.77 MiB |
| Graph rebuild | 1 | 23.29 s | — | 235.39 MiB |
| Bundle export | 1 | 16.81 s | — | 120.28 MiB |

Database size was 388.61 MiB, Tantivy index size 6.77 MiB, and bundle size
55.68 MiB. Single-sample maintenance observations establish only capacity and
correctness, not a latency distribution.

Retained evidence:

- [JSON report](benchmarks/v0.9.0-dev-100k-macos-arm64.json), SHA-256
  `e19c56afe9d7916a9d0c42f0d71ffeaf0df32dbc1ae0b6986689d4c040413fa7`;
- [CSV summary](benchmarks/v0.9.0-dev-100k-macos-arm64.csv), SHA-256
  `dc1d93eeaaa8aa08c0af84c7528ef9a89ca29588c5b389bd6cdf916ac6b073b1`.

Do not raise the 10,000-memory support boundary without a reviewed clean report
covering correctness, RSS, database/index size, graph rebuild, and bundle stages.
