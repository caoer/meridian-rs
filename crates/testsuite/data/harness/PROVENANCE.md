# harness/ — adversarial walk-law probe pack (M2-RESOLVE conformance)

`p2-walk-probes.json` is the gate-3 adversarial-harness Class-2 walk-law probe
set, vendored verbatim so `tests/walk_resolve.rs` runs it against `model::walk`
without an out-of-repo path dependency.

- Source: `18-02-meridian-rs/results/contract-v2-tournament/gate-3/adversarial-harness/probes/p2-walk-probes.json`
- sha256: `fa5dd04cbd8b1ffac3d0caf8f55117fcceb9852b2865b0c5860b662ca323e908`
- Vendored: 2026-07-19 (M2-RESOLVE, worker a5db0b80)
- Semantics: `gt=ruling` = ZT-verified gate-1 walk law (binding); `gt=oracle` =
  a prediction confirmed against the live oracle. The oracle-confirmed subset
  (WL-1..6b + WX-6) is pinned by `data/gt/obsidian-compat/resolution.expected.json`;
  this file is data, never hand-patched.
