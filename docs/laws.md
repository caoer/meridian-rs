# The three laws

`meridian-rs` is split into crates so that its core invariants are dependency
edges, not conventions. Each law is enforced by what a crate is allowed to
depend on — breaking it is a compile error, not a review comment.

## Law 1 — the wire cannot leak inward

`model`'s public types carry no `serde` derives. The in-memory world model is
deliberately non-serializable, so no wire shape can reach into the tree and no
serialization concern can shape the model's types. Anything that must cross the
process boundary is converted explicitly at the projection seam (Law 3), never
by deriving `Serialize` on a model type.

## Law 2 — nothing Go-facing exists beyond `wire`

`wire` is serde-only and does zero I/O. It is the single crate that defines the
vocabulary the host sees: paths, spans, node revisions, roots, and the
op/request/response/error types. If a type is not in `wire`, it is not on the
wire. The frozen contract in `wire-contract-v2.md` is exactly this crate's
surface.

## Law 3 — the bridge has two named organs; everyone else is a consumer

(As re-attested 2026-07-24. The original wording — "exactly two components
depend on both `wire` and `model`" — stopped being true when the typed edge
was extracted; ten crates now carry both deps. The law's INTENT is unchanged:
the bridge stays auditable because bridge *behavior* lives in exactly two
named places.)

- **`wire-map` is the projection seam.** The model tree flattens into wire
  shapes as a tested library function — projection behavior lives here and
  nowhere else. M1 added the host-face read facts (`facts`: dewey ordinals,
  sanitized hpath addresses, Go-exact word counts) to this seam.
- **`wire-serve` is the serve choke-point.** The strict-decode pass, the read
  arms (incl. the composed `read`), the `splice → commit` write choke-point,
  and the v3 vocabulary projection are ONE implementation here. "One served
  implementation, two hosts": the per-workspace `sidecar` and the resident
  `registry` daemon both dispatch through these arms. (Aspirational note: the
  hosts share the LEAVES, not the dispatch shell — sidecar drives
  `arms::dispatch` per request over a fresh corpus; the daemon drives
  `dispatch_read` over a resident warm engine. The shells stay host-owned.)

