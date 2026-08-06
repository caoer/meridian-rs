# tools/wire-contract

`docs/wire-contract.md` is **generated**. Do not edit it.

The citation-bearing master lives in the wiki domain
(`domains/knowledge/meridian-rs/wire-contract-v2.footnoted.md`) and is the ONE
PLACE the contract is authored. The repo carries the footnote-stripped
derivative so the constitution ships with the engine and reads self-contained —
no unresolvable references to session workspaces, no citation apparatus.

That split is the DX-04/07 resolution (2026-08-02). Before it, the master lived
in a closed session's `results/` dir, which made every edit either a fork or an
archaeology exercise.

## Verbs

```
tools/wire-contract/derivation-test.sh            # check
tools/wire-contract/derivation-test.sh --regen    # regenerate from the master
```

The check asserts `docs/wire-contract.md == strip(master)` byte for byte,
with the master pinned by wiki path + sha256. Two ways it fails, each naming its
own fix:

- **the master moved under the pin** — the master was edited; the derivative is
  stale. Run `--regen`, then commit the regenerated doc and the bumped
  `master_sha256` together.
- **published != strip(master)** — someone hand-edited the derivative. That
  change cannot survive; the next regeneration erases it. Move it into the
  master first.

Without a wiki checkout (`CCC_LLM_WIKI_PATH` unset) the identity gate is
**skipped loudly** and the master-free checks still run — strip-stability and
footnote residue. The wiki is a private checkout and this repo does not require
it to build.

## Files

| File | Origin |
| -- | -- |
| `strip-footnotes.sh` | verbatim from the 2026-07-19 closeout — the only transform between master and published |
| `verify-strip.sh` | verbatim from the same closeout — idempotency, residue, self-containment, body identity |
| `derivation-test.sh` | the DX-04/07 addition — pins the master and gates the two directions of drift |

`strip-footnotes.sh` is deliberately content-blind: it deletes footnote
definition blocks and inline `[^id]` markers and changes no other byte. Prose
that reads wrong after stripping is repaired **in the master**, never by
teaching the transform to rewrite text — `verify-strip.sh` gate D asserts
exactly that blindness, and a transform that edits prose makes every
regeneration unauditable.
