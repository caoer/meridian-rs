---
type: reference
id: status
status: standing
updated: 2026-08-09
description: What the binary exposes and verifies today, reproducible from the commands shown. Also the home of R12, the armed-plane exit reading.
owns: [what the binary exposes today, R12 — the armed-plane exit reading]
---

# Status

A snapshot of what is built and verified today. Numbers here are reproducible
from the commands shown — prefer running them over trusting this prose.

> **Standing:** Design law is `wire-contract.md` (one contract). Mint addresses = segments only. Receipts = armed wire facts. DuckDB/`view_path` not agent core. **Doc correct > code correct; docs first.** See `README.md`.

**How to read this file.** The CLI inventory and per-verb behaviour are
**descriptive** of the operator surface as shipped. Wire **design law** lives
only in `wire-contract.md` . Where this
page and design law conflict, **design wins** — doc correct > code correct.
See `README.md` for process.

## Build

- Toolchain: Rust edition 2024, `rust-version = 1.96`.
- `cargo build` builds the twenty-six default members (the engine planes plus
 the `workspace` / `cache` / `registry` / `mrd` CLI foundation, and attestation's
 `git` plumbing leaf); `perfsuite` is out of default-members (27 is the total
 member count) and builds under `cargo build -p perfsuite`.
- Fork: `pulldown-cmark` is consumed via a `[patch.crates-io]` rev pin (the
 `obsidian` branch); see the workspace `Cargo.toml`.

## Wire surface

**Design law:** `wire-contract.md` — one standing contract. Content-hash
noun is **`fingerprint`**. Mint addresses are **segments only**. Ops include
`hello`, `toc`, `cat`, `extract`, `read`, `resolve`, `links`, `splice` (only
write), `fingerprint`, `diff`, `sub`, plus standing additives (`plan_edits`,
`pin`, `create`, `hello.identity`, …).

**As shipped (may lag design — treat gaps as debt, not law):**

The daemon answers protocol 1 as `meridian-daemon/1.0.0` (derived:
`concat!("meridian-daemon/", env!("CARGO_PKG_VERSION"))`); the stdio sidecar
host is DROPPED (wire-contract §3.3, 2026-08-06). Live binaries still
carry a dual negotiation path and some legacy `root` / `if_root` spellings in
code and caps tables; **standing emission and agent teaching use
`fingerprint` / `if_fingerprint` / segments** per `wire-contract.md`.

Standing capabilities agents should assume (design):

```
hello (+ identity.build)
toc cat extract read
resolve links links.require_fingerprint
splice splice.if_node_rev splice.if_fingerprint splice.dry
splice.receipt splice.verdicts splice.plan_edits splice.pin
fingerprint diff sub create
```

Also on the standing surface: `meta.duration_us` on dispatched responses;
composed-read authz facts (`span` / `content_span` / `anchors[]`); read-mint
into session memory when `actor` is present; pin/error codes as in
`wire-contract.md` § A.

Residual host fields that still emit a **joined display string** are **debt,
not address law**.

## Workspace CLI

`mrd` (`crates/mrd`) is the operator CLI over the workspace foundation:

```
mrd init [PATH] [--name NAME]
 declare the root (PATH's own MERIDIAN.md,
 `type: meridian-root`), register its drawer, reconcile
 shadowed descendant drawers (amendment M2)
mrd unregister [PATH] drop the daemon entry (if a daemon answers) + the drawer
mrd resolve [PATH] report how a path resolves — the tier that answered and
 the root it named (read-only; writes nothing)
mrd links [PATH] the corpus edge map (whole corpus, or one file),
 answered by the daemon (auto-spawned) or in-process
mrd read <PATH>[#FRAG] [--section SEL]
 the composed read: addressing + content + render at
 ONE engine snapshot (daemon or in-process; human
 output is the rendered text verbatim). SEL is a
 heading chain, a `^id`, or a dewey ordinal — the
 chain joins on `/` and that delimiter is dead for
 heading text (§ the joined selector coat). A section
 read is bounded: at most 20000 words served and 64
 distinct selectors per call, refused never truncated,
 with repeated selectors collapsed and the collapse
 stated. The section map is never word-bounded — it
 prices every section (`words`) before you ask for one,
 and it is the way back in from a refusal. When the
 daemon's hello serves the `scoped-guards` cap
 (wire-contract §5.4 family), a `--json` read also
 captures the read file's scoped token through the
 §4.7 mint arm — one extra `fingerprint {scope}`
 exchange on the same connection — and carries the
 mint body as the frame's `mint` key beside `read`;
 the wire `read` body's own `fingerprint` stays the
 ambient world token (§5.1, unchanged). No cap, the
 human face, or the in-process degrade: no mint call,
 frame byte-identical to before. A mint the daemon
 refuses after advertising the family is a loud
 refusal, never a silent omission
mrd put <PATH> [--dry | --validate] [--force] [--actor A] [--now T]
 [--if-fingerprint FP] [--scope PATH] [--receipt PATH#ANCHOR] [--json]
 the batch write: the edits ride stdin as a BARE JSON
 array — the VALUE of the wire §4.4 `edits` field, not
 the request object around it (id / op / path are
 argv's here) — as a wire `splice` to the running
 daemon (authenticated IPC; no direct-publication
 fallback). The daemon must come up (`mrd daemon`, or
 the next call auto-spawns it). Scripts that used to
 write with no daemon now need the daemon up. A
 guardless put is a wire client: fingerprint-or-force
 applies (`--force` or `if_node_rev`). `--scope PATH`
 narrows the `--if-fingerprint` premise to the named
 node (wire-contract §5.4): FP is then that node's
 scoped token from the §4.7 mint arm, not the world
 value — a disjoint sibling's birth no longer refuses
 the put. The pair law is the CLI's own wall: `--scope`
 without `--if-fingerprint` is half a premise, exit 2.
 Cap-aware: when the connected daemon's hello does not
 serve `scoped-guards`, a scoped put refuses with a
 teaching at exit 2 before any engine write — the
 daemon cannot check the premise, so nothing is sent.
 The face teaches
 the grammar itself: `--help` states the target
 shapes ({"hpath":[…]} / {"anchor":"…"} / {"fm_key":"…"})
 and the nested edit shapes ({"match":{"old","new"}} /
 {"put":{"at","text"}}) with a working batch, and a
 malformed-stdin refusal repeats that working shape
 beside the decoder's own words. `--json` is the machine
 face on BOTH legs: a commit answers {workspace, put};
 an engine refusal answers {workspace, error} on stdout
 (the engine's §8 error body, v3 vocabulary — never
 empty stdout) beside the human stderr line, exit triad
 unchanged. The human commit face prints one line per
 FIRED intent beside the fingerprint —
 `fired: <rule_id> <action> → <target> (receipt <addr>)`,
 target omitted when the intent carries none, the
 receipt address VERBATIM (the pairing key delivery
 faces echo as `correlation`) — what the write armed,
 never that anything was delivered; a workspace with
 no armed hooks prints exactly what it always did
mrd rm <PAGE> --rev <FILE_REV> [--if-fingerprint FP] [--dry] [--actor A]
 [--now T] [--json]
 guarded file death (wire-contract § A.3 remove door):
 the write model's third mutation beside `new` (birth)
 and `put` (edit), through the daemon `remove` door over IPC
 (remove-what-you-read CAS + referential check + armed
 gate; no direct-write fallback). `--rev`
 is the page's whole-file rev from a prior read,
 REQUIRED — the engine demands it from every origin and
 there is no `--force` on this door. A page with inbound
 wikilinks, embeds, or ambient meridian-lock pins
 refuses `remove_refused` naming every referring file,
 its edge kind, and its count — unlink those edges,
 then rerun. `--json` answers both legs: a removal
 {workspace, rm}; an engine refusal {workspace, error}.
 Exit triad: 0 removed|dry / 1 refused / 2 bad
 invocation
mrd pin <PAGE> <TARGET>#<SELECTOR> [--vibe] [--dry] [--json]
 mint a meridian-lock pin: PAGE records the claim,
 TARGET#SELECTOR is the content being attested
 (heading path / `^id` / dewey — see § mrd pin)
mrd repair [PAGE] [--dry] [--json]
 lost-pin repair: walk the repository's own history for
 the content of pins whose evidence is gone (both planes
 dark — the live target no longer verifies the
 fingerprint AND git no longer holds the recorded blob),
 and repoint each recovered pin's hash at the durable
 blob carrying it. No match anywhere in history is a TRUE
 LOSS, reported and never auto-fixed
mrd retire <report|mark> [--id ID] [--dry-run] [--expect-root ROOT]
 the type-2 retirement DSL: report labels measured vs
 declared; mark sweeps the `~~term~~ replacer (retired: ID)`
 markers over meridian-retire blocks (idempotent; REQUIRES
 --expect-root unless --dry-run — quiesce the fleet and
 commit the vault first)
mrd walk <PAGE> [--down] [--depth N]
 the context-assembly listing over the pin graph;
 every answer cites the revs it read
mrd rules [PATH] [--workspace | --user] [--json]
 the effective-rules print verb: what governs at PATH
 after id-based override resolution — winner first, the
 pages it shadows beneath it, plus a separate armed
 column read from the attested armed set (read-only)
mrd config the MERIDIAN.md config plane: resolve the bootstrap
 (MERIDIAN_CONFIG, then $HOME/MERIDIAN.md) and print path,
 state, origin, rev/fingerprint, the BOUND mount table, and
 declared tools — this verb PUBLISHES the mount table
mrd check [--core] [--staged] [--commit-gate [--require-pins]] [--json]
 the pure READ validity verb: claim drift + the pin
 plane (pin verdicts + blob anchoring); writes nothing,
 mints no receipt. WRITE HISTORY is NOT assessed (NOT
 CHECKED, never grey) — the engine keeps no memory;
 green means the world still matches the pins, not how
 it got there
mrd status [--cwd PATH] the bare drift + freshness summary (pure-local,
 O(armed), fetch-less)
mrd sql <QUERY> **operator face** — SQL over an ephemeral in-process
 `:memory:` projection of the corpus (NOT agent core;
 see § Operator SQL face below)
mrd test --corpus <SPEC> the pre-arming corpus runner over synthetic changes
mrd test --history <WS> --rule <PAGE> [--spec <PAGE>]
 the same law replayed against the workspace's own past;
 --spec names the spec page whose ```golden fence
 declares the exceptions (its `rule:` must name <PAGE>)
