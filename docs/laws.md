---
type: contract
id: laws
status: standing
updated: 2026-08-15
description: The three architecture laws, enforced as crate dependency edges rather than conventions, plus the charter of every crate.
owns: [architecture laws, crate charters]
---

# The three laws

`meridian-rs` is split into crates so that its core invariants are dependency
edges, not conventions. Each law is enforced by what a crate is allowed to
depend on — breaking it is a compile error, not a review comment.

> **Standing:** Design law is `wire-contract.md` (one contract). Mint addresses = segments only. Receipts = armed wire facts. DuckDB/`view_path` not agent core. **Doc correct > code correct; docs first.** See `README.md`.

## Law 1 — the wire cannot leak inward

`model`'s public types carry no `serde` derives. The in-memory world model is
deliberately non-serializable, so no wire shape can reach into the tree and no
serialization concern can shape the model's types. Anything that must cross the
process boundary is converted explicitly at the projection seam (Law 3), never
by deriving `Serialize` on a model type.

## Law 2 — nothing host-facing exists beyond `wire`

`wire` is serde-only and does zero I/O. It is the single crate that defines the
vocabulary the host sees: paths, spans, node revisions, **fingerprints**
(workspace content hash), and the op/request/response/error types. If a type is
not in `wire`, it is not on the wire. The standing contract in
`wire-contract.md` is this crate's intended surface (code may lag; docs win).

## Law 3 — the bridge has two named organs; everyone else is a consumer

(Law 3 restated.)

- **`wire-map` is the projection seam.** The model tree flattens into wire
 shapes as a tested library function — projection behavior lives here and
 nowhere else. M1 added host-face read facts (`facts`: dewey ordinals,
 segment `hpath`, word counts) to this seam. Residual fields that still emit
 **joined / sanitized display strings** are host-facing interop debt being
 killed — **not** mint-plane address law (`wire-contract.md` §2.1: segments
 only ⇒ sanitization never necessary on machine addresses).
- **`wire-serve` is the serve choke-point.** The strict-decode pass, the read
 arms (incl. the composed `read`), the `splice → commit` write choke-point,
 and the standing vocabulary projection are ONE implementation here. "One
 served implementation, one host": the resident `registry` daemon dispatches
 through these arms. (The host shares the LEAVES, not the dispatch shell.
 The former second host — the per-workspace `sidecar` stdio binary — was
 ruled DROP, wire-contract §3.3, 2026-08-06.)

Everything else that names both `wire` and `model` is a host, a client, or a
test member consuming those two organs — never a second place where bridge
behavior may live: `registry` (the host, wiring-only),
`mrd` (a local client), `render` (consumes projection facts to produce the
text face), `check`/`preset`/`realise` (engine planes over wire vocabulary),
`testsuite` (observes). Growth pressure inside a host or client is the signal
a capability is missing from one of the two organs.

A corollary edge: `syntax` is the only crate that touches the pulldown-cmark
fork, so fork churn is a one-crate event.

## Additivity

New capability arrives as new leaf crates or new match arms, never as a
reshuffle of what already ships. New ops are new `Op` variants and new dispatch
arms, discovered by the host through the `hello` handshake's capability list;
`policy` and `query` are additive consumers of the model's index. Nothing that
has shipped is ever split.

## Crate charters

Each crate's `lib.rs` states its charter, what it owns, what it never does, and
which laws it carries. In one line each:

