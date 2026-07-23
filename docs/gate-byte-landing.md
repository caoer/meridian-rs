# The armed-plane gate — byte-landing enumeration (U4.2)

Status: enforcement doc for U4.2 `gate()` at the seam. Law: plan §4 Block 4, U4.2
byte-landing enumeration; `docs/wire-contract-v2-refusal-amendment.md`; `docs/laws.md`
§ the policy gate.

`gate()` refuses an armed change **after CAS, before bytes land** (§11.1 amendment).
For that guarantee to hold, EVERY place bytes reach disk must be either GATED
through the one evaluator (`policy::gate` over the change it produces) or EXEMPT
with a stated rationale. This page is that census — the load-bearing claim is that
the list below is complete.

## The two writer paths

The wire write door is served by exactly two hosts, both dispatching the `Splice`
op to the ONE `wire_serve::write` choke-point — so gating the choke-point gates
both:

| Writer path | Crate | Dispatch | Choke-point call |
|---|---|---|---|
| per-workspace **sidecar** | `sidecar` | `arms::dispatch` (`arms.rs`) | `wire_serve::write::splice` |
| resident **registry daemon** | `registry` | `dispatch_read` (`server.rs`) | `wire_serve::write::splice` |

Neither host dispatches `Create`/`Remove` as a wire op; those land via the run
plane's board-card mint and `mrd test`, both of which call the same gated
choke-point functions.

## Byte-landing census

| # | Byte-landing site | Status | How / rationale |
|---|-------------------|--------|-----------------|
| 1 | **wire splice** (`wire_serve::write::splice`) | **GATED here** | `crate::gate::enforce_gate` at the ONE §11.1 verdict site, after CAS + validate, before the D4 commit. Runs before the dry short-circuit (a rehearsal of a refused write is still refused). Covers BOTH writer paths (sidecar + registry). |
| 2 | **guarded create / remove** (`wire_serve::write::create` / `remove`, U2.6) | **GATED** | Same `enforce_gate` seam over the birth's `before=absent` / death's `after=absent` change surface. |
| 3 | **run-plane apply + `receipts/run.md` append** (`run::executor::apply_under`, U3.x) | **GATED** | The run plane lands bytes through `fs::apply_batch`, NOT the wire choke-point, so it mounts the SAME evaluator (`run::gate::refuse_reason` → `policy::gate`) at step 6b — before the commit. The `receipts/run.md` append rides the same sealed `fs::apply_batch`, so it is gated with the change that produces it. Scenario 7 + falsification. |
| 4 | **journal append** (`fs::append_line` → `meridian/journal.md`, U2.1) | **EXEMPT** | Receipt-engine-only path. An ordinary `splice`/`create`/`remove` targeting the reserved journal is REFUSED (`reserved_journal_guard`, `fs::domain::is_reserved_journal`); the engine's own append is not a wire op and does not re-enter the write choke-point. The gate would never (and must never) see it. |
| 5 | **migrate kit** (strict writer, U3.2) | **GATED by construction** | The migrate kit writes through the strict writer (`splice`/`commit_batch`), so it rides the gated choke-point (#1). NOTE: no migrate-kit byte-lander exists in-tree yet (U3.2 pin-leg in-progress at review time); when it lands it inherits row #1's gate — there is no second write path to add. |
| 6 | **`^inputs` lock-write** (`wire_serve::write::pin_lock`, U2.4) | **EXEMPT** (sanctioned, Block 4 leader ruling at U4.2 merge) | Engine's own act, same trust class as row 4: `new_text` is engine-rendered by `pin` (`crates/pin` is the only production caller; not dispatched as a wire op by either host), the triggering `inputs:` manifest edit rides the gated splice (#1), and the write itself is CAS + confinement + reserved-journal guarded and journaled. Gating it would gate an engine derivation, not a user change. Residual (stated): in-process code calling `pin_lock` with fabricated bytes is inside the TCB — the same residual class as the receipt engine's journal append. |

## Findings (byte-landers beyond the law's enumeration)

The plan's byte-landing enumeration lists rows 1–5. Surveying the current tree
surfaced ONE further byte-lander not in that list — reported, not silently gated
(per the U4.2 work order: "more than two byte-landing writer paths … is a finding"):

- **`wire_serve::write::pin_lock`** (the guarded `^inputs` lock-write, U2.4) —
  found ungated and absent from the plan's enumeration. **RULED EXEMPT** at the
  U4.2 merge (Block 4 leader, rationale verified against the code); now census
  row #6 above. Flagged upward for the plan's enumeration text to be amended by
  its owner.

- `wire_serve::write::commit_batch` has **no production caller** (tests only); it
  is the shared commit seam rows #1/#2 use internally, not a separate byte-lander.

## The evaluator is one

Rows 1–3 all evaluate the SAME `policy::gate(change, armed_set)` over a
`rulepack-api@2` change surface built from the before/after states. The armed set
is loaded and verified from the workspace's OWN attested INDEX + once-armed marker
inside the trusted write path (`wire_serve::gate::load_armed_set` /
`run::gate::load_armed_set`), never a caller-supplied set — so no caller can weaken
the decision at any of the three gated sites.
