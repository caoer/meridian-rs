# meridian-md pack law

The fixture corpus for the `MERIDIAN.md` in-file schema. Spec:
`docs/meridian-md-schema.md` (stage 3, U2). Consumed by U6 (parse + resolution) and U7 (the mount
table). **This pack is data — it ships no engine code, and U2 wrote no engine code.**

39 cases: **12 acceptances** and **27 refusals**, all listed in `cases.json` (the kind-sweep, 2026-08-13, retired the three kind-rule refusals and re-aimed `mount-unknown-kind` as `mount-stale-kind`; `empty-frontmatter` arrived 2026-08-24 with schema §4's empty-closed-block row).
7 of them have no fixture file — a file cannot express its own absence — so **`cases.json`, not the
file tree, is the pairing mechanism.**

```
cases.json      every case paired with its required outcome
corpus/*.md     8 well-formed configs   (the acceptances)
refusals/*.md   24 malformed configs    (the refusals)
```

## The pairing rule

**Every fixture has exactly one case in `cases.json`, and every case names its required outcome.**
A fixture with no case is not testable and is a defect in this pack; a case with no required outcome
is a wish. Case shape:

| Field | Carries |
|---|---|
| `id` | the case name — also the fixture's basename, where there is a fixture |
| `fixture` | path relative to this directory, or `null` when the case has no file |
| `env` | `MERIDIAN_CONFIG`, `HOME`, and which fixture (if any) is placed at each rung. Absent key = unset; explicit `null` = unset |
| `expect` | `outcome` (`accept` / `refuse`), the §2.2 `state`, and then either the parsed `mounts`/`tools` or the `reason` word, `line`, and what the message must `name` |
| `expect_not` | what must **not** be true — used where the wrong behaviour still looks healthy |
| `law` | the spec section the case pins |
| `kills` | **what wrong implementation this case rules out** |

`kills` is not commentary. A case that cannot say what it kills is probably vacuous, and writing the
sentence is how you find that out before the case ships.

## Conventions, and the reason each exists

| Convention | Why |
|---|---|
| Each fixture's **H1 carries its outcome in parentheses** — `# My system (refused: unknown-field)` | the `crates/mrd/tests/corpus/specs/` convention; a reader opening one file alone knows what it asserts |
| Each fixture's **body is prose explaining why that outcome is the law** | a bare fixture teaches nothing when it fails three months from now |
| Fixtures are **realistic configs**, not minimal reductions | this schema's whole premise is that a human opens and reads the file; a fixture that no human would write tests a grammar no human will use |
| Refusal fixtures often carry a **valid mount block anyway** | so `expect_not` can prove the ratified **no partial mount table** — a fixture with nothing loadable cannot prove nothing was loaded |
| `.gitattributes` sets `* -text` | one case (`tool-declared`) asserts a tool payload **byte-verbatim**, indentation included; EOL normalization would rewrite it |

## Line numbers are asserted, and they are 1-based in the FILE

Every refusal case states the `line` its refusal must point at. This is not decoration: the ratified
requirement is a refusal *"naming what is broken **and where**"*, and a reason word alone does not
satisfy it.

Which line, per spec §8.1a:

| The fault is about | The line is |
|---|---|
| Something **present** | its own line |
| Something **absent** | the opening line of the construct that should have carried it — the block's opening fence, or line 1 for a frontmatter fault |
| A **duplicate** | the **second** occurrence; the message names the first |

**Editing a fixture moves its line numbers.** Re-derive them and update `cases.json` in the same
change — a stale expected line is a test that passes for the wrong reason.

## Prose is intended, and so are the decoys

`corpus/prose-decoys.md` carries four constructs that look like machine surface and are not: a
` ```yaml ` block with mount-shaped keys, a ` ```meridian-mount ` nested inside a four-backtick fence,
an indented snippet, and inline code. **All four must be inert, and the file must yield exactly one
mount.** This is deliberate, not a mistake in the fixture: a reader that scans for `name:`/`path:`
lines passes every other acceptance in this pack and fails only here.

The same logic runs the other way. `corpus/multi-root.md` is the acceptance that a
refuse-everything build cannot pass — **a guard proven only by what it blocks is indistinguishable
from a guard that blocks everything** (S3-R8(c)). The pack carries acceptances and refusals in one
manifest for exactly that reason.

## What this pack does NOT cover, on purpose

- **Mount-table semantics** — canonicalization at bind, `workspace::deny_reason`, equal-or-nested
  refusal, declared-vs-bound name checking, the grey classes. **U7's**, with U7's fixtures.
- **Two mounts resolving to the same path.** Decidable only after canonicalization (symlinks,
  trailing slashes, `..`), so it is U7's. **Name** collision is decidable from the bytes and is here
  (`duplicate-mount-name`).
- **The `root:` address grammar** and the literal-path ambiguity — **U3's**, then U10/U11.
- **Project-local walk-up discovery** — deferred by the ratifying decision; not built, not fixtured.

Adding a case for any of these to this pack means two owners for one fact. Take it to that unit.

## For the unit that consumes this pack (U6)

This pack ships **no** loader, because U2 ships no code. Three things the consuming unit must do, one
of which fails silently if missed:

1. **Add the accessor** to `crates/testsuite/src/lib.rs`, alongside the five that are already there —
   `meridian_md_dir()` returning `Path::new(env!("CARGO_MANIFEST_DIR")).join("data/meridian-md")`.
   Every pack in this directory is reached that way; no fixture path is hardcoded elsewhere.
2. **Register the test module in `crates/testsuite/tests/main.rs`.** `crates/testsuite/Cargo.toml`
   sets `autotests = false`, so a stray `tests/meridian_md.rs` is **never compiled and never runs** —
   a green board that measured nothing.
3. **Assert the case count** (`assert_eq!(checked, 37, …)`), the way `gt_parse.rs:98` pins its ten GT
   files. Directory-scan discovery plus a cardinality assertion is what stops an empty replay from
   passing vacuously.

Disagreements between this pack and the spec are findings: neither is silently adjusted to match the
other. Route them through the stage-3 leader.