mrd run <PAGE> [TASK] run a task block declared in the page's frontmatter
mrd script [--json] [--expect-armed DIGEST]
 the script entry of the run plane: caller-supplied
 inline source on stdin, run as the caller through the
 one write path (`--json` emits the trace; the human
 face is non-normative — see `run-plane.md`).
 `--expect-armed` refuses BEFORE the splice unless what
 this run armed hashes to DIGEST — the commit half of
 the arm/commit split a gating host runs
mrd new <KIND> <ID> file birth: fill the def's template, validate, birth
 the first rev through the guarded create
mrd unfold <PRESET> materialize a preset's declared scaffold
mrd reconcile <PRESET> reconcile the tree toward a preset's declared scaffold
mrd realise <PAGE> the reconciliation loop: observe -> check -> apply
 (only on drift, once) -> re-check
mrd cache ls list the on-disk cache drawers
mrd cache clean [--all] reap stale / orphaned / retired drawers
mrd daemon run the registry daemon in the foreground
mrd --version the build identity, one line: package version + the
 tree the build read — a bare commit where that tree
 was clean, `<commit>-dirty` where tracked content
 diverged from it, `unknown` where neither could be
 read (read, never invented)
```

⛔ **The commit names the tree THE BUILD READ. It is not a claim about YOUR
HEAD, and a bare commit is not proof the build came from YOUR commit.** Three
outcomes are what `build.rs` can WRITE — bare commit, `-dirty`, `unknown` — and
there is a FOURTH STATE none of them distinguishes: a binary whose stamp was
never re-computed at all, because it was built from artifacts seeded out of
another tree whose git paths its freshness still watches. That reads as outcome
one, in a clean tree at porcelain 0, and `(read, never invented)` is exactly the
reassurance it defeats — the value WAS read faithfully, from a stale artifact of
a repository you cannot see. (Measured 2026-08-09; the demonstrated instance is
preserved at `/Users/Shared/scratch/act1-019e7ce2/{W2,Z2}`.)

And the probe is not the only producer: `build.rs` takes the value from the
environment first (`env_sha()`, falling back to the probe), so **`MRD_BUILD_SHA`
rides verbatim with no probe at all** — a supported input that can name a commit
unrelated to the tree, independently of any stale artifact. Supplying it to make a
gate agree invents the answer the pin exists to give; the supplier owns that
claim. Read at `crates/mrd/build.rs`, HEAD `b8fe2a43`.

⛔ **So the first rung is the environment, not the stamp.** A supplied value
passes the sha match, the ancestor discriminator AND the watch-list grep — every
read-time check below is defeated by it, because none of them can see where the
value came from. **An unset `MRD_BUILD_SHA` is a precondition of every other rung
here.** Check it first.

Then three checks, and they answer different questions:

```
env | grep MRD_BUILD_SHA                       # rung 0: the precondition. Any value voids everything below
mrd --version  vs  git rev-parse HEAD          # the SYMPTOM, in that dir, on that dir's own binary
git rev-parse --git-dir ; git rev-parse --git-common-dir   # the CAUSE: what "yours" means
grep -h rerun-if-changed target/debug/.fingerprint/mrd-*/run-build-script-*.json | sort -u
```

**A watched path is YOURS when it sits under your own `--git-dir` or
`--git-common-dir` AND every `/worktrees/<name>/` segment in it names YOUR OWN
worktree. Anything else is FOREIGN.** The `/worktrees/<name>/` clause is
load-bearing and a common-dir test alone does not cover it: a foreign worktree OF
THE SAME REPOSITORY sits inside your common dir and is exactly the hazard, so
"inside my common dir = normal" passes the case it was written to catch. ⭐ It is
also the one check that fires at SEED TIME: a foreign worktree name is wrong
immediately, before the receiving tree's HEAD has moved and while the sha still
agrees.

📌 **Predicate provenance, because this one moved faster than the page.** Derive
the rate rather than trusting this sentence: the `ts:` frontmatter of the fleet
notices that retired each form gives **three predicates in 5.2 minutes**, and
**8.6 minutes** to the fourth revision that also replaced the file this section
tells you to read (`all-hands/0015` `16:18:29Z` → `0018` `16:23:44Z` → `0020`
`16:27:05Z`).
The form above is the one measured into fleet law on 2026-08-09, superseding a
common-dir-only test and, before that, a bare *any absolute path* test — each
retired for admitting or flagging the wrong population. **A detector that young
rots faster than the page around it**, so treat this paragraph as the last form
this document SAW, not as proof it is the last form there is: if a later fleet
notice sharpens it, that notice is current and this line is how you know to go
looking. The stable half of this section — that the commit names the tree the
build READ, that a fourth state exists which no output distinguishes, and that
builds after `2500a4be` cannot enter it — does not move with the predicate.

Do not tighten it into *any absolute path*, either — **a linked worktree's own
refs genuinely live in the common dir, so absolute paths there are expected.**
That loose form flags every worktree on the machine and teaches the reader to
ignore the check. The predicate is *governed by a DIFFERENT REPOSITORY OR A
DIFFERENT WORKTREE*, never *an absolute path*. And **list them all: the dep-info
is a UNION, not one donor** — a tree can be governed by several invisible
repositories at once, so the count is the interesting part and finding one is not
finishing.

⛔ **Name the instrument, because the two files disagree and only one decides.**

| | File | Read it for |
|---|---|---|
| **INSTRUMENT** | `target/debug/.fingerprint/mrd-*/run-build-script-*.json` | what cargo STORED and what it COMPARES — the verdict |
| **MISLEADING** | `target/debug/build/mrd-*/output` | what the script EMITTED — absolute by nature, understanding only |

Cargo relativises a path under the package root BEFORE storing it, so the emitted
file reads absolute where the stored form is relative and harmless. **A check
globbing `output` cries wolf on every donor-seeded clone** — measured: six emitted
paths, five naming the shared repo, reading exactly like the hazard, with the
stored lists beside them relative and clean. A check that fires on healthy trees
teaches the shrug just as surely as one that misses the hazard. Never take a
verdict from `output`.

The split is predictable from git's output form rather than empirical:
`git rev-parse --git-path HEAD` returns a RELATIVE `.git/HEAD` in a main tree
and an ABSOLUTE `…/.git/worktrees/<name>/HEAD` in a linked worktree, and
`Path::join` discards the manifest prefix when its argument is absolute. On a disagreement,
`git merge-base --is-ancestor <baked-sha> HEAD` splits the readings: YES is
ordinary staleness (truthful about an earlier state of its own history). **NO is
NOT yet the hazard — a third rung decides it**, because a CHERRY-PICKED landing
puts your content under a NEW sha, so a seat that did everything the build-order
law asks finds its baked sha is no longer an ancestor of `main` the moment its
own work lands:

```
cd <the directory under test>                  # NOT the shared checkout you are standing in
git cherry $(git rev-parse HEAD) <baked-sha>
git show <baked-sha> | git patch-id --stable   # compare against the landing directly
```

`-` means your change LANDED UNDER ANOTHER SHA — benign, and exactly what a
cherry-picked landing looks like from the candidate side. **`+` does NOT settle
it on its own** — it proves only that THIS PATCH is not upstream, and it cannot
separate *never landed* from *superseded by a different patch that did land*. A
cherry-pick PRESERVES the patch; an AMEND REPLACES it, and both leave
`is-ancestor NO` with `+`. So the rung is three-way, run in the directory under
test:

| reading | verdict |
|---|---|
| `is-ancestor NO` + `-` | landed under another sha. **BENIGN** |
| `is-ancestor NO` + `+` + `git reflog` shows `commit (amend)` there | **the HANDOVER SENTENCE is stale**, not a hazard. Verify the dir and land what is in it |
| `is-ancestor NO` + `+` + no such reflog entry | the foreign-checkout **HAZARD** stands |

**No wording fix reaches this** — it is why the middle row cites the reflog
rather than a sharper reading of `+`. Receipt: a card declared `5312ea1b` while
its dir sat at `f6ed1aa7`, `+` with genuinely different patch-ids, and the
reflog named `commit (amend)`. `+` was TRUE and its stated meaning was FALSE.

⛔ **EMPTY output is a THIRD answer and it is not "no finding".** `git cherry`
lists nothing when the baked sha is already an ancestor of the HEAD you gave it —
so running it from the SHARED CHECKOUT, which is where everyone already is,
prints silence for a row that answers `+` against its own tree. **Name the HEAD
explicitly and stand in the directory under test**; treat empty as *wrong tree,
re-run*, never as a pass.

⛔ **Do NOT use tree identity for this.** Resolving the sha to its `^{tree}` and
hunting that tree in `<base>..HEAD` holds ONLY when the re-land sits on the SAME
PARENT; a cherry-pick onto a MOVED base produces a DIFFERENT TREE for the same
change, which is the normal landing shape here. Measured on candidate
`689dde53`: **no tree match anywhere in history — the tree rule calls it the
hazard — while `patch-id` is `b4c15235…` for both it and its landing `f42ace82`,
and `git cherry` prints `-`.** Tree said hazard, patch equivalence said landed,
and the second is right. Tree identity is a SPECIAL CASE of patch equivalence and
must not be used alone. **Without this rung the detector fires hardest on the
seats whose work just landed correctly** — the failure mode that retires a
detector fastest, and the reason this paragraph has now been sharpened twice.

⚠️ Run the first check against the binary in that
directory's own `target/`, never a PATH-resolved `mrd` — the installed engine is
held BEHIND the tree by design, so it disagrees with HEAD for every seat and
that is the pin working, not a defect. For an installed release compare the TAG
(`git rev-parse v1.0.0^{commit}`).

**Builds after `2500a4be` cannot enter this state**: the identity probe watches
a sentinel inside its own `OUT_DIR` and no git path at all, so there is nothing
foreign left to inherit. Measured 6 foreign paths → 0 across that commit.

`mrd help` is the authoritative surface — flags, refusal legs, and per-verb
exit codes live there.

**The exit triad is one law across the engine-backed verbs (read / put / pin):**
exit 1 is the ENGINE refusing — every engine refusal, `bad_request` included,
because a §4.4 batch the engine judges invalid (overlapping regions, a
multi-line upsert value) is the engine refusing a well-formed invocation.
Exit 2 is the CLI's OWN refusal — an unknown flag, malformed stdin, a
contradictory flag pair — issued before any engine contact. A script branches
on the exit alone: 2 means fix the invocation, 1 means read the engine's
message.

**The line between them is SHAPE vs VALUE.** The CLI's own refusals stop at the
shape of the invocation: a flag it does not know, stdin that is not the §4.4
batch shape (including a field outside an edit object's closed set). Every
judgment on a VALUE sitting inside a shape that is already legal — a block id
outside the §2.4 charset, an `old` that matches nothing, a region that overlaps
— is the ENGINE's, and leaves at exit 1 with its structured `--json` frame.
**A strict decoder at the CLI seam must not drag value laws across that line**:
doing so converts an engine refusal into a bad-invocation report and blanks the
`--json` frame the caller branches on.

### Teaching rows — five true facts about this face an agent would not predict

Each row is scoping, not defect: the CLI face is lawful and its behaviour still
surprises a reader who arrived from `wire-contract.md`. Recorded 2026-08-09
from the v1.0.0 dogfood sweep.

**`<PATH>#FRAG` is a MAP FILTER, never a body read.** `mrd read
notes/plan.md#Goals/Q3` serves the SUBTREE'S TOC ROW — its address, span and
rev — and no body. `--section SEL` is what serves bodies. This follows the wire
law (a `frag` scopes the subtree; it is not a content selector), but the usage
line above spells `<PATH>[#FRAG]` beside `[--section SEL]` without separating
them, so an agent reaching for `#FRAG` to read a section gets a map at exit 0.

