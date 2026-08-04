# RETIREMENT — how this crate deletes itself

> [!IMPORTANT] SUSPENDED — the trigger is UNFIRED and UNSCHEDULED (2026-08-04)
> **The conditions below are NOT rewritten and NOT false.** They were never met,
> and the event that would have met them is no longer scheduled on this docket.
>
> **The authority.** ZT ruled the field sweep aborted: *"MANUAL ONE-TIME CUTOVER
> FOR THE TWO REAL LOCKS. ABORT THE FLEET VAULT-WINDOW OPS"* — the cost and risk
> of a fleet campaign exceeding its value at n=2. The ruling **unschedules the
> FIELD RUN for this docket and explicitly does not unbuild anything**:
> ***"abort is not delete."*** The machinery — this crate, the governed
> `wire_serve::write::lock_migrate` door, and `--expect-root` — **stays BUILT AND
> GATED at `0f191cc2`** (*"u9b: arm the world guard — `mrd lock migrate
> --expect-root <ROOT>`"*).
>
> **So condition 1 below — *the real sweep EXECUTED over every vault in the
> field* — has no scheduled occasion to become true.** What went false is not the
> condition; it is the expectation that it fires on this docket.
>
> **THE SITES THIS SUSPENSION GOVERNS — a list, not a property:**
>
> 1. `crates/lockmigrate/RETIREMENT.md` — the § When conditions below.
> 2. `docs/laws.md` — the `lockmigrate` charter row's *"Deletes itself once the
>    sweep is executed and broadcast"*.
>
> Both carry this annotation. If a third site ever states the trigger, it is
> owed one and this list is owed a row — naming the sites is what keeps a
> suspension from being discharged in one place and left standing in another.
>
> **THE COUNTER, RECORDED RATHER THAN DISMISSED.** A suspended clause with no
> scheduled trigger **may never fire at all**, and this crate could carry a
> self-retiring label indefinitely — a permanent "temporary". That cost is
> **accepted**, not overlooked: rescheduling the field run is a ZT
> overturn-on-sight question and is carried to U25. The alternative — restating
> the trigger — would be authoring NEW retirement law, which is amendment-class
> and needs ZT.
>
> **What this annotation is.** The record catching up to law ZT already changed
> (restoration-class), never a new trigger (amendment-class). Nothing below is
> edited.

**SELF-RETIRING (U9b).** This crate is a migration kit. Its deletion is in U9b's
definition of done, not a cleanup someone might get to. The pattern and its
precedent: `1276c240` (the U3.2 migrate kit) → `0e17c143` (its retirement).

## When

After **all** of:

1. the real sweep has been EXECUTED over every vault in the field (the P13
   runbook's step 5), with a pre-sweep commit in each vault;
2. the migration report shows **zero refusals** and the NOT-ENGINE-PLACED list
   has been reviewed by a human and accepted;
3. the U22 repair has run over the expected drift (P13 step 7);
4. the cutover is broadcast — nothing in the field needs a v1 read again.

**Not before.** A vault that has not been swept still needs this tool, and a
deleted tool is not recoverable on the timescale of an operator discovering the
problem.

## What to delete

| Site | What |
|---|---|
| `crates/lockmigrate/` | the whole crate — `Cargo.toml`, `src/lib.rs`, `src/v1.rs`, `tests/gates.rs`, `tests/field_dryrun.rs`, this file |
| `crates/mrd/src/lockmigrate_cmd.rs` | the `mrd lock migrate` verb |
| `crates/mrd/src/lib.rs` | the `mod lockmigrate_cmd;` decl, the `"lock"` dispatch arm AND its `dispatch_lock` fn, and the USAGE block entry |
| `crates/mrd/Cargo.toml` | the `lockmigrate` dep line and its comment |
| `Cargo.toml` (workspace) | the `crates/lockmigrate` member + the `lockmigrate` dep line; regenerate `Cargo.lock` |

**`crates/wire-serve/src/write.rs`'s `lock_migrate` door dies with it** — args,
outcome and function — and with the door go:

| Site | What |
|---|---|
| `crates/wire-serve/tests/u12_door_enumeration.rs` | the `lock_migrate` `DoorPin` row; `Guarded` count **4 → 3**; the door count prose **8 → 7**; "all six guarded" → five; "five of the six" → four of the five |
| `crates/lock/src/lib.rs` | `block_spans` **only if nothing else has taken a dependency on it** — check first; `block_texts` must keep working either way |

## The assertions that prove it landed

Run these verbatim; each must hold.

```bash
# 1. The v1 grammar is gone from the repo entirely.
grep -rn --include='*.rs' '  - ref: ' crates/ | grep -v tests/    # → 0 hits

# 2. The self-retirement marker is gone.
grep -rn 'SELF-RETIRING' .                                        # → 0 hits

# 3. The verb is gone, and says so.
mrd lock migrate --vault /tmp   # → exit 2, "unknown subcommand: lock"
mrd --help | grep 'lock migrate'  # → 0 hits

# 4. The census shrank by exactly one door.
cargo test -p wire-serve --test u12_door_enumeration              # → green at 7

# 5. Everything still builds and passes.
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --no-fail-fast
```

**Assertion 1 is the one that matters.** The whole reason `crates/lock` may read
v2 only is that the v1 grammar is spelled in one deletable place; after
retirement it is spelled nowhere, and P4 is no longer a claim about discipline
but a fact about the tree.

Until then the same property is gated live by
`tests/gates.rs::the_v1_grammar_is_spelled_only_in_this_crate`, which scans
production `src/` and fails if the grammar leaks out of this crate. That gate
dies with the crate, and assertion 1 is what replaces it.

## Known cutover debt this crate does NOT own

Measured while building the quarantine gate, recorded here because it is the
natural place a retiring engineer will look. **Seven test suites still BUILD v1
`meridian-lock` fixtures**, and those pages are unreadable by `crates/lock`:

- `crates/run/tests/executor.rs`
- `crates/wire-serve/src/positions.rs` (inside its `mod tests`)
- `crates/wire-serve/tests/s2fix_artifact_guard.rs`
- `crates/mrd/tests/status_e2e.rs`
- `crates/mrd/tests/address_owner.rs`
- `crates/mrd/tests/f6_check_sees_the_mount_table.rs`
- `crates/mrd/tests/s2fix_cross_surface.rs`
- `crates/mrd/tests/color_planes_e2e.rs`

These are test DATA, not readers, so they do not defeat P4 — but they must be
refreshed to v2 by whoever owns those suites, and assertion 1 above deliberately
excludes `tests/` so that this debt cannot silently block the retirement.
