---
updated: 2026-07-19
---

# Status

A snapshot of what is built and verified today. Numbers here are reproducible
from the commands shown — prefer running them over trusting this prose.

## Build

- Toolchain: Rust edition 2024, `rust-version = 1.96`.
- `cargo build` builds the twelve default members; `perfsuite` is out of
  default-members and builds under `cargo build -p perfsuite`.
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
itself a capability.

## Tests

`cargo test --workspace` — full suite green. As of this snapshot: **285 tests
passed, 0 failed** across unit, integration, and doc tests. The `testsuite`
crate carries the frozen ground-truth pack; rung-1 parse truth (every node
reproduced byte-for-byte) is gated there.

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
