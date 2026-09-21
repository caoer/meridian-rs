---
type: contract
id: laws
status: standing
description: The three architecture laws, enforced as crate dependency edges rather than conventions, plus the charter of every crate.
owns: [architecture laws, crate charters]
---

# The three laws

This file is the architecture law of the engine: the three laws below, the
charter of every crate, the named residues and candidate rows that are ruled
to wait, and the amendments that extend the laws. It does not define the wire
itself; `wire-contract.md` does.

`meridian-rs` is split into crates so that each core invariant is a dependency
edge, not a convention: breaking a law is a compile error, not a review
comment.

> Standing law: `README.md` (process and standing corrections) and `wire-contract.md` (the wire contract).

## Law 1 — the wire cannot leak inward

`model`'s public types carry no `serde` derives. The in-memory model stays
non-serializable, so no wire shape reaches the tree. Host-facing facts cross
the projection seam (Law 3), never by deriving `Serialize` on a model type.
The private `parse-cache` adapter explicitly encodes disposable documents
for restart reuse (node-rev-merkle-spec §6.9). Its storage vocabulary is not
wire vocabulary, and it adds neither serialization nor persistence to `model`.

## Law 2 — nothing host-facing exists beyond `wire`

`wire` is serde-only, does no I/O, and alone defines the host-visible
vocabulary: paths, spans, node revisions, **fingerprints** (the workspace
content hash), and the op/request/response/error types. A type not in `wire`
is not on the wire. `wire-contract.md` is its intended surface; code may lag,
docs win.

## Law 3 — the bridge has two named organs; everyone else is a consumer

The bridge is everything that turns the model tree into wire answers. It lives
in exactly two crates.

- **`wire-map` is the projection seam.** The model tree flattens into wire
  shapes here as a tested library function, nowhere else, with the host-face
  read facts (`facts`: dewey ordinals, segment `hpath`, word counts). Fields
  still emitting **joined or sanitized display strings** are interop debt,
  not address law (`wire-contract.md` §2.1: segments only).
- **`wire-serve` is the serve choke-point.** Strict decode, the read arms
  (including the composed `read`), the `splice → commit` write choke-point
  and the standing vocabulary projection are one implementation. The resident
  `registry` daemon dispatches them, and that daemon is the one wire door
  (wire-contract §3.3). The host shares the leaves, not the dispatch shell.

Every other crate naming both `wire` and `model` only consumes them:
`registry` (host, wiring only), `mrd` (local client), `render` (projection
facts → text face), `check`/`preset`/`realise` (engine planes over wire
vocabulary), `testsuite` (observes). Growth inside one of these consumers
means a capability is missing from `wire-map` or `wire-serve`.

A corollary edge: `syntax` is the only crate that touches the pulldown-cmark
fork, so fork churn is a one-crate event.

## Additivity

New capability arrives as new leaf crates or match arms, never as a reshuffle
of what ships. A new op is a new `Op` variant and a new dispatch arm, which
the host discovers through `hello`'s capability list. `policy` and `query` are
additive consumers of the model's index. Nothing shipped is ever split.

## Crate charters

Each crate's `lib.rs` states its charter: what it owns, never does, and
which laws it carries.

