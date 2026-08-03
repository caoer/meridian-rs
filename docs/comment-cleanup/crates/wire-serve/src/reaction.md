# crates/wire-serve/src/reaction.rs — comment-cleanup notes

## Module charter (from reaction.rs:1)

Reaction-mode evaluation for one landed document change. Shared feeder leaf for both hosts (`docs/laws.md` Law 3): derives the reaction payload from the same before/after model states the guarded write and external watcher hold, resolves armed HOOKs governing the changed path, and evaluates them. Does not interpret `how:`, deliver an intent, or make a delivery claim.

## Resolution is the ARM effective set only (from reaction.rs:9)

The armed law comes from the attested artifact at `policy::armed::ARMED_RULES_PATH` (registration ruling § 4) through exactly ONE call, `policy::armed_law::resolve_armed_law`, which pivots on the once-armed state, selects rows governing the path, freeze-checks them, and loads each page. This leaf composes none of that by hand: `verify_at`'s own documentation records that each obvious hand-composition of `select_at` + `verify` is a live defect, in opposite directions.

The once-armed state is the `meridian/attested` MARKER, read through `crate::armed_disk` both hosts use. It is the marker rather than the artifact's presence because an absent artifact on an armed workspace is a FAULT, not a disarm: pivoting on the artifact would make deleting it read as "never armed" — the silent-disarm attack. Both armed-law surfaces pivot on that one marker so the workspace cannot disagree with itself about whether it is armed.

The feeder walks no `conventions/` folder and reads no `kind:` frontmatter to decide what is armed — both are dead registration surfaces under the ruling. A row's `mode` carries its kind, and `armed` is hook vocabulary, so firing hook rows fall out of that one call with no kind test of our own.

## A reaction never vetoes — every fault is REPORTED (from reaction.rs:31)

Everything here runs AFTER the write has landed, so this leaf can neither refuse nor mutate. Silence was its only failure mode: an artifact fault used to reach `.unwrap_or_default()` at both call sites and read as "nothing to react to".

Faults ride the frame as `wire::EffectFinding::ArmedFault` — this host's CHANNEL onto the one artifact-fault surface, rendered in that surface's own words (`policy::armed_law::ArmedFault`). The door refuses on the same faults with the same text; disposition differs by kind, vocabulary does not.

A fault is isolated per ROW: one armed page that will not load reports itself and leaves every other rule at the path firing. Propagating it instead silenced the whole path — the same silent disarm one layer down.

## feed_landed_change contract (from reaction.rs:53)

File revisions come from the supplied model states (cursor coordinates for the canonical receipt address); no caller can substitute a different post-change revision. Empty evaluations are dropped so they do not perturb no-effects wire bytes — a FAULT envelope is never empty and never dropped. No error return: a reaction may not fail a write that has already landed; the only honest shape is "envelopes, some of which are faults".

## Marker early-return and mode filter (from reaction.rs:74)

The MARKER is the once-armed pivot. A never-armed workspace reacts to nothing; the no-op is free and silent (return before deriving). On an armed workspace an absent artifact falls through to the resolver, which reports it as a fault rather than silence.

`mode == armed` is hook vocabulary — that filter alone leaves firing HOOK rows; a check row governing the same path is the door's business.

An already-loaded predicate faulting at evaluation aborts the batch (the evaluator's all-or-nothing). Reported through the same channel so it is a fault the operator reads, never an empty frame.

## Fixture: HOOK page dual key sets (from reaction.rs:209)

The first tag-registered HOOK page carries BOTH key sets on purpose: `tags:` + `id:` are the REGISTRATION layer (ruling § 1/§ 2), while `kind:`/`severity:`/`paths:`/`caps:`/`budget:`/`how:` are C1's declaration layer, which § 5 leaves untouched. The feeder reads only the former to decide what is armed.

## Fixture: arm layout and marker (from reaction.rs:258)

Pages sit DIRECTLY in the folder they govern (§ 3 gitignore-style layout), mounting at that folder under the landed `mount_dir`. The ruled layout-folder style (`<scope>/rules/*.md` mounting at `<scope>`) is deliberately not used: the mount law is ruled but not yet implemented; this card consumes armed rows rather than minting the mount rule.

Arming is ONE act over TWO files (artifact + once-armed MARKER). A fixture that wrote only the artifact would be testing a workspace this engine considers never armed, so every reaction would be a no-op that passed for a pass.

## Unreadable artifact vs never-armed (from reaction.rs:396)

An artifact that EXISTS and cannot be read is a fault, not a never-armed workspace. Folding the two would be silent disarm: making the law unreadable would turn the gate off. A directory at the artifact path reads as an error, not absence.

## Pinned-page drift reporting (from reaction.rs:461)

Silent to the WRITE — a hook never vetoes — but never silent to the operator: drift is reported through the same channel every other artifact fault takes.

## Arm-root selection (from reaction.rs:473)

Armed at `sessions/s1`, so a write under sibling `sessions/s2` is not under any arm root and selects nothing.

## Feed-error / F2 / F-1 fault shapes (from reaction.rs:496)

**Feed-error shape.** A corrupt artifact on an ARMED workspace used to map to "no reaction" at both call sites (`.unwrap_or_default()`), so a gate-disabling edit dressed as a parse reached nobody. It now reaches the operator on the frame.

**F2.** Row deletion LOOKS legitimate — well-formed artifact page with title, preamble, byte-exact header, and zero rows. Bound to the once-armed marker it is now a reported fault instead of a silent total disarm; at the door the same fault refuses the write.

**F-1.** Two hooks armed over the same path and one page will not load. The loop used to propagate that fault with `?`, so ONE bad page silenced every reaction at the path. The fault is now isolated to its own row.

## Kind seam / eval fault (from reaction.rs:571)

The kind seam has ONE enforcement point (`load_rule`), so a page in the ruled shape — registration tag, no `kind:` key — fires. Loading through the filename-shaped hook loader instead asserted `kind: hook`, which let a page arm cleanly and never fire.

An already-loaded predicate faulting at EVALUATION aborts the evaluator's batch — its all-or-nothing, one layer below this leaf. It travels the same channel so the operator reads a fault rather than an empty frame.

## Review verdict: GO
- reviewer: codex-exec
- code-invariance: pass — PASS: comment-only diff
- info-preservation: pass — ARM resolution, marker pivot, silent-disarm defenses, fault isolation, fixture constraints, and test rationale remain in comments or these notes.
- style: pass — Remaining comments are concise and accurate; no narration, commented-out code, or unsafe blocks.
- detail: All shortened load-bearing contracts and historical defect explanations are preserved, including supplied-state receipt revisions and the one-call armed-law seam.