**`mrd put` / `mrd pin` / `mrd rm` / `mrd retire mark` are wire clients.**
They talk to the running daemon over authenticated IPC (the same hello +
socket-law identity check the script entry already uses). There is no direct-publication fallback: a down daemon is a taught refusal (exit 2),
never a local write. Auto-spawn still runs; if the daemon cannot come up
the face names that fact and the recovery (`mrd daemon`, shorten
`XDG_CACHE_HOME` when sun_path is the cause). A guardless put is now
inside § A.1's fingerprint-or-force demand — pass `--force` or
`if_node_rev`. `--dry` is the daemon rehearsal; the old in-process
unified candidate-diff was never a wire field and does not ride. The
old `workspace_busy` class (LOCK_EX|LOCK_NB on the CLI process) left
this path with the direct lane. A commit now rides the daemon epoch, so
`seq` is the ring's, not `0`.

**A `--json` face answers `{workspace, error}` on EVERY leg that can refuse
(settled 2026-08-09; was an open asymmetry).** `mrd put --json` already did;
`mrd read --json` answered EMPTY stdout plus the human stderr line on every
refusal leg (all-fail, ambiguous, duplicate-anchor, unaddressable-host,
§2.4 charset, ambiguous-domain). **An absent frame is indistinguishable, to a
parsing agent, from success with no output** — which is why this is a defect and
not a face preference: the caller cannot tell "refused" from "nothing to say"
without reading a sentence off stderr. So the refusal envelope is the law of the
`--json` face itself, not of one verb: the engine's §8 error body in the v3
vocabulary, on stdout, beside the unchanged human stderr line and exit triad.
A.3's `reason` vocabulary and its `candidates` arrays reach a machine consumer
at the leg the plane was invented to structure.

**Two MEASURED instances, which is what makes this a class and not an
anecdote.** The rule is stated once above rather than per-verb because the
defect arrived twice, from two unrelated directions:

| # | leg | what `--json` stdout served |
|---|---|---|
| 1 | the ambiguous-domain refusal (`io_error`, two domain configs) | **EMPTY** — §8's `{cause}` and its remedy swallowed on both faces |
| 2 | the §2.4 charset violation at the unified decoder | **NOTHING** — where v1.0.0 had served a structured frame carrying `code` and `recovery` |

Instance 1 is the s10 dogfood fragment; instance 2 was measured during
verification of the `edit-object` card, where a candidate REGRESSED a frame the
release binary served. One instance reads as a verb's oversight and gets fixed
in that verb. Two instances from unrelated legs read as what they are — the
face never held the invariant — and the fix belongs at the face.

**`io_error` carries its `{cause}` onto the human face.** §8 computes
`io_error{cause}` and the engine composes real prose into it — the
ambiguous-domain refusal names both config files and which one to delete. The
CLI printed the bare token `mrd: io_error`, so the one refusal whose remedy is
genuinely non-obvious was the one whose teaching was swallowed. The cause is
rendered verbatim; nothing is invented beside it.

**The human read face carries no VALUE plane.** The composed read's `props`
rows (A.3) are machine-face facts. `mrd read`'s human output is the rendered
text verbatim, so frontmatter values arrive only as rendered source. Read
`--json` when you want the decoded value and its `prop_rev`.

### The joined selector coat — one dead delimiter, two escapes

`--section SEL` and `mrd pin`'s `#SELECTOR` share ONE human-string door
(`wire::ReadSel::parse`). Its heading arm joins on `/`, so **a heading whose
raw text carries `/` is not addressable by the joined spelling** — the door
misses rather than serving a different section, and widening the coat is C2,
reserved by ruling (`laws.md` D-1).

- The limitation is **per delimiter, per ingress**. `#` is NOT a delimiter of
 this door: `--section 'Top/C#D'` serves, and `PATH#FRAG` splits on the FIRST
 `#` only, so `notes.md#Top/C#D` serves as a frag-scoped map.
- **Two escapes, both published by the toc row the caller already read:** the
 **dewey ordinal** (`--section 1.2`) and the **raw heading segments** as the
 wire's hpath array (`{"hpath":[{"h":"Guide"},{"h":"A/B"}]}`) — one entry per
 heading, no joining. The machine plane addresses and pins such a heading
 end-to-end; only the joined coat cannot spell it.
- A miss on this door **teaches both escapes in the refusal** — a remedy that
 named only the toc read handed back the same un-feedable title.

