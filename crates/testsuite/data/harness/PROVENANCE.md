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

# p4-regression-probes.json + walkvault/ (D2-DISPATCH conformance)

`p4-regression-probes.json` is the gate-3 adversarial-harness Class-4 wire-level
regression probe set, vendored verbatim so `sidecar/tests/p4_probes.rs` runs
the rung-applicable subset (MP-1..4, MP-8) against the LIVE dispatch.
`walkvault/` is its fixture vault, vendored byte-identical.

- Source: `18-02-meridian-rs/results/contract-v2-tournament/gate-3/adversarial-harness/`
  (`probes/p4-regression-probes.json`, `fixtures/walkvault/`)
- sha256 p4-regression-probes.json: `bf40f19d3719b57f270d2d01a4e155f1349d97ea3ab9c95d7c19105c7f5b1ba4`
- sha256 walkvault/alias-target.md: `ab997bca801bfea013d680e3735cd2a523eab41da26f763eea3744c33d322c4f`
- sha256 walkvault/blocks.md: `b2a3e96a976c6270dad3ae79753b1655cb64e2115b9a2fde69be4b7272d83652`
- sha256 walkvault/walk.md: `9cefbba3df8ded74a583342eaf5aab4a3cebb1ba866a4470cb54fa4a22bf0cf0`
- sha256 walkvault/sub/walk.md: `92e6e0153b60d9d5981e10ac3a310ec3a9de9a18c6349a419b3dd6bfaa9b5c4a`
- Vendored: 2026-07-19 (D2-DISPATCH, leader e2138ec2)
- Rung disposition: MP-5..7 are `splice`-op probes — they answered
  `unknown_op` while splice was unarmed (rungs 2–3) and are BOUND since
  D4-SPLICE (MP-5/MP-6 refuse `bad_request` at the strict decode; MP-7 runs
  against the wsfix S0 state its expect note names and succeeds guardless);
  MP-9 is a GT-provenance check, not a wire frame. MP-8's recorded `expect.ok:true` predates the frozen D-C5 law
  (unknown `kinds` value → `bad_request{unknown_kinds}`; `block_anchor` is not
  in the closed §4.3 kind enum — the spelling is `anchor`): the runner asserts
  the TEXT-lawful answer and records the probe-file deviation loudly.
