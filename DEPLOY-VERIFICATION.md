# DEPLOY-VERIFICATION — wire-contract-v2 campaign

**Audience: ZT, deploying.** A hands-on verification playbook: for each feature
the campaign built, run the command and confirm the real output. Every
**Expected** block below is a **REAL captured output** from a fresh run at the
landed commit `26147ba` (`main` == `origin`), not hand-written — a step that was
not actually run would be a defect. Every command was run (and independently
spot-checked) before this doc shipped.

## How to use

- **Base:** meridian-rs at `26147ba` (== `main` == `origin`). The engine / wire /
  policy checks run there; the **ratchet** checks run against `ccc-statusd` at
  commit `cf59e31`.
- **Each block:** **What** it does · **Verify** (copy-pasteable command) ·
  **Expected** (the real output to confirm). Run the command; if your output
  matches, the feature works.
- **The law** each feature implements is the FROZEN wire contract v2 (decision
  014): `…/18-02-meridian-rs/results/wire-contract-v2.md` — cited per feature.

> [!warning] DEPLOY-CRITICAL — the g-ratchet is INERT at `cf59e31`
> ZT is deploying the g-ratchet. At `cf59e31` the ratchet is **fully implemented
> and tested (28/28 gates green) but NOT yet wired into the live send path** —
> `ratchet.Evaluate` has **zero live callers** and is dead-code-eliminated from
> the shipped binary. **Deploying `cf59e31` changes nothing about how writes
> behave:** it is day-one permissive (guaranteed three independent ways), and it
> stays permissive **regardless of config** until a LATER commit wires
> enforcement into the send path. This is a scope fact, **not a defect** — the
> code is correct and proven. See **Policy + Ratchet** below for the full
> evidence and ZT's exact post-deploy sequence.

## Contents

1. **Model / Engine** — parse, build + merkle roots (oracle 69/69), resolve / walk, GT / fixtures, span / rev laws
2. **Wire** — toc / cat / extract / links, splice / write, delta / diff / root, sub push, watch external, wire_agreement, p4 probes
3. **Policy + Ratchet** — verdicts ride splice (§11.1), daemon_only (§8), the g-ratchet's 5 gates (post-deploy verification)

---

## Model / Engine

**Engine under test:** lane-model worktree
`/Users/Shared/repos/meridian-rs/.claude/worktrees/lane-model` — branch
`worktree-lane-model` == main == origin, HEAD **`26147ba`**
(`26147ba0f6d566852b35798b396beaf27dfa4b86`). Its own cargo target (no race with
other lanes). Frozen contract (decision 014):
`…/18-02-meridian-rs/results/wire-contract-v2.md`.

> All commands below were actually run at `26147ba`; every **Expected** block is a
> REAL captured output tail, not hand-written. `cd` where stated so the commands
> are copy-pasteable as-is.

```bash
# Every command in this section assumes you are in the lane-model worktree:
cd /Users/Shared/repos/meridian-rs/.claude/worktrees/lane-model
git rev-parse --short HEAD    # must print 26147ba before you trust anything below
```

---

#### Parse — markdown → `(kind,span)` node tree  (§2.1 / R1-PARSE)
- **What:** the frozen-contract strict-mint grammar (§2.1 "the ONE fleet grammar") parses markdown into the `(kind,span)` node tree — ATX headings, frontmatter, wikilinks/embeds/fragments, callouts, code fences, tasks/tables — with a deterministic total order.
- **Verify:**
    cd /Users/Shared/repos/meridian-rs/.claude/worktrees/lane-model
    cargo test -p syntax
- **Expected (real, @26147ba):**
    ```
    running 17 tests
    test tests::is_block_id_enforces_one_charset ... ok
    test tests::anchor_underscore_rejected_app_exact ... ok
    test tests::span_law_multibyte_split_refusal ... ok
    test tests::frontmatter_span_and_keys ... ok
    test tests::headings_atx_only_level_and_text ... ok
    test tests::wikilinks_embeds_and_fragments ... ok
    test tests::total_order_is_deterministic ... ok
    test tests::span_law_inline_includes_delimiters ... ok
    test tests::span_law_block_excludes_terminator ... ok
    ...
    test result: ok. 17 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
    ```

---

#### Build + Merkle roots — R0/R1/R2, domain filter, prefix bump  (§12)
- **What:** build the workspace tree and compute merkle roots per §12.2 interior encoding (leaf = `blake3(file)` 32 B; interior = blake3 over children sorted by raw name bytes, `varint(len)‖name‖type‖hash32`; empty dirs pruned; workspace root name never hashed), including the domain filter (§12.1) and the domain-rule prefix bump (§12.3, R2). **The oracle 69/69 is the §12.2 ground-truth check** — it recomputes every worked root/rev value clean-room from the contract prose.
- **Verify:**
    # (a) engine merkle unit gates
    cd /Users/Shared/repos/meridian-rs/.claude/worktrees/lane-model
    cargo test -p model -- merkle_root root_prefix empty_dir

    # (b) §12.2 GROUND-TRUTH oracle — 69/69 clean-room recompute
    cd /Users/Shared/projects/home-wiki-sessions/year=2026/month=07/18-02-meridian-rs/results
    uvx --from blake3 python3 wire-contract-v2-verify.py