### Operator SQL face — `mrd sql` (NOT agent core)

**Normative framing (`wire-contract.md` §10.3–§10.4; `README.md` standing C):**

- Agent core = parse + hash + §4 fact ops (`toc` / `cat` / `extract` / `read` /
 `splice` / … as contracted). Orientation surfaces that assume a SQL board
 are **not** wire ops and must not be taught as the agent path.
- Nothing on the agent path assumes SQL/DB. No wire op, field, or error
 **names DuckDB** (§10.4).
- **RULED — DROP (§10.4, 2026-08-06):** the `view_path` wire op, the
 daemon-published `view.duckdb` file, and `mrd view status` (whose subject
 was that file's freshness) are deleted. The former "keep / reshape / drop"
 line is closed.
- The sql face also projects **`.base` (Obsidian Bases) files** — the `base`,
 `base_view` and `base_formula` relations, plus `link.exclusion_path`
 (`docs/base-projection.md`). They are view-lane only and carry NO wire
 surface, and their bytes enter no fingerprint: they ride their own
 `base_fold` witness, which the freshness frame reports as a SECOND plane
 (`base_plane` in `--json`, its own banner line in human mode) so "the corpus
 moved" and "the base plane moved" stay different sentences.
- `mrd sql` is **operator convenience**: it builds an ephemeral `:memory:`
 projection of the corpus per query, writes nothing to disk, and folds
 post-result for an honest freshness frame. It is **not** a peer of
 `mrd read` / `mrd put` / wire `splice`, and its per-query build cost is
 O(corpus) — on a large tree it is a slow operator tool, by design.

### `mrd repair` — lost-pin repair (U22 / H1)

A pin carries two planes that fail independently: the CLAIM plane (its `fp1.…`
fingerprint, verified against the live target) and the RETRIEVAL plane (its
`hash`, a git blob sha). **A pin is LOST when both are dark** — the live target
no longer verifies the fingerprint AND git no longer holds the recorded blob, so
nothing in the workspace can answer *what did this pin cover?*. A red pin whose
blob is still held is ordinary drift with its evidence intact, and this verb does
not touch it.

- **The walk** is ONE `git log` plus ONE `cat-file --batch` for the whole run,
 never a spawn per pin or per commit. Each recorded version of a lost target is
 rebuilt and put to the SAME `classify_pin` the walk, `status` and `check`
 colour with; a green answer means those bytes are the pinned content.
- **Recovery is possible because the grain differs.** The `hash` is the blob of
 the whole FILE while the fingerprint covers ONE SECTION, so a commit whose
 file bytes differ elsewhere can still carry the pinned section — which is what
 the walk finds.
- **The forgery invariant.** Repair rewrites the pin's `hash` and nothing else;
 `object`, `selector` and `fingerprint` are never touched. A target that
 genuinely drifted is STILL RED after a successful repair, and that is the
 correct outcome — rewriting the claim to fit what history held would be forgery
 wearing a repair.
- **TRUE LOSS** — no version in that path's history carries the pinned content —
 is reported and never auto-fixed. The engine invents no evidence.
- **Jurisdiction:** the ambient root only. A pin naming another root names
 another object store; those pins are skipped and their count is stated.
- **`--dry`** is the skip-the-final-write rehearsal (the walk runs, the lock
 write does not), never a diff face. Progress counts ride stderr, so `--json`
 on stdout stays machine-clean.
- **The write** goes through the existing guarded `lock_write` door, so the U12
 byte-landing door census is unchanged.
- **Exit triad:** 0 nothing lost or all repaired (or `--dry` rehearsed) / 1 at
 least one TRUE LOSS / 2 bad invocation or a tool failure.

### `mrd pin` — the attestation verb

`mrd pin` mints a real `meridian-lock` pin through the same write choke-point
every other write uses: one flock, one rename (`wire-contract.md`
`wire-contract.md` § A.3).

- **Addressing** is `PAGE TARGET#SELECTOR`, two positionals, the `#` splitting
 on its first occurrence. A page-level pin is REFUSED on purpose: a change
 anywhere in the page would redden every dependent, which is what
 section-level pins exist to avoid.
- **The selector** (CLI host face) may be a **heading chain**, a **block
 anchor** (`^id`), or a **dewey ordinal** (`1.2`). Machine mint-plane address
 is **segments only** — e.g.
 `{"hpath":[{"h":"Guide"},{"h":"Leader's Guideline"}]}` or
 `{"anchor":"r-000042"}` — never a joined writeable form
 (`Guide/Leader's-Guideline`, `Guide>…`, sanitized slug) as the canonical
 address (`wire-contract.md` §2.1; segments-only law). A dewey ordinal may
 **resolve** for operator convenience but must **not** be carried as the
 canonical write address when the lock stores path arrays / segment form.
 Receipts and armed wire facts are normative (design stance); do not paste a
 display-joined string into a later `put`.
- **`--vibe`** additionally writes the target's blob into git's object store
 (`git hash-object -w`), so the pin is retrievable before any commit references
 it. Without it, the oid is computed read-only. When git cannot answer, the
 retrieval plane carries no entry — never a fabricated sha.
- **`--dry`** rehearses and writes nothing. **`--json`** prints the whole
 projected splice response under a `pin` key; human output is a confirmation
 line plus the minted fingerprint, the anchor, the blob, and the new workspace
 fingerprint.
- **No `--actor`.** The read-mint gate keys on a daemon-derived session
 identity, and a CLI invocation has no session: the bare `mrd pin` is
 local-operator-trusted and bypasses the gate, exactly as `mrd put` bypasses
 the host's authz. An `--actor` flag here would either be a meaningless label
 or a way to spell an identity the process does not have.
- **Exit triad:** 0 pinned (or `--dry` rehearsed) / 1 refused
 (`read_mint_required`, `pin_target_missing`, `write_conflict`,
 an armed gate refusal — the engine's verbatim message) / 2
 bad invocation (including a down daemon). The write is IPC, same as `mrd put`.

A pin written through the resident daemon or MCP is gated: the actor must have
read that exact selector in this session, in mode `sections`. "You cannot attest
content that was never in your context."

### `mrd rules` — the effective law, shown

`mrd rules [PATH] [--workspace | --user] [--json]`. Registration by tag plus
id-based override makes the effective rule set a **computed quantity**, and a
computed quantity the engine cannot show is one nobody can trust. This verb shows
it (registration ruling § 7).

```text
rules at sessions/s1
 workspace /var/…/ws
 user-scope «local-path» (anchor «local-path»)
 armed-set none
 task.review-notify armed=-
 winner sessions/s1/notify.md rev=018b942787febb31 layer=workspace depth=2 kinds=hook
 shadowed notify.md rev=e0dc53f2203c5969 layer=workspace depth=0 kinds=hook
 collide.here REFUSED collision at layer=workspace depth=2 — this id resolves to nothing
 tied sessions/s1/a.md rev=936e2eddf8bdf331 layer=workspace depth=2 kinds=hook
 tied sessions/s1/b.md rev=cefb207bdf220b88 layer=workspace depth=2 kinds=hook
```

Chain lines spell the resolution scope as `layer=… depth=…` — the same two
fields the `--json` face ships — never `scope=`. At this surface `scope` is the
armed artifact's ARM-ROOT column, a directory; one label carrying both
vocabularies produced hand-armed rows that parsed clean and governed nothing
(the pasted `workspace:0` now refuses at parse with a teaching).

- **The chain is never collapsed.** Per id, the winning page then every page it
 shadows, in ladder order (`git config --show-origin`). A collided id renders
 `REFUSED` naming every tied page: it resolves to nothing, so printing an
 arbitrary winner would be a coin-flip dressed as law.
- **Scope ladder**, outermost to innermost: user space (rules under the
 `MERIDIAN.md` anchor's scope) → workspace root → folder/session tree.
 Resolution is **narrowed** to PATH's own chain, so a same-id page on a sibling
 chain is no conflict — the normal case once sessions copy rule templates.
 `--workspace` prints the workspace-root layer alone, `--user` the user layer
 alone.
- **The user layer is bounded by an anchor, deliberately.** Its candidates are
 `<user-scope>/rules/**.md` where the user scope is the directory containing the
 resolved `MERIDIAN.md`. No anchor ⇒ an empty user layer that says so — never a
 `$HOME` walk, because a machine that never declared a user scope has not
 implicitly declared all of it.
- **The `armed-set` header states what is, never the engine's storage.** An
 unarmed workspace reads `armed-set none` — the whole honest answer. Where an
 armed set would live is teaching, and teaching lives in docs on demand, never
 as a parenthetical charged to every invocation (ZT ruling 4, 2026-08-15). A
 present or corrupt artifact still names its path: there the path is the
 diagnostic, not a footnote.
- **`armed=` is a separate column**, read from the attested armed set
 (`meridian/armed-rules.md`) and joined on `(id, arm root)` narrowed to PATH —
 never on id alone, never recomputed. `-` registered but unarmed · `<mode>`
 armed on the page that governs · `<mode>@<page>` armed on a DIFFERENT page,
 which is the freeze in visible form (arming pins resolution; later discovery
 never moves it) · `(drifted)`/`(missing)` when the pinned page no longer
 stands. A corrupt artifact reads `UNREADABLE`, never "nothing armed".
- **`armed=-` names one thing only: nothing governs HERE.** An armed row whose
 arm root does not contain PATH is not squeezed into that cell — it prints
 beneath the rows under `armed rows counted above whose arm root does NOT
 contain this path:`, naming its mode, its arm root, and its pinned page. Before
 this, `-` spelled both "armed nowhere" and "armed elsewhere" while the header
 counted a row the reader could not find, and a row that was BOTH out of scope
 and drifted rendered only the silent half at exit 0. The law: **containment is
 a fact and reddens nothing** (arming a sibling scope is normal), **redness is a
 fault wherever it lives** and is named and counted — so this verb and `mrd
 status` never disagree about one artifact.
- **One resolver, two consumers.** The verb calls `policy`'s own
 `RuleIndex::discover` → `narrowed_to` → `resolve` and
 `ArmedArtifact::verify_at` — the ONE composition of select-then-verify, not
 `select_at` + `verify` assembled at the call site (C3 gate finding F-4). The
 elsewhere population is the same rule: `ArmedArtifact::verify_elsewhere_at` is
 one composed call in `policy`, because a containment predicate written at the
 CLI would be exactly the second resolver F-4 removed. The
 CLI layer holds no override law, and a test asserts that structurally. A second
 resolver here could report a law the write door does not enforce — the exact
 failure the verb exists to prevent.
- **Read-only:** arms nothing, mints no receipt, spends no cap. A gate drives
 every view and asserts the workspace's merkle root *and* its whole file tree
 are unchanged afterwards.
- **Exit triad:** 0 clean / 1 a finding (a collision, a refused rule page, a red
 armed row, an unreadable armed set) / 2 bad invocation, or a PATH outside the
 workspace or not on disk — refused rather than quietly answered at the root.
 The not-on-disk refusal forecloses nothing: a folder that does not exist yet
 can mount no rules of its own, so `mrd rules <nearest-existing-ancestor>`
 answers the hypothetical exactly. The refusal only declines to dress a typo
 as an answer.

**Refusal scoping — LANDED.** The registration ruling's § 3 "Refusal scoping"
amendment (2026-08-01) rules that refusals are narrowed exactly like rules, and
they now are: a scoped query reddens only for **on-chain** refusals — the exact
subtree the refused page would have governed — while every corpus-wide walk
(discovery sweep, ARM act, cutover sweep) reports ALL refusals it encounters,
always. Off-chain reddening re-couples siblings through diagnostics, the denial
shape the narrowing amendment already rejected for rules; fail-loud survives
where enforcement lives.

- **`RegisterError` carries its own mount scope**, path-derived: `mount_dir`
 needs no frontmatter, so "cannot be answered" applies to the page's
 registration TAG and never to its mount. A refused page therefore answers the
 same mount question a registered one does — through the same `mount_dir_of`,
 `rules/`-parent lift included. **One mount law, not two.**
- **`narrowed_to` filters refusals through the same predicate it filters rules
 through.** The CLI gained no split of its own — § 7's no-mount-arithmetic-in-
 the-CLI rule is intact, and the verb prints what `policy` handed it.
- **Fail-CLOSED on the refusal itself is unchanged:** a page whose frontmatter
 does not parse never registers, from any path. Scoping decides who HEARS about
 a broken rule page, never whether it is enforced.
- **`mrd rules` on meridian-rs itself now exits 0**, and a named e2e gate
 (`meridian_rs_itself_is_clean_while_a_refusal_still_reddens_its_own_subtree`)
 measures that on the real repo alongside the other half — a refusal still
 reddening its own subtree, and the corpus-wide walk still naming it. Narrowing
 alone could not have delivered the repo half: walks report every refusal
 always, so the testsuite's deliberately-malformed schema fixture
 (`meridian-md refusal fixtures`) had
 to leave the **hash domain**, which it did through a declared, documented
 ignore in `meridian/domain.md`. The fixture is still on disk and still tested
 by the schema pack; it is simply no longer attested content that every
 discovery consumer sweeps.

**Exclusion consistency — LANDED (dogfood F11, 2026-08-15).** The declined
voices — `not offered to registration` and `cannot be answered` — enumerate by
the projection's own walk law: a dot-prefixed segment is never entered, one
shared predicate (`fs::domain::dot_segment`) spelling §12.1 rule 2 for the
hash-domain walk, the link fallback index, and this scan alike. So `mrd rules`
can never caveat a path the record projection refuses to serve (measured: 16 of
20 caveat lines and their noise came from a dot-named snapshot directory the
projection holds zero records for). The custom-ignore class stays voiced,
exit-neutral: an operator-declared `meridian/domain.md` exclusion is
vault-visible content whose silent drop is the defect session decision 0017
ended — the repo's own excluded schema fixture is the standing example, named
under `cannot be answered` at exit 0. Findings, and exit 1 with them, are
attributable only to served-corpus conditions: collisions, on-chain refusals,
red armed rows, unreadable in-domain files, an unreadable armed set. The USER
rung is untouched — it has no projection to be consistent with, and its
dot-declined pages stay named (two feeds, two sentences).

**Extended to the domain-excluded note (2026-08-15, card
voice-excluded-walk-consistency).** The stderr note the in-process enumerating
faces voice — `mrd sql`, `mrd walk --down`, `mrd check`, pageless `mrd repair`
— enumerates through the same walk law (`fs::declined_markdown`; a
dot-prefixed segment is never entered, one predicate `fs::domain::dot_segment`
spelling §12.1 rule 2). Its count and sample therefore name the custom-ignore
class only, and the note can never voice a dot-prefixed path the record
projection refuses to serve — the same disease F11 measured on `mrd rules`,
closed at this face. The custom-ignore class stays voiced and exit-neutral
(decision 0017). The machine channel is untouched: the `excluded` key of bare
`mrd links --json` stays the complete §12.1 outside-domain enumeration (§4.6),
so capping the voice to the declined class silences no machine consumer.

**Extended to the `links` face itself (2026-08-15, card walk-law-audit).** The
stderr note bare `mrd links` voices keeps riding the answer's own `excluded`
key — the door/face split: the face never re-derives by a second disk walk —
but projects that key through the same one predicate before voicing: a member
with a dot-prefixed segment (`fs::domain::dot_segment`) leaves the count and
the sample, and the prose is capped to the same one spelling the in-process
faces print (full count, `EXCLUDED_SHOWN` sample, remainder clause, machine
pointer). On this face the pointer is self-serving in the good sense: the
complete list it names — the `excluded` key of `mrd links --json` — is this
very verb's own machine answer, which stays the complete §12.1 enumeration
(§4.6), byte-identical. Before this, the face voiced the wire key verbatim
and uncapped: dot paths in the voice (the F11 disease) and unbounded prose
(the 2026-08-10 3.1M-character measurement's shape), both closed here.

**Extended to the `mrd retire` human render (2026-08-15, card
retire-cmd-cap-join).** This one is the CAP class only, never the walk-law
class: `retire` certifies absence, so its outside-domain population is
lawfully COMPLETE — dot paths included — and stays so on `files_excluded`,
byte-identical. What was uncapped was the human line, which joined that whole
population into prose. It now names the same `EXCLUDED_SHOWN` sample with the
same remainder clause as every other face (one spelling, `capped_sample`), and
points at `files_excluded` on this verb's own `--json` as the complete list.
The count stays the full population: capping the sample bounds the prose, it
never re-scopes what was excluded.

**Extended to the `mrd rules` undecidable line (2026-08-15, card
rules-undecidable-carrier).** The CAP class again, and the last uncapped
human-face join in this verb. `cannot be answered` is the one declined voice
whose population is genuinely UNBOUNDED: `register` splits frontmatter and
refuses on `FrontmatterUnparsed` **before** any tag is read
(`policy/src/registration.rs`), so every excluded file with a malformed
frontmatter block lands here whether or not it ever meant to carry a rule — no
registrar narrowing stands between it and a generated corpus. Its two
neighbours (`not offered to registration`, both feeds) are narrowed to pages
that OFFER THEMSELVES to registration (`rule_pages_among`) and are bounded by
construction; they are not in this class and are not capped. The line now
prints the full count, the same `EXCLUDED_SHOWN` sample and remainder clause as
every other face (one spelling, `capped_sample`), and points at
`not_offered.undecidable` on this verb's own `--json` as the complete list.

**The carrier was already there** — `not_offered` has carried all three
declined populations complete since 2026-08-09, `--json` untouched by this
card. A prior audit (card cap-convention-audit) read only the top-level keys of
`to_json`, missed the nested block, and therefore ruled this site
"REPORT, do not land" for want of a machine carrier. The verdict was right for
the wrong reason: the site needed the cap, and nothing needed minting. Its
Reason 1 — the other two declined voices are registrar-narrowed, hence out of
the class — is independent and stands.

### `mrd check` — does the world still match the pins

Both layer-0 planes the core reads are memory-free: the claim plane (pinned
content drift) and the pin plane (the pin verdicts and the anchoring state of
every pinned blob) are observed against the CURRENT tree. `check` answers
at-rest truth — does the world still match the pins — writing nothing and
minting no receipt. `status = freshness, check = validity`: this verb answers
"what lies?".

**Write history is not assessed: the engine keeps no memory.** History is
pinned to git at lock, and anything between locks is not history — so chain
continuity and last-receipt-vs-live are **not checked here at all: not grey,
NOT CHECKED**. Green means the world still matches the pins, never how it got
there. Every face carries the `write_history: not-assessed` disclosure naming
the narrowing — what the green stopped covering, and that the answer lives in
git. The disclosure states the narrowed CLAIM, never the engine's mechanism:
the WHY above is this document's to teach, on demand, not a footnote charged
to every invocation (the report-voice law, ZT rulings 3–5, 2026-08-15).

**The interval this verb spans.** The `worktree` interval — the bytes on disk
— is always assessed. `--staged` adds the interval a commit records: git
commits the INDEX while `domain_snapshot` reads the worktree, so whenever the
index carries anything the worktree does not, the staged bytes are assessed as
a second pass running the same reads over different bytes. The exit is
worst-of across both intervals, and every refusal names the interval it came
from. The interval line states which case ran:
`coincides` (the index adds nothing, so one pass IS the interval a commit
would record), `diverges` (N paths differ, assessed separately),
`no-repository`, or — asked but unreadable — `grey(cannot-assess)`, which
fails closed on exit 1 rather than silently degrading to the worktree answer.

**The exit triad stays closed** (0 green / 1 finding / 2 bad invocation): grey
rides leg 1 — a grey pin or an unaskable object store refuses
`grey(cannot-assess)`, because unknown is not clean — rather than inventing a
fourth code, because the exit code answers exactly one question — *may this
proceed?* — and red and grey both answer no. The *reason* is a different fact,
and it lives in the output where a reader can read it.

#### The findings enumeration is COMPLETE, never worst-of

The refusal on stderr is a numbered list — `check refuses (<interval>) — N
findings:` — and a caller reads it as the fix-list. **It carries every finding
the refusing interval holds, red AND grey, in worst-of ORDER but never worst-of
SELECTION.** The exit code is one bit and may be decided by the worst colour; the
list is the reason half, and a reason half that drops the greys sends an operator
away believing the corpus clean once the reds are fixed. Grey findings sort after
red ones and keep their own reason word; the count is the count of everything
listed.

(Measured against v1.0.0, `mrd-dogfood` s14-70: a grey pin beside four red
findings was neither counted nor named — "4 findings" while five questions stood.
Unmasked at s14-40 the same grey WAS the one-finding list. One unverifiable pin
may not hide inside a green fleet; it may not hide inside a red fleet's findings
list either.)

#### Two independent axes: WHICH BYTES, and WHICH QUESTION

`mrd check [--core] [--staged] [--commit-gate [--require-pins]] [--json]`.

**`--staged` picks the interval.** `domain_snapshot` reads the worktree; git
commits the INDEX. Forge a pinned section, `git add` it, restore the governed
bytes to the worktree, and an unscoped check answers green over bytes no commit
would record. `--staged` assesses the index whenever it carries anything the
worktree does not, and the exit is worst-of across both intervals, each refusal
naming which one it came from.

**`--commit-gate` picks the question, and implies `--staged`.** It narrows the
exit to ONE interval — the one a commit records — so a finding from the
worktree cannot swamp a clean answer about the bytes being committed, and it
gates on the pin plane alone: a pin is a claim about the bytes being
committed, so it belongs to the interval, not to any history. The passing word
is **`pins-hold`** — it names the plane that actually answered, and cannot be
misread as a claim about write history. A fence whose verdict did not vary
with what is staged would carry zero information about the commit it guards;
this one re-reads the index's pin plane at every commit. **This is why the
emitted fence body runs `mrd check --commit-gate` and not `mrd check
--staged`.**

**`--require-pins` tightens the gate, opt-in.** A corpus that declares no pin
PASSES the gate by default — over zero pins "does the world still match the
pins" is vacuously true; nothing is unknown because nothing was asked. A
caller that wants no-coverage to mean refuse says so with `--require-pins` and
gets it in the exit code, under its own word (`no-pin-coverage`, never
grey's). A grey pin or an unaskable object store fails CLOSED either way. A
fail-closed default would make the gate un-adoptable on every vault that has
not started pinning.

| | gates the exit | reads |
|---|---|---|
| unscoped | worst-of across every interval assessed | claim drift + the pin plane, per interval |
| `--commit-gate` | ONE interval — the one a commit records | the pin plane alone over those bytes |

**The pin population is the pins the workspace DECLARES, not the hash domain.**
`mrd pin` admits a holder page the hash domain excludes — a dot-segment path, a
`meridian/domain.md` ignore rule — and mints the pin at exit 0. The pin plane
therefore reads its rows from **every markdown page under the root**, not only
from the corpus the fold is taken over: a page the domain excludes is excluded
from HASHING, and a pin it holds is a claim the workspace has made regardless.
Reading the rows only from the hashed corpus makes `--commit-gate` assert *every
pin in the interval holds* over a population it silently narrowed, and answer
GREEN over a pin that has drifted.

⚠️ **The narrowing is a property of EXCLUSION, not of dot segments** — the
custom ignore list reproduces it identically.

**What does NOT change is the target's colour.** The hash domain still gates
HASHING and never addressing: a pin whose TARGET the domain excludes stays
`grey(outside-hash-domain)`, reported and never gated, exactly as before
(`wire-contract.md` §12.1, verdict-plane clause). Holder and target are
independent axes (session decision 0045). Widening the population of pin SOURCES
does not widen the corpus that resolves pin TARGETS, and the excluded holder's
bytes never enter the merkle root.

**There is no permanence, because there is no memory.** Nothing in the verb
looks backward: no standing break is printed, no verdict is carried forward
from an older run, and a gated pass is never spelled as a claim about a
record — the word is `pins-hold`, full stop. How the bytes came to be staged
is git's business; whether their pins hold is the gate's.

**The declared blind spot is named, not assumed.** Pin rows held by
domain-excluded pages are read at their WORKTREE bytes for both intervals, so
a holder that is both domain-excluded AND staged-modified has a pin row added
or removed in the index alone go unseen.

**The `--json` face.** Each interval emits `{workspace, red, write_history,
core: {drifted_claims}, pins}`; the top-level `red` is worst-of across
intervals, and the `interval` block carries the `state`, `spans_the_commit`,
the `diverged_paths`, and the nested staged answer. The `commit_gate` key is
present ONLY when the scoped question was asked — `{gated_interval, permits,
verdict, detail, gated_planes: ["pins"], write_history, pin_coverage,
require_pins}` — under this face's own law: an absent field reads as "not
checked", where a `null` would assert a read that never happened. A top-level
`fence` block reports the checkout's fence coverage on every run.

**The `fence:` line** is a proposition about the local checkout's
configuration, not the corpus, and it never touches the exit code:
`$GIT_DIR/hooks` is never a tracked path, so fence coverage is per-checkout
and opt-in — a fresh clone being unfenced is a supported state.

### The composed status line

`mrd status` renders five orthogonal axes on one line, worst-of WITHIN each axis
and never across them:

```
pin green · lock none · anchor at-tip (anchor as-known) · armed off · vibe-debt 0 blobs (0 bytes)
```

| axis | answers | values |
|---|---|---|
| `pin` | the ARMED SET's evidence drift — each armed row's live PAGE rev against the `rev` its armed-rules row attested (PAGE rev uniformly, `armed-plane.md` §4; the pinned-`armed_rev` `CHECK.md` surface is retired) | `green` · `red content-drifted` |
| `lock` | every `meridian-lock` pin's FINGERPRINT verdict, rolled up | `none` · `<color> [N pins]` · `unreadable (<why>)` |
| `anchor` | how current the working copy is against origin's tip, plus the trust of that knowledge | `at-tip` / `behind`, qualified — see the colors amendment § The anchor axis |
| `armed` | whether armed law refuses this change | `off` · `warn` · `block` · `armed` (hook mode) |
| `vibe-debt` | how much of the retrieval plane is held by this machine alone | `N blobs (M bytes)` · `unknown (<why>)` |

Two axes are new in stage 2, and both are read wrong by default:

**`lock` is orthogonal to `pin` and neither subsumes the other.** `pin` rolls up
the armed set; `lock` rolls up fingerprint verdicts. The `lock` roll-up is
worst-of **red > grey > green** — grey ABOVE green is load-bearing, because a
roll-up that let one unverifiable pin hide inside a green fleet would render the
exact false green the color law forbids.

**A green `lock` axis does NOT imply the tree is current. Currency lives on the
`anchor` axis.** The plan text listed "origin tip-compare currency" inside the
drift-color unit; the shipped code deliberately does not fold it into the pin
tone. Currency is a REPOSITORY-level fact, while a pin verdict is per-pin and
content-addressed. Folding them would merge two axes of the composed legend, and
it would re-root a computation that D12 requires to be root-independent. So
`lock` says whether the pinned content still matches the working copy, and
`anchor` says how current that working copy is against origin. Read together,
never multiplied.

**`vibe-debt` is a meter, never a gate.** It counts the lock-referenced blobs
git HAS but no commit reaches — exactly the `pending-anchor` population, which
is the window named residual G1 leaves open (`gc.pruneExpire`, git default two
weeks). A blob absent from the object database (`never-anchored`, pruned or
freshly cloned) is NOT counted: that is past debt, not debt, and its bytes no
longer exist to sum. Debt never enters the findings verdict and never refuses a
write — the gauge reports the size of the window, it does not shorten it. Zero
renders (`0 blobs (0 bytes)`); a gauge that hides at zero is not a gauge.

`--json` always-emits both axes. `composed.lock.pins` is `0` and
`composed.lock.color` is `null` on a corpus with no pins — never an absent field
a reader could mistake for "not checked".

#### The exit reads the ARMED PLANE ONLY — every other axis is a reading (R12)

`status`'s exit-1 leg is narrow, and deliberately so. Spelled here because the
prose above reads wider than the face acts, and one surface may not carry two
spellings:

| axis | moves the exit | |
|---|---|---|
| the ARMED plane — an armed rule drifted, or the armed-rules artifact faulted | **yes** ⇒ exit 1 | the `pin` axis and the `rules:` line (label per ZT ruling 5, 2026-08-15: the line states the RULES facts; `armed-rules` is the artifact's name, not the report's voice) |
| `lock` | **no** — `red …`, `grey …` and `unreadable (<why>)` all exit 0 | a READING, not a gate |
| `anchor` | no | freshness, and `status` cannot fetch |
| `vibe-debt` | no | a meter, never a gate (above) |

**This is R12, and it is ratified design, not debt.** The lock roll-up is
rendered so a reader sees it; it is not a verdict the shell may branch on.
THREE in-repo gate files assert it by name — `crates/mrd/tests/u14_check_pin_plane.rs`
("`mrd status`'s exit triad does NOT change — the rollup is a reading, not a
gate"), `crates/mrd/tests/u13_per_root_anchoring.rs` (five exit-0 arms across
its four `#[test]` fns — `grep -c 'code, 0'` reads 5, and four of those arms
name `R12` in the assertion message), and `crates/mrd/tests/status_e2e.rs`
("debt is not a finding"). Three counts FILES, five counts ASSERTION ARMS —
different units, so neither number checks the other.

**So `mrd status || alarm` does NOT fire on attestation drift. The fail-closed
door is `mrd check`** — a shell that must refuse on a red or grey pin runs
`check`, whose exit triad answers exactly that question and whose grey leg fails
closed. `status` answers *what does this workspace look like*, and only the armed
plane makes it refuse.

Read the composed line, not the exit, for the lock verdict. The roll-up's
worst-of law (grey above green) governs what is RENDERED — it never promised an
exit code, and none of the three lock verdicts has ever produced one (measured on
v1.0.0 three ways: `mrd-dogfood` s14-70 red, s14-40 grey, s14-43 lock-refused).

Output is JSON under `--json`, a human table otherwise; exit codes are 0 clean /
1 findings (the armed plane, per the table above) / 2 tool failure. The workspace it ran over is printed with the tier
that answered — `status <root> (git-root)` — and `--json` carries the same word
as `source`, because a status line that named only a path would leave the reader
to guess which root it judged.

### The resolution ladder — three rungs, and every answer names itself

`workspace::resolve` has **three** rungs, and the marker tier is gone
(marker-retirement ruling, 2026-07-26). **A `.meridian.toml` or
`.meridian.yaml` still sitting in a tree is inert** — no code path in this
engine reads either file, so one left on disk anchors nothing, grants no
`[run.caps]`, and changes no answer below. Removing them is an operator's
choice, not this engine's business:

| Tier | What it answers | How |
|---|---|---|
| `env-override` | the environment named the root | `MERIDIAN_WORKSPACE` |
| `git-root` | where the version-control boundary is | the **nearest** ancestor `.git` (directory or worktree pointer file) |
| `cwd-default` | nothing — a convenience default | the canonical cwd |

**Every resolution states which tier answered and which root it named** — this
is the ruling's requirement, not a rendering preference, and it is enforced by
the type: `Answer` has no public path field, `root` returns `None` on
`cwd-default`, and reaching the defaulted path takes the greppable
`root_or_cwd`. `mrd resolve` prints both (`source:` plus the path); `mrd status`
prints both on its header line; `mrd init` prints the ladder's answer for the
directory it just declared.

A refusal states them too. When `mrd run` misses its page and an answered rung
named the root, the refusal appends both facts:
`page not found: <ref> (workspace <root>, source: env-override)` — same form
for `git-root`. The ref is the part of the invocation most likely to be
correct; the root is the part the environment may have swapped underneath it
(a sticky `MERIDIAN_WORKSPACE` from an earlier shell is the field case —
dogfood F6), so a miss that hides the root points diagnosis at the wrong
suspect. A `cwd-default` miss stays bare: `root` is `None` there — a defaulted
cwd is not a workspace, and the refusal does not promote it to one.

An answered rung opens the hashed drawer directly. A `cwd-default` tree adopts a
running daemon's registered ancestor if one answers, else degrades to an
ephemeral, per-invocation store that writes nothing — it is never silently
registered. The adopted daemon may be a different build than the caller: the
adoption itself exchanges only a registration record, and any content that
follows rides a v3 connection where the socket law
(`docs/wire-contract.md` §A.3, 0025) compares `hello.identity.build` at connect
and refuses across builds.

The CLI's word for a `cwd-default` answer is therefore the **refinement**, not
the tier: `daemon-adopted` (the daemon supplied the root) or `ephemeral` (nothing
did, and this invocation's store writes nothing). Both imply `cwd-default` and
both name the root beside it, so the four words `env-override` / `git-root` /
`daemon-adopted` / `ephemeral` are a strict refinement of the three tiers — what
happened after the ladder fell through is the fact an operator needs.

**A root's `MERIDIAN.md` self-declaration is NOT a rung.** It is read by
`crates/config` (mount binding, and `crates/run`'s `run.caps.*` /
`run.timeout_secs`), never by the ladder: existence-only detection is what the
retired marker did wrong, since it cannot tell a `meridian-root` declaration
from a `meridian-config`. So `mrd init` below a git root declares that directory
a root **and still resolves to the git root** — init says so, and names the two
ways to change the answer (`MERIDIAN_WORKSPACE`, or address the root by name
through the mount table). Whoever used a marker to carve a sub-root uses one of
those, or registers the tree with the daemon.
### `mrd skill hook` — the commit fence, as a DOCUMENT

`mrd skill hook` prints one markdown document to stdout and does nothing else:
no file is written, no git directory is read, no workspace is resolved. **The
markdown IS the contract** — what to place, where, when to refuse to place it,
how to verify — and the agent reading it does the placing. Exits 0 (the document
was written) or 2 (bad invocation). There is no `--json` face: a JSON envelope
around a markdown string is a second contract for the same bytes.

**The install plane was deleted, not shimmed** (2026-07-29; `mrd hook` no longer
parses, and USAGE names the successor). A verb that wrote into `$GIT_DIR` had to
carry an uninstaller that refused a foreign file, an `flock` held across a
read-decide-write section spawning `git` three times inside it, a downgrade guard
with its own environment escape, and a partial-coverage migration state — four
planes of imperative machinery encoding rules that are, in the end, prose. They
are now legible content of the emitted document instead of code paths that have
to be trusted, and the one thing that cannot be prose — reading bytes off a disk
to say what generation is standing there — is what stayed, as `mrd check`'s
`fence:` line.

**The door set is three, and that is a claim about coverage.** `pre-commit`,
`pre-merge-commit` and `pre-applypatch` are every hook git dispatches for a commit
it builds from a prepared index, so the fence's question is the same at all three
and **one body serves them all**. A set of one let `git merge` and `git am` land
commits past a fence that printed nothing.

**Placed per `$GIT_COMMON_DIR`, not per worktree.** N linked worktrees are N
meridian workspaces sharing ONE `hooks/` directory, so the fence is written once
and reads the committing worktree from git's working directory at run time — it
bakes no path in. Per `--git-dir` writes N files of which git runs one; per
worktree top-level overwrites the same file N times. The `chmod +x` is part of
placing it: a hook git cannot execute is a hook git skips, silently.

**The body runs `mrd check --commit-gate`** and rejects on its exit — the scoped
question above. It holds **zero markdown semantics**: nothing in it parses a
selector, reads a rev, or spells a colour word, and refusal's legal home stays
engine-side. `crates/mrd/tests/skill_hook_emit.rs` asserts that over the emitted
bytes rather than promising it.

**Three commit-creating paths stay open and are declared, not papered over:**
`git cherry-pick`, `git revert` and `git rebase` replay dispatch no veto-capable
hook that can read the index. So the fence's guarantee is *no out-of-band write
reaches history through `commit`, `merge`, or `am`* — it is **not** *no drift
reaches history*. Across the replay paths the engine's read-time `mrd check` is
the only guarantee.

**Coverage is per checkout and opt-in, permanently.** `$GIT_DIR/hooks` is never a
tracked path, so no clone can carry the fence. A global `init.templateDir` would
transport it and is refused on its collateral: it fences every unrelated
repository the operator clones or inits, abolishing the opt-in premise the body's
no-membership-test design rests on. A fresh clone being unfenced is a supported
state, which is why `mrd check` says so unasked.

**Escapes at commit time**, both named in every refusal the fence prints:
`MRD_HOOK_FORCE=1 git commit …` (the ratified `--force`, in the spelling a hook
that receives no arguments can carry) and git's own `git commit --no-verify`.

**`MRD_HOOK_FORCE` is a two-sided grammar with a loud third leg.** The value is
parsed, never merely tested for non-emptiness — `[ -n … ]` opened the gate on
every spelling of *"do not force"*, because it read whether a value was typed and
never what it said.

| `MRD_HOOK_FORCE` (trimmed, any case) | Verdict |
|---|---|
| `1` `true` `yes` `on` | **bypass** — and the bypass is printed on stderr, naming the value and stating that nothing was checked |
| `0` `false` `no` `off`, empty, whitespace, unset | **fence normally**, silently — the specificity half of the notice |
| anything else | **refuse the commit**, exit 1, naming the value it could not parse |

**The fence declares its own generation and the engine reads it.**
`# mrd-hook-fence <n>` on line 2 is parsed and compared against the engine's own,
yielding a three-valued relation — a byte-equality test collapsed *older* and
*newer* into one answer and then asserted a direction it never measured. The
document and `crates/mrd/src/hook.rs`'s `FENCE_VERSION` are held to each other by
the emitter's design tests, so a body change without a bump fails in CI rather
than shipping.

| `mrd check` fence state | Meaning | Remedy |
|---|---|---|
| `installed` | every door carries this engine's fence | — |
| `installed-partial` | some doors unfenced | place the body at the rest |
| `installed-superseded` | the fence is older than this engine emits | re-place from `mrd skill hook` |
| `installed-ahead` | **the fence is NEWER than the engine answering** | put the current engine first on PATH — **do NOT re-place**, which would downgrade the fence |
| `installed-unversioned` | marker present, generation undeclarable | refuse rather than guess |
| `foreign-hook` | a door carries a file this engine did not write | move or remove it |

The document tells its reader to refuse the last three, plus a submodule (its
hooks live at `<super>/.git/modules/<name>/hooks`, which this engine does not
compute), a set `core.hooksPath` (git runs hooks from there, and if that path
already carries a `pre-commit`, writing there would write into another checkout's
hook directory), a workspace root that is not the worktree top-level, and a root
that is not a git repository at all — a supported workspace state with simply
nowhere to put a hook, not a fault.

At commit time the fence **fails closed**: `mrd` absent from `PATH` refuses with a
teaching message naming both escapes and how to delete the file, because a commit
nobody could vouch for is not a verified one. An `mrd` on PATH that predates
`--commit-gate` exits 2, which the body handles with a refusal naming the skew and
the commands that decide it — the ordinary state of a cutover, and it still fails
closed rather than falling back to a check that reads the wrong bytes.

**Verify with `mrd check`.** Its `fence:` line carries the set's word, the count of
doors carrying the marker, and a teaching; its `fence doors:` line names each door
with its own word so a disagreement can be located. Under `--json` the same
reading is the `fence` object with `doors[]`, `fenced_doors`, `total_doors`,
`engine_version` and `gates_the_exit: false`. **That line never moves the exit
code** — fence coverage is a property of a local checkout, not of the corpus.

Nothing in this engine writes into a git directory any more, so a root that is
merely looked at comes away byte-identical, including the roots that refuse.

## Tests

`cargo test --workspace` — full suite green. The attestation integration branch
`stage2-core` gates every merge, most recently **1267 passed / 0 failed / 2
ignored** (M1 shipped at 1117). Export `CARGO_PROFILE_TEST_DEBUG=0` before the
run: a full-debug `target/` in this workspace costs ~26G, and it is the repo's
own CI lever. The
`testsuite` crate carries the frozen ground-truth pack (rung-1 parse truth:
every node reproduced byte-for-byte) plus the U0 read/put parity pack
(`data/parity/`, captured from the live host face): `u0_read_parity`
(addressing facts), `u4a1_render_parity` (rendered text), and
`u4a2_composed_read` (the composed op through the live serve loop, refusal
texts included) replay it. The CLI foundation's end-to-end gates live in
`crates/mrd/tests/e2e.rs`.

### Harness caveat (standing C)

Green tests on **tiny synthetic workspaces** are not a claim that every surface
is proven on a **real corpus**. In particular:

- the operator SQL face (`mrd sql`, an ephemeral DuckDB projection) rebuilds
 the whole corpus per query — slow on production trees, by design (the
 published-view organ was dropped by ruling, §10.4 2026-08-06);
- address and write paths must be verified against segment form and armed
 receipts, not against display-joined strings that only appear in harness
 fixtures.

Do not claim “proven end-to-end” without that caveat. Session pressure that
forced this wording: `05-19-meridian-socket-mcp-leg` (read→write address break;
view organ vs real corpus).

## Performance

`perfsuite` carries a claims registry (`crates/perfsuite/claims.toml`) whose
verdicts are computed, not asserted, and written to
`crates/perfsuite/`. Current tally: **2 PASS, 7 MEASURED, 14
UNTESTED** over 23 claims — the untested claims are perf rungs whose baselines
land on the first fleet run; the passing ones cover cold ingest and codec bulk
cost. Run the benches to refresh:

```sh
cargo bench -p perfsuite
```

## Known gaps

- Perf rungs are largely UNTESTED pending baselines (see the tally above).
- `policy` verdicts ride every splice response as `[]` until rule packs are
 loaded; where packs are sourced is the host's concern.
- **`require_fingerprint` has no CLI spelling.** `links.require_fingerprint`
 is a served wire cap (`wire-contract.md` §10.2; release §2.1), but
 `mrd links --require-fingerprint` answers `unknown flag`, exit 2. The §10.2
 posture — refuse with `stale_view` instead of answering in an unnamed tense
 — is reachable at the socket only, never from the operator face (dogfood
 2026-08-09, s9).
- **CLI-committed writes advance the fingerprint, never `changes_seq`.** An
 `mrd put` commit moves the workspace fingerprint the daemon serves
 immediately, and mints no Delta, so `changes_seq` reads the same value
 before and after (dogfood 2026-08-09, s9; declared at `wire-contract.md`
 §18 row 12). A consumer polling `changes_seq` as a change monotone misses
 every CLI-lane write — diff by fingerprint (§4.7) instead.

Accepted residuals (attestation surfaces) — documented, not prevented. Full statements in
`wire-contract.md` § Named residuals.

- **G1** a `--vibe` blob is reachable from no ref, so `gc.pruneExpire` (git
 default two weeks) is its durability horizon. The vibe-debt gauge measures the
 window; committing the file is the only durable anchor.
- **G2** the write flock serializes COOPERATING writers only. An out-of-band
 write is detected by the drift color, never prevented — the git pre-commit
 hook fence is stage-3.
- **G3** a pin writes two inodes and is not all-or-nothing. A failure between
 them leaves a rev-neutral, slug-derived anchor that a re-pin reuses and heals,
 never silent corruption.
- **G4** refs are intra-root only. Cross-root addressing, the mount table, and
 the MERIDIAN.md config engine are stage-3; stage 2 only keeps the seam open
 and never entrenches "there is exactly one root".
- **G5** anchor promotion into an unowned target churns that file's CAS token.
 Accepted for the core loop because the promotion is rev-neutral; the fence and
 the authz tightening are stage-3.

Also stage-3, and NOT shipped: the receipt / `predicate_type` representation
unification (the read-mint ledger and the persisted `^receipt` projection are
still two representations of one receipt family), the defsarm bridge-legs drop,
and full-document re-attest.
