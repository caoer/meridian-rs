---
updated: 2026-07-24
---

# Status

A snapshot of what is built and verified today. Numbers here are reproducible
from the commands shown — prefer running them over trusting this prose.

## Build

- Toolchain: Rust edition 2024, `rust-version = 1.96`.
- `cargo build` builds the twenty-seven default members (the engine planes plus
  the `workspace` / `cache` / `registry` / `mrd` CLI foundation, and stage-2's
  `git` plumbing leaf); `perfsuite` is out of default-members and builds under
  `cargo build -p perfsuite`.
- Fork: `pulldown-cmark` is consumed via a `[patch.crates-io]` rev pin (the
  `obsidian` branch); see the workspace `Cargo.toml`.

## Wire surface

The sidecar answers protocol 1 as `meridian-sidecar/2.0`. Armed v2
capabilities, reported in the `hello` handshake (`crates/sidecar/src/lib.rs`,
`CAPS`) — frozen:

```
toc  cat  extract  resolve  resolve.content
links  links.require_root
splice  splice.if_node_rev  splice.if_root  splice.dry  splice.receipt  splice.verdicts
root  diff  sub
```

Every op in `docs/wire-contract-v2.md` is armed. `hello` answers but is not
itself a capability. A v2 session is byte-for-byte unchanged (a live-consumer
trace pins this in `crates/sidecar/tests/v2_compat_e2e.rs`).

A client-declared contract v3 session
(`docs/wire-contract-v3-amendment.md`) additionally serves, on BOTH hosts
(sidecar + registry daemon):

- the `root` → `fingerprint` vocabulary,
- the composed `read` op (advertised as the v3-only cap `read`): addressing +
  content + rendered text at one engine snapshot,
- `meta.duration_us` in-band timing on every dispatched response frame,
- `extract` heading nodes enriched with the host-face addressing facts
  (`n` / `hpath_text` / `words`).

Stage 2 (2026-07-25) adds, all v3-only and additive
(`docs/wire-contract-v3-amendment.md` § Stage-2 additive surface):

- composed-read **authz facts** — `span` + `content_span` on every `toc` row,
  and the `^id` block anchors in their OWN always-emitted `anchors[]` array.
  The anchor plane is a property of the RESPONSE, not the mode: it is emitted in
  toc mode and sections mode alike, and `[]` means "no addressable anchor here"
  and nothing else. This is what let ccc-statusd delete its markdown mirror,
- the **read-is-the-mint receipt** — the composed read's `actor` slot, unread in
  M1, now mints `{actor, path, selector, sec_rev}` into daemon session memory.
  Sections mode only; a blank or absent actor mints nothing,
- **`splice.pin`** — one optional sibling field lowering a pin through the
  existing `commit_batch` two-file-under-one-flock primitive. No `Op::Pin`, no
  `pin.actor` field; advertised in `caps` as `splice.pin` **by the v3 projection
  only**, since a v2 session refuses the field,
- four **error codes**: `read_mint_required` and `pin_target_missing` (both
  `fix`), plus the pin firing conditions on `write_conflict` (`refresh`) and
  `workspace_busy` (`retry`).

## Workspace CLI

`mrd` (`crates/mrd`) is the operator CLI over the workspace foundation:

```
mrd init [PATH]          mark a workspace, register its drawer, reconcile
                         shadowed tier-4 drawers (amendment M2)
mrd unregister [PATH]    drop the daemon entry (if a daemon answers) + the drawer
mrd resolve [PATH]       report how a path resolves (read-only; writes nothing)
mrd links [PATH]         the corpus edge map (whole corpus, or one file),
                         answered by the daemon (auto-spawned) or in-process
mrd read <PATH>[#FRAG] [--mode toc|sections] [--section SEL]
                         the composed read: addressing + content + render at
                         ONE engine snapshot (daemon or in-process; human
                         output is the rendered text verbatim)
mrd put <PATH> [--dry] [--force] [--actor A] [--now T]
        [--if-fingerprint FP] [--receipt PATH#ANCHOR]
                         the batch write: edits JSON on stdin (wire §4.4
                         grammar), through the production splice choke-point
                         (CAS + armed gate + write flock)
mrd pin <PAGE> <TARGET>#<SELECTOR> [--vibe] [--dry] [--json]
                         mint a meridian-lock pin: PAGE records the claim,
                         TARGET#SELECTOR is the content being attested
                         (sanitized heading path, `^id`, or dewey ordinal)
mrd walk <PAGE> [--down] [--depth N]
                         the context-assembly listing over the pin graph;
                         every answer cites the revs it read
mrd check [--core]       the pure READ validity verb: receipt-chain continuity
                         + the foreign_edit trace; writes nothing
mrd status [--cwd PATH]  the bare drift + freshness summary (pure-local,
                         O(armed), fetch-less)
mrd sql <QUERY>          client-side SQL over the daemon-published DuckDB view
mrd view status          per-workspace view freshness + refresh telemetry
mrd test <PATH>          the scenario runner (also --corpus / --history tiers)
mrd run <PAGE> [TASK]    run a task block declared in the page's frontmatter
mrd new <KIND> <ID>      file birth: fill the def's template, validate, birth
                         the first rev through the guarded create
mrd unfold <PRESET>      materialize a preset's declared scaffold
mrd reconcile <PRESET>   reconcile the tree toward a preset's declared scaffold
mrd realise <PAGE>       the reconciliation loop: observe -> check -> apply
                         (only on drift, once) -> re-check
mrd cache ls             list the on-disk cache drawers
mrd cache clean [--all]  reap stale / orphaned / retired drawers
mrd daemon               run the registry daemon in the foreground
```