| Crate | Charter |
|---|---|
| `addr` | The agent-plane address `[root:]path[#selector]`: a fallible type with an optional canonical root name, plus the bound-name projection (`MountSet`) every plane resolves through. A `std`-only leaf upstream of `syntax`; nothing downstream re-splits a string. `Addr::parse` is the sole constructor; the path field carries no root prefix. Colon law: root wins, **no fallback to the literal reading**. Parse is not resolve: whether a named root is bound is the resolver's grey answer, never a parse error |
| `timing` | The `MRD_TIMING` switch, resolved once per process into a sink, and the phase span emitting one `mrd-timing cmd=… who=… phase=… us=…` line per completed phase (`who=`: process and thread). A `std`-only leaf with zero dependencies, used by `fs`, `run`, `mrd`, `registry`. Never touches stdout or an exit code; reads no clock when off. Not a log framework or tracer: no levels, spans-in-flight, subscriber, or second time unit (`us`, the wire's `meta.duration_us` noun). Surface: `status.md` § The timing mode; `mrd run` phases: `run-plane.md` § Timing phases |
| `syntax` | Markdown bytes → dialect node list with byte-exact spans; sole owner of the pulldown-cmark fork. A wikilink's target carries the address and none of markdown's own escapes: the `\|` a table cell writes before the alias pipe is the table's byte, dropped here, so no plane downstream resolves a spelling the author never wrote. The node's SPAN still covers it — the escape is in the bytes, just not in the address |
| `model` | The governed node tree, resolve, CAS-splice validation, workspace fingerprints; non-serializable (Law 1). The frozen heading predicate (`gotext`), the one address law its two dependents share. The content-identity plane: the `fp1.…` CID-token; `verify_content`'s four-arm verdict (`Unverifiable` carries the whole `version.codec.hashfn` triple); the one reason-carrying `Color` model every drift surface uses. The frontmatter scalar codec (`scalar`): sole owner of the `wire-contract.md` § A.6 value law; decode for every read seam, double-quoted encode for every value-plane write door |
| `fs` | Disk read/walk/watch into the model; atomic tmp+fsync+rename splice execution. The `.base` membership walk (`base-projection.md` §3): hash-domain rules with the floor swapped from `*.md` to `*.base`, returning raw bytes per member plus the `bf:` witness (`base-projection.md` §6.2); stays YAML-free: it hands the bytes up and `view` parses them. **And the `.canvas` carrier walk** (`move.md` §4 class 5): the same floor swap once more, `*.md` to `*.canvas`, over the one shared extension walk — it stays JSON-free, and `query` parses those bytes |
| `wire` | The serde-only wire vocabulary (Law 2) |
| `wire-map` | The model→wire projection seam (Law 3) |
| `git` | Git plumbing: shell-out content-addressing (blob object ids, the eager `-w` write) and object reachability against a `Repo` handle; asks git and reports its answer, never computes or guesses an oid. A `std`-only leaf, no production dependency |
| `receipt` | Two planes: the persisted `^receipt` line renderer, committed in the same batch as its edit (shipped default template; facts normative, template replaceable); the origin-freshness anchor axis and three-state blob classification (`anchored` / `pending-anchor` / `never-anchored`). No read-side state: pin proof rides the request (wire-contract § A.3); the engine records no reads. Depends on `wire` only, by gate; `git` does the I/O |
| `transport` | Untyped NDJSON envelope + codec seam; framing without meaning |
| `policy` | Ruleset compile + assertion evaluation under budgets; edit-time verdicts; the blocking `gate` at the armed change plane (`policy::gate`; § Amendment — the policy gate) |
| `query` | Corpus reads over the model's borrowed index; applies nothing. **And the move plan** (`relocate`, `move.md` §4–§6): the corpus-wide, span-exact reference-rewrite plan for a file or directory move — would-break decided by re-running the address owner against the re-keyed corpus, the new spelling minted at the class the author wrote, the ambiguity refusal and the immutable skips as plan facts. **And the JSON Canvas carrier** (`canvas`, `move.md` §4 class 5): a `.canvas` file's node slots located structurally in the raw bytes, so a rewrite is span-exact and every byte the move does not own survives — the parse is a leaf module of this crate, beside its only consumer. A plan, never an application |
| `wire-serve` | The shared typed edge (Law 3 choke-point): one implementation, one host (wire-contract §3.3). Agent/stored address seam: `put` translates cross-root `root:` to the `obsidian://` stored form and `read` back, at the candidate document (`address-grammar.md` §9). Reads `config`'s mount table lazily when a candidate can carry a cross-root position. **And the in-process move door** (`relocate::relocate`, `move.md` §8–§9): the one composed multi-file write — plan under the write flock, one atomic replace per referring page, the rename last — beside `create` and `remove`, which the preset lane already calls in-process; never a wire op (`wire-contract.md` §16). |
| `render` | The compiled-in render plane: `Renderer` + node-grain walker producing the TOON-compact projection through its own encoder (`render::toon`), with block-elision and claim-link decoration hooks. Decorations arrive as data: no `render → lock → fingerprint` edge |
| `lock` | The `meridian-lock` fenced-block format: canonical writer/reader, engine sole-writer; owns the reserved `meridian-*` block-language namespace. Reads the current v2 schema; refuses unsupported versions |
| `effects` | The effect kernel: pure Starlark evaluation, rules in, effect descriptors out; zero I/O, advisory-only |
| `run` | The mrd-local run plane: plan/execute under the workspace run lock. Owns `Authority` (capabilities real for starlark, absent for bash). See `run-plane.md` |
| `realise` | Observe → check → apply per claim, on the run plane |
| `view` | **Ephemeral projection + lock-aware read face** (`wire-contract.md` §10.3–§10.4; not agent core): the parsed corpus in an in-process `:memory:` DuckDB (`build_memory`, the `mrd sql` operator face); walk/status colour reads. **Writes nothing to disk**: no persistent published file, no `view.duckdb`, no `view_path` wire op (`wire-contract.md` §10.4). The `.base` projection (`base-projection.md`): three `base` relations, `link.exclusion_path`, the `base_fold` second witness; its parse is the leaf module `view::base`, hence the third permitted `serde_yaml` taker (`base-projection.md` §9, enforced by `yaml_confinement`) |
| `check` | The pure read verb of the reconciliation loop |
| `preset` | Presets + session birth: def-pinned convention floor; `new`/`unfold`/`reconcile` through the guarded create. See `run-plane.md` (preset section) |
| `config` | The `MERIDIAN.md` plane, the one entry point, parsed as content. Bootstrap chain: `MERIDIAN_CONFIG`, then `$HOME/MERIDIAN.md`. Four resolution states; absent and zero-mount reach one mount table. The strictest parse in the system: closed `&'static str` reason set, 1-based file lines, a teaching refusal stating nothing loaded. Rev and fingerprint: `blake3(bytes)[:16]`, no new rev noun. `Config` has private fields, `parse` its only constructor: no partial table. Downstream of `model`. **Mount table (`mount.rs`)** binds a declared entry, in order: canonicalize; the `workspace::deny_reason` ceiling, reused whole; three-way uniqueness (name ↔ Obsidian vault name ↔ path), refusing equal-or-nested paths; **the root declares, `MERIDIAN.md` binds**: a mismatch fails the whole parse, an absent declaration renders grey. Per-root state is grey-exit-1's closed vocabulary (one `bound`, four `grey(...)`, one `red(...)`); every non-bound state refuses on exit 1 with its own reason word. Mount-as-claim: a mount may pin its root, verified through `model::fingerprint::verify_content`; no new codec or hash law. `MountTable`'s field is private, `bind` its only constructor. **Bridge period (`bridge.rs`):** `CCC_LLM_WIKI_PATH` and `CCC_LLM_WIKI_REPOS_ROOT` become mount entries; until they demote to overrides, each is checked against the bound table through `MountTable::by_path` — the canonicalize-at-bind comparison reused whole, never a second one, so the symlinked, trailing-slash and real spellings of one tree are one lookup. On disagreement **the file wins** (`Bridged::mount` is `Some` only on agreement); the divergence is reported once per process, per variable, never on an exit code. An empty table is `unchecked`, not divergent. **Projection:** `MountTable::projection` yields the `addr::MountSet`: bound names, each bound vault root's **vault name** (the stored-plane spelling), and unreachable declared names with the path to check. Not `mrd walk`'s projection, which also marks a root unreachable when its corpus will not build |
| `workspace` | Workspace identity: the discovery ladder (named argument → env override → git root → cwd default), canonicalization, the deny ceiling; pure filesystem functions (a leaf, `std` + `cache` only). Every answer names the rung that answered; `Answer::root` is `None` on the cwd default. The top rung is provenance, not a path. `workspace::Base` (a type, not a flag) says whether the caller named the path or it is the ambient cwd. `MERIDIAN_WORKSPACE` answers only for the ambient case, so an explicit operand outranks ambient state: otherwise `MERIDIAN_WORKSPACE=victim mrd unregister target` would remove victim. Not rungs: the mount table (`config::MountTable`; `config` depends on this crate) and the declared root, the hello `workspace` field on the serve path, pinned by `registry::Registry::pin_declared`. All three planes meet at `deny_reason`, reused whole, never re-implemented |
| `cache` | The hashed cache drawer: addressing, atomic sentinel registration, corrupt-is-a-miss probing, last-use GC |
| `parse-cache` | The private, generation-bound document codec: explicit model conversions, bounded decoding, checksum/content/span validation. No filesystem I/O, no wire types, no persisted corpus index. Registry owns placement and lifecycle; `fs::update_corpus` remains the single reconciliation implementation. |
| `registry` | The daemon-held workspace registry: unix-socket RPC server + client, first-writer-wins, atomic state, idle-reap, and the resident budget (LRU eviction of whole warm workspaces, `MRD_MAX_RESIDENT_BYTES`) |
| `mrd` | The workspace CLI: wires `workspace`/`cache`/`registry` into `init`/`unregister`/`resolve`/`cache`/`daemon` and mounts the local run plane (`mrd run` via `crates/run`). A local client, never a resident organ or on the serve path; its `run`→`model` edge stays one reviewable dependency. The CLI rooted lane (`rooted.rs`): the one seam every page-taking door resolves `[root:]path` through (address-grammar § 4.6) |
| `testsuite` | Integration tests + the frozen ground-truth pack as data |
| `perfsuite` | Perf harness and claims registry (out of default-members) |

**Bulk lock migration is out-of-product:** a bulk v1→v2 lock rewrite is
script-class, not a product door.

## Amendment — the policy gate (armed change plane)

Law: `wire-contract.md` § A.2 (armed plane) and § Refusal taxonomy.

`crates/policy` owns advisory edit-time verdicts — findings the host may act on
or ignore — and the blocking gate at the armed change plane.

- **The seam.** `policy::gate` (`crates/policy/src/gate.rs:108`) is
  `gate(change, law) → GateOutcome`, which is
  `Ok(verdicts) | Refusal(violations)`. It blocks where the advisory
  `evaluate_verdicts` only advises: after CAS, before bytes land, in both
  writer paths. In an armed workspace, a block-severity verdict or a door-law
  violation refuses the write with a `{code, recovery}` pair from the closed
  taxonomy (`wire-contract.md` §8).
- **Trusted-path armed set.** The armed law is loaded and verified from the
  workspace path inside the trusted write path (`resolve_armed_law`,
  `crates/policy/src/armed_law.rs:257`, on its own disk seam); no
  caller-supplied ruleset takes part. On a never-armed workspace an absent
  INDEX is a bit-for-bit no-op; on a once-armed workspace a missing INDEX
  fails closed (`convention-fault`).
- **Additivity holds (§ Additivity).** The gate is a new match arm at the
  write seam; `model`, `wire` and the projection seam are untouched (Law 1);
  refusals mint only through `wire`'s error types (Law 2).

**Scope of the refusal claim.** Refusal makes violations "unrepresentable
through an armed change plane", nothing stronger. The genesis epoch
(pre-first-arming writes) renders grey, never green. Out-of-band mutation — an
offline pre-push git rewrite, a root-preserving forged journal row — is caught
by the git witness plus the receipt-engine-only write restriction, or is a
named residual, never rendered green.

## Named residues and candidate rows

A **named residue** is a construction the engine's law disapproves, whose
behaviour is correct today, kept with its reason recorded. A **candidate row**
is a change nobody has ordered yet, named so a future docket inherits a
decision, not a defect. Neither is a TODO: each row is ruled to wait.

| # | Row | Kind | Status |
|---|---|---|---|
| R1.6-a | The stored→agent re-join/re-parse in `wire-serve::positions` | residue | deferred — see below |
| C-1 | The link plane resolves cross-vault refs in-process, not in the daemon | residue | a degrade — **successor named below** |
| H-1 | The `#` refusal on a heading whose raw text carries `#` | candidate | **a pointer** — see below |
| S-1 | The stored-plane narrowing refusal | candidate | **owed** — see below |
| D-1 | The joined `--section` coat splits on `/`, so a heading whose raw text carries `/` is not addressable by that one spelling | residue | **widening the coat is C2, and C2 stays reserved** — see below |
| G-1 | The wire-contract §2.4 block-id charset is enforced at the structured ingress only, so an unmintable `^id` misses at the read and walk doors instead of refusing | candidate | **face decision proposed, unratified** — see below |

### S-1 — the stored-plane narrowing refusal, and why it is owed

Two planes spell an address. The **agent plane** is `Addr`'s
`[root:]path[#selector]` form; the **stored plane** is the spelling a vault
carries, a wikilink or an `obsidian://` URI.

The stored-plane narrowing refusal — refuse at the translation door with a
named `TranslateError` — is owed wherever the wikilink ingress can mint a value
the agent-plane grammar cannot represent unambiguously. That premise holds for
the first of the three values below only.

- **Multi-segment hpath — law on the mint plane; the joined form is residual
  debt.** A machine address is segment objects only:
  `{"hpath":[{"h":"Design"},{"h":"Sub"}]}` (`wire-contract.md` §2.1). S-1
  tracks the stored/wikilink ingress that can still mint a `/`-bearing opaque
  string (`[[sessions:notes.md#Design/Sub]]` via
  `syntax::split_wikilink_target`, round-tripped as one string on both
  planes). `Design/Sub` is ambiguous — one segment or three — so the refusal
  acts on that joined residual only, never on segment-form law.
- **Dewey** — the agent-plane grammar on `main` has no dewey spelling.
  `[[x.md#1.2]]` is a heading literally named `1.2`, and `heading=1.2` stores
  it correctly.
- **Occurrence index** — `Addr` has no spelling for it.

The last two have no value at the translation seam to detect; a refusal
would be a variant with no constructor. **Do not land dead variants for
symmetry.**

### Q7 — why the **optional view organ**'s cross-root destination is three columns, not two

*(View organ / SQL board only — not agent core; `wire-contract.md` §10.3–§10.4.
The core path never assumes this schema.)*

A link may point into another mounted root — another vault.

A cross-root link row stores `dest_root` + `dest_root_path` and leaves
`dest_path` NULL, so `dest_path` always means "a path in this corpus".
`link.dest_path` carries an enforced foreign key into `doc(path)`; that FK is
the only thing in the schema that makes a link row pointing at a missing
document unrepresentable.

- *A nullable `dest_root` beside `dest_path`* — rejected: a cross-root path
  is not a key in this corpus, so that shape required dropping the FK. The
  DuckDB schema answers such a row *"Violates foreign key constraint because
  key `path: notes.md` does not exist in the referenced table"*.
- Illegal states close structurally:
  `CHECK ((dest_root IS NULL) = (dest_root_path IS NULL))` and
  `CHECK (dest_path IS NULL OR dest_root IS NULL)`.
- `dangling` tests `dest_path IS NULL AND dest_root IS NULL` so a resolved
  cross-vault link is not reported broken
  (`crates/view/tests/u21_cross_root_link_rows.rs`), and `AND exclusion IS
  NULL` so a deliberately unhashed target is not reported broken
  (`crates/view/tests/dangling_exclusion.rs`).

### R1.6-a — the stored→agent re-join, and why it stays

`stored_occupants` (`crates/wire-serve/src/positions.rs:443`) decodes a stored
URI into parts, then re-joins and re-parses them:

```rust
// crates/wire-serve/src/positions.rs, in stored_occupants
let address = match &parsed.selector {
 Some(sel) => format!("{name}:{}#{sel}", parsed.path),
 None => format!("{name}:{}", parsed.path),
};
let occupant = Occupant {
 addr: Addr::parse(&address)…
```

That is a joined string address on a machine surface, which the machine-surface
law disapproves — *"Arrays for machines, TOON for humans. No string address
forms in machine surfaces."*

- **The behaviour is correct.** Join and split agree; the round trip is
  asserted byte-identical
  (`positions.rs::tests::the_agent_plane_form_round_trips_byte_identically`).
  Only the construction is disapproved.
- **Why it waits.** `Addr` has no `from_parts` by deliberate invariant
  (`address-grammar.md` §2.2: *"there is no `Addr::from_parts` a caller can
  use to smuggle an unparsed prefix into the `path` field"*); that keeps every
  downstream guard checkable, so redesigning it is its own act with its own
  gate.
- **Successor act:** give `Addr` a fallible parts constructor running the same
  checks `parse` runs, then delete the join.

### C-1 — the link plane's in-process degrade, and its named successor

The daemon warms one workspace's corpus, keyed by its canonical path and
invalidated by its fingerprint (`crates/registry/src/registry.rs:317-321`,
`warm_or_build`). It holds no mounted-root corpora, so the `Op::Links` arm
serves ambient state only (`crates/registry/src/server.rs:1121-1135`). The
link plane resolves a cross-vault link by **degrading that one op to
in-process**, loading mounted corpora as the walk plane does
(`crates/mrd/src/walk_cmd.rs:146`).

- **A documented contract.** For this op the daemon is knowingly less capable
  than in-process; a page with a cross-vault link pays a cold corpus build,
  as with the exit-code asymmetry in `address-grammar.md`.
- **Narrow.** The degrade fires only when an unresolved head names a root the
  mount table declares (`addr::head_names_declared_root` — bound or
  declared-but-unreachable), never on any `:`-bearing head. An external URI
  parses as a root (`https://…` → head `https`), and a wide lexical gate would
  buy the whole corpus a cold rebuild to say that `https` is not a mounted
  root: measured, 137 s on an 11.5k-file vault over 6 such `[[https://…]]`
  links.
- **Table-external heads** — external scheme, undeclared root, or a name
  outside `[a-z0-9-]` — keep the daemon's ambient `unresolved`, verbatim,
  exit 0; the address plane still refuses those spellings on every door that
  consults it (`walk`, pins, a named read). A mount table that will not resolve
  or bind keeps the old posture and degrades.

> **Named successor — option (A): the daemon holds mounted corpora.** Deferred
> deliberately: it needs per-root fingerprint invalidation, residency and reap
> — a designed subsystem with its own design element and gate, not a detail of
> a link-plane fix.

### H-1 — the `#` refusal

The `#` refusal survives: `#` is a live delimiter in both the wikilink and
`path#fragment` ingress. Shipped form: `refuse_unrepresentable_heading`
(`crates/wire-serve/src/write.rs`). This row is a pointer; the refusal's shapes
are asserted where it lives.

### D-1 — the `/`-coat limitation, and why C2 stays reserved

The **coat** is the joined human-string spelling of a selector — what
`--section SEL` and `mrd pin` take. The `/`-heading law has two halves:

- **Machine half.** A `/`-bearing heading is representable and pinnable as
  one hpath array segment (`{"hpath":[{"h":"Guide"},{"h":"A/B"}]}`).
  Gate:
  `crates/wire-serve/tests/s7_pin.rs::a_slash_bearing_heading_pins_end_to_end_and_stores_as_one_array_element`.
- **Coat half.** `ReadSel::parse` (`crates/wire/src/lib.rs`, the one
  human-string ingress door) splits on `/`, so the joined spelling resolves to
  nothing: the door **misses** rather than serving a different section.
  Characterization test:
  `crates/wire-serve/tests/s7_pin.rs::the_cli_string_coat_still_cannot_address_a_slash_bearing_heading`.

**D-1 and G-1 are two properties of one function.** `ReadSel::parse` is
infallible by signature (`pub fn parse(s: &str) -> Self`): it splits the
heading arm on `/` (D-1) and takes `^id` verbatim with no charset test (G-1).
Making it fallible moves both rows; the D-1 test above shows it.

**The coat is not widened.** Widening it is an escape grammar over a flat
selector — C2 — and C2 stays reserved until a real need appears (a string
selector is not the ideal machine form, the put path is an unambiguous array,
sanitization is never necessary).

**A miss, not a refusal.** The one law for this row and its neighbours:

> **Refuse what can never exist; miss what exists but this door cannot spell —
> and the taught recovery must be the one that actually repairs it.**

Input that no corpus could ever carry is outside the minting grammar, so the
door **refuses** it with `bad_request` (wire-contract §2.4's `_`-bearing block
ids). A `/`-bearing heading is the other case: the corpus carries it and the
machine plane pins it. Only this ingress cannot spell it, so the door
**misses** and owes the caller the spellings that do reach it.

**Scoping is per delimiter, per ingress:**

| Ingress | `/` | `#` |
|---|---|---|
| CLI joined `--section SEL`, `mrd pin`'s selector (`ReadSel::parse`) | **delimiter** — splits; a `/`-bearing heading is unreachable by this spelling | **heading text** — `--section 'Top/C#D'` serves |
| CLI `PATH#FRAG` (the frag door) | inherited from `ReadSel::parse` — same split | splits on the first `#` only; the tail is selector bytes, so `notes.md#Top/C#D` serves |
| wire / MCP segment arrays (`{"hpath":[…]}`) | heading text | heading text |
| wikilink / `path#fragment` heading refusal | — | **H-1's column, untouched by this row** |

**Two escapes the face must teach**, both from the published toc row: the
**dewey ordinal** (`--section 1.2`, the number that toc row carries) and the
**raw heading segments** as an hpath array (one entry per heading, no
joining). Pointing at the toc alone loops, because it hands back the same
title this door cannot take. Teaching site: `wire_serve::section_recovery`
(precedent: the duplicate-heading refusal, which teaches machine address +
dewey).

> **The script plane executes the teaching it prints.** The commit leg carries
> the engine's refusal verbatim, so the plane that receives this teaching must
> accept both taught forms. `section=` on the script `put()` and `read()`
> builtins takes the wire-contract §2.1 segment array (run-plane.md § the
> arming surface); a `str`-only `section=` would meet the hpath array with a
> type error. The script toc face publishes each heading row's raw segments as
> `hpath`. So the taught recovery is executable on every plane that prints it,
> and the coat itself is untouched.

### G-1 — the §2.4 charset is enforced at one ingress of two

This row records a **divergence** in where the block-id charset is enforced.
It is a candidate only because the fix is a face decision nobody has ratified;
D-1's refuse/miss line governs it.

wire-contract §2.4 rules one block-id charset, `[A-Za-z0-9-]+`, on both
planes. A `_`-bearing anchor is outside the strict-plane grammar
(`bad_request`). §4.5 and GOAL 2 say the same for the walk plane ("refuses
loudly"). Three doors answer that one law three ways:

| Door | `_`-bearing id | Recovery taught |
|---|---|---|
| write (`put`, structured) | `bad_request`, charset named, §2.4 cited | **fix** — the ruled shape |
| read (composed / `--section`) | `no_match` + nearest list | re-read |
| walk (`resolve`) | `ref_not_found{stage:2}` | refresh |

**Why a divergence.** `no_match` and `ref_not_found` teach *not there right
now*, which is false forever for an id §2.4 forbids minting; only
`bad_request`/fix terminates.

**One ingress carries the decode-time charset guard; the other does not.**
That guard refuses, at decode, an id that already exists out of grammar — the
case §2.4 rules present-tense. It is not the *mint-guard* §2.4 assigns to
§13.8, which governs minting going forward.

- Structured ingress: refuses at decode (`wire-serve::decode::decode_anchor`,
  `wire-serve::read::to_model_ref`).
- Human-string ingress: `wire::ReadSel::parse` takes `^id` verbatim, so CLI
  `--section`, the `PATH#FRAG` frag door and `mrd pin` carry the id into
  resolution, where it can only miss.
- Walk leg: the same omission in `model::walk::parse_linktext`, whose
  `Miss{stage,dest}` has no arm for a grammar refusal.

`wire-contract.md` §18 assumes a `_`-bearing anchor refuses loudly and so
carries no walk-plane charset deviation; code does not yet meet that premise.

**Proposed face decision, awaiting ratification:**

- Enforce the §2.4 boundary at decode, at every ingress, before any lookup, so
  an out-of-grammar id never becomes a selector or a miss.
- The other two doors adopt the write door's refusal string verbatim.
- The guard sits at the `resolve` op boundary, never inside `walk()`, which
  stays pure best-effort app-parity (§4.5).
- The refusal never becomes an `unresolved` row; that is resolution
  vocabulary.

**Nothing moves in code under this row.**

## Amendment — capabilities do not apply to bash

Gate: `crates/mrd/tests/law_no_caps_on_bash.rs`.

> **Capabilities do not apply to `bash` tasks. Not now, not later, not in a
> weaker form.**
>
> 1. A bash task carries **no `caps:` line**, no cap resolution, no cap source,
> and no `deny-default`.
> 2. The engine **never prints a claim about what a bash task may do** — above
> all not `(read-only)`.
> 3. Bash is **unsandboxed by definition**, and no human surface prints the
> word `unsandboxed`: the engine has no sandbox for it to contrast with, so the
> only honest description is *undeclared effects*. The class survives
> structurally (`GuaranteeClass::Unsandboxed`, the `--json` `guarantee` key);
> a guarantee word renders only where positive: `hermetic`.
> 4. Capabilities remain a real, enforceable contract for **starlark**, and
> only starlark.

**The guarantee is impossible.** A capability claim promises what a process
cannot do, and no layer bounds a bash process:

| layer | what exists | why it does not bound the process |
|---|---|---|
| in-window writes | the `out-of-band delta` detector | **detects, never prevents** — `run-plane.md` scopes it so, and the offending file persists |
| after the window | nothing | the detector's own wording is *"during exec window"*; a `nohup`, launchd plist, cron line or daemon writes with no observer |
| outside the corpus | nothing | env scrubbing does not restrict network, credentials, SSH, or `rm -rf` — none of it is an "effect"; and there is no cwd isolation at all, the step runs where `mrd` runs |

**No honest value exists for a bash `caps:` field — not `none`, not
`(read-only)`.** A resolution ladder there would only mislead by adjacency:
`caps:` is true on the starlark row, so a value beside it claims the same
enforcement.

**Structural, not cosmetic.** `run::caps::Authority` has two variants —
`Capabilities(CapResolution)` and `Unsandboxed` — validated at the executor's
choke point. The bash dispatcher has no capability field
(`dispatch_bash::BASH_AUTHORITY`), and `resolve_authority`, the only
language-aware entry, does not read a bash task's `task.<name>.caps`
declaration. Deleting the printing while resolution still runs fails the
gate's second half.

**Not weakened.** Starlark keeps the whole contract: hermetic evaluator, closed
builtin surface, no `exec`/`os`/`subprocess`, every effect a descriptor the
applier gates. The same gate file asserts both refusal shapes: `md.*` without
its cap → `capability denied`, exit 1; `proto.*` without its cap →
`state: unexecuted-no-capability`, exit 0. The `check-*` / `verify-*` bash
fence refusal also survives, as a name law, not a capability.

**Bash has no governed-tree effect channel** — no effect-shim fd, no frame
grammar, no descriptor apply. A bash block observes and reports; governed
writes ride the wire faces (MCP `put`) or a starlark task. A gated bash channel
would not bound the block: a denied block writes with `sed -i` instead, where
the detector sees the change and never rolls it back. The gate file's second
half asserts the governed page stays byte-identical after any bash run.

## Amendment — the face-honesty law

Gate: `crates/mrd/tests/law_face_honesty.rs`.

A **face** is one surface the engine answers on: the human face a verb prints,
and the machine face (`--json`, the wire).

> **Every face states the bound of its own answer.**

1. **A subset answer is marked:** the count withheld, the criterion, and a
   pointer to the full face. Enumeration stays on the machine face. A flooding
   payload delivers less truth — the walk-payload failure — and
   `links --json` already enumerates every file, so marking costs one line.
2. **A limit that can refuse is discoverable before it refuses,** in the
   verb's help, never only by tripping it. A raise flag is not ruled in;
   raisability is cost policy, not discoverability. The machine half is an
   `ack_bounds` grammar: a core-declared number a client reads. This tree does
   not carry it — `ack_bounds`, `protocol.limits` and `"limits"` return zero
   hits, while the same tooling in the same scope does find `exceeded the read
   budget` and `walk root not in the corpus`, the positive control beside that
   zero. So only the help half lands here; the machine half is an owed wire
   dependency.
3. **A refusal carries its recovery** at the human face, as the wire rules
   on frames: name the verb that answers the caller's evident question when
   one clearly exists; otherwise nothing, because a wrong pointer is worse
   than none.
4. **Engine-owned files are counted and labeled**, never silently either way.
   `mrd init` writes `MERIDIAN.md` (`config::CONFIG_FILENAME`) into the
   corpus it declares; excluding it is a hidden filter, and counting without
   a label pollutes the content count. Ruled form: *"4 files: 3 content + 1
   engine-owned"*.

Defect: `mrd links` printed 6 lines naming 2 files of 112 and said nothing
about the 110 withheld; only `--json` showed them. A filtered answer and an
empty world look identical.

**Not authorized:** raising a budget (cost policy); a recovery pointer a face
is not sure of; an enumeration on the human face. The `script` budget refusal
is the positive example and does not change: *"exceeded the read budget of 64
reads per attempt — refused, never truncated"* names number and units and
gives absence one meaning.

## Amendment — no hard-coded flow (mechanism in code, semantics in markdown)

The engine hard-codes no flow concept. Users, not all of them engineers, hold
different notions of what a flow or a kanban is. The flexibility therefore
lives in the hook features: a user describes the flow and its rules in a
markdown file.

**Mechanism in code, semantics in markdown.** Engine code carries the
evaluator; the user's markdown carries every concrete flow concept.

1. **No baked folder names.** No engine path decides where user content
   lives; no folder name is a validity predicate on user markdown. A user's
   authoring directory is read from their markdown, defaulted in code at
   most.
2. **No baked flow vocabulary.** No status word, state-key name, card,
   kanban, role or lane name in an engine decision: not in a comparison, a
   refusal string, or bytes written into the user's tree.

Flow semantics live in user pages: rule and hook pages on the policy and
effects planes, and the frontmatter of the page a verb is invoked on. They
name the folders, the states, the key that spells state, and the prose; the
engine reads them as data. This extends `wire-contract.md` §1.1 ("zero
consumer concepts") and §11 (*pack data behind a generic manifest; no
evaluator hard-coded*) to the whole engine.

**Generic evaluation.** The engine matches on structure (a frontmatter key
exists, a selector resolves, a pin verifies, a rule fires), never on a flow
literal; concreteness is a user value echoed back unread. Model:
`preset::DEFAULT_ROOT_RECORD` is a fallback, `fm_scalar(&doc, "root")` is
the answer. Every flow-touching site should read like that one.

**Boundary.** The engine's own vocabulary is mechanism, and this law does
not cover it: its
state and mount convention (`MERIDIAN.md`, `.meridian/`, receipt paths, the
daemon's socket and state files) and its verdicts (`realise`'s `converged` /
`drifted-fixed` / `non-convergent`, the rules registry's `collision`). What
the engine writes into the user's tree or reads as the user's law is
semantics; what it keeps for itself is mechanism. Tests and fixtures may use
concrete flow words.

**Three sites where the concreteness lives in the user's markdown**, each
with its own gate:

- **`realise`'s board directory:** generic in the library
  (`RealiseSpec::board_dir`); the CLI seam reads `realise.board_dir` off the
  realising page and defaults in code only, never a pinned `"board"`. Gate:
  `crates/mrd/tests/realise_cli.rs`.
- **`realise`'s `render_card`:** the claim's `realise.card` template page
  supplies the card's entire vocabulary via the one template mechanism
  (`preset::template_of` + `preset::fill_slots`); the engine fills only its
  slots (`{{selector}}`, `{{rule}}`, `{{detail}}`, `{{now}}`, `{{actor}}`).
  An unresolvable declared template refuses the mint; the baked body mints
  only with no template declared. Gate: the card-template scenarios in
  `crates/realise/tests/scenarios.rs` (the matchability receipt uses the
  engine's own `FieldEquals` on the user's `status:` spelling).
- **`preset`'s floor prefix:** `FLOOR_PREFIX = "conventions/"` would be a
  folder name as validity predicate, so the constant is
  `DEFAULT_FLOOR_PREFIX`, the fallback behind the def's own `floor:` key
  (`run-plane.md` § 6, Law 6.3); `pins_floor` measures pins against
  `PresetDef::floor_prefix`, so `standards/` is as valid as `conventions/`.
  Gate: `crates/preset/tests/gates.rs`.

## Amendment — the one state owner (fingerprint grain)

Law: the sentence below; its merkle-spec half is
`node-rev-merkle-spec.md` §6.3.

> **The workspace naming tree, the parsed world, the journal seq, and every
> minted generation advance under one generation name `(instance, seq)`, in
> one act, owned by one per-workspace state owner, so tree, parse cache,
> journal, and tokens never skew.**

- **One act.** The state-owner step applies a commit's settled delta to the
  resident tree, the parse cache and the journal, and assigns the next
  `(instance, seq)`. Roots chain
  (`commit[n].root_before == commit[n-1].root_after`). Only that µs in-memory
  advance plus the journal append is linearized (group-committable). Staging,
  validation and durability I/O run outside it, parallel and unordered across
  disjoint writers.
- **One name.** A script's pinned entry generation, a node's `last_seq`
  stamp, a checkpoint binding, and a delta frame carry the same
  `(instance, seq)`.
- **The audit edge.** `last_seq` stamps and digests share one guarded write
  path, so the hash instrument audits the stamp instrument
  (`node-rev-merkle-spec.md` §6.3). Stamps are instance-bound: an instance
  mismatch degrades to the content-fold compare. Hash tokens are epoch-free;
  cursors are not.
- **The home is the registry/serve seam;** the law binds whichever home: no
  second place may advance any of the four.

The linearized step is built in two halves: publication in
`crates/wire-serve/src/publish.rs`, the lease in `authority.rs`.

- **Reservation algebra.** Admission atomically reserves the complete
  premise/read region set `R` — the regions the plan relied on reading — and
  the physical write region set `W`.
  Compatibility is conventional OCC (optimistic concurrency control): `R/R` is
  compatible, while `W/W`, `R/W` and `W/R` conflict on a spatial
  intersection. A root read intersects every write; a folder
  premise intersects writes at or under it; a point-file premise does not
  block a disjoint folder; an absence premise reserves the exact parent/name
  edge. Target-only reservation is insufficient when a root, ancestor, absence,
  enumeration, selector, or sql premise shaped the plan. Overlapping callers
  wait inside the authority (never `workspace_busy`); disjoint callers never
  wait. Staging may precede the reservation, which holds from final reverify
  through visible renames, the state-owner step, and durable finalization.
- **Durable intent.** Before the first visible rename, each transaction
  creates one checksummed `O_EXCL` intent (`.meridian/intents/<txn>`, outside
  the hash domain). After that intent lands, no error is a refusal: the result
  is `commit_unknown` until recovery proves completion or restores the complete
  declared set. A destination matching neither old nor new identity is
  `recovery_ambiguous` (quarantine, never guess). Group commit may share a
  durability flush only if per-member decision ordering and failure
  attribution survive: a member rejected before the group manifest is an
  individual refusal; an indeterminate member is `commit_unknown` under its
  own id; one member's semantic error never lands on a neighbor.
- **Pre-image verify is the second-writer refusal.** Writes are
  daemon-routed, so the write flock is off the publish path; the
  `apply_batch` pre-image compare (content + receipt) refuses a second
  in-process writer inside the one authority. Editors, git, and bash remain
  the external-race residual.
- **The µs step.** The state owner applies each settled path delta to the
  then-current authenticated tree (never the planning `root_before`) and
  assigns a contiguous `root`/`seq`/frame, with no disk sync in that mutex.

## Amendment — the fsync class (fingerprint grain)

Law: the class below.

> **Plain `fsync(2)` is the durability class of every sync site on every
> platform. `F_FULLFSYNC` is never issued, neither on the ack path nor as a
> background flush. Drive cache is accepted: the engine makes no power-loss
> or platter-safety claim.**

- **macOS:** std `sync_all` and `sync_data` both issue `fcntl(F_FULLFSYNC)`
  (std 1.97.1, aarch64), so no std call gives the ruled class there. A sync
  site uses `fs::honest_sync` /
  `fs::honest_sync_path`, or `libc::fsync` with a checked return where its
  crate does not depend on `fs`; never `sync_all`/`sync_data`.
- **Linux:** already complies — `sync_all` is `fsync(2)`, `sync_data` is
  `fdatasync(2)`, zero fcntl.
- **`F_BARRIERFSYNC`** is never a substitute: it orders without promising
  durability.
- **Return values are checked:** a failed `fsync` is an `io::Error`, never a
  dropped rc. Whether a site then propagates the error or stays best-effort
  is that site's stated policy; dir-sync-after-visible-rename sites never turn
  a committed write into a reported failure.
- **Scope: local disk only,** no NAS, no network mounts. `ENOTSUP`/`ENOTTY`
  on a network mount is a recorded limit, not a built fallback.
