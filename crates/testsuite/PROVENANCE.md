# testsuite crate provenance — regen log

Crate-level regen/verification events for the golden surfaces this crate
carries. Pack-scoped provenance stays in each pack's own `PROVENANCE.md`
(`data/gt/`, `data/gt/obsidian-compat/`, `data/harness/`, `data/wsfix/`,
`data/charset-guard/`).

## PF-FIXTURES — frozen-worked-value sweep + id-72 close (2026-07-19)

Contract: `wire-contract-v2.md` FROZEN (ZT, 2026-07-18, decision 014); banner
verified directly at claim (frontmatter `status: frozen`; `> [!important] FROZEN`).
Base: this tree @ `379f63b` (PF-GOLDENS landed). Unit: pack §10.5 + advisor
dispositions (9074c86b). Oracle-of-record (PACK ERRATA — `scratch/compute.py`
does not exist): `wire-contract-v2-verify.py::root_of` (frozen §12.2).

**What landed:**

- **`data/wsfix/s2/receipts/2026-07-18.md` (474 B, COMPUTED)** — the id-72
  closure. S0 receipts + frozen §6.3 E3/E4 receipt lines; `file_rev
  9167b12b0eb13be6` (§7.1). Never hand-written; re-derived + byte-checked in the
  sweep. Provenance: `data/wsfix/PROVENANCE.md`.
- **`tests/pf_frozen_sweep.rs`** — the frozen-worked-value SWEEP (gate 4): every
  distinct worked value the frozen text prints (6 roots + 17 revs + 17 span
  literals + 8 §3.1 id lexemes) pinned by an engine-recomputed assertion
  (`model::build`, `wire_map::project_toc`, `model::merkle_root`, `model::walk`,
  `transport::scan_id`) over the committed `wsfix/` S0 bytes + frozen §4.4 edits.
  Includes the §4.5 worked resolve fixture (id-72 closure). Grain discipline
  pinned explicit: **A4/A5** distinct terminator families (248↔249, 473↔474
  as-is, never normalized), **A7** full-token root compare (`b3:` vs `b3a:`,
  hex-tails equal / tokens not), **A3** by-design `2731acfa…` equality named,
  **A2/A6** single-sourced values re-derived not copied.

**Result: sweep GREEN (14 tests), workspace 285/0. Zero engine-crate lines
(boundary diff: `crates/testsuite` + `data/` only). No wrong-grain value found —
no grain-defect-#3.** The completeness proof (grep frozen → pin map, zero
unmapped) and the deviation-row enumeration (gate 2) are session artifacts under
`results/pf-fixtures/`.

Gate commands (this run, verbatim, &&-chained):

```
cargo fmt --all -- --check && cargo test --workspace \
  && cargo clippy --workspace --all-targets -- -D warnings
uvx --from blake3 python3 .../wire-contract-v2-verify.py   # 69/69, exit 0
```

## PF-GOLDENS — POST-FLIP goldens regen (2026-07-19)

Contract: `wire-contract-v2.md` FROZEN (ZT, 2026-07-18, decision 014); banner
verified directly at claim. Base: main `09b3afd` (32/36). Unit ruled by the
advisor of record (9074c86b) at the comprehension-echo gate: the §10.5
objective — every golden byte traces to FROZEN law — is satisfied **by
construction**, because this tree stores no draft-era golden files to
regenerate.

**Result: zero-diff by construction; zero golden bytes changed; zero
hand-edited golden bytes.**

The worked-exchange and projection goldens are *computed* Flag-A assertions —
the engine re-derives every worked value on every test run and asserts it
byte-exact against the frozen printed values, a continuous regen strictly
stronger than a one-time file regen. The planned `wire_golden` stored-golden
module (`tests/main.rs` doc comment) never landed because it was never needed.

Regen commands (this run, verbatim):

```
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Read-only convergence cross-check (frozen §0.3 printed bytes vs `data/wsfix/`,
byte-identical, `cmp` exit 0 both files; sizes 136/26/150 per §0.3):
`cmp <(§0.3 printed bytes) data/wsfix/s0/{notes/plan.md,receipts/2026-07-18.md}`.
The `compute.py` oracle RE-RUN is PF-FIXTURES' (§10.5 assigns it there).

Golden-surface classes at this regen:

| Class | Location | Values produced by | Regen disposition |
|---|---|---|---|
| Worked-exchange + projection assertions | `tests/{delta_e3e4,wsfix_oracle,wire_vocab}.rs`; `sidecar/tests/dispatch_v2.rs` (worked §4.1 toc / §4.2 cat exchanges) | engine (`syntax::parse`, `model::build`, `model::merkle_root`, wire serde) at every run | computed, zero-diff by construction |
| Parse-projection GT | `data/gt/ground-truth/` | lane0-frozen parser-bench pack (sha-manifest-pinned) | not contract-derived; PF-GT-RETARGET's demotion |
| App-oracle pack | `data/gt/obsidian-compat/` | live Obsidian resolver (`generate.sh`) | app-truth; regen trigger is app-version drift (§13.3–13.4), not the flip |
| Adversarial probe packs | `data/harness/` | vendored verbatim, sha256-pinned | provenance law: never regenerated; draft-vs-frozen deviations asserted text-lawful runner-side |
| §0.3 fixture pack | `data/wsfix/` | `compute.py` blake3 oracle | byte-verified vs frozen §0.3 this run; oracle re-run is PF-FIXTURES' |
| Charset pack | `data/charset-guard/` | `syntax::is_block_id` via `model::Ref::anchor` | decision-011 derived; nothing draft-era |

Frozen §4.3 prints no worked `extract` exchange ("stands as frozen
(`crates/wire` §5)") — extract's pins are vocab/behavior assertions
(`wire_vocab.rs` kind enum + ordinals; `dispatch_v2.rs` unknown-kinds refusal,
kinds filter, S2/L22 node_rev). No printed extract values exist to converge to;
PF-FIXTURES' sweep is the independent backstop.