`mrd help` is the authoritative surface — flags, refusal legs, and per-verb
exit codes live there.

### `mrd pin` — the attestation verb

`mrd pin` mints a real `meridian-lock` pin through the same write choke-point
every other write uses: one flock, one rename (`docs/wire-contract-v3-amendment.md`
§ Stage-2 item 8).

- **Addressing** is `PAGE TARGET#SELECTOR`, two positionals, the `#` splitting
  on its first occurrence. A page-level pin is REFUSED on purpose: a change
  anywhere in the page would redden every dependent, which is what
  section-level pins exist to avoid.
- **The selector** is a sanitized heading path (`Guide/Leader's-Guideline`), a
  block anchor (`^id`), or a dewey ordinal (`1.2`). A dewey ordinal resolves but
  is never carried — the canonical hpath is what the lock and the receipt use.
- **`--vibe`** additionally writes the target's blob into git's object store
  (`git hash-object -w`), so the pin is retrievable before any commit references
  it. Without it, the oid is computed read-only. When git cannot answer, the
  retrieval plane carries no entry — never a fabricated sha.
- **`--dry`** rehearses and writes nothing. **`--json`** prints the whole
  projected splice response under a `pin` key; human output is a confirmation
  line plus the minted fingerprint, the anchor, the blob, and the new workspace
  fingerprint.
- **No `--actor`.** The read-mint gate keys on a daemon-derived session
  identity, and a CLI invocation has no session: the bare `mrd pin` is
  local-operator-trusted and bypasses the gate, exactly as `mrd put` bypasses
  the host's authz. An `--actor` flag here would either be a meaningless label
  or a way to spell an identity the process does not have.
