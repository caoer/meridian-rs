---
ignore:
  - "crates/testsuite/data/meridian-md/refusals/**"
  - "crates/testsuite/data/parity/**"
---

# The hash domain of meridian-rs

This is `fs::domain::DOMAIN_CONFIG_PATH` — the workspace's custom-ignore
declaration, layered over the structural floor (md-only, no dot segments, the
reserved journal). It is markdown because the ignore list is a decision a human
maintains: the frontmatter filters, and this body says WHY each entry is here.

**Do not "tidy up" the entry below. It is load-bearing.**

## `crates/testsuite/data/meridian-md/refusals/**`

The `MERIDIAN.md` in-file schema pack keeps one **deliberately malformed** config
per refusal class under `refusals/` — that is the directory's entire purpose, and
`crates/testsuite/data/meridian-md/README.md` says so. `cases.json` pairs each
file with the refusal it must produce, and `crates/testsuite/tests/meridian_md.rs`
reads them straight off disk, so nothing about testing them needs them attested.

Leaving them in the hash domain cost something real. `refusals/frontmatter-unparseable.md`
opens a `tags:` flow sequence it never closes, so registration cannot answer
whether the page carries a `rules/*` tag and refuses it fail-closed — correctly.
The hash domain is what every discovery consumer sweeps, so that one fixture was
reported by **every corpus-wide walk in this repo, permanently**: the discovery
sweep, the ARM act, and the cutover sweep. `mrd rules` on meridian-rs itself
exited 1 from every path, naming a test fixture.

Ruled and approved:

- The § 3 **"Refusal scoping"** amendment (2026-08-01) in the registration/arming
  ruling narrows refusals exactly like rules, which stops an off-chain refusal
  reddening a *scoped* query. It deliberately does **not** touch corpus-wide
  walks: those report ALL refusals they encounter, always — fail-loud survives
  where enforcement lives.
- So the walks keep biting until the fixture leaves the domain. The advisor
  approved this exclusion **on independent grounds** (it bites every corpus-wide
  walk), and the leader ruled its shape; the work is the `refusal-scoping` card
  of session `30-19-subscribe-notify-impl`.

The rule names the **directory, not the one file**, because the ground is about
the class: a fixture whose whole purpose is to be malformed does not belong in the
attested surface, and the next such fixture must not silently re-introduce the
bite. Fixtures elsewhere in `crates/testsuite/data/` stay in the domain — they are
well-formed markdown, and frozen packs are worth attesting. **Except a pack that
is not ours to keep well-formed: see the entry below.**

## `crates/testsuite/data/parity/**`

The U0 read/put parity pack is a **byte-exact copy of another repository's
corpus** — `ccc-statusd internal/mcpserver/testdata/parity/`, captured at
`50b15ae`. `crates/testsuite/data/parity/README.md` sets the discipline: *"the
corpus bytes ARE the test; never normalize line endings or whitespace"*, and
*"On re-capture, re-copy — disagreements between this pack and the harness are
findings, not things to edit around."*

`corpus/meridian-block/corpus/meridian-block.md` carries a ` ```meridian-lock `
block reading `version: 1`, and the pin plane refuses it. **It has never been a
readable lock in any engine:** the v1-capable reader refused it too, as
`malformed at line 3: unrecognized line` (measured 2026-07-26), and the v2-only
reader only changed the wording. Its purpose was never locking — the pack tests
that reserved blocks survive read/put verbatim, so the lock body is incidental
payload. Because the hash domain is what every corpus-wide walk sweeps, that one
fixture held `mrd check` at exit 1 **permanently, on every path**, exactly as
`refusals/frontmatter-unparseable.md` did before it.

The entry above rests on *deliberately malformed*. **The ground here is
different and it is stronger: these bytes are not ours.** We are forbidden to
edit them, so the bite cannot be fixed at the fixture; and attesting a mirror
**couples this repository's root fingerprint to another repository's capture
cadence** — a re-copy would move our workspace root for a reason that has
nothing to do with our corpus. The parity tests read these files straight off
disk (`crates/testsuite/src/lib.rs :: parity_dir`), so nothing about testing
them needs them attested.

The rule names the **pack, not the one file**, for the same reason the entry
above names a directory: the next re-capture must not silently re-introduce the
bite with a second lock-bearing fixture.

⚠️ Excluding this pack takes bare `mrd check` to exit 0, and that green is
**vacuous** — this repository then declares no pin at all, and the engine says
so under `--require-pins` (`no-pin-coverage`). A green here is not evidence the
doc corpus is attested; it is evidence there is nothing to attest yet.

No `version:` key, so the § 12.3 merkle prefix stays `b3:`. The version counter
exists to stop a stale cursor matching a re-scoped world; this workspace keeps no
journal, no receipts and no armed set, so there is no cursor to protect and
inventing a `b3a:` world would only make this repo's roots incomparable with every
tool that has read it.
