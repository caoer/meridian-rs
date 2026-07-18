# meridian-rs bench results

Run 2026-07-18 17:43 UTC · `zmax` (aarch64 macos, Apple M4 Max, 16 cores) · git `35ac2b7` · rustc 1.96.0 (ac68faa20 2026-05-25) · report schema v1

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
| `transport.codec.ndjson_roundtrip_p99` | us | — | — | 19.343 | MEASURED | LIVE day 1 (NdjsonCodec is implemented); no baseline yet — first fleet run establishes it |
| `transport.codec.proto_roundtrip_p99` | us | — | — | — | UNTESTED | LIVE (typed protobuf path, ZT ruling 2026-07-18); same frame mix as the ndjson claim for codec-to-codec comparison; no baseline yet — first fleet run establishes it |
| `ingest.cold.vault_1gb` | ms | 22200.000 | ≤ 2220000 | 22188.716 | PASS | walk+read today; syntax::parse joins the pass at rung 1 |
| `ingest.codec.ndjson.vault_1gb` | ms | 1780.000 | ≤ 178000 | 1782.094 | PASS | the untyped seam's real bulk cost: JSON string escaping both ways. Frames pre-built outside the timed window |
| `ingest.codec.proto.vault_1gb` | ms | 583.000 | ≤ 58300 | 582.968 | PASS | the typed path's real bulk cost on identical content — codec-to-codec comparison against the ndjson claim |
| `policy.assertion.p99` | us | — | ruleset | — | UNTESTED | rung 6: thresholds are product data — Budget{class, p99_us} per rule manifest (policy-schema T2; mlua predicates, fuel-limited). Fuel budgets are deterministic and enforced as tests, not benched; this claim is the wall-time half |
| `policy.pack_load.fixtures` | ms | — | — | — | UNTESTED | rung 6: every rule ships pass/fail fixtures run at pack load — edit .lua → fixtures → done must stay in milliseconds; baseline TBD on first fleet run |

## Latency distributions (hdrhistogram path, µs)

| id | headline | unit | samples | p50 | p90 | p99 | p99.9 | max |
|---|---|---|---|---|---|---|---|---|
| `transport.codec.ndjson_roundtrip_p99` | 19.343 | us | 20000 | 12.3 | 15.7 | 19.3 | 27.0 | 71.0 |

## Criterion estimates

| bench | mean | median |
|---|---|---|
| `assemble_noop_until_rung_1` | 1 ns | 1 ns |
| `parse/placeholder_corpus_traversal` | 103 ns | 91 ns |
| `policy_noop_until_rung_6` | 1 ns | 1 ns |
| `project_noop_until_rung_1` | 1 ns | 1 ns |
| `roundtrip/ndjson_decode_1000` | 7.54 ms | 6.80 ms |
| `roundtrip/ndjson_encode_1000` | 1.58 ms | 1.55 ms |

Baseline provenance: parser-bench tournament (RESULTS.md, frozen 2026-07-17). Corpora are recipe-generated (`corpusgen`), never committed; claims are data in `claims.toml`; UNTESTED is the perf mirror of a `todo!()` body.