- **Expected (real, @26147ba):**
    (a) engine merkle gates:
    ```
    test tests::merkle_root_counterfactual_12_1 ... ok
    test tests::merkle_root_r0_r1_r2 ... ok
    test tests::merkle_root_domain_bump_12_3 ... ok
    test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 38 filtered out; finished in 0.00s
    ```
    (b) oracle tail:
    ```
    OK   §12.1   wrong-ignore root: '75a61c883e372102cfe7d75e94992b9be65e33fbe95956897a4cf2ea45bb8f1b'
    OK   §12.3   v0-with-drafts root: '05f0c6192308db5937c3e1352d1f9a6fc31b89b1a57175c8af6ce7903525aa4a'
    OK   §12.3   bumped hex equals R2 hex: '83b4ba591c0291d9f2a05428cac38e5820858fbb9c47720ab352344ddccc8f68'

    69/69 values recomputed and matched
    ```

---

#### Resolve / walk — the walk plane (where things are)  (§4.5)
- **What:** ref resolution across the read-only walk plane — hpath descent by containment (byte-exact), duplicate-sibling ambiguity, frontmatter-key targeting, anchor/block resolution, case-insensitive frontmatter alias — plus the obsidian-compat GT pack green through the walk harness. Walk returns **no rev field** (revs are a mint-plane concern, §4.5).
- **Verify:**
    cd /Users/Shared/repos/meridian-rs/.claude/worktrees/lane-model
    # (a) obsidian-compat pack + walk probes, green end-to-end
    cargo test -p testsuite --test main -- walk_resolve
    # (b) engine resolve/walk unit gates
    cargo test -p model -- resolve_ walk::
- **Expected (real, @26147ba):**
    (a) walk_resolve:
    ```
    running 4 tests
    test walk_resolve::charset_walk_vs_mint_split_underscore ... ok
    test walk_resolve::duplicate_anchor_split_mint_loud_walk_silent ... ok
    test walk_resolve::obsidian_compat_pack_green_through_walk ... ok
    test walk_resolve::harness_p2_walk_probes_green_over_walkvault ... ok
    test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 30 filtered out; finished in 0.00s
    ```
    (b) model resolve/walk gates:
    ```
    test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 33 filtered out; finished in 0.00s
    ```

---

#### GT / fixtures — dialect GT convergence + frozen worked-value sweep  (§2.1, §12.2)
- **What:** two fixture gates. `gt_pack` = **dialect GT convergence**: `syntax::parse` output equals the frozen parser-bench pack's `(kind,span)` truth **byte-exact** (the pack is demoted to data, re-derived by the engine — not a hand-written oracle). `pf_frozen_sweep` = the **frozen worked-value sweep**: every worked frame (spans, revs, file_revs, roots, CAS actuals, domain roots, resolve frames) from the contract, recomputed and matched against frozen values.
- **Verify:**
    cd /Users/Shared/repos/meridian-rs/.claude/worktrees/lane-model
    # (a) dialect GT convergence: engine parse == pack (kind,span) byte-exact
    cargo test -p testsuite --test main -- gt_pack
    # (b) frozen worked-value sweep
    cargo test -p testsuite --test main -- pf_frozen_sweep
- **Expected (real, @26147ba):**
    (a) gt_pack:
    ```
    running 2 tests
    test gt_pack_smoke::gt_pack_loads ... ok
    test gt_parse::gt_pack_parse_truth ... ok
    test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 32 filtered out; finished in 0.00s
    ```
    (b) pf_frozen_sweep:
    ```
    running 14 tests
    test pf_frozen_sweep::s03_states_byte_counts_and_committed_s2_identity ... ok
    test pf_frozen_sweep::s03_file_revs_and_roots ... ok
    test pf_frozen_sweep::s121_s123_domain_roots_and_a7_full_token ... ok
    test pf_frozen_sweep::s45_worked_resolve_frames_id72_closure ... ok
    ...
    test result: ok. 14 passed; 0 failed; 0 ignored; 0 measured; 20 filtered out; finished in 0.00s
    ```

---

#### Span / rev laws — content_span, node_rev, file_rev minting  (§1, §4.1)
- **What:** the rev sub-laws (§1): `node_rev` = `blake3-256(full span bytes)[:16 hex]` (heading-inclusive for sections, so a rename invalidates the token); `content_span` starts after the heading line terminator and mints **no** rev (exactly one rev per node); `file_rev` = `blake3(whole file)[:16]`, same family/width. Span laws: block spans exclude the final terminator, inline spans include delimiters, multibyte-split mints are refused. `toc` carries revs riding along free (§4.1).
- **Verify:**
    cd /Users/Shared/repos/meridian-rs/.claude/worktrees/lane-model
    # (a) span laws (syntax plane)
    cargo test -p syntax -- span_law
    # (b) rev minting: root node_rev == file_rev over whole-file bytes + uleb128 encode
    cargo test -p model -- root_node_rev write_uleb128
- **Expected (real, @26147ba):**
    (a) span laws:
    ```
    test tests::span_law_block_excludes_terminator ... ok
    test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 14 filtered out; finished in 0.00s
    ```
    (b) rev minting:
    ```
    test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 41 filtered out; finished in 0.00s
    ```
    (the two: `tests::root_node_rev_equals_file_rev_over_whole_file_bytes`, `tests::write_uleb128_matches_spec`)

---

