# meridian-rs bench results

Run 2026-07-26 08:51 UTC · `zmax` (aarch64 macos, Apple M4 Max, 16 cores) · git `7bcc74b9` · rustc 1.97.1 (8bab26f4f 2026-07-14) · report schema v1

## Claim verdicts

| claim | metric | baseline | gate | measured | verdict | note |
|---|---|---|---|---|---|---|
| `parse.throughput.corpus` | MB/s | 103.000 | ≥ 82 | — | UNTESTED | rung 1 (syntax::parse) |
| `parse.p99.file` | ms | 3.200 | ≤ 5 | — | UNTESTED | rung 1 |
| `parse.p99.monster_10mb` | ms | 473.000 | ≤ 600 | — | UNTESTED | rung 1 |
| `parse.reparse.full_46kb` | ms | 0.220 | ≤ 0.5 | — | UNTESTED | rung 1; the cold-edit path Go feels per keystroke-scale change |
| `assemble.p99.file` | us | — | — | 85.247 | MEASURED | LIVE (model::build landed 730c37b7/M2-BUILD): hdr p99 of one build over the parsed corpus, parse+clone outside the timed window. Metric is us because the hdr path stages percentile_us and the join is unit-blind. No baseline yet — the first fleet run establishes it |
| `project.p99.file` | us | — | — | — | UNTESTED | UNTESTED by dependency, not by dormancy: wire_map::project landed 8883e5ca/M2-PROJECT, but perfsuite has no wire-map dev-dep — wiring benches/project.rs is a stage-4 card. Registers the gate; never a fabricated PASS. Stage as us when wired |
| `roundtrip.op.p99` | ms | — | — | — | UNTESTED | rung 1+; the number Go actually feels — composes parse+assemble+project+codec |
| `transport.codec.ndjson_roundtrip_p99` | us | — | — | 23.343 | MEASURED | LIVE day 1 (NdjsonCodec is implemented); no baseline yet — first fleet run establishes it |
| `transport.codec.proto_roundtrip_p99` | us | — | — | 5.503 | MEASURED | LIVE (typed protobuf path, ZT ruling 2026-07-18); same frame mix as the ndjson claim for codec-to-codec comparison; no baseline yet — first fleet run establishes it |
| `ingest.cold.vault_1gb` | ms | 22200.000 | ≤ 2220000 | 34336.030 | PASS | walk+read today; syntax::parse joins the pass at rung 1 |
| `ingest.codec.ndjson.vault_1gb` | ms | 1780.000 | ≤ 178000 | 3974.977 | PASS | the untyped seam's real bulk cost: JSON string escaping both ways. Frames pre-built outside the timed window |
| `ingest.codec.proto.vault_1gb` | ms | 583.000 | ≤ 58300 | 1232.451 | PASS | the typed path's real bulk cost on identical content — codec-to-codec comparison against the ndjson claim |
| `policy.assertion.p99` | us | — | ruleset | — | UNTESTED | rung 6: thresholds are product data — Budget{class, p99_us} per rule manifest (policy-schema T2; fenced Starlark predicates, ruling 008, metered under EvalBudget{steps,mem}). Step/mem budgets are deterministic (post-eval tick-count + peak heap) and enforced as tests, not benched; this claim is the wall-time half |
| `policy.pack_load.fixtures` | ms | — | — | — | UNTESTED | rung 6: every rule ships pass/fail fixtures run at pack load — edit the fenced Starlark predicate → fixtures → done must stay in milliseconds; baseline TBD on first fleet run |
| `policy.evaluate.p99.vault_1gb` | us | — | ruleset | 96.191 | MEASURED | P6-EVAL wall-time: p99 of one policy::evaluate call over a real model::build AST (blurb-required Starlark pack). Measured not asserted (risk R2 posture); gate arrives from policy vocab() Budget{p99_us} when it lands — ruleset-sourced so a local number never falsely PASS/FAILs |
| `policy.evaluate.p99.fence_bomb` | us | — | ruleset | 130.879 | MEASURED | P6-EVAL wall-time on the adversarial fence-saturated shape: p99 of one policy::evaluate call over a real AST. Measured not asserted (risk R2); ruleset-sourced gate pending vocab() |
| `policy.eval.budget.steps.vault_1gb` | steps | — | — | 10000.000 | MEASURED | P6-EVAL eval budget as data: the EvalBudget.steps the compiled blurb pack was admitted under (CompiledRuleset::budget()), deterministic/platform-free (post-eval tick-count). Runtime grounding: 0 of the sample's evals emitted a budget_exceeded finding (logged at run) — the declared envelope holds at scale. Best-effort in-process guard; the hard bound is OS/subprocess (daemon), never asserted here |
| `policy.eval.budget.mem.vault_1gb` | bytes | — | — | 4194304.000 | MEASURED | P6-EVAL eval budget as data: the EvalBudget.mem (peak heap-arena bytes) the compiled blurb pack was admitted under (CompiledRuleset::budget()). Runtime grounding: 0 of the sample's evals emitted a budget_exceeded finding. Best-effort in-process guard; the hard bound is OS/subprocess (daemon), never asserted here |
| `policy.splice.match_scan.p99` | us | — | ruleset | 139.647 | MEASURED | D-C1 (risk R2): match{old,new} resolved server-side by scanning target span bytes per edit, bounded by section size (the bytes just reparsed), measured not asserted (threshold_source=ruleset). MEASURED on the real model::validate_batch path: M4-VALIDATE landed validate_batch (validate_splice todo!() retired) and P6-VERDICTS armed it end-to-end on the splice arm; benches/roundtrip.rs::record_match_scan_claim times one match edit whose old sits at the tail of a large section so the scan walks the full span. A local number renders MEASURED, never a false PASS/FAIL (gate owned by the ruleset budget, not claims.toml). |
| `view.build.p99.vault_2026` | ms | — | — | — | UNTESTED | OD8: view::build_memory/publish over the warm parsed corpus; baseline TBD on the first fleet run. No harness in V1 — this row registers the gate (UNTESTED until benches/view.rs lands). |
| `view.freshfold.p99.vault_1gb` | ms | — | ruleset | — | UNTESTED | OD8: the O(file_count + corpus_bytes) post-result fold (no stat short-circuit). Gated against the perf ruleset's NAMED interactive-latency bound, not an invented literal. Stays MEASURED (never PASS) until that named rule is resolved to a concrete threshold — resolving it is itself part of arming fold-by-default. Landed cold-ingest measured 22.2 s / 1 GB / 180k files, so fold-per-query at that scale is not an interactive default. |
| `view.freshfold.envelope.filecount` | files | — | ruleset | — | UNTESTED | OD8: one of the fold's TWO independent cost terms. A single MB/s throughput number must NOT arm fold-by-default — it collapses the per-file term and lets a tiny-file corpus (high count, low bytes) falsely pass. Eligibility needs a passing envelope bounding BOTH file count and bytes, measured across a tiny-file AND a large-byte profile. Ruleset-sourced; UNTESTED until benches/view.rs lands. |
| `view.freshfold.envelope.bytes` | bytes | — | ruleset | — | UNTESTED | OD8: the second independent fold cost term, paired with view.freshfold.envelope.filecount. Both must pass across the tiny-file and large-byte profiles so neither term is extrapolated away. Ruleset-sourced; UNTESTED until benches/view.rs lands. |
| `view.sql.roundtrip.p99.vault_2026` | ms | — | — | — | UNTESTED | OD8: the number an agent feels for `mrd sql` — open the RO view, read the _meridian_view stamp as authoritative as_of, run the query client-side, fold live AFTER the result. Baseline TBD on the first fleet run. UNTESTED in V1 (registers the gate). |