| Crate | Charter |
|---|---|
| `addr` | The agent-plane address: `[root:]path[#selector]` parsed into a fallible type carrying an optional canonical root name, plus the resolution-facing bound-name projection (`MountSet`) every plane resolves through. A `std`-only leaf UPSTREAM of `syntax` — it is where an address becomes a value, so nothing downstream re-splits a string. `Addr::parse` is the sole constructor and the path field carries no root prefix by construction; the colon law is root-wins with **no fallback to the literal reading**, because a fallback turns a typo'd root into a wrong success. Parse is not resolve: whether a named root is BOUND is the resolver's question and its answer is grey, never a parse error |
| `syntax` | Markdown bytes → dialect node list with byte-exact spans; sole owner of the pulldown-cmark fork |
| `model` | The governed node tree, resolve, CAS-splice validation, workspace fingerprints — non-serializable by design (Law 1); the frozen heading predicate (`gotext`), the single address law its two dependents share; and the content-identity plane: the `fp1.…` CID-token, `verify_content`'s four-arm verdict (carrying the whole `version.codec.hashfn` triple on `Unverifiable`, so a render names WHICH member is unknown), and the ONE reason-carrying `Color` model every drift surface computes through; and the frontmatter scalar codec (`scalar`) — the single owner of the § A.6 value law, decode for every read seam and the double-quoted encode every value-plane write door emits through |
| `fs` | Disk read/walk/watch into the model; atomic tmp+fsync+rename splice execution |
| `wire` | The serde-only wire vocabulary — the whole host-visible surface (Law 2) |
| `wire-map` | The named model→wire projection seam, tested as a library function (Law 3) |
| `git` | The git plumbing organ: shell-out content-addressing (blob object ids, the eager `-w` write) and object reachability against a `Repo` handle. git owns content-addressing — this crate asks git and reports what git said, and NEVER computes or guesses an oid. A `std`-only leaf: no production dependency, so `git`-invocation churn is a one-crate event |
| `receipt` | The receipt family, at three planes: the persisted `^receipt` line renderer committed in the same batch as its edit (the shipped default template — facts normative, template replaceable); the origin-freshness anchor axis plus the three-state blob classification (`anchored` / `pending-anchor` / `never-anchored`); and the ephemeral in-memory read-mint ledger. Dependencies are `wire` only, by gate — so it CLASSIFIES facts it never gathers (the `git` crate does the I/O), and stage-3 unifies the ledger's representation with the persisted projection |
| `transport` | Untyped NDJSON envelope + codec seam; framing without meaning |
| `policy` | Ruleset compile + assertion evaluation under budgets; edit-time verdicts; the blocking `gate` at the armed change plane (`policy::gate`) — see § Amendment |
| `query` | Corpus reads over the model's borrowed index; applies nothing |
| `wire-serve` | The shared typed edge (Law 3 choke-point): strict decode, read arms incl. the composed `read`, the `splice → commit` write choke-point, the standing projection — one implementation, one host (wire-contract §3.3). Agent/stored address seam: `put` translates cross-root `root:` into `obsidian://` stored form and `read` translates back, at the candidate document (see `address-grammar.md` §9). Reads `config` for the mount table lazily when a candidate can carry a cross-root position. |
| `render` | The compiled-in render plane: `Renderer` + node-grain walker producing TOON-compact projection through its own encoder (`render::toon`), with block-elision and claim-link decoration hooks. Decorations arrive as data — no `render → lock → fingerprint` edge. |
| `lock` | The `meridian-lock` fenced-block format: canonical writer/reader, engine sole-writer; owns the reserved `meridian-*` block-language namespace. Reads current R4 schema and fails loud on unsupported versions. |
| `effects` | The effect kernel: pure Starlark evaluation — rules in, effect descriptors out; zero I/O, advisory-only |
| `run` | The mrd-local run plane: plan/execute under the workspace run lock. Owns `Authority` (capabilities real for starlark, absent for bash). See `run-plane.md`. |
| `realise` | The realise engine: observe → check → apply per claim, on the run plane |
| `view` | **Ephemeral projection + lock-aware read face** (`wire-contract.md` §10.3–§10.4; not agent core). Projects the parsed corpus into an in-process `:memory:` DuckDB (`build_memory`, the `mrd sql` operator face) and owns the lock-aware read face for walk/status colour. **Writes nothing to disk.** The persistent published-file organ (`view::publish`, `view.duckdb`, the `view_path` wire op) was DROPPED by ruling — §10.4, 2026-08-06. |
| `check` | The check engine: the pure READ verb of the reconciliation loop |
| `preset` | Presets + session birth: def-pinned convention floor; `new`/`unfold`/`reconcile` through the guarded create. Design element: `run-plane.md` (preset section). |
| `config` | The `MERIDIAN.md` plane: the one entry point parsed as CONTENT — the two-rung bootstrap chain (`MERIDIAN_CONFIG`, then `$HOME/MERIDIAN.md`), the four resolution states with absent and zero-mount reaching ONE mount table, the strictest parse in the system (a closed `&'static str` reason set, 1-based FILE lines, a teaching refusal that states nothing loaded), and the config's own rev and fingerprint by the shipped laws — `blake3(bytes)[:16]`, no new rev noun minted. No partial mount table is a property of the TYPE: `Config` has private fields and `parse` is its only constructor. Downstream of `model`. **And the mount table (`mount.rs`), which is where a declared entry BECOMES a bound root:** canonicalize at bind, then the `workspace::deny_reason` ceiling **reused whole and never re-implemented** — so a config cannot bypass the ceiling through a file that is itself ordinary editable content — then the three-way map's uniqueness invariants (name ↔ Obsidian vault name ↔ path, refusing equal-or-nested paths so one tree cannot be bound twice under two names), then each root's own self-declaration: **the root declares, `MERIDIAN.md` binds**, so a declared-vs-bound mismatch fails the whole parse and an absent declaration renders grey. Per-root state is grey-exit-1's closed vocabulary — one `bound`, four `grey(...)`, one `red(...)`, every non-bound state refusing on exit 1 with its own reason word. Mount-as-claim lives here too: a mount may pin the root it declares, verified through `model::fingerprint::verify_content` — no new codec, no new hash law. `MountTable`'s field is private and `bind` is its only constructor, so no partial mount table can exist to be observed. **And the bridge period (`bridge.rs`), which is the "checked against" half of the env-var inversion:** `CCC_LLM_WIKI_PATH` and `CCC_LLM_WIKI_REPOS_ROOT` become mount entries, and until they demote to overrides each is checked against the bound table **through `MountTable::by_path`** — the canonicalize-at-bind law reused whole, never a second comparison — so the symlinked, trailing-slash and real spellings of one tree are one lookup. When they disagree **the FILE WINS** (`Bridged::mount` is `Some` only on agreement, so a diverging variable names no root) and the divergence is **reported once per process, per variable, and never on an exit code** — fail-loud here would brick the CLI on every machine exporting the variable, and a bridge whose mismatch is fatal is not a bridge. An empty table is `unchecked`, not divergent: every unmigrated machine exports both variables, and a check that fires on all of them is deleted before it ever guards anything. **And that projection is now here (U12):** `MountTable::projection` yields the `addr::MountSet` the planes that resolve and translate consume — which names this machine BINDS, the **vault name** each bound vault root carries (the stored plane is spelled in vault names), and which declared names are unreachable here WITH the path to check, so a refusal for a declared-but-unreadable root never prescribes a declaration that already exists. It is not `mrd walk`'s projection and the difference is a FACT, not a second spelling: walk also marks a root unreachable when its CORPUS will not build, which only a caller holding corpora can know |
| `workspace` | Workspace identity: the discovery ladder (env override → git root → cwd default), canonicalization, the deny ceiling — pure filesystem functions (a leaf, `std` + `cache` only). The ladder answers ONE question — *which root does this path belong to* — and every answer names the rung that answered: `Answer::root` is `None` on the cwd default, so a caller cannot inherit an unanchored cwd silently (marker-retirement ruling, 2026-07-26). The two EXPLICIT planes are deliberately NOT rungs here: the mount table (`config::MountTable`) cannot be one without a dependency cycle, since `config` depends on this crate for the ceiling; and a declared root arrives on the serve path as the hello `workspace` field, pinned exactly by `registry::Registry::pin_declared`, because a daemon has no meaningful cwd to walk. All three planes meet at exactly one point: `deny_reason`, reused whole, never re-implemented |
| `cache` | The hashed cache drawer: addressing, atomic sentinel registration, corrupt-is-a-miss probing, last-use GC |
| `registry` | The daemon-held workspace registry: unix-socket RPC server + client, first-writer-wins, atomic state, idle-reap |
| `mrd` | The workspace CLI — wires `workspace`/`cache`/`registry` into `init`/`unregister`/`resolve`/`cache`/`daemon`, and mounts the local run plane (`mrd run` via `crates/run`). A local CLIENT of the engine crates, never a resident organ and never on the serve path; its `run`→`model` edge stays a single reviewable dependency |
| `testsuite` | Integration tests + the frozen ground-truth pack as data |
| `perfsuite` | Perf harness and claims registry (out of default-members) |

