# meridian-rs bench results

Run 2026-07-18 17:35 UTC · `zmax` (aarch64 macos, Apple M4 Max, 16 cores) · git `26c96b5` · rustc 1.96.0 (ac68faa20 2026-05-25) · report schema v1

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
| `transport.codec.ndjson_roundtrip_p99` | us | — | — | 20.015 | MEASURED | LIVE day 1 (NdjsonCodec is implemented); no baseline yet — first fleet run establishes it |
| `transport.codec.proto_roundtrip_p99` | us | — | — | 4.375 | MEASURED | LIVE (typed protobuf path, ZT ruling 2026-07-18); same frame mix as the ndjson claim for codec-to-codec comparison; no baseline yet — first fleet run establishes it |
| `policy.assertion.p99` | us | — | ruleset | — | UNTESTED | rung 6: thresholds are product data — Budget{class, p99_us} per rule manifest (policy-schema T2; mlua predicates, fuel-limited). Fuel budgets are deterministic and enforced as tests, not benched; this claim is the wall-time half |
| `policy.pack_load.fixtures` | ms | — | — | — | UNTESTED | rung 6: every rule ships pass/fail fixtures run at pack load — edit .lua → fixtures → done must stay in milliseconds; baseline TBD on first fleet run |

## Latency distributions (hdrhistogram path, µs)

| id | headline | unit | samples | p50 | p90 | p99 | p99.9 | max |
|---|---|---|---|---|---|---|---|---|
| `transport.codec.ndjson_roundtrip_p99` | 20.015 | us | 20000 | 12.9 | 15.8 | 20.0 | 27.8 | 43.3 |
| `transport.codec.proto_roundtrip_p99` | 4.375 | us | 20000 | 2.9 | 3.5 | 4.4 | 8.7 | 50.7 |

## Criterion estimates

| bench | mean | median |
|---|---|---|
| `roundtrip/ndjson_decode_1000` | 6.71 ms | 6.71 ms |
| `roundtrip/ndjson_encode_1000` | 1.53 ms | 1.53 ms |
| `roundtrip/proto_decode_1000` | 1.38 ms | 1.38 ms |
| `roundtrip/proto_encode_1000` | 421.61 µs | 421.64 µs |

Baseline provenance: parser-bench tournament (RESULTS.md, frozen 2026-07-17). Corpora are recipe-generated (`corpusgen`), never committed; claims are data in `claims.toml`; UNTESTED is the perf mirror of a `todo!()` body.
