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

Protocol v2 reports median, median absolute deviation (MAD), a deterministic
bootstrap 95% interval for the median, p95 (only with at least 20 samples),
minimum, maximum, peak RSS, and input/output sizes. The JSON report additionally
records:

- platform, Rust and Python versions;
- Git commit and dirty state;
- binary and combined benchmark-protocol SHA-256 hashes;
- seed, execution order, exact sample schedule, run counts, and cache model;
- database, Tantivy index, and bundle sizes;
- bundle snapshot, validation, hashing, archive, and install stage timings.

Bundle export overlaps hashing with gzip archive creation after all snapshot and
secret/schema validation succeeds. Consequently, protocol-v2 `hash_ms` and
`archive_ms` are concurrent wall-clock observations and must not be summed to
infer total latency.

CI retains the JSON and CSV reports and applies
`scripts/benchmark-guardrails.json` through
`scripts/check-benchmark-regression.py`. These generous, cross-runner limits
catch catastrophic regressions and hangs; they are not performance claims.
For meaningful regression percentages, run both binaries in one seeded,
sample-level interleaved process, then compare the paired reports:

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
identity, and sample schedule. Override platform or protocol checks only for an
explicitly reviewed methodology comparison. Schema-v1 reports remain valid as
historical evidence, but cannot be directly compared with protocol v2.

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

Run the default 20 interactive and five maintenance samples before publishing
performance claims. Keep the JSON report so results remain tied to a binary
hash, commit, exact schedule, uncertainty summary, cache model, and correctness
checks. The candidate binary must match `Cargo.toml`; an older paired binary is
identified separately by `BASELINE_MEM_BIN` and `BASELINE_GIT_COMMIT`. Use
`ALLOW_VERSION_MISMATCH=1` only when the candidate mismatch is intentional.

## 100,000-memory capacity canary

A 100,000-memory capacity canary is available but is not a release baseline:

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

The 100,000-memory entries are deliberately broad catastrophic guardrails for
manual canaries, not CI performance targets. Do not infer a service-level
objective from that canary. Review correctness,
peak RSS, database/index size, graph rebuild latency, and bundle stages before
raising the documented support envelope beyond 10,000 memories.

### Retained 100,000-memory development canary

A clean development build from commit
`d67823742da2a650b31cce81fdcf498dce615f86` was exercised on 2026-07-29 on
the same Apple M2 Max platform with Rust 1.97.0 and Python 3.14.6. The binary
SHA-256 was
`198fa025e11f428a5cd5534fe13d6af2187cca4cf1d09e74bc8cedcfbf798025`.
All operation-level correctness assertions and the portable 100,000-memory
catastrophic guardrails passed.

| Operation | Samples | Median | Median 95% interval | Peak RSS |
| --- | ---: | ---: | ---: | ---: |
| Import | 1 | 18.86 s | — | 276.34 MiB |
| Query | 5 | 32.74 ms | 31.32–35.85 ms | 46.03 MiB |
| Prime | 5 | 160.04 ms | 158.46–164.77 ms | 12.77 MiB |
| Graph rebuild | 1 | 23.29 s | — | 235.39 MiB |
| Bundle export | 1 | 16.81 s | — | 120.28 MiB |

The populated database was 388.61 MiB, the Tantivy index was 6.77 MiB, and the
bundle was 55.68 MiB. Bundle export spent 3.11 s on the SQLite snapshot and
8.76 s on validation; its 4.07 s hash and 4.89 s archive observations overlap.
The single maintenance samples establish capacity and correctness evidence,
not a latency distribution. They did not show a correctness failure or an
obvious nonlinear graph-growth alarm, so no graph insertion or parsing change
was made.

Retained evidence:

- [`benchmarks/v0.9.0-dev-100k-macos-arm64.json`](benchmarks/v0.9.0-dev-100k-macos-arm64.json)
  (SHA-256 `e19c56afe9d7916a9d0c42f0d71ffeaf0df32dbc1ae0b6986689d4c040413fa7`);
- [`benchmarks/v0.9.0-dev-100k-macos-arm64.csv`](benchmarks/v0.9.0-dev-100k-macos-arm64.csv)
  (SHA-256 `dc1d93eeaaa8aa08c0af84c7528ef9a89ca29588c5b389bd6cdf916ac6b073b1`).