**Bulk lock migration is out-of-product:** any bulk v1→v2 lock rewrite is
script-class, not a product door.

## Amendment — the policy gate (armed change plane)

Law: `wire-contract.md` § A.2 (armed plane) and § Refusal taxonomy.

`crates/policy` originally owned advisory edit-time verdicts only — findings the
host could act on or ignore. This amendment extends the charter: `policy` now
also owns the **blocking gate** at the armed change plane.

- **The seam.** `gate(change, law) → GateOutcome`
 (`Ok(verdicts) | Refusal(violations)`) is `policy::gate`
 (`crates/policy/src/gate.rs:108`), the blocking form of the advisory
 `evaluate_verdicts` seam — evaluated after CAS, before bytes land, in both
 writer paths.
 When a workspace is armed, a block-severity verdict or a door-law violation
 refuses the write; the refusal carries a `{code, recovery}` pair from the
 closed §8 taxonomy (`wire-contract.md` §8 and § A.2).
- **Trusted-path armed set.** The armed law is loaded and verified from the
 workspace path inside the trusted write path (`resolve_armed_law`,
 `crates/policy/src/armed_law.rs:257`, fed by the write path's own disk seam);
 the caller-supplied ruleset parameter is removed from the gating decision. Absent INDEX on a
 never-armed workspace is a no-op bit-for-bit; a missing INDEX on an
 once-armed workspace fails CLOSED (`convention-fault`).
- **Additivity holds (Law § Additivity).** `policy` is still an additive
 consumer of the model's index; the gate is a new match arm at the write seam,
 not a reshuffle of what ships. `model`, `wire`, and the projection seam are
 untouched — the engine still never derives `Serialize` on a model type (Law 1),
 and the gate mints wire refusals only through `wire`'s error types (Law 2).

**ATTACK-034 scoping.** Refusal makes violations "unrepresentable through an
armed change plane" — never a stronger claim. The genesis epoch (pre-first-arming
writes) renders grey, never green. The gate governs only the armed change plane:
out-of-band mutation (an offline pre-push git rewrite, a root-preserving forged
journal row) is caught by the git witness plus the receipt-engine-only write
restriction, or it is a named residual — it is never rendered green by refusal.

## Named residues and candidate rows

A **named residue** is a construction this engine's own law disapproves, whose
BEHAVIOUR is correct today, deliberately left in place with its reason recorded.
A **candidate row** is a change nobody has ordered yet, named so a future docket
inherits it as a decision rather than rediscovering it as a defect.

Both exist for one reason: an undocumented compromise becomes the architecture
by forgetting. Neither is a TODO — a row here has been ruled, and the ruling is
that it waits.

| # | Row | Kind | Status |
|---|---|---|---|
| R1.6-a | The stored→agent re-join/re-parse in `wire-serve::positions` | residue | recorded by U21, deferred |
| C-1 | The link plane resolves cross-vault refs IN-PROCESS, not in the daemon | residue | U21's degrade — **successor named below** |
| H-1 | The `#` refusal on a heading whose raw text carries `#` | candidate | **owed by U14** — see below |
| S-1 | The stored-plane narrowing refusal (U21 Q1a) | candidate | **owed by U14** — see below |
| D-1 | The joined `--section` coat splits on `/`, so a heading whose raw text carries `/` is not addressable by that one spelling | residue | **ruled by ZT — widening the coat is C2, and C2 stays reserved** — see below |
| G-1 | The §2.4 block-id charset is enforced at the structured ingress only, so an unmintable `^id` MISSES at the read and walk doors instead of refusing | candidate | **face decision proposed, advisor ratifies** — see below |

### S-1 — the stored-plane narrowing refusal, and the trigger that makes it owed

**Operational trigger, so this is a tracked obligation and not a hope: the
obligation fires when BOTH `ReadSel` AND this row are on `main`. WHICHEVER LANDS
SECOND CARRIES THE CHECK.**

