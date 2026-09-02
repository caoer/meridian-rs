# meridian-rs bench results

Run 2026-07-29 16:19 UTC · `zmax` (aarch64 macos, Apple M4 Max, 16 cores) · git `df446396` · rustc 1.97.1 (8bab26f4f 2026-07-14) · report schema v1

## Claim verdicts

| claim | metric | baseline | gate | measured | verdict | note |
|---|---|---|---|---|---|---|
| `parse.throughput.corpus` | MB/s | 103.000 | ≥ 82 | — | UNTESTED | rung 1 (syntax::parse) |
| `parse.p99.file` | ms | 3.200 | ≤ 5 | — | UNTESTED | rung 1 |
| `parse.p99.monster_10mb` | ms | 473.000 | ≤ 600 | — | UNTESTED | rung 1 |
| `parse.reparse.full_46kb` | ms | 0.220 | ≤ 0.5 | — | UNTESTED | rung 1; the cold-edit path Go feels per keystroke-scale change |
| `assemble.p99.file` | us | — | — | 46.591 | MEASURED | LIVE (model::build landed 730c37b7/M2-BUILD): hdr p99 of one build over the parsed corpus, parse+clone outside the timed window. Metric is us because the hdr path stages percentile_us and the join is unit-blind. No baseline yet — the first fleet run establishes it |
| `project.p99.file` | us | — | — | — | UNTESTED | UNTESTED by dependency, not by dormancy: wire_map::project landed 8883e5ca/M2-PROJECT, but perfsuite has no wire-map dev-dep — wiring benches/project.rs is a stage-4 card. Registers the gate; never a fabricated PASS. Stage as us when wired |
| `roundtrip.op.p99` | ms | — | — | — | UNTESTED | rung 1+; the number Go actually feels — composes parse+assemble+project+codec |
| `transport.codec.ndjson_roundtrip_p99` | us | — | — | 20.591 | MEASURED | LIVE day 1 (NdjsonCodec is implemented); no baseline yet — first fleet run establishes it |
| `ingest.cold.vault_1gb` | ms | 22200.000 | ≤ 2220000 | 26015.136 | PASS | walk+read today; syntax::parse joins the pass at rung 1 |
| `ingest.codec.ndjson.vault_1gb` | ms | 1780.000 | ≤ 178000 | 1768.831 | PASS | the untyped seam's real bulk cost: JSON string escaping both ways. Frames pre-built outside the timed window |
| `policy.assertion.p99` | us | — | ruleset | — | UNTESTED | rung 6: thresholds are product data — Budget{class, p99_us} per rule manifest (policy-schema T2; fenced Starlark predicates, ruling 008, metered under EvalBudget{steps,mem}). Step/mem budgets are deterministic (post-eval tick-count + peak heap) and enforced as tests, not benched; this claim is the wall-time half |
| `policy.pack_load.fixtures` | ms | — | — | — | UNTESTED | rung 6: every rule ships pass/fail fixtures run at pack load — edit the fenced Starlark predicate → fixtures → done must stay in milliseconds; baseline TBD on first fleet run |
| `policy.evaluate.p99.vault_1gb` | us | — | ruleset | 31.375 | MEASURED | P6-EVAL wall-time: p99 of one policy::evaluate call over a real model::build AST (blurb-required Starlark pack). Measured not asserted (risk R2 posture); gate arrives from policy vocab() Budget{p99_us} when it lands — ruleset-sourced so a local number never falsely PASS/FAILs |
| `policy.evaluate.p99.fence_bomb` | us | — | ruleset | 42.239 | MEASURED | P6-EVAL wall-time on the adversarial fence-saturated shape: p99 of one policy::evaluate call over a real AST. Measured not asserted (risk R2); ruleset-sourced gate pending vocab() |
| `policy.eval.budget.steps.vault_1gb` | steps | — | — | 10000.000 | MEASURED | P6-EVAL eval budget as data: the EvalBudget.steps the compiled blurb pack was admitted under (CompiledRuleset::budget()), deterministic/platform-free (post-eval tick-count). Runtime grounding: 0 of the sample's evals emitted a budget_exceeded finding (logged at run) — the declared envelope holds at scale. Best-effort in-process guard; the hard bound is OS/subprocess (daemon), never asserted here |
| `policy.eval.budget.mem.vault_1gb` | bytes | — | — | 4194304.000 | MEASURED | P6-EVAL eval budget as data: the EvalBudget.mem (peak heap-arena bytes) the compiled blurb pack was admitted under (CompiledRuleset::budget()). Runtime grounding: 0 of the sample's evals emitted a budget_exceeded finding. Best-effort in-process guard; the hard bound is OS/subprocess (daemon), never asserted here |
| `policy.splice.match_scan.p99` | us | — | ruleset | 104.191 | MEASURED | D-C1 (risk R2): match{old,new} resolved server-side by scanning target span bytes per edit, bounded by section size (the bytes just reparsed), measured not asserted (threshold_source=ruleset). MEASURED on the real model::validate_batch path: M4-VALIDATE landed validate_batch (validate_splice todo!() retired) and P6-VERDICTS armed it end-to-end on the splice arm; benches/roundtrip.rs::record_match_scan_claim times one match edit whose old sits at the tail of a large section so the scan walks the full span. A local number renders MEASURED, never a false PASS/FAIL (gate owned by the ruleset budget, not claims.toml). |
| `view.build.p99.vault_2026` | ms | — | — | — | UNTESTED | OD8: view::build_memory/publish over the warm parsed corpus; baseline TBD on the first fleet run. No harness in V1 — this row registers the gate (UNTESTED until benches/view.rs lands). |
| `view.freshfold.p99.vault_1gb` | ms | — | ruleset | — | UNTESTED | OD8: the O(file_count + corpus_bytes) post-result fold (no stat short-circuit). Gated against the perf ruleset's NAMED interactive-latency bound, not an invented literal. Stays MEASURED (never PASS) until that named rule is resolved to a concrete threshold — resolving it is itself part of arming fold-by-default. Landed cold-ingest measured 22.2 s / 1 GB / 180k files, so fold-per-query at that scale is not an interactive default. |
| `view.freshfold.envelope.filecount` | files | — | ruleset | — | UNTESTED | OD8: one of the fold's TWO independent cost terms. A single MB/s throughput number must NOT arm fold-by-default — it collapses the per-file term and lets a tiny-file corpus (high count, low bytes) falsely pass. Eligibility needs a passing envelope bounding BOTH file count and bytes, measured across a tiny-file AND a large-byte profile. Ruleset-sourced; UNTESTED until benches/view.rs lands. |
| `view.freshfold.envelope.bytes` | bytes | — | ruleset | — | UNTESTED | OD8: the second independent fold cost term, paired with view.freshfold.envelope.filecount. Both must pass across the tiny-file and large-byte profiles so neither term is extrapolated away. Ruleset-sourced; UNTESTED until benches/view.rs lands. |
| `view.sql.roundtrip.p99.vault_2026` | ms | — | — | — | UNTESTED | OD8: the number an agent feels for `mrd sql` — open the RO view, read the _meridian_view stamp as authoritative as_of, run the query client-side, fold live AFTER the result. Baseline TBD on the first fleet run. UNTESTED in V1 (registers the gate). |
| `check.worktree.p99` | ms | — | ≤ 2000 | — | UNTESTED | THRESHOLD PROVENANCE: design-intent statement, quoted — "our meridian-rs runs 180k md doc with total of 1GB for less than 2 seconds". DESIGN INTENT ONLY, deliberately NOT benchmark-backed, and no baseline is declared because no bench has ever measured this path. The <2s figure at 180k/1GB belongs to ingest.codec.proto.vault_1gb — encode+decode of ALREADY-IN-MEMORY content, frames pre-built outside the timed window, no filesystem and no parse. This lane's own recorded filesystem number at that scale is ingest.cold.vault_1gb: 22.2 s baseline, 34.3 s at the 2026-07-26 run. Citing the codec envelope as backing for the walk+read+parse path was an unmeasured join; it is not repeated here. UNTESTED by absence of harness, never a fabricated PASS. Field evidence pending the bench (bfa6affe, field-notes, 2026-07-28, /usr/bin/time -p, 3 runs): 34.78 s cold / 22.07 s / 18.38 s warm, user CPU FLAT at 3.63-3.99 s, sys 6.43-13.89 s — 51-58% CPU utilisation, so roughly half the wall clock is off-CPU blocking I/O. The path is I/O-bound and serial, not parse-bound: userland total bounds parse+model from above at ~4 s. `mrd status` shares the same tree-fold cost and needs its own claim once this one has a harness. |

