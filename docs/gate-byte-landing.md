# The armed-plane gate — the law, and what is derived (U4.2; census struck)

Status: enforcement doc for the U4.2 `gate()` seam. Law: plan §4 Block 4;
`docs/wire-contract-v2-refusal-amendment.md`; `docs/laws.md` § the policy gate.

**Measured at `b7c92d5a`, 2026-07-26.** This page states a law and describes an
instrument. It contains no census — see § Why the census is gone.

## The law

`gate()` refuses an armed change **after CAS, before bytes land** (§11.1
amendment). Every gated site evaluates the SAME
`policy::gate(change, armed_set)` over a `rulepack-api@2` change surface built
from the before/after states. The armed set is loaded and verified from the
workspace's OWN attested INDEX + once-armed marker inside the trusted write
path (`wire_serve::gate::load_armed_set` / `run::gate::load_armed_set`), never
a caller-supplied set — so no caller can weaken the decision at any gated site.

## What is derived from source

`crates/wire-serve/tests/u12_door_enumeration.rs` is the only instrument that
reads the tree. Stated exactly, because the difference matters:

It walks every crate's production `src/` except `model`, truncates each file at
its first `#[cfg(test)]`, skips lines beginning with `//`, and looks for two
constructor names — `candidate_of_body(` and `candidate_of_batch(`. A file
carrying at least one such call is recorded **once**. The test then asserts that
this **set of FILES** equals the set its pinned table names.

At `b7c92d5a` that derived set is **three files**:

- `crates/wire-serve/src/write.rs`
- `crates/mrd/src/realise_cmd.rs`
- `crates/run/src/fp.rs`

**That is the entire source-derived claim: three file names.** It fails when a
candidate is minted in a file not on that list — which is a real and useful
guarantee, and is the whole of it.

## What is NOT derived — do not read it as checked

The same test carries a hand-written table classifying eight doors by
`file::function`, and two further assertions. None of the following is measured
against the tree:

- **Which function in a file mints.** The set comparison keeps the file column
  and discards the function column, so every `file::function` row is prose. It
  is accurate prose, written by U12; it is not a check.
- **A new mint inside a file already on the list.** The scan records a file once
  and stops reading it. A ninth mint added to `write.rs` changes the derived set
  not at all.
- **The door count.** The assertion that the table holds eight rows measures the
  hand-written array against itself.
- **Whether any door calls the policy gate.** A guard is a call, not a type, and
  no assertion attributes a call to a function. The test that counts guard calls
  in `write.rs` counts lines in a file; moving a call between functions in that
  file does not fail it.

Gate coverage is therefore **not stated on this page and not derived anywhere**.
Determining it is a source-reading exercise whose result rots; the standing gap
is recorded with the Core lane rather than restated here as prose nobody checks.

## Why the census is gone

This page carried a six-row prose census whose load-bearing claim was that the
list was complete. It was last measured at `340c4de6` (2026-07-23) and carried
no measurement stamp. By `b7c92d5a` it had rotted past repair: one row named
`wire_serve::write::pin_lock` and a `crates/pin` crate, **neither of which
exists** (see `crates/mrd/tests/retired_verbs.rs`); another row's migrate kit
has no crate in-tree; the anchor promotion in `write.rs` and the `realise`
deploy door were never in it; and it dismissed `wire_serve::write::commit_batch`
as *"not a separate byte-lander"* on the strength of a **caller count** — a
criterion the code itself has since rejected in `commit_batch`'s own comment.

**Re-derive or strike, no third state** (S3-R23(4)). The predicate this page
needed — *lands bytes, gated or exempt* — is not the predicate the instrument
derives, and re-deriving it means building a second instrument. So the census is
struck rather than restated, relocated, or re-pinned in another form. What
survives above is the law, and an honest description of what one test checks.