**Capture attestation:** every command in this section was actually run against the
landed engine at `26147ba`; every **Expected** block is a real captured output tail
(cargo test result lines / the oracle's `69/69 values recomputed and matched`), not
hand-written or asserted. No engine code, tests, or fixtures were modified.


---


## Wire domain — how to verify each wire feature

> [!note] Provenance (audit hygiene)
> Every `Expected` block below is a **real captured output** from a fresh `cargo test`
> run against the **main checkout** `/Users/Shared/repos/meridian-rs` at HEAD
> `26147ba0f6d566852b35798b396beaf27dfa4b86` (tree clean before and after — zero
> source/test/fixture edits; `cargo` writes only to the git-ignored `target/`).
> Nothing here is paraphrased or hand-written. Each feature maps to the campaign's
> own named test whose body was read to confirm it drives the **real op** through
> the live `sidecar::serve` loop (or the real commit path), not just a green bar.
> Section citations are to the FROZEN wire contract v2 (decision 014):
> `…/18-02-meridian-rs/results/wire-contract-v2.md`.

---

#### toc / cat / extract / links / resolve  (§4.1–4.6)

- **What:** The single-file read/mint surface + walk plane — `toc` (map + `node_rev` per node, §4.1), `cat` (full-span bytes + rev, §4.2), `extract` (frozen §5 node surface, §4.3), `resolve` (Obsidian walk plane, no rev field, §4.5), `links` (corpus edge map + staleness triple, §4.6) — all driven byte-in/frame-out through the live `sidecar::serve` loop over the §0.3 `wsfix` S0/S2 fixtures.
- **Verify:**
    ```
    cd /Users/Shared/repos/meridian-rs && cargo test -p sidecar --test dispatch_v2
    ```
- **Expected (real, @26147ba):**
    ```
         Running tests/dispatch_v2.rs (target/debug/deps/dispatch_v2-e899734c11e59a74)

    running 34 tests
    test gate1_worked_s0_cat_exchange ... ok
    test gate1_worked_s0_toc_exchange ... ok
    test emission_3_1_idless_request_is_served ... ok
    test gate2_extract_unknown_kinds_echoed_loud ... ok
    test d3_root_op_serves_current_root_and_epoch_seq ... ok
    test gate1_cat_whole_file_serves_file_rev ... ok
    test errors_file_not_found_vs_ref_not_found ... ok
    test errors_dot_ignored_md_stays_addressable ... ok
    test gate2_bad_path_refused ... ok
    test d3_diff_empty_or_root_unknown_truthfully ... ok
    test gate2_unsupported_proto ... ok
    test d3_gate5_root_diff_strict_decode_rejections ... ok
    test d3_restart_boundary_never_correlates_epochs ... ok
    test emission_3_1_pipelined_frames_correlate_in_order ... ok
    test gate3_hello_caps_equal_frozen_full_list ... ok
    test gate4_extract_kinds_filter ... ok
    test gate2_unknown_field_rejected_per_armed_op ... ok
    test gate5_charset_underscore_on_walk_plane_misses_silently ... ok
    test gate5_charset_underscore_anchor_mint_position_bad_request ... ok
    test emission_3_1_non_request_frames_answer_bad_frame ... ok
    test emission_3_1_bad_id_lexemes_answer_id_null_plus_id_raw_verbatim ... ok
    test q5_gate1_worked_stale_view_refusal_computed ... ok
    test gate3_unarmed_and_unknown_ops_answer_unknown_op ... ok
    test gate4_node_rev_on_every_toc_cat_extract_node ... ok
    test gate2_unknown_or_mistyped_value_rejected_per_armed_op ... ok
    test q5_worked_links_exchange_computed ... ok
    test resolve_worked_s2_frames ... ok
    test q5_gate3_whole_corpus_edge_map ... ok
    test gate3_s2l22_node_rev_must_when_splice_in_caps ... ok
    test q5_links_path_outside_domain_is_file_not_found ... ok
    test q5_require_root_current_succeeds ... ok
    test resolve_content_true_returns_bytes_and_never_a_rev ... ok
    test q5_gate2_honest_tense_unstrict_read_never_refuses ... ok
    test q5_gate4_links_strict_decode_rejections ... ok

    test result: ok. 34 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.10s
    ```

---

#### splice / write  (§4.4, failure split §5.2, receipts §6.3)

- **What:** The ONLY write op — batch-only, Edit-exact `match{old,new}` / `put{at,text}`, guards `if_root`/`if_node_rev`, dry runs, and the receipt append riding the same sealed batch. `splice_e2e` drives the frozen §4.4 E3/E4 worked exchanges END-TO-END (real decode → validate → apply → emit → disk), asserts armed responses + roots R0→R1→R2, the §5.2 failure split (`cas_mismatch`/`no_match`/`not_unique`), the `fm_key` dry exchange, and that the engine's own receipt renderer produces the frozen §6.3 bytes.
- **Verify:**
    ```
    cd /Users/Shared/repos/meridian-rs && cargo test -p sidecar --test splice_e2e
    ```
- **Expected (real, @26147ba):**
    ```
        Finished `test` profile [unoptimized + debuginfo] target(s) in 0.08s
         Running tests/splice_e2e.rs (target/debug/deps/splice_e2e-3cd47f66241329af)

    running 5 tests
    test if_root_checked_first_root_mismatch ... ok
    test dry_run_touches_nothing ... ok
    test splice_strict_decode_rejections ... ok
    test receipt_parent_dir_created_by_caller_real_only ... ok
    test worked_session_e3_e4_failures_dry_links_root_byte_exact ... ok

    test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.08s
    ```

---

#### delta / diff / root  (§7 — Delta noun, replay ≡ live §7.3; root/diff ops §4.7)

- **What:** The Delta noun (fifth noun, §7.1) + `diff`/`root` integrity ops. `replay_live` is THE named replay ≡ live test: it drives S0→S1→S2 through the REAL commit path (`validate → fs::apply_batch → delta computation → ring.advance`) using the frozen §4.4 requests, asserts the two LIVE Deltas equal the printed §7.1 frames value-for-value, and proves `diff`'s replay range `(R0,R2)` returns **byte-identical** Delta objects (§7.3). (The `root`/`diff` op *dispatch* — current-root read, empty-batch diff, `root_unknown` resync, strict-decode — additionally rides `dispatch_v2`'s `d3_*` tests captured in the block above.)
- **Verify:**
    ```
    cd /Users/Shared/repos/meridian-rs && cargo test -p sidecar --test replay_live
    ```
