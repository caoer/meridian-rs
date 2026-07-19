# wsfix — the §0.3 worked-fixture pack (M2-BUILD oracle)

Promoted from the tournament winner's independent oracle for the M2-BUILD gate.
These bytes are **data, not authored** — every expected `node_rev` / `file_rev`
the gate asserts is computed by the oracle script, not hand-written.

## Source

- **Independent oracle (PACK ERRATA — the pack's `scratch/compute.py` does NOT
  exist):** `wire-contract-v2-verify.py::root_of` (frozen §12.2), at
  `.../18-02-meridian-rs/results/wire-contract-v2-verify.py`. It rebuilds the
  §0.3 fixture from the FROZEN document's own text and asserts every value
  (blake3-256 via the official `blake3` PyPI wheel; byte offsets counted over
  the raw fixture bytes).
- Contract law the values back: `results/wire-contract-v2.md` §0.3 (the fixture),
  §1 (rev law + span sub-laws), §18 row 3 (frontmatter fence-to-fence span),
  §4.1 (the worked `toc` at S0 — every value shown), §6.3 (the receipt lines),
  §7.1 (the S1/S2 `file_rev`s).

## Regeneration / oracle re-run (PF-FIXTURES, 2026-07-19)

```
# Gate-1 oracle re-run — byte-exact against the fixture the doc rebuilds:
uvx --from blake3 python3 \
  .../18-02-meridian-rs/results/wire-contract-v2-verify.py    # 69/69, exit 0
```

## Files here

| Path | Bytes | Provenance | file_rev (oracle) |
|---|---|---|---|
| `s0/notes/plan.md`          | 136 | §0.3 printed bytes | `e3c4acaceb75b907` |
| `s0/receipts/2026-07-18.md` |  26 | §0.3 printed bytes | `920a40c4ee23d37c` |
| `s2/notes/plan.md`          | 150 | §4.4 E3+E4 applied to S0 (cross-checked ≡ derived) | `5f27a2814b517680` |
| `s2/receipts/2026-07-18.md` | 474 | **COMPUTED (id-72): S0 receipts + frozen §6.3 E3/E4 lines** | `9167b12b0eb13be6` |

**id-72 closure.** `s2/receipts/2026-07-18.md` (474 B) is the S2 receipts state
the §4.5 worked resolve fixture needs (`resolve 2026-07-18` → whole file
`[0,474]`). It is **computed, never hand-written**: `receipts_v0` (the committed
26 B S0 file) + the frozen §6.3 E3 receipt line + E4 receipt line (each with its
trailing `\n`; the interpolated `root_before` values are the oracle-computed
R0/R1). `tests/pf_frozen_sweep.rs::s03_states_byte_counts_and_committed_s2_identity`
re-derives the same bytes from the engine and asserts the committed file is
byte-identical (len 474, `file_rev 9167b12b0eb13be6`) — a hand edit would fail
it. The python oracle constructs the identical `RECEIPTS_V2` independently.

Consumed by `tests/wsfix_oracle.rs` (S0/S2 plan node+file revs) and
`tests/pf_frozen_sweep.rs` (the full frozen-worked-value sweep + the §4.5 resolve
fixture). Every embedded `node_rev`/`file_rev`/`span` is engine-recomputed and
independently blake3-checked (a test-only dev-dependency, conforming per M2-BUILD
gate 3), never transcribed.
