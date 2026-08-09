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
- `cargo build` builds the twenty-seven default members (the engine planes plus
 the `workspace` / `cache` / `registry` / `mrd` CLI foundation, and attestation's
 `git` plumbing leaf); `perfsuite` is out of default-members and builds under
 `cargo build -p perfsuite`.
- Fork: `pulldown-cmark` is consumed via a `[patch.crates-io]` rev pin (the
 `obsidian` branch); see the workspace `Cargo.toml`.

## Wire surface

**Design law:** `wire-contract.md` — one standing contract. Content-hash
noun is **`fingerprint`**. Mint addresses are **segments only**. Ops include
`hello`, `toc`, `cat`, `extract`, `read`, `resolve`, `links`, `splice` (only
write), `fingerprint`, `diff`, `sub`, plus standing additives (`plan_edits`,
`pin`, `create`, `hello.identity`, …).

**As shipped (may lag design — treat gaps as debt, not law):**

The daemon answers protocol 1 as `meridian-daemon/0.1`; the stdio sidecar
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
 heading text (§ the joined selector coat)
mrd put <PATH> [--dry | --validate] [--force] [--actor A] [--now T]
 [--if-fingerprint FP] [--receipt PATH#ANCHOR] [--json]
 the batch write: the edits ride stdin as a BARE JSON
 array — the VALUE of the wire §4.4 `edits` field, not
 the request object around it (id / op / path are
 argv's here) — through the production splice
 choke-point (CAS + armed gate + write flock). The face
 teaches the grammar itself: `--help` states the target
 shapes ({"hpath":[…]} / {"anchor":"…"} / {"fm_key":"…"})
 and the nested edit shapes ({"match":{"old","new"}} /
 {"put":{"at","text"}}) with a working batch, and a
 malformed-stdin refusal repeats that working shape
 beside the decoder's own words. `--json` is the machine
 face on BOTH legs: a commit answers {workspace, put};
 an engine refusal answers {workspace, error} on stdout
 (the engine's §8 error body, v3 vocabulary — never
 empty stdout) beside the human stderr line, exit triad
 unchanged
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
mrd walk <PAGE> [--down] [--depth N]
 the context-assembly listing over the pin graph;
 every answer cites the revs it read
mrd rules [PATH] [--workspace | --user]
 the effective-rules print verb: what governs at PATH
 after id-based override resolution — winner first, the
 pages it shadows beneath it, plus a separate armed
 column read from the attested armed set (read-only)
mrd check [--core] the pure READ validity verb: receipt-chain continuity
 + the foreign_edit trace; writes nothing. Refuses
 grey(cannot-assess) when the journal cannot date the
 live tree (no rows, or a stale last receipt)
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
 commit the build read (`unknown` when it could reach
 no repository — read, never invented)
```

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
 `workspace_busy`, an armed gate refusal — the engine's verbatim message) / 2
 bad invocation.

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
 armed-set none (meridian/armed-rules.md absent)
 task.review-notify armed=-
 winner sessions/s1/notify.md rev=018b942787febb31 scope=workspace:2 kinds=hook
 shadowed notify.md rev=e0dc53f2203c5969 scope=workspace:0 kinds=hook
 collide.here REFUSED collision at scope=workspace:2 — this id resolves to nothing
 tied sessions/s1/a.md rev=936e2eddf8bdf331 scope=workspace:2 kinds=hook
 tied sessions/s1/b.md rev=cefb207bdf220b88 scope=workspace:2 kinds=hook
```

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
- **`armed=` is a separate column**, read from the attested armed set
 (`meridian/armed-rules.md`) and joined on `(id, arm root)` narrowed to PATH —
 never on id alone, never recomputed. `-` registered but unarmed · `<mode>`
 armed on the page that governs · `<mode>@<page>` armed on a DIFFERENT page,
 which is the freeze in visible form (arming pins resolution; later discovery
 never moves it) · `(drifted)`/`(missing)` when the pinned page no longer
 stands. A corrupt artifact reads `UNREADABLE`, never "nothing armed".
- **One resolver, two consumers.** The verb calls `policy`'s own
 `RuleIndex::discover` → `narrowed_to` → `resolve` and
 `ArmedArtifact::verify_at` — the ONE composition of select-then-verify, not
 `select_at` + `verify` assembled at the call site (C3 gate finding F-4). The
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

### `mrd check` — grey when it cannot date the tree

Both layer-0 journal detectors rest on ONE assumption: that the last receipt's
recorded `root_after` still accounts for the live tree. The assumption fails in
two measured ways, and the same mechanism causes both — **a governed `splice`
advances the tree root and writes no journal row**:

| journal state | what it used to say | truth |
|---|---|---|
| **no rows** (a `pin`/`put`-only workspace) | `chain: green · foreign_edit: none`, exit 0 | nothing was assessed — an out-of-band edit is invisible |
| **rows, then any governed splice** | `foreign_edit: RED`, exit 1 | it accused a fully governed workspace |

So `mrd check` now renders **`grey(cannot-assess)`** on both detector lines
whenever it cannot show its baseline is current, `--json` carries `core.chain` and
`core.foreign_edit` as `null` plus a `cannot_assess` block (`reason:
"grey(cannot-assess)"` — the same word, plus the `baseline` evidence when there is
any), and the verb exits **1**. `red` stays `false` — grey is not red. Where the
last receipt DOES account for the live tree, every byte of the render is what it
always was.

The `foreign_edit` **accusation** is withdrawn, not the evidence: the mismatch is
still printed with both roots and the last receipt. What check no longer does is
name a culprit it cannot identify — an out-of-writer edit and a governed splice
leave the same trace.

**The exit triad stays closed** (0 green / 1 finding / 2 bad invocation): grey
refuses on leg 1 rather than inventing a fourth code, because the exit code
answers exactly one question — *may this proceed?* — and red and grey both answer
no. The *reason* is a different fact, and it lives in the output where a reader
can read it: `grey(cannot-assess)` versus `red(…)`.

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

**This is honest degradation, not the missing capability.** A workspace that has
ever been spliced still refuses — now truthfully. Making `check` able to answer
(journaling the splice, or reading the pin plane, which it has no `lock`/`git`/
`view` dep for) is a separate unit with its own gates.

#### Two independent axes: WHICH BYTES, and WHICH QUESTION

`mrd check [--core] [--staged] [--commit-gate] [--json]`.

**`--staged` picks the interval.** `domain_snapshot` reads the worktree; git
commits the INDEX. Forge a pinned section, `git add` it, restore the governed
bytes to the worktree, and an unscoped check answers green over bytes no commit
would record. `--staged` assesses the index whenever it carries anything the
worktree does not, and the exit is worst-of across both intervals, each refusal
naming which one it came from.

**`--commit-gate` picks the question, and implies `--staged`.** Without it the
verb asks *"is everything this corpus's record says true?"* — a claim about the
whole write history, **permanent** once a row breaks. With it the verb asks the
narrower, per-commit one: *were these bytes produced by a governed write?*

Three distinct propositions rode the single exit `1`: a journal chain break
(about the **past**, permanent by design), an out-of-band write in this index
(about **this interval**, per-commit), and `grey(cannot-assess)` (about
**evidence availability**, per-state). A commit fence branches on the code alone,
so past the first break its verdict stopped varying with what was staged — a
guard whose answer no longer depends on the thing it guards carries zero
information about it, and the per-commit enforcement is destroyed with it. The
fix is not a fourth code: it is asking the question whose answer is actually
per-commit. **This is why the emitted fence body runs `mrd check --commit-gate`
and not `mrd check --staged`.**

| | gates the exit | reads |
|---|---|---|
| unscoped | worst-of across every interval assessed | the whole write history |
| `--commit-gate` | ONE interval — the one a commit records | whether the record accounts for it, and whether its pins hold |

**The permanence is untouched.** Unscoped `mrd check` stays red forever, citing
the same row; under `--commit-gate` the standing break is **printed on stderr at
every commit**, pass or refuse. The blocking is downgraded; the telling never is.
And a gated pass over a broken record is never spelled green — it carries the
weaker word `accounted(unvouched-record)`, because the record that accounts for
the interval may itself hold a forged row.

### The composed status line

`mrd status` renders five orthogonal axes on one line, worst-of WITHIN each axis
and never across them:

```
pin green · lock none · anchor at-tip (anchor as-known) · convention off · vibe-debt 0 blobs (0 bytes)
```

| axis | answers | values |
|---|---|---|
| `pin` | the ARMED SET's evidence drift — each armed row's live PAGE rev against the `rev` its armed-rules row attested (PAGE rev uniformly, `armed-plane.md` §4; the pinned-`armed_rev` `CHECK.md` surface is retired) | `green` · `red content-drifted` |
| `lock` | every `meridian-lock` pin's FINGERPRINT verdict, rolled up | `none` · `<color> [N pins]` · `unreadable (<why>)` |
| `anchor` | how current the working copy is against origin's tip, plus the trust of that knowledge | `at-tip` / `behind`, qualified — see the colors amendment § The anchor axis |
| `convention` | whether armed law refuses this change | `off` · `warn` · `block` |
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
| the ARMED plane — an armed rule drifted, or the armed-rules artifact faulted | **yes** ⇒ exit 1 | the `pin` axis and the armed-rules line |
| `lock` | **no** — `red …`, `grey …` and `unreadable (<why>)` all exit 0 | a READING, not a gate |
| `anchor` | no | freshness, and `status` cannot fetch |
| `vibe-debt` | no | a meter, never a gate (above) |

**This is R12, and it is ratified design, not debt.** The lock roll-up is
rendered so a reader sees it; it is not a verdict the shell may branch on. Five
in-repo gates assert it by name — `crates/mrd/tests/u14_check_pin_plane.rs`
("`mrd status`'s exit triad does NOT change — the rollup is a reading, not a
gate"), `crates/mrd/tests/u13_per_root_anchoring.rs` (five arms), and
`crates/mrd/tests/status_e2e.rs` ("debt is not a finding").

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

An answered rung opens the hashed drawer directly. A `cwd-default` tree adopts a
running daemon's registered ancestor if one answers, else degrades to an
ephemeral, per-invocation store that writes nothing — it is never silently
registered.

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
`crates/perfsuite/`. Current tally: **3 PASS, 1 MEASURED, 11
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