- **Exit triad:** 0 pinned (or `--dry` rehearsed) / 1 refused
  (`read_mint_required`, `pin_target_missing`, `write_conflict`,
  `workspace_busy`, an armed gate refusal — the engine's verbatim message) / 2
  bad invocation.

A pin written through the resident daemon or MCP is gated: the actor must have
read that exact selector in this session, in mode `sections`. "You cannot attest
content that was never in your context."

### The composed status line

`mrd status` renders five orthogonal axes on one line, worst-of WITHIN each axis
and never across them:

```
pin green · lock none · anchor at-tip (anchor as-known) · convention off · vibe-debt 0 blobs (0 bytes)
```

| axis | answers | values |
|---|---|---|
| `pin` | the ARMED SET's evidence drift — each armed convention's live `CHECK.md` rev against its pinned `armed_rev` | `green` · `red content-drifted` |
| `lock` | every `meridian-lock` pin's FINGERPRINT verdict, rolled up | `none` · `<color> [N pins]` · `unreadable (<why>)` |
| `anchor` | how current the working copy is against origin's tip, plus the trust of that knowledge | `at-tip` / `behind`, qualified — see the colors amendment § The anchor axis |
| `convention` | whether armed law refuses this change | `off` · `warn` · `block` |
| `vibe-debt` | how much of the retrieval plane is held by this machine alone | `N blobs (M bytes)` · `unknown (<why>)` |

Two axes are new in stage 2, and both are read wrong by default:

**`lock` is orthogonal to `pin` and neither subsumes the other.** `pin` rolls up
the armed set; `lock` rolls up fingerprint verdicts. The `lock` roll-up is
worst-of **red > grey > green** — grey ABOVE green is load-bearing, because a
roll-up that let one unverifiable pin hide inside a green fleet would render the
exact false green the color law forbids.

**A green `lock` axis does NOT imply the tree is current. Currency lives on the
`anchor` axis.** The plan text listed "origin tip-compare currency" inside the
drift-color unit; the shipped code deliberately does not fold it into the pin
tone. Currency is a REPOSITORY-level fact, while a pin verdict is per-pin and
content-addressed. Folding them would merge two axes of the composed legend, and
it would re-root a computation that D12 requires to be root-independent. So
`lock` says whether the pinned content still matches the working copy, and
`anchor` says how current that working copy is against origin. Read together,
never multiplied.

**`vibe-debt` is a meter, never a gate.** It counts the lock-referenced blobs
git HAS but no commit reaches — exactly the `pending-anchor` population, which
is the window named residual G1 leaves open (`gc.pruneExpire`, git default two
weeks). A blob absent from the object database (`never-anchored`, pruned or
freshly cloned) is NOT counted: that is past debt, not debt, and its bytes no
longer exist to sum. Debt never enters the findings verdict and never refuses a
write — the gauge reports the size of the window, it does not shorten it. Zero
renders (`0 blobs (0 bytes)`); a gauge that hides at zero is not a gauge.

`--json` always-emits both axes. `composed.lock.pins` is `0` and
`composed.lock.color` is `null` on a corpus with no pins — never an absent field
a reader could mistake for "not checked".

Resolution follows the settled ladder: tiers 1-3 (env override → `.meridian.toml`
marker → git root) open the hashed drawer directly; a tier-4 bare tree adopts a
running daemon's registered ancestor, else degrades to an ephemeral,
per-invocation store that writes nothing. Output is JSON under `--json`, a human
table otherwise; exit codes are 0 clean / 1 findings / 2 tool failure.

## Tests

`cargo test --workspace` — full suite green. The stage-2 integration branch
`stage2-core` gates every merge, most recently **1267 passed / 0 failed / 2
ignored** (M1 shipped at 1117). Export `CARGO_PROFILE_TEST_DEBUG=0` before the
run: a full-debug `target/` in this workspace costs ~26G, and it is the repo's
own CI lever. The
`testsuite` crate carries the frozen ground-truth pack (rung-1 parse truth:
every node reproduced byte-for-byte) plus the U0 read/put parity pack
(`data/parity/`, captured from the live Go host face): `u0_read_parity`
(addressing facts), `u4a1_render_parity` (rendered text), and
`u4a2_composed_read` (the composed op through the live serve loop, refusal
texts included) replay it. The CLI foundation's end-to-end gates live in
`crates/mrd/tests/e2e.rs`.

## Performance

`perfsuite` carries a claims registry (`crates/perfsuite/claims.toml`) whose
verdicts are computed, not asserted, and written to
`crates/perfsuite/results/RESULTS.md`. Current tally: **3 PASS, 1 MEASURED, 11
UNTESTED** — the untested claims are perf rungs whose baselines land on the
first fleet run; the passing ones cover cold ingest and codec bulk cost. Run
the benches to refresh:

```sh
cargo bench -p perfsuite
```

## Known gaps

- Perf rungs are largely UNTESTED pending baselines (see the tally above).
- `policy` verdicts ride every splice response as `[]` until rule packs are
  loaded; where packs are sourced is the host's concern.
- `transport-proto` is an opt-in typed path; the default transport is the
  untyped NDJSON codec.

Stage-2 accepted residuals — documented, not prevented. Full statements in
`docs/wire-contract-v3-amendment.md` § Named residuals.

- **G1** a `--vibe` blob is reachable from no ref, so `gc.pruneExpire` (git
  default two weeks) is its durability horizon. The vibe-debt gauge measures the
  window; committing the file is the only durable anchor.
- **G2** the write flock serializes COOPERATING writers only. An out-of-band
  write is detected by the drift color, never prevented — the git pre-commit
  hook fence is stage-3.
- **G3** a pin writes two inodes and is not all-or-nothing. A failure between
  them leaves a rev-neutral, slug-derived anchor that a re-pin reuses and heals,
  never silent corruption.
- **G4** refs are intra-root only. Cross-root addressing, the mount table, and
  the MERIDIAN.md config engine are stage-3; stage 2 only keeps the seam open
  and never entrenches "there is exactly one root".
- **G5** anchor promotion into an unowned target churns that file's CAS token.
  Accepted for the core loop because the promotion is rev-neutral; the fence and
  the authz tightening are stage-3.

Also stage-3, and NOT shipped: the receipt / `predicate_type` representation
unification (the read-mint ledger and the persisted `^receipt` projection are
still two representations of one receipt family), the `defsarm` Go-legs drop,
and full-document re-attest.