Everything else that names both `wire` and `model` is a host, a client, or a
test member consuming those two organs — never a second place where bridge
behavior may live: `sidecar` and `registry` (the two hosts, wiring-only),
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
| `addr` | The agent-plane address: `[root:]path[#selector][@fp]` parsed into a fallible type carrying an optional canonical root name, plus the resolution-facing bound-name projection (`MountSet`) every plane resolves through. A `std`-only leaf UPSTREAM of `syntax` — it is where an address becomes a value, so nothing downstream re-splits a string. `Addr::parse` is the sole constructor and the path field carries no root prefix by construction; the colon law is root-wins with **no fallback to the literal reading**, because a fallback turns a typo'd root into a wrong success. Parse is not resolve: whether a named root is BOUND is the resolver's question and its answer is grey, never a parse error |
| `syntax` | Markdown bytes → dialect node list with byte-exact spans; sole owner of the pulldown-cmark fork |
| `model` | The governed node tree, resolve, CAS-splice validation, Merkle roots — non-serializable by design (Law 1); the frozen Go-text heading predicate (`gotext`), the single address law its two dependents share; and the content-identity plane: the `fp1.…` CID-token, `verify_content`'s four-arm verdict (carrying the whole `version.codec.hashfn` triple on `Unverifiable`, so a render names WHICH member is unknown), and the ONE reason-carrying `Color` model every drift surface computes through |
| `fs` | Disk read/walk/watch into the model; atomic tmp+fsync+rename splice execution |
| `wire` | The serde-only wire vocabulary — the whole Go-visible surface (Law 2) |
| `wire-map` | The named model→wire projection seam, tested as a library function (Law 3) |
| `git` | The git plumbing organ: shell-out content-addressing (blob object ids, the eager `-w` write) and object reachability against a `Repo` handle. git owns content-addressing — this crate asks git and reports what git said, and NEVER computes or guesses an oid. A `std`-only leaf: no production dependency, so `git`-invocation churn is a one-crate event |
| `receipt` | The receipt family, at three planes: the persisted `^receipt` line renderer committed in the same batch as its edit (the shipped default template — facts normative, template replaceable); the append-only journal and its chain-continuity forgery detector; the origin-freshness anchor axis plus the three-state blob classification (`anchored` / `pending-anchor` / `never-anchored`); and the ephemeral in-memory read-mint ledger. Dependencies are `wire` only, by gate — so it CLASSIFIES facts it never gathers (the `git` crate does the I/O), and stage-3 unifies the ledger's representation with the persisted projection |
| `transport` | Untyped NDJSON envelope + codec seam; framing without meaning |
| `policy` | Ruleset compile + assertion evaluation under budgets; edit-time verdicts; the blocking `gate()` at the armed change plane (`policy::authorize`) — see § Amendment |
| `query` | Corpus reads over the model's borrowed index; applies nothing |
| `wire-serve` | The shared typed edge (Law 3 choke-point): strict decode, read arms incl. the composed `read`, the `splice → commit` write choke-point, the v3 projection — one implementation, two hosts. **And the agent/stored SEAM for an address (U12):** `put` translates a cross-root `root:` address into its `obsidian://` stored form and `read` translates it back, at the CANDIDATE DOCUMENT and never at a verb (D9) — the transform rewrites the payload that introduced the address (a retained one is not this write's to move), and one door-facing artifact guard closes every byte-landing door in the crate. The positional law is `docs/address-grammar.md` §9: the wikilink target and the markdown link URL translate; the lock's address key keeps the canonical `root:` form by ratified law (A-4; spelled `object:` since R4 — the retired `objects:` table was §9.4 position P6), and frontmatter is not an address position. It reads `config` for the mount table — LAZILY, only when a candidate can carry a cross-root position, because the stored plane is spelled in Obsidian VAULT names and the mount table is that axis's single authority |
| `sidecar` | The per-workspace NDJSON host binary — wiring only; dispatches through `wire-serve` (Law 3) |
| `render` | The compiled-in render plane: `Renderer` trait + node-grain walker producing the `readText` text projection, with the block-elision and claim-link decoration hooks. Decorations arrive as DATA — a `decorations` map on the render header, keyed by the link addresses the body literally contains — so the caller resolves and this crate never grows a `render → lock → fingerprint` edge; an empty map is byte-identity with the undecorated render |
| `lock` | The `meridian-lock` fenced-block format: canonical writer/reader, engine sole-writer; owns the reserved `meridian-*` block-language namespace predicate. **Reads R4 schema v2 ONLY and fails loud on v1** (`UnsupportedVersion`, naming the file) — the v1 grammar lives nowhere in this crate, so no reader can drift back into interpreting an old shape as a new one. It also exposes `block_spans`, which LOCATES lock blocks without parsing them, because the migration door must find a block this crate refuses to read |
| `effects` | The effect kernel: pure Starlark evaluation — rules in, effect descriptors out; zero I/O, advisory-only |
| `run` | The mrd-local run plane: plan/execute under the workspace run lock. Owns `Authority`, the two-variant type the executor's choke point validates against — capabilities are real for starlark and do not exist for bash, structurally, see § Amendment |
| `realise` | The realise engine: observe → check → apply per claim, on the run plane |
| `view` | The DuckDB view organ: a write-only leaf projecting the warm corpus into a disposable, fingerprint-stamped file. Also the lock-aware read face — it reads `meridian-lock` `pins:` as a third pin form beside the legacy `^inputs` forms, and renders each pin's color through `model::selector`'s ONE computer, so the walk listing and the SQL board cannot answer the same question two ways. It renders the color; it never computes a second one |
| `check` | The check engine: the pure READ verb of the reconciliation loop |
| `preset` | Presets + session birth: def-pinned convention floor; `new`/`unfold`/`reconcile` materialize through the guarded create. `new` births ONE record from the def's `^template`, `unfold` the whole declared scaffold, `reconcile` the missing half of it — and `reconcile` alone also SUBTRACTS, by the `# Ephemeral` allowlist and never by set-difference, so undeclared content is a finding and never a deletion (ZT ruling #3). The design element these are audited against is `docs/preset-session-birth.md`; where it and the code disagree, the element wins |
| `config` | The `MERIDIAN.md` plane: the one entry point parsed as CONTENT — the two-rung bootstrap chain (`MERIDIAN_CONFIG`, then `$HOME/MERIDIAN.md`), the four resolution states with absent and zero-mount reaching ONE mount table, the strictest parse in the system (a closed `&'static str` reason set, 1-based FILE lines, a teaching refusal that states nothing loaded), and the config's own rev and fingerprint by the shipped laws — `blake3(bytes)[:16]`, no new rev noun minted. No partial mount table is a property of the TYPE: `Config` has private fields and `parse` is its only constructor. Downstream of `model`. **And the mount table (`mount.rs`), which is where a declared entry BECOMES a bound root:** canonicalize at bind, then the `workspace::deny_reason` ceiling **reused whole and never re-implemented** — so a config cannot bypass the ceiling through a file that is itself ordinary editable content — then the three-way map's uniqueness invariants (name ↔ Obsidian vault name ↔ path, refusing equal-or-nested paths so one tree cannot be bound twice under two names), then each root's own self-declaration: **the root declares, `MERIDIAN.md` binds**, so a declared-vs-bound mismatch fails the whole parse and an absent declaration renders grey. Per-root state is S3-R6's closed vocabulary — one `bound`, four `grey(...)`, one `red(...)`, every non-bound state refusing on exit 1 with its own reason word. Mount-as-claim lives here too: a mount may pin the root it declares, verified through `model::fingerprint::verify_content` — no new codec, no new hash law. `MountTable`'s field is private and `bind` is its only constructor, so no partial mount table can exist to be observed. **And the bridge period (`bridge.rs`), which is the "checked against" half of the env-var inversion:** `CCC_LLM_WIKI_PATH` and `CCC_LLM_WIKI_REPOS_ROOT` become mount entries, and until they demote to overrides each is checked against the bound table **through `MountTable::by_path`** — the canonicalize-at-bind law reused whole, never a second comparison — so the symlinked, trailing-slash and real spellings of one tree are one lookup. When they disagree **the FILE WINS** (`Bridged::mount` is `Some` only on agreement, so a diverging variable names no root) and the divergence is **reported once per process, per variable, and never on an exit code** — fail-loud here would brick the CLI on every machine exporting the variable, and a bridge whose mismatch is fatal is not a bridge. An empty table is `unchecked`, not divergent: every unmigrated machine exports both variables, and a check that fires on all of them is deleted before it ever guards anything. **And that projection is now here (U12):** `MountTable::projection` yields the `addr::MountSet` the planes that resolve and translate consume — which names this machine BINDS, the **vault name** each bound vault root carries (the stored plane is spelled in vault names), and which declared names are unreachable here WITH the path to check, so a refusal for a declared-but-unreadable root never prescribes a declaration that already exists (S3-R50). It is not `mrd walk`'s projection and the difference is a FACT, not a second spelling: walk also marks a root unreachable when its CORPUS will not build, which only a caller holding corpora can know |
| `workspace` | Workspace identity: the discovery ladder (env override → git root → cwd default), canonicalization, the deny ceiling — pure filesystem functions (a leaf, `std` + `cache` only). The ladder answers ONE question — *which root does this path belong to* — and every answer names the rung that answered: `Answer::root` is `None` on the cwd default, so a caller cannot inherit an unanchored cwd silently (marker-retirement ruling, 2026-07-26). The two EXPLICIT planes are deliberately NOT rungs here: the mount table (`config::MountTable`) cannot be one without a dependency cycle, since `config` depends on this crate for the ceiling; and a declared root arrives on the serve path as the hello `workspace` field, pinned exactly by `registry::Registry::pin_declared`, because a daemon has no meaningful cwd to walk. All three planes meet at exactly one point: `deny_reason`, reused whole, never re-implemented |
| `cache` | The hashed cache drawer: addressing, atomic sentinel registration, corrupt-is-a-miss probing, last-use GC |
| `registry` | The daemon-held workspace registry: unix-socket RPC server + client, first-writer-wins, atomic state, idle-reap |
| `mrd` | The workspace CLI — wires `workspace`/`cache`/`registry` into `init`/`unregister`/`resolve`/`cache`/`daemon`, and mounts the local run plane (`mrd run` via `crates/run`). A local CLIENT of the engine crates, never a resident organ and never on the serve path; its `run`→`model` edge stays a single reviewable dependency |
| `lockmigrate` | **SELF-RETIRING (U9b).** The lock v1→R4-v2 field migration: the ONE quarantined place the dead v1 grammar is spelled in engine Rust, plus the vault sweep that lands every rewrite through the governed `wire_serve::write::lock_migrate` door. It exists so `lock` can be v2-only — landing that crate without an executed sweep locks every vault in the field out of its own locks. Dry-run-first, idempotent, resumable; it REFUSES a vault with no git (the restore point is a pre-sweep commit) and rewrites only ENGINE-PLACED page locks, never a v1 block illustrated inside a document. Deletes itself once the sweep is executed and broadcast — `crates/lockmigrate/RETIREMENT.md` |
| `testsuite` | Integration tests + the frozen ground-truth pack as data |
| `perfsuite` | Perf harness and claims registry (out of default-members) |

## Amendment — the policy gate (armed change plane)

Law: ZT ruling #2; plan unit U4.2; wire-contract v2 refusal amendment
(`docs/wire-contract-v2-refusal-amendment.md`).

`crates/policy` originally owned advisory edit-time verdicts only — findings the
host could act on or ignore. This amendment extends the charter: `policy` now
also owns the **blocking gate** at the armed change plane.

- **The seam.** `gate(change, armed_set) → Ok(verdicts) | Refusal(violations)`
  fills `policy::authorize` and converts the advisory `evaluate_verdicts` seam
  to blocking — evaluated after CAS, before bytes land, in both writer paths.
  When a workspace is armed, a block-severity verdict or a door-law violation
  refuses the write; the refusal carries a `{code, recovery}` pair from the
  closed §8 taxonomy (the refusal-amendment table).
- **Trusted-path armed set.** `gate()` loads and verifies the attested INDEX
  from the workspace path inside the trusted write path; the caller-supplied
  ruleset parameter is removed from the gating decision. Absent INDEX on a
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

- **Multi-segment hpath — REACHABLE TODAY, and refusing it now would break
  shipped green behaviour.** `syntax::split_wikilink_target`
  (`crates/syntax/src/lib.rs:434-443`) puts everything after the first `#` into
  the heading fragment verbatim unless it starts with `^`, so the ordinary
  wikilink `[[sessions:notes.md#Design/Sub]]` mints a `/`-bearing selector, and
  `("v", "dir/a:b.md", Some("Design/Sub"))` is a PASSING row in
  `every_stored_form_decodes_back_to_the_parts_that_minted_it`
  (`crates/addr/src/stored.rs:472`). **On `main` a selector is one opaque string
  on BOTH planes**, so the round trip is a genuine fixed point and there is no
  second reading for the stored form to lose. The ambiguity `Design/Sub` would
  be ambiguous *against* — one segment or three — comes into existence WITH
  U14's segmented hpath, and the refusal is additive against that grammar.
- **Dewey** — there is no dewey spelling in the agent-plane address grammar on
  `main`. `[[x.md#1.2]]` is a heading literally named `1.2`, and `heading=1.2`
  stores it correctly.
- **Occurrence index** — no spelling in `Addr` at all.

**The last two are not merely unreachable, they are UN-IMPLEMENTABLE, and that
is why no variant was landed for them.** There is no value at the translation
seam to detect, so the refusal would be a variant with no constructor —
S3-R23(4)'s weakened middle, a claim nothing checks. **Do not land dead variants
for symmetry, and do not let a later reader land them for tidiness.**

### Q7 — why the view's cross-root destination is THREE columns, not two

U21 Q5 ruled a nullable `dest_root` **beside** `dest_path`. Implementation
measured the fact the ruling was written without: `link.dest_path` carries an
**enforced** foreign key into `doc(path)` — DuckDB answers *"Violates foreign key
constraint because key `path: notes.md` does not exist in the referenced
table"* — and a cross-root path is not a key in this corpus. The literal
two-column shape therefore required DROPPING that FK.

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
projector's discipline. `dangling`'s two-place clause (`dest_path IS NULL AND
dest_root IS NULL`) is what stops a resolved cross-vault link reading as broken,
and it is pinned by a red test, mutation-proved one-edit, in
`crates/view/tests/u21_cross_root_link_rows.rs`.

### R1.6-a — the stored→agent re-join, and why it stays

`stored_occupants` (`crates/wire-serve/src/positions.rs:511`) decodes a stored
URI into its parts, then **re-joins them into one string and re-parses it**:

```rust
// crates/wire-serve/src/positions.rs:539-545
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

> *"there is no `from_parts` a caller can use to smuggle an unparsed root prefix
> into the `Addr::path` field"* — `crates/addr/src/lib.rs:10-15`

That invariant is what makes every downstream guard checkable. Redesigning it is
its own considered act with its own gate, not a rider slipped into another
unit's train. **The successor act is: give `Addr` a fallible parts constructor
running the same checks `parse` runs, then delete the join.**

### C-1 — the link plane's in-process degrade, and its NAMED SUCCESSOR

The daemon's warm state is one workspace's corpus, keyed by that workspace's own
canonical path and invalidated by its own fingerprint
(`crates/registry/src/registry.rs:317-321`, `warm_or_build`). **It holds no
mounted-root corpora**, so the link arm serves ambient state only
(`crates/registry/src/server.rs:782-789`). U21 therefore resolves a cross-vault
link by DEGRADING that one op to in-process, where the mounted corpora can be
loaded the way the walk plane already loads them
(`crates/mrd/src/walk_cmd.rs:146`).

**The asymmetry is a documented contract, not a bug awaiting discovery:** for
this one op the daemon is knowingly less capable than the in-process path, and a
page carrying a cross-vault link pays a cold corpus build. Stated here rather
than smoothed, on the same discipline as the exit-code asymmetry in
`docs/address-grammar.md`.

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

**This row is a POINTER, not a claim about this tree.** U14 is not merged at the
time of writing — `lock_ref_fragment` is still the name here
(`crates/wire-serve/src/write.rs:1319`) and U14's rename does not exist on main.
The row's owner is U14, and U14 fills in the detail when it lands. Writing its
shapes here now would assert a tree that this tree contradicts, which is the
defect this whole section exists to prevent.

## Amendment — capabilities do not apply to bash

Law: ZT ruling, made verbally, re-litigated in code, and ruled again live
2026-08-01. **Gate: `crates/mrd/tests/law_no_caps_on_bash.rs`** — that file is
what makes this hold, and this section is what it enforces. A reader who
proposes "just a small cap check on bash" must answer both.

> **Capabilities do not apply to `bash` tasks. Not now, not later, not in a
> weaker form.**
>
> 1. A bash task carries **no `caps:` line**, no cap resolution, no cap source,
>    and no `deny-default`.
> 2. The engine **never prints a claim about what a bash task may do** — most of
>    all not `(read-only)`.
> 3. Bash is **unsandboxed by definition.** The only honest description on any
>    surface is: *unsandboxed shell, undeclared effects.*
> 4. Capabilities remain a real, enforceable contract for **starlark**, and only
>    starlark.

**The guarantee is impossible, not merely difficult.** A capability claim is a
promise about what a process CANNOT do, and for bash the engine holds no
mechanism that makes one:

| layer | what exists | why it does not bound the process |
|---|---|---|
| in-window writes | the `out-of-band delta` detector | **detects, never prevents** — `docs/run-plane.md` scopes it so, and the offending file persists |
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
