# Presets and session birth — the design element

A **preset** is a def page that declares a shape. **Session birth** is the act
of turning that declared shape into files. This document is the design element
the `preset` crate and the `new` / `unfold` / `reconcile` verbs are audited
against: it states what the plane is FOR, the laws it may not break, and the
boundaries it may not cross.

It is not a description of the current code. Where the code and this element
disagree, the element wins and the code is rebuilt.

## 1. The premise — a shape is declared once, in a page

The alternative this plane exists to replace is a scaffolding script: a
generator that knows the shape in code, births files nobody can re-derive, and
drifts from the shape the day someone edits the tree by hand.

The premise here is the opposite. **The shape lives in a page**, in the same
markdown the engine already governs, and a born tree **pins the def it came
from at the def's rev** — so the shape that produced any file is recoverable
forever, from the file, without the tool that wrote it.

Everything below follows from that premise.

## 2. The def grammar

A preset def is a page carrying `type: def`. Its frontmatter declares:

| Key | Meaning | Absent |
|---|---|---|
| `type` | must be `def` — anything else is not a preset | refuse (tool failure) |
| `defines` | the kind this def births (`session`, `task`, …) | empty kind |
| `root` | the root record the scaffold pins the preset into | `SESSION.md` |
| `births` | the `{{id}}`-filled target path template for one record | `{{kind}}/{{id}}.md` |
| `inputs` | the convention-floor pins — a **block sequence** | no floor pinned |

Its body declares, in named sections:

- `# Properties` (`^properties`) — the rules a born record must satisfy. One
  `- key` or `- key = value` list item per rule.
- `# Template` (`^template`) — the fenced body one record is born from.
- `# Unfold` — the declared scaffold: the file paths a whole birth materializes,
  in declared order.
- `# Ephemeral` — the **allowlist** of declared-disposable paths. Empty by
  construction, so a def that declares nothing disposable can prune nothing.

**Law 2.1 — `inputs` is read and written whole.** It is a multi-line block
sequence, read through the whole-value frontmatter grain and written as whole
birth bytes. A line-oriented scan that stops at the key line, or a single-line
properties upsert, corrupts it. The read half and the render half are one round
trip and are audited as a pair.

## 3. The birth law — one door

**Law 3.1 — every byte a preset lands rides the guarded create.** No
`fs::write`, no second write path, no exception for a stub, a dry run, or a
scaffold file the author thinks is uninteresting. The guarded create carries
three things a raw write cannot: the `if_absent` CAS, the journaled birth
receipt, and the gate seam.

**Law 3.2 — a birth never clobbers.** An occupied target is the CAS's answer,
not the plane's decision. It surfaces as a `cas_mismatch` finding and the file
on disk is left byte-untouched. This holds on a dry run too: a rehearsal that
would have clobbered still refuses.

**Law 3.3 — every removal rides the guarded remove**, read-then-delete under
the live rev. The one exception is an empty directory, which carries no
governed rev and no bytes to protect; it is removed with a raw `rmdir` and that
exception is stated here so it cannot be widened silently.

**Law 3.4 — the plane mints no identity and no clock.** `actor` and `now` are
caller-supplied and stamped exactly as given. Absent stays absent. A crate that
reads the wall clock cannot be tested against a fixture and cannot be replayed.

## 4. The three verbs

The plane offers exactly three births, and they differ only in **what set of
paths they act on**. They share one def loader, one renderer pair, and one
guarded door — a verb that grows its own copy of any of those is wrong-design.

| Verb | Acts on | Refuses when |
|---|---|---|
| `new <kind> <id>` | ONE record, from `^template` | the def is invalid, or the target exists |
| `unfold <preset>` | EVERY declared scaffold path | any declared path already exists |
| `reconcile <preset>` | the MISSING declared paths only | — (an occupancy is not a failure here) |

