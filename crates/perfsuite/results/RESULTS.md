# meridian-rs bench results

Run 2026-07-18 16:20 UTC · `zmax` (aarch64 macos, Apple M4 Max, 16 cores) · git `40f85cd` · rustc 1.96.0 (ac68faa20 2026-05-25) · report schema v1

## Claim verdicts

| claim | metric | baseline | gate | measured | verdict | note |
|---|---|---|---|---|---|---|
| `parse.throughput.corpus` | MB/s | 103.000 | ≥ 82 | — | UNTESTED | rung 1 (syntax::parse) |
| `parse.p99.file` | ms | 3.200 | ≤ 5 | — | UNTESTED | rung 1 |
| `parse.p99.monster_10mb` | ms | 473.000 | ≤ 600 | — | UNTESTED | rung 1 |
| `parse.reparse.full_46kb` | ms | 0.220 | ≤ 0.5 | — | UNTESTED | rung 1; the cold-edit path Go feels per keystroke-scale change |
| `assemble.p99.file` | ms | — | — | — | UNTESTED | rung 1 (model::build); baseline TBD on first fleet run |
| `project.p99.file` | ms | — | — | — | UNTESTED | rung 1 (wire_map::project); baseline TBD on first fleet run |
| `roundtrip.op.p99` | ms | — | — | — | UNTESTED | rung 1+; the number Go actually feels — composes parse+assemble+project+codec |
| `transport.codec.ndjson_roundtrip_p99` | us | — | — | 19.503 | MEASURED | LIVE day 1 (NdjsonCodec is implemented); no baseline yet — first fleet run establishes it |
| `policy.assertion.p99` | us | — | ruleset | — | UNTESTED | rung 6: thresholds are product data — Budget{class, p99_us} per rule manifest (policy-schema T2; mlua predicates, fuel-limited). Fuel budgets are deterministic and enforced as tests, not benched; this claim is the wall-time half |
| `policy.pack_load.fixtures` | ms | — | — | — | UNTESTED | rung 6: every rule ships pass/fail fixtures run at pack load — edit .lua → fixtures → done must stay in milliseconds; baseline TBD on first fleet run |

## Latency distributions (hdrhistogram path, µs)

| id | headline | unit | samples | p50 | p90 | p99 | p99.9 | max |
|---|---|---|---|---|---|---|---|---|
| `transport.codec.ndjson_roundtrip_p99` | 19.503 | us | 20000 | 13.6 | 15.9 | 19.5 | 45.4 | 65.0 |

Baseline provenance: parser-bench tournament (RESULTS.md, frozen 2026-07-17). Corpora are recipe-generated (`corpusgen`), never committed; claims are data in `claims.toml`; UNTESTED is the perf mirror of a `todo!()` body.
