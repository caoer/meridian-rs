# charset-guard fixture pack — provenance

**Rule:** decision 011 + § Addendum / wire-contract-v2 §2.4 — ONE block-id
charset `[A-Za-z0-9-]` (Obsidian app-exact), on BOTH the mint and walk planes.
The old two-plane mint superset (the app-exact set plus a `_`) is DEAD.

**What this pack is.** The discrimination pack authored by CHARSET-GUARD for the
downstream units to ride. `discrimination.json` carries, per mint position, a
`_`-bearing refusal input (`input_id`) that must be refused `bad_request` at
that position, plus the clean ids that mint, plus the pack-pinned walk answer
(the SAME id via the walk plane → the app's silent-drop answer).

**How the mint disposition is derived (not hand-asserted).** The rung-2 core
proves the mechanism: `model::Ref::anchor(input_id)` returns `Err(BadAnchorId)`
for every `input_id` and `Ok` for every `clean_accepts` id. That constructor
routes through `syntax::is_block_id` — the single normative charset predicate.
Downstream units (M2-RESOLVE, D2-DISPATCH, W4-AMEND, RC4-RECEIPT) feed the same
`input_id` values into their own mint positions and assert the same refusal.

**How the walk answer is pinned (not hand-written).** The walk disposition is
NOT authored here — it is the B3-APP-GT obsidian-compat@1.12.7 oracle answer,
probe WX-6, in `../gt/obsidian-compat/resolution.expected.json`
(`blocks#^under-probe_x` → `ref_not_found`, stage 2). This pack references it;
the test asserts the referenced answer still says what this pack claims. Per
decision 013 consequence 3, loud mint-plane refusal AND that pinned app walk
answer BOTH conform.

**Exemption (assert-by-listing, never migrate — 011 § Addendum).** The `_`
fixtures stay verbatim; they are test inputs, never live ids, and they pin the
`_`-rejection behavior. In-repo: `../gt/obsidian-compat/walkvault/blocks.md`
line 7 `^under-probe_x` and the WX-6 answer. Out-of-repo (18-02-meridian-rs
session, untouched by construction — this lane's diff never reaches them):
`probes/zzprobe-anchors-a.md ^a_04`,
`results/fixture-pack/inputs/underscore-anchors.md ^my_block`,
`results/contract-v2-tournament/team-f/scratch/w3.md ^ok_1`
(source: `gate-3/underscore-inventory.md`).

**Drift posture.** If the obsidian-compat pack is re-frozen at a new app version,
re-check the WX-6 answer this pack references (contract §13.3–§13.4). The mint
charset is a frozen contract law (§2.4), not app-version-derived.