**`new` validates before it writes.** The filled template is parsed and checked
against every `^properties` rule; the FIRST violation refuses `def_invalid`
naming the rule verbatim. A def with no `^properties` block, no `^template`, or
a rule that did not parse is itself invalid — the same refusal, because a def
that cannot state its own contract cannot birth a record that satisfies it.

**`unfold` is the first birth; `reconcile` is every birth after it.** That is
the whole difference: unfold treats an occupied path as a finding because it
expected to create the world, and reconcile treats it as convergence because it
expected the world to be partly there.

## 5. The reconcile asymmetry (ZT ruling #3)

**Law 5.1 — reconcile is additive by set-difference, subtractive by
allowlist.** These are not two spellings of one operation and must never be
refactored into one:

- **Materialize** every declared path missing from the tree. Set-difference.
- **Prune** only paths matching the `# Ephemeral` allowlist, plus empty
  undeclared directories. Allowlist.
- **Everything else** — undeclared content — renders as a **finding**. Never a
  prune action, never under any flag.

**Law 5.2 — "undeclared" is not "unwanted".** The tempting symmetry (delete
whatever the def does not declare) is the one thing this design forbids. A user's
file that the def has never heard of is a report to the user, not garbage. The
asymmetry is the safety property; a change that makes the two halves symmetric
has deleted the design, whatever the tests say.

**Law 5.3 — reconcile stays inside the shape's territory.** The scan scope is
the set of directories the declared scaffold occupies. Reconcile never reads,
reports on, or prunes a path outside it. Engine and system files (dotfiles, the
reserved journal) are never "undeclared content".

A prunable **directory** is one that lives strictly beneath a directory the
scaffold itself creates, is not an ancestor of a declared path, holds no
finding, and is empty. A scaffold declaring only top-level files creates no
directory and therefore prunes none: the workspace root is never walked for
directory candidates, because every empty directory in a user's workspace is not
this shape's territory.

**Law 5.4 — pruning is opt-in.** Without `--prune`, reconcile materializes and
reports and removes nothing.

## 6. The convention floor

A session preset's `inputs` pin the convention floor — the rule pages the born
session lives under — at a path and a rev. The root record is born carrying that
pin, so the law a session was born under is readable from the session itself
long after the def has moved on.

**Law 6.1 — a floor pin is a pin, not a copy.** The preset records `path@rev`;
it never inlines the floor's content into the born tree.

## 7. Refusals and exit codes

The plane distinguishes two failure kinds and never conflates them:

| Kind | Exit | Examples |
|---|---|---|
| **Finding** — the plane ran and reported | 1 | `def_invalid{rule}`, `cas_mismatch`, an undeclared-content finding |
| **Tool failure** — the plane could not run | 2 | the def is unreadable, the page is not a def, a write faulted for a reason other than the CAS |

**Law 7.1 — a refusal names the rule it enforced.** `def_invalid` carries the
source text of the violated `^properties` rule. A refusal that says only "the
def is invalid" makes the author guess, and this plane's whole value is that the
shape is stated in a page they can read.

## 8. Boundaries — what this plane never does

- It holds **no session policy and no liveness**. Whether a session is active,
  expired, or archived belongs to the customer that dials this plane.
- It owns **no CLI**. `mrd new` / `unfold` / `reconcile` are thin clients:
  argument parsing, workspace resolution, output shape. Every decision this
  document states lives in the crate, so a second host reaches the same
  behaviour without re-deriving it.
- It invents **no write path, no hash law, no rev noun**. It composes the
  shipped ones.

## 9. The user-facing surface carries no internal tags

**Law 9.1 — a verb's help text is written for the person typing the verb.**
Internal planning identifiers — unit numbers, block numbers, plan-section
references, the tags a docket uses to track its own work — are project
bookkeeping. They are legitimate in source comments, crate metadata, and test
names, where the reader is a contributor holding the plan. They must not appear
in `mrd help` output, where the reader is a user who has never seen the docket
and for whom `(U5.3)` is noise that reads as a version, a flag, or an error
code.

This is gated by a test over the real help output, not by review discipline.