**Tally:** 7 MEASURED · 2 PASS · 14 UNTESTED — over **23 claims registered** in `claims.toml` at this run.

## Latency distributions (hdrhistogram path, µs)

| id | headline | unit | samples | p50 | p90 | p99 | p99.9 | max |
|---|---|---|---|---|---|---|---|---|
| `assemble.p99.file` | 46.591 | us | 8000 | 13.0 | 22.9 | 46.6 | 96.1 | 640.0 |
| `policy.evaluate.p99.fence_bomb` | 42.239 | us | 8000 | 30.8 | 34.3 | 42.2 | 108.7 | 181.2 |
| `policy.evaluate.p99.vault_1gb` | 31.375 | us | 8000 | 22.6 | 26.2 | 31.4 | 91.7 | 178.3 |
| `policy.splice.match_scan.p99` | 104.191 | us | 8000 | 83.8 | 90.4 | 104.2 | 176.0 | 228.5 |
| `transport.codec.ndjson_roundtrip_p99` | 20.591 | us | 20000 | 14.3 | 16.7 | 20.6 | 27.9 | 64.7 |

## Criterion estimates

| bench | mean | median |
|---|---|---|
| `assemble_build_vault2026` | 14.72 µs | 14.67 µs |
| `parse/placeholder_corpus_traversal` | 62 ns | 62 ns |
| `policy_evaluate_vault1gb_batch` | 4.52 ms | 4.49 ms |
| `roundtrip/ndjson_decode_1000` | 7.18 ms | 7.16 ms |
| `roundtrip/ndjson_encode_1000` | 1.39 ms | 1.39 ms |
| `roundtrip/proto_decode_1000` | 1.46 ms | 1.46 ms |
| `roundtrip/proto_encode_1000` | 548.67 µs | 544.04 µs |

Baseline provenance: a prior benchmark run (frozen 2026-07-17). Corpora are recipe-generated (`corpusgen`), never committed; claims are data in `claims.toml`; UNTESTED is the perf mirror of a `todo!()` body.