- **Expected (real, @26147ba):**
    ```
        Finished `test` profile [unoptimized + debuginfo] target(s) in 0.08s
         Running tests/replay_live.rs (target/debug/deps/replay_live-d855e3110b6dedbf)

    running 4 tests
    test refused_batch_emits_no_delta ... ok
    test receipt_node_rides_its_files_delta_entry_computed ... ok
    test deepest_changed_node_never_duplicates_ancestors ... ok
    test replay_equals_live_e3_e4_through_the_commit_path ... ok

    test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.08s
    ```

---

#### sub push  (T5 — §4.7 reserved `sub` shape, replay ≡ live at the transport §7.3)

- **What:** The `sub` push path through the live serve loop: `sub{from_seq:N}` acks with the ruled Root-shaped anchor, each commit pushes one Notification AFTER its response (§4.7 order), and the pushed frames are **byte-identical** to `diff(R0,R2)`'s batches — one shape, two transports (§7.3). Also covers mid-epoch replay-then-live, `from_seq==current` live-only, stale `from_seq` → `root_unknown`/resync, and strict-decode rejections.
- **Verify:**
    ```
    cd /Users/Shared/repos/meridian-rs && cargo test -p sidecar --test sub_push
    ```
- **Expected (real, @26147ba):**
    ```
        Finished `test` profile [unoptimized + debuginfo] target(s) in 0.08s
         Running tests/sub_push.rs (target/debug/deps/sub_push-32defb17ff86ab72)

    running 6 tests
    test sub_strict_decode_rejections ... ok
    test repeated_sub_adds_a_subscription ... ok
    test stale_from_seq_refuses_root_unknown_resync ... ok
    test mid_epoch_sub_replays_then_goes_live ... ok
    test from_seq_at_current_is_live_only ... ok
    test sub_then_splice_pushed_frames_byte_identical_to_diff ... ok

    test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.20s
    ```

---

#### watch — external edits  (F5 — §7.1: external deltas carry actor/now ABSENT)

- **What:** External changes ("a human editing in Obsidian") through the live serve loop: a scripted source mutates the workspace between input lines and the line-boundary reconcile detects, emits, and pushes. The §7.1 law under test — external deltas carry `actor`/`now` **key-absent** (never null) with `seq` assigned at detection — plus the ruled `renamed`/`from_path` byte-identity classification and the refuse-to-guess delete+create fallback.
- **Verify:**
    ```
    cd /Users/Shared/repos/meridian-rs && cargo test -p sidecar --test watch_external
    ```
- **Expected (real, @26147ba):**
    ```
        Finished `test` profile [unoptimized + debuginfo] target(s) in 0.08s
         Running tests/watch_external.rs (target/debug/deps/watch_external-2662a2ffe91650ba)

    running 4 tests
    test out_of_band_edit_emits_absent_actor_now_delta ... ok
    test rename_plus_edit_refuses_to_guess_delete_create ... ok
    test external_edit_reaches_live_sub_and_replays_identically ... ok
    test pure_external_rename_classifies_renamed_with_from_path ... ok

    test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.04s
    ```

---

#### wire_agreement — proto↔wire conformance floor  (§14 wire-agreement drift pin; §8 error taxonomy)

- **What:** The mechanical drift pin between `crates/wire` types and `crates/transport-proto`: every request op (all 10), every response body, every node kind (11) + info shape, every toc-row shape, and all 20 `ErrorCode` values are converted `wire→proto→encoded frame→proto→wire` via exhaustive struct-destructuring + exhaustive matches, `assert_eq` against the original (a new wire field/variant or proto-only oneof arm stops compiling here). `unspecified_severity_refuses_loud` is the negative twin — a `SEVERITY_UNSPECIFIED` proto zero-value is refused loud at the seam, never silently defaulted.
- **Verify:**
    ```
    cd /Users/Shared/repos/meridian-rs && cargo test -p transport-proto --test wire_agreement
    ```
- **Expected (real, @26147ba):**
    ```
        Finished `test` profile [unoptimized + debuginfo] target(s) in 0.07s
         Running tests/wire_agreement.rs (target/debug/deps/wire_agreement-38045350f105b2db)

    running 3 tests
    test unspecified_severity_refuses_loud - should panic ... ok
    test every_request_op_survives_wire_proto_wire ... ok
    test every_response_body_and_error_code_survives_wire_proto_wire ... ok

    test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
    ```

---

#### p4 probes — kill-class regression set MP-1..9  (§2.1/§2.4 mint plane, §4.3 kinds, §4.5 walk, §5.1 CAS)

- **What:** The named kill-class probe set (`p4-regression-probes.json`, MP-1..9) driven against the LIVE `sidecar::serve` dispatch over the vendored `walkvault`/`wsfix` fixtures: UC-R9 case-fold/duplicate mint refusals (§2.1/§2.4), W2 resolve-carries-no-rev + splice-speaks-strict-grammar-only (§2/§4.5), 007 no-offset-field + guardless-splice-legal (§4.4/§5.3), 011 charset-invalid id refused (§2.4), B3 GT-from-live-oracle (§14). MP-8's recorded `expect.ok:true` predates the frozen D-C5 §4.3 law; the runner asserts the frozen-text-lawful `bad_request{unknown_kinds}` answer (probe-file staleness resolved to frozen text — a documented adjudication, **not** a code finding; see `results/cv2-final-gate-wire.md` §3-A1).
- **Verify:**
    ```
    cd /Users/Shared/repos/meridian-rs && cargo test -p sidecar --test p4_probes
    ```