---

## Appendix — the conformance audit (2026-08-03)

The first audit of the `preset` crate and the `new` / `unfold` / `reconcile`
verbs against this element. Every law is listed, including the ones the code
already satisfied — an audit that reports only its failures cannot be checked by
the next reader, who has no way to tell an unexamined law from a passing one.

| Law | Verdict | Evidence |
|---|---|---|
| 2.1 whole-value `inputs` | **conformant** | `read_inputs_grain` resolves `FmKey("inputs")` and spans the whole block; `render_block_sequence` is its writing half. The round trip is gated by an existing test. |
| 3.1 one write door | **conformant** | Every landing byte in the crate goes through `birth` → `wire_serve::write::create`. No `fs::write` exists in the crate. |
| 3.2 never clobber | **conformant** | `if_absent` CAS; `BirthResult::Occupied` is a finding, never a fallback write. `opts.dry` is passed to the door, so a dry run refuses too. |
| 3.3 guarded remove | **conformant** | `prune_file` reads the live rev and removes under it. The `rmdir` exception is the empty-directory case, now stated in the element rather than only in a comment. |
| 3.4 no minted identity or clock | **conformant** | `actor` / `now` are `Option<String>` on `BirthOptions` and are never defaulted from a clock; `fill_vars` renders an absent one as empty. |
| 4 three verbs, one door | **conformant** | All three call `load_def` and `birth`; none carries a private write path or a second renderer. |
| 4 `new` validates before writing | **conformant** | Structural def checks, then `first_violated_rule`, then birth. A def that cannot satisfy its own `^properties` refuses before any byte moves. |
| 5.1 additive by diff, subtractive by allowlist | **conformant** | `reconcile_plan` is a pure fold and keeps the two halves as separate fields. |
| 5.2 undeclared is not unwanted | **conformant** | `findings` is never read by the prune path. |
| 5.3 territory — file half | **conformant** | `scan_scope` walks only the directories directly holding a declared path; dotfiles and the reserved journal are skipped. |
| **5.3 territory — directory half** | **WRONG-DESIGN — deleted and rebuilt** | `prune_empty_dirs` drew its candidates from `scope_dirs(declared)` and its skip set from `declared.flat_map(ancestors_of)` — the same expression. Every candidate matched the skip set, so the function returned an empty vector for every possible input and `pruned_dirs` was dead. Rebuilt to walk the live tree beneath the scaffold's own directories, bounded so a top-level-only scaffold never reaches the workspace root. Gated by three new tests, one of which is the bound. |
| 5.4 prune is opt-in | **conformant** | Gated by an existing test. |
| 6 floor pin is a pin | **conformant** | `render_root_record` writes `path@rev`; the floor's content is never inlined. |
| 7 finding vs tool failure | **conformant** | `RefusalReason` (exit 1) and `PresetError` (exit 2) are separate types; only a non-CAS write fault crosses into the latter. |
| 7.1 a refusal names its rule | **conformant** | `def_invalid` carries `PropRule::raw`, the source text verbatim. |
| 8 no session policy, no CLI ownership | **conformant** | The crate holds no liveness state; the three `mrd` modules parse arguments and shape output only. |
| **9.1 no internal tags in help** | **VIOLATION — fixed** | Four help descriptions opened with `(U5.3)` or `(U3.5b; ZT ruling #3)`. Stripped. A derived test now scans every page the CLI can print. Source comments and crate metadata keep their tags, which §9.1 permits. |

Two documentation rows were also found stale against §4 and §8 and corrected:
the `docs/laws.md` crate charter omitted `reconcile` entirely, and
`crates/mrd/src/preset_cmd.rs` described itself as serving "the two preset
verbs" while three verbs dial its `def_path` and five dial its `resolve_root`.

**No part of this plane was found to warrant removal.** The audit's one
wrong-design finding is a bounded rebuild of a single function, and the ZT
ruling to KEEP the feature is untouched by it.
