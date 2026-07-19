# wsfix — the §0.3 worked-fixture pack (M2-BUILD oracle)

Promoted from the tournament winner's independent oracle for the M2-BUILD gate.
These bytes are **data, not authored** — every expected `node_rev` / `file_rev`
the gate asserts is computed by the oracle script, not hand-written.

## Source

- Fixture bytes + oracle: `results/contract-v2-tournament/round-3/team-c/scratch/`
  - `compute.py` — the master computation (blake3-256 via the official `blake3`
    PyPI wheel; byte offsets counted over the raw fixture bytes).
  - `wsfix/` — the fixture files it writes.
- Contract law the values back: `results/wire-contract-v2.md` §0.3 (the fixture),
  §1 (rev law + span sub-laws), §18 row 3 (frontmatter fence-to-fence span),
  §4.1 (the worked `toc` at S0 — every value shown).

## Regeneration

```
cd results/contract-v2-tournament/round-3/team-c/scratch
uvx --from blake3 python3 compute.py    # prints all values, (re)writes wsfix/
```

## Files here (byte-exact copies)

| Path | Bytes | Source in scratch | file_rev (oracle) |
|---|---|---|---|
| `s0/notes/plan.md`         | 136 | `wsfix/v0/notes-plan.md` | `e3c4acaceb75b907` |
| `s0/receipts/2026-07-18.md`|  26 | `wsfix/v0/receipts.md`   | `920a40c4ee23d37c` |
| `s2/notes/plan.md`         | 150 | `wsfix/notes/plan.md`    | `5f27a2814b517680` |

Consumed by `crates/testsuite/tests/wsfix_oracle.rs`. The `node_rev` / `file_rev`
expectations embedded there are the `compute.py` output verbatim (S0 `toc`,
S2 `toc_s2`); the test recomputes `file_rev` independently with blake3 (a
test-only dev-dependency, conforming per M2-BUILD gate 3) and asserts it equals
both the oracle value and the built root node's `node_rev`.