**Tally:** 8 MEASURED · 3 PASS · 13 UNTESTED — over **24 claims registered** in `claims.toml` at this run.

## Latency distributions (hdrhistogram path, µs)

| id | headline | unit | samples | p50 | p90 | p99 | p99.9 | max |
|---|---|---|---|---|---|---|---|---|
| `assemble.p99.file` | 85.247 | us | 8000 | 19.9 | 35.4 | 85.2 | 1223.7 | 16244.7 |
| `policy.evaluate.p99.fence_bomb` | 130.879 | us | 8000 | 60.1 | 65.9 | 130.9 | 306.7 | 1080.3 |
| `policy.evaluate.p99.vault_1gb` | 96.191 | us | 8000 | 43.4 | 49.9 | 96.2 | 201.2 | 339.2 |
| `policy.splice.match_scan.p99` | 139.647 | us | 8000 | 115.1 | 126.8 | 139.6 | 168.6 | 211.6 |
| `transport.codec.ndjson_roundtrip_p99` | 23.343 | us | 20000 | 8.3 | 20.3 | 23.3 | 34.7 | 70.3 |
| `transport.codec.proto_roundtrip_p99` | 5.503 | us | 20000 | 4.7 | 5.1 | 5.5 | 7.3 | 32.0 |

## Criterion estimates

| bench | mean | median |
|---|---|---|
| `assemble_build_vault2026` | 93.72 µs | 74.71 µs |
| `parse/placeholder_corpus_traversal` | 143 ns | 136 ns |
| `policy_evaluate_vault1gb_batch` | 8.92 ms | 8.91 ms |
| `roundtrip/ndjson_decode_1000` | 11.11 ms | 11.04 ms |
| `roundtrip/ndjson_encode_1000` | 2.26 ms | 2.17 ms |
| `roundtrip/proto_decode_1000` | 1.94 ms | 1.94 ms |
| `roundtrip/proto_encode_1000` | 682.79 µs | 672.74 µs |

Baseline provenance: a prior benchmark run (frozen 2026-07-17). Corpora are recipe-generated (`corpusgen`), never committed; claims are data in `claims.toml`; UNTESTED is the perf mirror of a `todo!()` body.
