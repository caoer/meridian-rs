---
updated: 2026-07-24
---

# Status

A snapshot of what is built and verified today. Numbers here are reproducible
from the commands shown — prefer running them over trusting this prose.

## Build

- Toolchain: Rust edition 2024, `rust-version = 1.96`.
- `cargo build` builds the twenty-six default members (the engine planes plus
  the `workspace` / `cache` / `registry` / `mrd` CLI foundation); `perfsuite`
  is out of default-members and builds under `cargo build -p perfsuite`.
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

## Workspace CLI

`mrd` (`crates/mrd`) is the operator CLI over the workspace foundation:

```
mrd init [PATH]          mark a workspace, register its drawer, reconcile
                         shadowed tier-4 drawers (amendment M2)
mrd unregister [PATH]    drop the daemon entry (if a daemon answers) + the drawer
mrd resolve [PATH]       report how a path resolves (read-only; writes nothing)
mrd cache ls             list the on-disk cache drawers
mrd cache clean [--all]  reap stale / orphaned / retired drawers
mrd daemon               run the registry daemon in the foreground
```

Resolution follows the settled ladder: tiers 1-3 (env override → `.meridian.toml`
marker → git root) open the hashed drawer directly; a tier-4 bare tree adopts a
running daemon's registered ancestor, else degrades to an ephemeral,
per-invocation store that writes nothing. Output is JSON under `--json`, a human
table otherwise; exit codes are 0 clean / 1 findings / 2 tool failure.

## Tests

`cargo test --workspace` — full suite green (0 failures; the M1 CI worktree
gates every merge to `m1-bios`, most recently 1117 passed / 0 failed). The
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

- The Workspace CLI verb list above is HELD at its pre-M1 shape: the
  `mrd read` / `mrd put` verbs (and the u3-era verb additions) land with the
  in-flight U1 unit, which owns that section's refresh so docs match the
  shipped help text.
- Perf rungs are largely UNTESTED pending baselines (see the tally above).
- `policy` verdicts ride every splice response as `[]` until rule packs are
  loaded; where packs are sourced is the host's concern.
- `transport-proto` is an opt-in typed path; the default transport is the
  untyped NDJSON codec.
