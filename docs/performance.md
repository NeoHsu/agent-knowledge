# Performance Baseline

`mnemark` includes a deterministic scale benchmark for production canary work:

```bash
scripts/build-release.sh
scripts/benchmark-scale.sh
```

The benchmark creates isolated temporary stores, imports generated memories, and
measures release-binary import, lexical query, prime, graph rebuild, and bundle
export. It performs no network access and removes the stores afterward.

## 0.6.0 local baseline

Measured on 2026-07-11 with an Apple M2 Max (`arm64`, Darwin 25.5.0), Rust
1.97.0, SQLite bundled through `rusqlite`, and the optimized release profile:

| Memories | Import | Query (20) | Prime (4,000 chars) | Graph rebuild | Bundle export |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 100 | 727 ms | 45 ms | 37 ms | 57 ms | 1.06 s |
| 1,000 | 5.06 s | 48 ms | 41 ms | 468 ms | 7.13 s |
| 10,000 | 52.54 s | 159 ms | 201 ms | 4.76 s | 68.22 s |

These figures are a regression baseline, not a cross-machine service-level
objective. Query and prime remain interactive at 10,000 memories. Bulk import
and bundle export are maintenance operations; bundle export includes an online
SQLite snapshot, full durable-field secret scan, artifact scan, and SHA-256 for
every bundled file.

Use `SCALES="100 1000" scripts/benchmark-scale.sh` for bounded development runs.
Record hardware, operating system, Rust version, and profile when comparing
results.
