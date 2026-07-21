---
updated: 2026-07-20
---

# Status

A snapshot of what is built and verified today. Numbers here are reproducible
from the commands shown — prefer running them over trusting this prose.

## Build

- Toolchain: Rust edition 2024, `rust-version = 1.96`.
- `cargo build` builds the sixteen default members (the sidecar plane plus the
  `workspace` / `cache` / `registry` / `mrd` CLI foundation); `perfsuite` is out
  of default-members and builds under `cargo build -p perfsuite`.
- Fork: `pulldown-cmark` is consumed via a `[patch.crates-io]` rev pin (the
  `obsidian` branch); see the workspace `Cargo.toml`.

## Wire surface

The sidecar answers protocol 1 as `meridian-sidecar/2.0`. Armed capabilities,
reported in the `hello` handshake (`crates/sidecar/src/lib.rs`, `CAPS`):

```
toc  cat  extract  resolve  resolve.content
links  links.require_root
splice  splice.if_node_rev  splice.if_root  splice.dry  splice.receipt  splice.verdicts
root  diff  sub
```

Every op in `docs/wire-contract-v2.md` is armed. `hello` answers but is not
itself a capability. The `root` → `fingerprint` rename ships only under a
client-declared contract v3 (`docs/wire-contract-v3-amendment.md`); a v2 session
is byte-for-byte unchanged (a live-consumer trace pins this in
`crates/sidecar/tests/v2_compat_e2e.rs`).

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

`cargo test --workspace` — full suite green. As of this snapshot: **357 tests
passed, 0 failed** across unit, integration, and doc tests. The `testsuite`
crate carries the frozen ground-truth pack; rung-1 parse truth (every node
reproduced byte-for-byte) is gated there. The CLI foundation's own end-to-end
gates live in `crates/mrd/tests/e2e.rs` (init/ls/unregister lifecycle, M2
reconciliation, tier-4 ephemeral degrade, downgrade cold-start, deny ceiling,
and a real `mrd daemon` resolve-adopt round-trip).

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