> [!NOTE] Why the trigger is stated as a conjunction rather than as one merge gate
> It first read *"when `ReadSel` lands on `main` … the U14-merge gate checks this
> row."* That names a gate that **may not exist at the moment it is supposed to
> fire.** Measured 2026-08-04 at `origin/u21-cross-vault-links` `c23810d3`: this
> row is on THIS branch and nowhere else — `S-1` returns 0 hits at `origin/main`
> and 0 at `origin/u14-arrays`, with `Law 1` returning 3 at all three revisions
> as the positive control that the query reached the file. So if U14 lands first,
> its merge gate reads a `laws.md` that does not contain this row, the check
> passes vacuously, and the obligation then sits on `main` **true and
> unwatched**.
>
> A conjunction has no such ordering hole: neither branch can land second without
> both being present, so the second merge always has both the trigger and the row
> in front of it. **This is an obligation with a trigger that cannot be relied on
> to fire — the same defect as an obligation with no trigger, one level in**, and
> it is the defect this row was written to prevent.
>
> **Invalidation condition:** this wording stops being necessary only when both
> `ReadSel` and this row are on `main`, at which point the obligation has fired
> and the trigger is spent. It is unaffected by either branch moving, by further
> commits to this file, or by the order the two merges are eventually planned in.

U21 Q1 was ruled (a) — refuse at the translation door with a named
`TranslateError` — on the stated premise that *today's wikilink ingress cannot
mint the affected values*. U21 measured that premise and **it does not hold for
one of the three**, so the ruling was re-taken as (i): the refusal lands with
U14.

The three values, each with why it waits:

- **Multi-segment hpath — LAW on the mint plane; joined form is residual debt.**
 Machine address is segment objects only:
 `{"hpath":[{"h":"Design"},{"h":"Sub"}]}` (`wire-contract.md` §2.1; multi-segment
 is reachable and normative in segment form). What S-1 still tracks is the
 **stored/wikilink ingress** that can mint a `/`-bearing *opaque string*
 (`[[sessions:notes.md#Design/Sub]]` via `syntax::split_wikilink_target`,
 round-tripped as one string on both planes before full segmented lock
 storage). That joined string is **stored-plane debt being
 killed**, not a second writeable address grammar. The ambiguity
 `Design/Sub` would be ambiguous *against* — one segment or three — is why
 U14's segmented hpath + stored-plane narrowing refusal exist; the refusal
 is additive against the joined residual, never against segment-form law.
- **Dewey** — there is no dewey spelling in the agent-plane address grammar on
 `main`. `[[x.md#1.2]]` is a heading literally named `1.2`, and `heading=1.2`
 stores it correctly.
- **Occurrence index** — no spelling in `Addr` at all.

**The last two are not merely unreachable, they are UN-IMPLEMENTABLE, and that
is why no variant was landed for them.** There is no value at the translation
seam to detect, so the refusal would be a variant with no constructor —
re-derive-or-strike's weakened middle, a claim nothing checks. **Do not land dead variants
for symmetry, and do not let a later reader land them for tidiness.**

### Q7 — why the **optional view organ**'s cross-root destination is THREE columns, not two

*(View organ / SQL board only — not agent core; winner §10.3–§10.4. Core path never assumes this schema.)*

U21 Q5 ruled a nullable `dest_root` **beside** `dest_path`. Implementation
measured the fact the ruling was written without: `link.dest_path` carries an
**enforced** foreign key into `doc(path)` — the organ's DuckDB schema answers
*"Violates foreign key constraint because key `path: notes.md` does not exist
in the referenced table"* — and a cross-root path is not a key in this corpus.
The literal two-column shape therefore required DROPPING that FK.

Ruled (B), FK preserved: `dest_root` + `dest_root_path`, with `dest_path` left
NULL for a cross-root edge. The FK is the only thing in the schema that makes *a
link row pointing at a document that does not exist* unrepresentable, and
trading it away inside the unit about the link plane answering with the wrong
document is the wrong direction of travel. It also keeps the column honest at
its grain: **`dest_path` means "a path in THIS corpus" always, rather than only
sometimes.**

The third column widens the error space, so the illegal states are closed
STRUCTURALLY — `CHECK ((dest_root IS NULL) = (dest_root_path IS NULL))` and
`CHECK (dest_path IS NULL OR dest_root IS NULL)` — rather than by the
projector's discipline. `dangling`'s two destination clauses (`dest_path IS
NULL AND dest_root IS NULL`) are what stops a resolved cross-vault link reading
as broken, pinned by a red test, mutation-proved one-edit, in
`crates/view/tests/u21_cross_root_link_rows.rs`; the third clause (`AND
exclusion IS NULL`, ruling 2026-08-14) is what stops a deliberately-unhashed
target reading as broken, pinned the same way in
`crates/view/tests/dangling_exclusion.rs`.

### R1.6-a — the stored→agent re-join, and why it stays

`stored_occupants` (`crates/wire-serve/src/positions.rs:423`) decodes a stored
URI into its parts, then **re-joins them into one string and re-parses it**:

```rust
// crates/wire-serve/src/positions.rs:451-457
let address = match &parsed.selector {
 Some(sel) => format!("{name}:{}#{sel}", parsed.path),
 None => format!("{name}:{}", parsed.path),
};
let occupant = Occupant {
 addr: Addr::parse(&address)…
```

The parts were already separated; they are joined only to be split again. That
is a joined string address on a machine surface, which **ZT decision 14 / R1.6
disapproves** — *"Arrays for machines, TOON for humans. No string address forms
in machine surfaces."*