- **Expected (real, @26147ba):**
    ```
        Finished `test` profile [unoptimized + debuginfo] target(s) in 0.08s
         Running tests/p4_probes.rs (target/debug/deps/p4_probes-7d87c1003dd97f86)

    running 1 test
    test p4_probes_applicable_at_rung_2 ... ok

    test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.03s
    ```

---

> [!success] Wire-domain verification — all green, all real
> 7 named tests, **57 test cases, 0 failures**, fresh at `26147ba`:
> `dispatch_v2` 34/34 · `splice_e2e` 5/5 · `replay_live` 4/4 · `sub_push` 6/6 ·
> `watch_external` 4/4 · `wire_agreement` 3/3 · `p4_probes` 1 (MP-1..9). No STOP
> findings. `git -C /Users/Shared/repos/meridian-rs status --short` empty at end
> (verification-only, zero edits). The `wire_agreement` (3/3) and `p4_probes`
> (MP-1..9) tails match the final-gate `results/cv2-final-gate-wire.md` §1–§2.


---

## Policy + Ratchet

> **Provenance.** Every command below was **actually run by the worker**; every
> "Expected (real capture)" is a **real captured output**, not hand-written.
> Part A ran in the meridian-rs **main checkout** `/Users/Shared/repos/meridian-rs`
> @ `26147ba`. Part B ran at ccc-statusd commit **`cf59e31`** in an isolated
> detached worktree `/tmp/dv-ratchet-cf59e31` (`git worktree add --detach
> /tmp/dv-ratchet-cf59e31 cf59e31`). **Operational fence honored:** the running
> fleet daemon (PID 4294, started `Sun Jul 19 16:21:54 2026`) was never touched —
> no deploy / restart / install / `killall`; the installed binary
> `~/.local/bin/ccc-statusd` (mtime `Jul 19 16:21`) is unchanged; the one binary I
> built went to `/tmp/dv-ccc-statusd-cf59e31.bin` and was **never installed nor run
> against the fleet socket**.
>
> Two output provenance tags appear in Part B: **[gate-suite]** = the g-ratchet's
> own test suite run at `cf59e31` (real `go test` runs), **[live daemon]** = a
> command against a running daemon. **Both are real runs.** A hand-written
> "expected" would be a defect; there are none.
>
> ⚠️ **MATERIAL DEPLOY FINDING (read before deploying — does NOT block):** at
> `cf59e31` the ratchet is **implemented + tested but NOT yet wired into the live
> send path**. `internal/ratchet` is imported only by `ratchet_actor.go`, whose
> `ratchetActorForRequest` has **zero live callers** (only its own test);
> `ratchet.Evaluate` / `ResolveRatchet` are never called outside the package's own
> tests. Go's dead-code elimination therefore **strips the ratchet from the
> production binary** (`strings /tmp/dv-ccc-statusd-cf59e31.bin | grep -c 'ratchet
> refusal|unknown requirement key|none (permissive)'` → **0**). Consequence:
> deploying `cf59e31` is **behaviorally inert** for writes — day-one-permissive is
> guaranteed **three ways** (zero config requirements · `Evaluate` not on the live
> path · code DCE'd from the shipped binary). The gate suite is the standing proof
> that each gate behaves correctly for when the live wiring + config land in a
> LATER commit. See §"Day-one permissive" and the ZT post-deploy sequence.

---

## Part A — Policy (meridian-rs policy/sidecar, main checkout @ `26147ba`)

The §11.1 verdicts surface and the §8 `daemon_only` law are driven end-to-end
through `sidecar::serve` + `sidecar::admit` (the real policy + parse composition
edge) in `crates/sidecar/tests/verdicts_e2e.rs`. One command runs all of Part A.

```
$ cd /Users/Shared/repos/meridian-rs && git rev-parse HEAD
26147ba0f6d566852b35798b396beaf27dfa4b86
$ cargo test -p sidecar --test verdicts_e2e -- --nocapture
```

Real captured tail (`/tmp/dv-partA-verdicts.log`):

```
running 5 tests
test gate4_no_pack_still_carries_empty_verdicts ... ok
test gate2_corpus_class_pack_is_daemon_only_loud ... ok
test gate1_verdict_rides_dry_splice ... ok
test gate1_verdict_rides_real_splice ... ok
test ruling2_dry_verdicts_equal_real_verdicts_byte_identical ... ok

test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.09s
EXIT=0
```

#### Verdicts ride splice — dry AND real  (§11.1)
- **What:** every splice response, dry-run and real, carries `verdicts:[]`; the frozen §11.1 worked verdict (`blurb-required`, span `[20,150]`, `node_rev 5a8faa717fbcdb04`) rides both, and dry ≡ real byte-identically.
- **Verify:**
    cd /Users/Shared/repos/meridian-rs && cargo test -p sidecar --test verdicts_e2e -- --nocapture
- **Expected (real capture):**
    test gate1_verdict_rides_real_splice ... ok      # §11.1 verdict rides the REAL response; root advanced
    test gate1_verdict_rides_dry_splice ... ok       # identical verdict rides the DRY twin; root_after:null, dry:true
    test ruling2_dry_verdicts_equal_real_verdicts_byte_identical ... ok   # dry verdict bytes ≡ real verdict bytes
    test gate4_no_pack_still_carries_empty_verdicts ... ok   # no pack loaded → verdicts:[] (shape rides from birth)
    test result: ok. 5 passed; 0 failed; ... finished in 0.09s   (EXIT=0)

#### daemon_only — corpus-class pack refused loud in sidecar mode  (§8)
- **What:** a corpus-class rule pack (WHEN needs the resident corpus name index, e.g. `link_resolves`) loaded against a sidecar-mode engine is **refused LOUD at admission** with `daemon_only`, an **env**-class error (§8 / §11.3 `BudgetClass::Corpus` law).
- **Verify:**
    cd /Users/Shared/repos/meridian-rs && cargo test -p sidecar --test verdicts_e2e gate2_corpus_class_pack_is_daemon_only_loud -- --nocapture
- **Expected (real capture):**
    test gate2_corpus_class_pack_is_daemon_only_loud ... ok
    # asserts: err.code == wire::ErrorCode::DaemonOnly  AND  err.recovery == wire::Recovery::Env
    test result: ok. 1 passed; 0 failed; ...   (EXIT=0, within the 5/5 run above)

---

## Part B — Ratchet (ccc-statusd g-ratchet @ `cf59e31`)

Commit `cf59e31` is the advisor-slot merge that landed the wire-contract-v2 §5.3
policy ratchet (`merge: g-ratchet … wt-g-ratchet 08ca140`) onto ccc-statusd
`origin/main`. It is an ancestor of `origin/main`; the ratchet lives in
`internal/ratchet/` (ratchet.go · scope.go · resolve.go · refuse.go). **All five
gates are unit-level and live in `internal/ratchet` — no daemon restart needed to
prove them.** One command runs the whole gate suite:

```
$ cd /Users/Shared/repos/ccc-statusd && git worktree add --detach /tmp/dv-ratchet-cf59e31 cf59e31
$ cd /tmp/dv-ratchet-cf59e31 && git rev-parse HEAD
cf59e31b04728a5b19a6fcd2e57e39900a12e53f
$ go test ./internal/ratchet/... -v
```

Real captured result (`/tmp/dv-partB-ratchet-gates.log`) — **28/28 PASS, EXIT=0**:

```
PASS
ok  	github.com/caoer/ccc-statusd/internal/ratchet	1.179s
EXIT=0
```

The five gates below map the task's enumeration to the g-ratchet card's gate
numbers (task gate 5 = the card's gate 3, the fix-at-freeze flag-1 selector gate).

#### Gate 1 — unknown requirement key → HARD error naming the closed set  (g-ratchet gate 1)
- **What:** the requirement vocabulary is CLOSED; a typo'd key refuses loud at parse time (naming the whole set + a "did you mean…?" near-miss), never silently resolving to zero requirements.
- **Verify:**
    cd /tmp/dv-ratchet-cf59e31 && go test ./internal/ratchet/... -v -run 'TestParseRequirementKey|TestResolveRatchet_UnknownKeyIsHardErrorNotDegrade|TestResolveRatchet_FrontmatterUnknownKeyHardErrors'
- **Expected (real capture — [gate-suite]):**
    --- PASS: TestParseRequirementKey_UnknownIsHardErrorNamingSet (0.00s)
    --- PASS: TestParseRequirementKey_NearMissHint (0.00s)
    --- PASS: TestParseRequirementKey_NoHintForUnrelated (0.00s)
    --- PASS: TestResolveRatchet_UnknownKeyIsHardErrorNotDegrade (0.00s)
    --- PASS: TestResolveRatchet_FrontmatterUnknownKeyHardErrors (0.00s)
    # the exact hard-error string these tests pin (ratchet.go ParseRequirementKey):
    #   unknown requirement key "require_bogus" (closed set: require_actor, require_if_node_rev, require_receipt)
    #   — did you mean require_receipt…?      (near-miss hint appended for a typo of a real key)

#### Gate 2 — malformed scope → LOUD + PERMISSIVE  (g-ratchet gate 2)
- **What:** a broken selector (walk-grammar section, empty path) or an unparseable `ratchet.toml` **drops the scope (requirements do NOT fire) and renders a loud ⚠ degrade marker** — never silently strict, never silently off. This is the opposite polarity from Gate 1 (unknown key = ✖ hard error); the two never blur.
- **Verify:**
    cd /tmp/dv-ratchet-cf59e31 && go test ./internal/ratchet/... -v -run 'TestResolveRatchet_MalformedScopeLoudAndPermissive|TestResolveRatchet_EmptyPathDegrades|TestResolveRatchet_UnparseableFileDegrades'
- **Expected (real capture — [gate-suite]):**
    --- PASS: TestResolveRatchet_MalformedScopeLoudAndPermissive (0.00s)
    --- PASS: TestResolveRatchet_EmptyPathDegrades (0.00s)
    --- PASS: TestResolveRatchet_UnparseableFileDegrades (0.00s)
    # degrade markers rendered by ResolvedRatchet.Armed() (resolve.go):
    #   ⚠ malformed-scope     (broken selector — scope dropped, valid siblings keep arming)
    #   ⚠ malformed-config    (a present-but-unparseable ratchet.toml — loud, still permissive)

#### Gate 3 — default config = zero requirements → DAY-ONE PERMISSIVE  (g-ratchet gate 4)
- **What:** with no config present, `ResolveRatchet` yields ZERO scopes, is not degraded, and the armed notice reads `ratchet: none (permissive)`; the write-path gate PERMITS an actor-less, receipt-less, unguarded splice. Tightening a scope is a config edit, never a release.
- **Verify:**
    cd /tmp/dv-ratchet-cf59e31 && go test ./internal/ratchet/... -v -run 'TestResolveRatchet_DefaultIsZeroRequirements|TestBuiltinScopes_Empty|TestEvaluate_NoScopesPermitsEverything'
- **Expected (real capture — [gate-suite]):**
    --- PASS: TestResolveRatchet_DefaultIsZeroRequirements (0.00s)
    --- PASS: TestBuiltinScopes_Empty (0.00s)
    --- PASS: TestEvaluate_NoScopesPermitsEverything (0.00s)
    # armed notice pinned by the test (resolve.go Armed()):  ratchet: none (permissive)
    # and Evaluate(actor-less/receipt-less/unguarded splice, day-one) returns nil → PERMITTED

#### Gate 4 — refusal names SCOPE + missing FIELD + PROVENANCE layer  (g-ratchet gate 5)
- **What:** when a matching scope arms a requirement the frame does not satisfy, the write-path gate refuses **before the frame is sent**, and the refusal names the scope selector, every missing wire field, and the config provenance layer. The SAME frame, once satisfied, is PERMITTED (the gate discriminates — not a blanket deny).
- **Verify:**
    cd /tmp/dv-ratchet-cf59e31 && go test ./internal/ratchet/... -v -run 'TestEvaluate_RefusalNamesScopeFieldProvenance|TestEvaluate_RequireIfNodeRev_PerEdit|TestEvaluate_RequirementsAccumulate'
- **Expected (real capture — [gate-suite]):**
    --- PASS: TestEvaluate_RefusalNamesScopeFieldProvenance (0.00s)
    --- PASS: TestEvaluate_RequireIfNodeRev_PerEdit (0.00s)
    --- PASS: TestEvaluate_RequirementsAccumulate (0.00s)
    # refusal shape pinned by the test (refuse.go Refusal.Error() / Violation.String()):
    #   ratchet refusal: splice to "results/2026-07-19.md" blocked before send —
    #     scope "results/**" [session] requires wire field "actor" (require_actor, unsatisfied);
    #     scope "results/**" [session] requires wire field "receipt" (require_receipt, unsatisfied)
    # require_if_node_rev names the specific unguarded target, e.g. "... — unguarded edit Goals>Q3"
    # provenance layers ∈ {frontmatter, session, global, builtin}

#### Gate 5 — scope selector rejects non-strict-plane grammar  (g-ratchet gate 3 · fix-at-freeze flag 1)
- **What:** a scope's `section` member parses the **strict-plane sel grammar ONLY** (`{hpath=[…]}` / `{anchor=…}` / `{fm_key=…}`, §2.1); walk-plane linktext (`plan#Goals#Q3`, `#Goals`, `plan#^r-000042`), bare strings, arrays, non-strings, multi-member tables, and `_`-bearing anchors (§2.4 charset) all refuse.
- **Verify:**
    cd /tmp/dv-ratchet-cf59e31 && go test ./internal/ratchet/... -v -run 'TestParseSection_RejectsWalkPlaneGrammar|TestParseSection_AcceptsStrictForms|TestParseSection_RejectsUnderscoreAnchor|TestParseSection_ExactlyOneMember'
- **Expected (real capture — [gate-suite]):**
    --- PASS: TestParseSection_RejectsWalkPlaneGrammar (0.00s)
    --- PASS: TestParseSection_AcceptsStrictForms (0.00s)
    --- PASS: TestParseSection_RejectsUnderscoreAnchor (0.00s)
    --- PASS: TestParseSection_ExactlyOneMember (0.00s)
    # reject strings pinned (scope.go ParseSection):
    #   section member "plan#Goals#Q3" is not strict-plane grammar: ... the walk-plane linktext algebra is not accepted
    #   anchor "..." is outside the strict-plane charset [A-Za-z0-9-]+ (§2.4) — e.g. a '_'-bearing id is walk-plane only
    #   section member sets more than one of hpath / anchor / fm_key — exactly one strict-plane form is allowed (§2.1)

#### §9 actor chain — Go populates the wire actor (supporting, not a gate)
- **What:** the retired engine env chain moves to Go: `ratchetActorForRequest` fills the wire `actor` from the daemon's SINGLE identity chain (`detectSource()`: `CCC_AGENT_NAME` → `CLAUDE_SESSION_ID` → `cli`), preserving a caller-supplied actor and nil-safe. (Present in source; see the wiring finding — no live caller at `cf59e31`.)
- **Verify:**
    cd /tmp/dv-ratchet-cf59e31 && go test . -run 'TestRatchetActorForRequest' -v
- **Expected (real capture — [gate-suite]):**
    --- PASS: TestRatchetActorForRequest_ReusesDetectSourceAgentName (0.00s)
    --- PASS: TestRatchetActorForRequest_SessionIDTier (0.00s)
    --- PASS: TestRatchetActorForRequest_PreservesCallerActor (0.00s)
    --- PASS: TestRatchetActorForRequest_NilSafe (0.00s)
    ok  	github.com/caoer/ccc-statusd	0.556s      (EXIT=0)

#### Repo-native standing battery @ `cf59e31` (g-ratchet card gate 6)
- **What:** the whole repo's Go battery is green at the landed commit — confirms the ratchet did not break any sibling package.
- **Verify:**
    cd /tmp/dv-ratchet-cf59e31 && (cd injection && bun install --frozen-lockfile && bun run build) && go test ./... && go vet ./...
    #   (the injection embed is a gitignored //go:embed frontend asset; built fence-safely from the
    #    worktree's own source — NOT the daemon binary, NOT installed. Needed only for package-main tests.)
- **Expected (real capture — [gate-suite], `/tmp/dv-partB-battery.log`):**
    ok  	github.com/caoer/ccc-statusd	39.154s        # package main incl. the §9 actor tests
    ok  	github.com/caoer/ccc-statusd/internal/ratchet	2.667s
    ... (all packages ok; the rest are [no test files]) ...
    GO_TEST_EXIT=0
    GO_VET_EXIT=0
    # fence-safe compile of the daemon binary (NOT installed, NOT run):
    #   go build -o /tmp/dv-ccc-statusd-cf59e31.bin .   → BUILD_EXIT=0
    #   /tmp/dv-ccc-statusd-cf59e31.bin --version → ccc-statusd version dev (compatible with Claude Code 2.1.197)

---

## Day-one permissive — the confirmation ZT cares about

**Deploying `cf59e31` changes nothing about how writes behave.** Full permissiveness
holds by THREE independent guarantees, any one of which suffices:

1. **Zero config requirements (Gate 3).** `builtinScopes()` is empty and no
   `ratchet.toml` / frontmatter `ratchet:` line exists in the fleet, so
   `ResolveRatchet` → `ratchet: none (permissive)`, and `Evaluate` permits every
   frame. Tightening is a future config edit, never this release.
2. **Not on the live path.** `ratchet.Evaluate` (the write-path gate) has **no live
   caller** at `cf59e31` — the actual wiring into the daemon send path is a later
   commit. Even a hypothetical config could not fire because nothing calls the gate.
3. **DCE'd from the shipped binary.** Because (2), Go's linker strips the ratchet
   message code from the production binary (0 ratchet literals in
   `/tmp/dv-ccc-statusd-cf59e31.bin`).

So the day-one truth is stronger than "permissive by config": the ratchet is
**inert** at `cf59e31`. Enforcement arrives only when a LATER commit both wires
`Evaluate` into the send path AND a config adds requirements.

### ZT post-deploy command sequence (rebuild + restart, then confirm)

`cf59e31` is on `origin/main`; deploy from there. **Reversible** (a normal rebuild
+ restart; the prior generation of the binary rolls back by rebuilding the prior
commit). Run in the ccc-statusd repo:

```
# 0. fetch + land on a tree that contains cf59e31 (origin/main does)
cd /Users/Shared/repos/ccc-statusd
git fetch origin && git merge-base --is-ancestor cf59e31 origin/main && echo "cf59e31 ∈ origin/main ✓"

# 1. rebuild + install + restart the daemon (ZT's normal deploy step)
go build -o ~/.local/bin/ccc-statusd . && ccc-statusd restart      # or your usual restart path

# 2. LIVE confirm the daemon is back on the new binary
ccc-statusd --version           # → ccc-statusd version <your build> (compatible with Claude Code …)
ccc-cli session ps              # → the fleet roster still computes (daemon serving)

# 3. LIVE confirm day-one permissive: nothing is refused
#    (there is no `ratchet` subcommand at cf59e31 — the ratchet is not on the live
#     path yet — so "permissive" = normal messaging/splicing continues unchanged.)
ccc-statusd session send <some-id> 'post-deploy ping'   # delivers normally, no ratchet refusal
ls ~/.config/ccc-statusd/ratchet.toml 2>/dev/null || echo "no ratchet.toml → ResolveRatchet = none (permissive)"

# 4. BEHAVIORAL proof of every gate against the deployed tree (standing suite)
go test ./internal/ratchet/... -v      # → 28/28 PASS, ok … EXIT=0  (the [gate-suite] output above)
```

**Expected LIVE outcome:** steps 2-3 show the daemon healthy and messaging
unchanged — **no ratchet refusal is possible**, because at `cf59e31` the gate is
not wired in. Step 4 reproduces the 28/28 gate suite, the standing proof that each
gate will behave as specified once the enforcement wiring + config land in a later
commit. There is no `[live daemon]` capture of a ratchet *refusal* at this commit
**by construction** — a refusal requires the live wiring that `cf59e31` does not
carry; the gate-suite output is the real proof standing in for it, exactly as the
task anticipates.

---

## Evidence index (all real, on this host)

| File | What |
|---|---|
| `/tmp/dv-partA-verdicts.log` | Part A — `cargo test -p sidecar --test verdicts_e2e` (5/5 PASS, EXIT=0) |
| `/tmp/dv-partB-ratchet-gates.log` | Part B — `go test ./internal/ratchet/... -v` (28/28 PASS, EXIT=0) |
| `/tmp/dv-partB-battery.log` | Part B — embed build + `go test ./...` + `go vet ./...` (all EXIT=0) |
| `/tmp/dv-partB-actor-and-build.log` | §9 actor tests (4/4 PASS) + fence-safe daemon build (BUILD_EXIT=0) |
| worktree `/tmp/dv-ratchet-cf59e31` | detached `cf59e31` checkout used for all Part B runs |

**STOP-and-report note:** no check FAILED — Part A 5/5, Part B 28+4 gates + full
battery all green. The one flagged item is a **scope fact, not a failure**: the
ratchet is not yet wired into the live send path at `cf59e31` (implemented +
tested + DCE'd), which is surfaced here rather than papered over and which
*strengthens* the day-one-permissive guarantee. No policy/daemon/test/fixture code
was modified.