**It is not producing a wrong answer.** The join and the split agree, and the
round trip is asserted byte-identically
(`positions.rs::tests::the_agent_plane_form_round_trips_byte_identically`).
**Only the CONSTRUCTION is disapproved, not the behaviour** — and that is what
separates this row from the `PinSpec.selector` case U14 settled, where a refusal
had been lifted while the capability was still missing: a half-delivered
capability blocking an ordered proof had to be finished, and this does not.

**Why it waits.** The fix requires `Addr` to gain a parts constructor, and
`Addr` has **no `from_parts` by deliberate invariant**:

> *"there is no `Addr::from_parts` a caller can use to smuggle an unparsed
> prefix into the `path` field"* — `address-grammar.md` §2.2

That invariant is what makes every downstream guard checkable. Redesigning it is
its own considered act with its own gate, not a rider slipped into another
unit's train. **The successor act is: give `Addr` a fallible parts constructor
running the same checks `parse` runs, then delete the join.**

### C-1 — the link plane's in-process degrade, and its NAMED SUCCESSOR

The daemon's warm state is one workspace's corpus, keyed by that workspace's own
canonical path and invalidated by its own fingerprint
(`crates/registry/src/registry.rs:317-321`, `warm_or_build`). **It holds no
mounted-root corpora**, so the link arm serves ambient state only
(`crates/registry/src/server.rs:1121-1135`, the `Op::Links` arm on the
daemon's one-workspace warm engine). U21 therefore resolves a cross-vault
link by DEGRADING that one op to in-process, where the mounted corpora can be
loaded the way the walk plane already loads them
(`crates/mrd/src/walk_cmd.rs:146`).

**The asymmetry is a documented contract, not a bug awaiting discovery:** for
this one op the daemon is knowingly less capable than the in-process path, and a
page carrying a cross-vault link pays a cold corpus build. Stated here rather
than smoothed, on the same discipline as the exit-code asymmetry in
`address-grammar.md`.

> **THE NAMED SUCCESSOR — option (A): the daemon holds mounted corpora.** That
> is the correct end state and it was deferred DELIBERATELY, not overlooked. It
> needs per-root fingerprint invalidation, residency and reap — a designed
> subsystem, which this docket handles design-first with its own element and
> gate (P8/P10), exactly as U20a exists for the push channel. Building it as an
> implementation detail of a link-plane fix would repeat the
> F3-as-a-port mistake.

**A degrade with a named successor is a decision; a degrade without one becomes
the architecture by forgetting.** That is why this row exists.

### H-1 — owed by U14, recorded here so it is not lost

U14 found that the `#` refusal must SURVIVE, because `#` is a live delimiter in
both wikilink and `path#fragment` ingress, and named it a candidate row rather
than an act.

**This row is a POINTER, not a claim about this tree.** The row's owner is U14,
which HAS landed — the detail it filled in is `refuse_unrepresentable_heading`,
named in the correction below. (This sentence read *"U14 fills in the detail
when it lands"* — a sunset the next paragraph already reports as fired.
Corrected under 

> **Corrected in the landing assembly:** the sentence that stood here said U14
> was not merged and `lock_ref_fragment` was still the name. That was true when
> written on `u21-cross-vault-links`; it is false in this tree. U14 IS merged and
> `refuse_unrepresentable_heading` exists. The careful half — that the row is a
> pointer rather than a claim — is what survives, and it is why the parenthetical
> was the only false part. Writing its
shapes here now would assert a tree that this tree contradicts, which is the
defect this whole section exists to prevent.

### D-1 — the `/`-coat limitation, and why C2 stays reserved

U14 landed the `/`-heading ruling in BOTH halves, and only one half reached a
doc. The machine half: a heading whose raw text carries `/` is representable
and pinnable as ONE segment of an hpath array (`{"hpath":[{"h":"Guide"},
{"h":"A/B"}]}`) — gated by
`crates/wire-serve/tests/s7_pin.rs::a_slash_bearing_heading_pins_end_to_end_and_stores_as_one_array_element`.
The coat half: `ReadSel::parse` (`crates/wire/src/lib.rs`, the ONE
human-string ingress door) splits its string on `/`, so the joined spelling
cannot address that heading — it yields a well-formed address resolving to
nothing, and the door MISSES rather than silently serving a different section.
Gated as a characterization test:
`crates/wire-serve/tests/s7_pin.rs::the_cli_string_coat_still_cannot_address_a_slash_bearing_heading`.

**D-1 and G-1 are two properties of ONE function.** `ReadSel::parse` is
infallible by signature (`pub fn parse(s: &str) -> Self`): it splits the
heading arm on `/` (this row) and takes an `^id` verbatim with no charset test
(G-1). A ratification that makes that signature fallible moves both rows at
once, and the D-1 characterization test above is where it shows.

**The coat is not widened.** Widening it means an escape grammar over a flat
selector, which is C2, and C2 stays reserved on ZT's own words (2026-08-01,
session `86449b4e`): *"using string as selector is not ideal … use C2 if we
really needed it"* and *"we never have to do sanitization. the put path is an
array, no ambiguity."* This row exists because that reservation lived only in
session archives until 2026-08-09, when live dogfood rediscovered it as an
unknown bug.

**A miss, not a refusal — and the line that makes this row and its neighbours
one law.** Agreed 2026-08-09 between the two seats carrying the delimiter rows,
stated once here and cited from the others rather than restated:

> **Refuse what can never exist; miss what exists but this door cannot spell —
> and the taught recovery must be the one that actually repairs it.**

The test is what a corpus edit could do. An input naming something **no corpus
could ever carry** is outside the minting grammar, so it REFUSES `bad_request`
— "look again" is a recovery that loops forever (§2.4's `_`-bearing block ids).
A `/`-bearing heading is the other case: the corpus carries it, the machine
plane pins it, and only this ingress cannot spell it — so the door MISSES, and
the miss owes the caller the spellings that DO reach it.

**The scoping is PER DELIMITER, PER INGRESS.** A blanket "live delimiters of
the joined spelling" claim would be wrong — each door has its own boundary:

| Ingress | `/` | `#` |
|---|---|---|
| CLI joined `--section SEL`, `mrd pin`'s selector (`ReadSel::parse`) | **delimiter** — splits; a `/`-bearing heading is unreachable by this spelling | **heading TEXT** — `--section 'Top/C#D'` serves |
| CLI `PATH#FRAG` (the frag door) | inherited from `ReadSel::parse` — same split | splits on the FIRST `#` only; the tail is selector bytes, so `notes.md#Top/C#D` serves |
| wire / MCP segment arrays (`{"hpath":[…]}`) | heading TEXT | heading TEXT |
| wikilink / `path#fragment` heading refusal | — | **H-1's column, untouched by this row** |

**The two escapes the face must teach.** A heading the joined coat cannot
spell is still addressable two ways, and both come off the row the toc already
published: its **dewey ordinal** (`--section 1.2`) and its **raw heading
segments** as an hpath array (one entry per heading, no joining). So the
refusal owes the caller those two forms — pointing back at the toc read alone
hands back the same un-feedable title and the recovery loops (dogfood finding
#1, reproduced on v1.0.0). The one teaching site is
`wire_serve::section_recovery`; the in-tree precedent it follows is the
duplicate-heading refusal, which already teaches machine address + dewey.

> **Amendment (dogfood r7 F1, card script-slash-heading-addressing):** the
> script plane used to be the one surface that RECEIVED this teaching (the
> commit leg carries the engine's refusal verbatim, U3) while rejecting both
> taught forms — `section=` was `str`-only, so the hpath array met a type
> error and the circle cost a caller several calls to disbelieve. `section=`
> on the script `put()` and `read()` builtins now takes the §2.1 segment
> array (run-plane.md § the arming surface), and the script toc face
> publishes each heading row's raw segments as `hpath`, so the taught
> recovery is executable on every plane that prints it. The coat itself is
> untouched — this row and C2's reservation stand.

### G-1 — the §2.4 charset is enforced at one ingress of two

**This row is a DIVERGENCE, named as a candidate because the fix is a face
decision nobody has ratified yet — not because the behaviour is defensible.**
D-1's neighbour in the table and its opposite in verdict: read the refuse/miss
line stated at D-1 first, because it is what makes these two rows one law
instead of two moods.

wire-contract §2.4 rules ONE block-id charset, `[A-Za-z0-9-]+`, on BOTH planes,
and states that a `_`-bearing anchor is outside the strict-plane grammar
(`bad_request`). §4.5 and GOAL 2 say the same thing twice more for the walk
plane ("refuses loudly"). Measured on v1.0.0 (`9318479730bf`), three doors
answer the one law three ways:

| Door | `_`-bearing id | Recovery taught |
|---|---|---|
| write (`put`, structured) | `bad_request`, charset named, §2.4 cited | **fix** — the ruled shape |
| read (composed / `--section`) | `no_match` + nearest list | re-read |
| walk (`resolve`) | `ref_not_found{stage:2}` | refresh |

**Why this is a divergence and not a taste.** `no_match` and `ref_not_found`
both teach *the thing you named is not there right now*. For an id §2.4
forbids minting anywhere, that sentence is false in a way no future corpus can
make true, so the taught recovery is a circuit with no exit — an agent that
typos `_` into an id is told "it dangles" instead of "it can never exist", and
the unrepresentable/merely-absent distinction is unobservable in any
transcript. Only `bad_request`/fix terminates.

**The doors do not disagree about the law — one ingress carries the
decode-time charset guard and the other does not.** (Not the *mint-guard*:
§2.4 assigns that named artifact to the phase-2 impl-taskpack, §13.8, and it
governs MINTING going forward. What G-1 measures is refusal at decode when
ADDRESSING an id that already exists out of grammar — which §2.4 rules
present-tense and defers nowhere.) The structured ingress refuses at decode
(`wire-serve::decode::decode_anchor`, `wire-serve::read::to_model_ref`). The
human-string ingress does not: `wire::ReadSel::parse` is infallible by
signature and takes `^id` verbatim with no charset test, so every door
inheriting it — CLI `--section`, the `PATH#FRAG` frag door, `mrd pin` — carries
an out-of-grammar id past decode into resolution, where it can only land as a
miss. The walk leg is the same omission in its own parser
(`model::walk::parse_linktext`), whose `Miss{stage,dest}` has no arm that could
carry a grammar refusal even if it wanted one. **This is the same function D-1
describes splitting on `/`: the two rows are two properties of one door.**

§18 already leans on the ruled behaviour — the former walk-plane charset row
was *dissolved* on the reasoning that a `_`-bearing anchor refusing loudly is
conforming. That dissolution's premise is currently unmet in code.

**The proposed face decision, for advisor ratification** (the full argument,
the measured fragments, and the blast-radius measurement are this row's own
body above and below): the §2.4 boundary is a DECODE-TIME boundary
enforced at every ingress before any lookup, so an out-of-grammar id never
becomes a selector and can never surface as a miss; the other two doors adopt
the write door's existing refusal string verbatim, so one law gets one
sentence; the guard sits at the `resolve` op boundary, never inside `walk()`,
which stays pure best-effort app-parity as §4.5 requires; and the refusal never
becomes an `unresolved` row, because that vocabulary is a *resolution*
vocabulary and a grammar arm inside it would re-create the conflation this row
exists to remove.

**Nothing moves in code under this row.** It is a candidate: the divergence is
recorded, the fix is proposed, and the ruling is that it waits.

## Amendment — capabilities do not apply to bash

Law: ZT ruling, made verbally, re-litigated in code, and ruled again live
2026-08-01. **Gate: `crates/mrd/tests/law_no_caps_on_bash.rs`** — that file is
what makes this hold, and this section is what it enforces. A reader who
proposes "just a small cap check on bash" must answer both.

> **Capabilities do not apply to `bash` tasks. Not now, not later, not in a
> weaker form.**
>
> 1. A bash task carries **no `caps:` line**, no cap resolution, no cap source,
> and no `deny-default`.
> 2. The engine **never prints a claim about what a bash task may do** — most of
> all not `(read-only)`.
> 3. Bash is **unsandboxed by definition.** The only honest description on any
> surface is: *unsandboxed shell, undeclared effects.*
> 4. Capabilities remain a real, enforceable contract for **starlark**, and only
> starlark.

**The guarantee is impossible, not merely difficult.** A capability claim is a
promise about what a process CANNOT do, and for bash the engine holds no
mechanism that makes one:

| layer | what exists | why it does not bound the process |
|---|---|---|
| in-window writes | the `out-of-band delta` detector | **detects, never prevents** — `run-plane.md` scopes it so, and the offending file persists |
| after the window | nothing | the detector's own wording is *"during exec window"*; a `nohup`, launchd plist, cron line or daemon writes with no observer |
| outside the corpus | nothing | env scrubbing does not restrict network, credentials, SSH, or `rm -rf` — none of it is an "effect"; and since U16 there is no cwd isolation at all, the step runs where `mrd` runs |

A guard escapable by `nohup` is not a guard, so **no honest value exists for a
bash `caps:` field — including `none`, including `(read-only)`.** What the
resolution ladder bought on that path was complexity with no guarantee behind
it, plus misleading-by-adjacency: `caps:` is TRUE on the starlark row above and
importing its enforcement model onto the row below asserted, in the engine's own
voice, a conclusion the engine cannot support.

**Structural, not cosmetic.** `run::caps::Authority` has two variants —
`Capabilities(CapResolution)` and `Unsandboxed` — and it is what the executor's
choke point validates against. The bash dispatcher holds no capability field at
all (`dispatch_bash::BASH_AUTHORITY`), so it cannot name, narrow, or
half-enforce one; `resolve_authority` is the only language-aware entry and does
not even READ a bash task's `task.<name>.caps` declaration, because validating a
value that governs nothing teaches that it might. Deleting the printing while
leaving resolution running underneath fails the gate's second half by
construction.

**What this does NOT weaken.** Starlark keeps the whole contract: hermetic
evaluator, closed builtin surface, no `exec`/`os`/`subprocess`, every effect a
descriptor the applier gates. Both refusal shapes are asserted in the same gate
file, so the bash half cannot be bought by weakening the starlark half:
`md.*` without its cap → `capability denied`, exit 1; `proto.*` without its cap
→ `state: unexecuted-no-capability`, exit 0. The `check-*` / `verify-*` bash
fence refusal also survives — that is a NAME law, not a capability.

**Behaviour changed, not just wording.** Bash reaches the tree through the
effect-shim fd and those descriptors were gated at the choke point, so an
undeclared bash block used to exit 1 with `capability denied` and now applies.
That gate never bounded the block: a denied block writes with `sed -i` instead,
where the bracket at most detects the change and never rolls it back. It only
pushed the write off the attested path.

## Amendment — the face-honesty law

Law: architect ruling 2026-08-10 (`4f144f09`), on leader-engine `8697ff5e`'s
routed dogfood case — worker `8cb84386` against the published binary at
`27cf2bca`. **Gate: `crates/mrd/tests/law_face_honesty.rs`** — that file is what
makes this hold, and this section is what it enforces.

> **Every face states the bound of its own answer.**
>
> 1. **A subset answer is MARKED.** A face that filters states the count
> withheld, the criterion, and the pointer to the full face. Enumeration stays
> machine-side — flooding the human face is the walk-payload failure from the
> other direction.
> 2. **A limit that can refuse is DISCOVERABLE BEFORE IT REFUSES** — stated in
> the verb's help, never learnable only by tripping it. A raise flag is *not*
> ruled in: raisability is cost policy, not discoverability.
> 3. **A refusal carries its RECOVERY** at the human face, as the wire already
> rules it on frames: point at the verb that answers the question the caller was
> evidently asking, when one clearly exists — **otherwise say nothing, because a
> wrong pointer is worse than none.**
> 4. **Engine-owned files are COUNTED AND LABELED**, never silently either way.

**The defect that produced it, in the measured form.** `mrd links` printed 6
lines naming 2 files while the corpus held 112, and said nothing about the 110
it withheld: *a person stops there and concludes the corpus holds 2 files.* The
information existed and only `--json` revealed it. This is the third instance of
one shape inside one hour across three layers — a passive meter read as measured
headroom, an uncredentialed 404 read as a dead API — and the shape is always the
same: **a face answers a different question than it appears to, and it fails
toward ABSENCE.** Absence is the dangerous direction because a filtered answer
and an empty world render identically.

**Why marking, not enumerating (clause 1).** The opposite failure is already
named in this repo: a payload that floods the reader carries the same amount of
truth and less of it arrives. So the human face states the bound and hands over
a pointer; the machine face carries the rows. `links --json` already enumerated
every file — clause 1 costs one line, not a new query path.

**Why COUNT AND LABEL, never exclude (clause 4).** `mrd init` writes
`MERIDIAN.md` (`config::CONFIG_FILENAME`) into the corpus it declares, so anyone
who enumerates a workspace and counts gets the engine in their denominator.
EXCLUDE was refused because a hidden filter is the exact disease this law
kills; COUNT-without-label was refused because it lets the engine pollute the
content count. The ruled form keeps both readings available and neither silent:
*"4 files: 3 content + 1 engine-owned"*.

**Clause 2 has a machine half this engine cannot yet serve, and that is named,
not hidden.** "Readable by the machine face" means the `ack_bounds` grammar — a
core-declared number a client reads — which belongs to **wire contract v1.2
(PR #11, head `ef7895d`) and is NOT merged at `27cf2bca`**: `ack_bounds`,
`protocol.limits`, and `"limits"` all return zero hits across this tree, with
`exceeded the read budget` and `walk root not in the corpus` found by the same
tooling in the same scope as the positive control beside that zero. So the help
half lands here and **the machine half is an owed wire-lane dependency**,
recorded so the next lane inherits a pointer rather than a gap.

**What this law does NOT authorize.** It does not authorize raising a budget
(cost policy, refused), it does not authorize a face inventing a recovery
pointer it is not sure of (clause 3's second half is as binding as its first),
and it does not authorize the human face growing an enumeration. **The `script`
budget refusal is the law's positive example and does not change** — *"exceeded
the read budget of 64 reads per attempt — refused, never truncated"* names the
number, names the units, and gives absence exactly one meaning.

## Amendment — no hard-coded flow (mechanism in code, semantics in markdown)

Law: ZT ruling 2026-08-15, made while answering a status-enum question — the
principle superseded the question. Verbatim:

> *"I want to make sure that in our code there is no hard-coded folder name,
> because the tool we designed will be and can be used by anyone, any user.
> Different user has different, like, reasoning and understanding about what is
> Kanban flow, what do they want. Some of them are not even engineers. So
> hard-code any concrete concept will be kind of like a waste, and like waste of
> our design's elegancy. So the flexibility will comes with the Meriden's hooks
> features, which anyone can design the Markdown file to describe what they want
> and what is the rule of it."*

**Mechanism in code, semantics in markdown.** Engine code carries the evaluator;
the user's markdown carries every concrete flow concept. The law has two halves
and both bind:

1. **No baked folder names.** No engine path may decide where a user's content
   lives, and no folder name may act as a validity predicate on user markdown.
   A directory a user is expected to author into is a value read from their
   markdown, defaulted in code at most.
2. **No baked flow vocabulary.** No status word, no state-key name, no card or
   kanban concept, no role or lane name may appear in an engine decision — not in
   a comparison, not in a refusal string, and not in bytes the engine writes into
   the user's tree.

**The user-markdown home.** Flow semantics live in user-authored pages — rule and
hook pages on the policy/effects planes, and the frontmatter of the page a verb
is invoked on. Those pages name the folders, the states, the key that spells
state, and the prose; the engine reads them as data. This is the same
replaceability the wire already rules for consumers (`wire-contract.md` §1.1,
"zero consumer concepts", and §11: *pack data behind a generic manifest; no
evaluator hard-coded*) — this amendment states it for the whole engine, not for
one plane.

**What "generic evaluation" means.** The engine matches on **structure** —
a frontmatter key exists, a selector resolves, a pin verifies, a rule fires — and
never on a flow literal. Concreteness enters only as a value the user supplied
and the engine echoes back unread. The shipped model is
`preset::DEFAULT_ROOT_RECORD`: the constant is a fallback, `fm_scalar(&doc,
"root")` is the answer, and a user who spells their root differently is served.
Every flow-touching site should read like that one.

**The boundary — engine-intrinsic vocabulary is mechanism, and is not covered.**
The engine's own state and mount convention (`MERIDIAN.md`, `.meridian/`,
receipt paths, the daemon's socket and state files) and the engine's own verdict
vocabularies (`realise`'s `converged` / `drifted-fixed` / `non-convergent`,
the rules registry's `collision`) are the engine speaking about itself, the
equivalent of `.git`. The test that separates the two: **what the engine writes
into the user's tree, or reads as the user's law, is semantics; what the engine
keeps for itself is mechanism.** Tests and fixtures may use concrete flow words
freely — a fixture is an example, not a decision.

**Status: docs-first, gate owed.** No test enforces this amendment yet, and code
at `073d184f1` violates it in three named places, recorded here so they are
inherited as decisions rather than rediscovered as defects: `realise`'s
`render_card` (`crates/realise/src/lib.rs:610`) writes a card whose type word,
state key, state value and prose are all baked, so a user's own rules cannot
match the page the engine minted for them; `realise`'s board directory is generic
in the library (`RealiseSpec::board_dir`) but unreachable from the CLI, which
pins `"board"` (`crates/mrd/src/realise_cmd.rs:42`). Each is owed a fix that moves
the concreteness into the user's markdown.

**Fixed — `preset`'s floor prefix (2026-08-15).** The third named place is
closed: `FLOOR_PREFIX = "conventions/"` was a folder name acting as a validity
predicate on a user's preset, and is now the fallback behind the def's own
`floor:` key (`run-plane.md` § 6, Law 6.3). `pins_floor` reads
`PresetDef::floor_prefix`, so a def filing its convention suite under
`standards/` is as valid as one under `conventions/`. Gated by
`crates/preset/tests/gates.rs`.
