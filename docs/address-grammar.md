---
type: spec
id: addr
status: standing
updated: 2026-08-18
description: Normative spec for cross-root addressing, the mount table, and the `addr::Addr` type. Ships no engine code.
owns: [cross-root addressing, mounts, "addr::Addr"]
---

# The address grammar — the cross-root address type and the mount-table law

> **Standing:** Design law is `wire-contract.md` (one contract). Mint addresses = segments only. Receipts = armed wire facts. DuckDB/`view_path` not agent core. **Doc correct > code correct; docs first.** See `README.md`.

**Status: normative SPEC** (ships no engine code). Where this document rules, the implementer has no design decision left; open points are named in § 10 with an owner surface inside this tree.

**Scope (read before §2).** This document owns the **cross-root / mount** grammar:
`[root:]path[#selector]`, the `addr::Addr` type, mount-table invariants, and
stored-form translation positions. It does **not** redefine mint-plane section
addressing. On the wire, a section is addressed as **segment objects**
(`{"hpath":[{"h":"Goals"},{"h":"Q3"}]}`) or `{"anchor":"…"}` / `{"fm_key":"…"}`
— never a joined mint-plane path (`Goals>Q3`, `Goals/Q3`, sanitized slug) as the
writeable form (`wire-contract.md` §2.1). The optional `#selector` on an agent-plane
`Addr` is an ingress/host-face slot; its machine-canonical resolution is still
segment / anchor form.

**Law already fixed here (not external):** the agent/stored split (`root:` vs
`obsidian://` stored form), name ownership (root declares, `MERIDIAN.md` binds —
see `meridian-md-schema.md`), the grey rule (unmounted ≠ red missing),
canonicalize-at-bind, refuse equal-or-nested mounts, and grey refusals on exit 1
with a distinct reason word. Do not re-litigate those by citing out-of-tree files.

---

## 1. The four senses of "root", disambiguated by name

Three senses of "root" already meet in this milestone and a fourth shares the spelling. A type
named `Root` collides with a shipped wire type, so this document names its own and states the
disambiguation here rather than leaving a reader to infer it.

| Name | What it is | Where it lives | Shape |
|---|---|---|---|
| `wire::Root` (type name) / **fingerprint** (design noun) | A **Merkle content-hash cursor** — the whole-domain content hash the world-guard compares. **Standing design name is `fingerprint`** (`wire-contract.md`); the Rust type may still be spelled `Root` (code lag) | `crates/wire` | `"b3:" + 64 hex`, opaque, equality only |
| `fs::WorkspaceRoot` | **One on-disk directory** — the single workspace every path is joined onto today | `crates/fs/src/lib.rs:33` | `PathBuf` |
| **`addr::MountName`** | **A canonical root NAME** — the mount-table key a cross-root address carries (`sessions`, `assets`) | `crates/addr` (NEW, § 7) | a lowercase name, never a path and never a hash |
| `root:` the frontmatter key | A **preset-def property** naming the root RECORD a session preset instantiates | `crates/preset/src/lib.rs:232`, fixtures at `crates/preset/tests/gates.rs:15`, `:321` and `:524` | an ordinary YAML scalar (`root: SESSION.md`) |

**`addr::MountName` collides with none of them**, and the collision it avoids is deliberate: it is
neither a hash (`wire::Root`) nor a directory (`fs::WorkspaceRoot`) nor a document property
(`root:`), and the word **Name** says so at every call site.

One adjacent name to keep apart: **`run::address::AddressError`** (`crates/run/src/address.rs:45`)
is the run plane's *task-binding* parser for same-file block refs `[[#^id]]` — a different grammar
with a different owner. This document's error type is **`addr::AddrError`**; the two never meet.

---

## 2. The type — D3 and D5 ruled

### 2.1 Neither D12 story as written

- **Story B — RIDE** keeps the prefix inside the spelling and resolves it by the same
 three rules. The rules are real — they live in `three_rules`
 (`model/src/lib.rs:1856-1871`): `docs.contains_key(spelling)`,
 `docs.contains_key(spelling + ".md")` and `resolve_linkpath(...)` — **all lookups into
 one root's corpus, a `BTreeMap<String, Document>` whose keys carry no root.** A
 `root:page` spelling misses all three and renders `red selector-unresolved`.
 Unimplementable as written: the story has no mount lookup. (In the shipped design the
 same three rules run only after the root is peeled and the corpus selected —
 `resolve_ref`, `model/src/lib.rs:1767-1782`.)
- **Story A — PEEL** strips a leading root textually at the lock face and then
 **discards it**. No such splitter ever shipped — the story's named helper exists
 nowhere in the tree, and a peel that discards has nowhere to put the root. The lock
 face does the opposite: the root stays ON the spelling until resolution (pinned by test —
 *"the root stays ON the spelling until the lookup is root-aware"*,
 `view/src/read_face.rs:840-841`), is readable as a VALUE from the parsed address
 (`declared_addr`, :847), and `LockItem` (`view/src/read_face.rs:294-334`) carries it
 in `to_root: Option<addr::MountName>` (:326) — the peel happens at resolution time
 (:398-400), into a field, not the void.

**Ruled: the address becomes a fallible TYPE carrying an optional root, and the resolver takes a
root-keyed corpus.** Construction is fallible, so the compiler produces the door list (R5): a
string convention that 16 sites re-parse is exactly R5's *"boolean helper a caller may ignore"*.

### 2.2 The type

```rust
// crates/addr — a std-only leaf, zero dependencies, UPSTREAM of `syntax`.

/// A canonical root name: the mount-table key. Lowercase `[a-z0-9-]`, non-empty.
pub struct MountName(String);

/// The agent-plane address `[root:]path[#selector]`, parsed.
pub struct Addr {
 root: Option<MountName>, // None = the ambient root
 path: String, // never carries a root prefix, by construction
 selector: Option<String>, // verbatim after the first `#`, to the END — `@` is a selector byte (§ 4.4)
}

pub enum AddrError { /* the closed set of § 4 */ }

/// The resolution-facing projection of the mount table: which canonical names
/// this machine binds. Constructed by `config` (§ 7), consumed by `model`.
pub struct MountSet { /* the bound names */ }
```

**Construction is the only way in.** `Addr::parse(&str) -> Result<Addr, AddrError>` is the sole
constructor; there is no `Addr::from_parts` a caller can use to smuggle an unparsed prefix into the
`path` field. The invariant *`Addr.path` carries no root prefix* is enforced at construction and is
what makes § 5's body-level guard checkable.

**Parse is not resolve.** `Addr::parse` answers *"is this a well-formed address?"* — it never
touches the mount table. Whether the named root is *bound* is the resolver's question, and its
answer is grey (§ 6), not a parse error. Conflating the two would make a well-formed address to an
unmounted root indistinguishable from a malformed one, which is the false-negative § 6 exists to
prevent.

---

## 3. The three-way translation invariant

The mount table is the **single authority** for the three-way map — canonical root name ↔ Obsidian
vault name ↔ local path — and it is a map in **all three directions**:

> **INV-1 (name is a key).** No two entries share a `MountName`.
> **INV-2 (path is a key).** No two entries share a canonicalized local path.
> **INV-3 (vault name is a key).** No two entries share an Obsidian vault name.
> **INV-4 (no containment).** After canonicalization, no bound path is equal to, or a
> path-segment-boundary prefix of, another bound path.
> **INV-5 (declared = bound).** A root's own self-declaration of its canonical name equals the name
> `MERIDIAN.md` binds it to, or the parse fails loud. Absent declaration renders grey; it is not a
> mismatch (D7 — *"MERIDIAN.md binds, it doesn't baptize"*).

Stated as a property with its negative cases:

| # | Input | Required outcome | Class |
|---|---|---|---|
| T1 | two entries named `sessions` | **parse fails loud, no partial mount table** | `duplicate-mount-name` |
| T2 | two entries with the same canonicalized path | **parse fails loud** | `duplicate-mount-path` |
| T3 | two entries with the same vault name | **parse fails loud** | `duplicate-vault-name` |
| T4 | `/a/wiki` and `/a/wiki/sub` both bound | **parse fails loud** (INV-4) | `nested-mount` |
| T5 | root declares `wiki`, table binds it as `field-notes` | **parse fails loud**, naming both spellings | `declared-bound-mismatch` |
| T6 | root declares no name | **grey for that root**, the missing declaration named | `grey(undeclared)` |

INV-1…INV-3 make the map a bijection, so "which name does this path have" and "which path does this
name have" each have exactly one answer. **A silent pick would make stored links machine-dependent**
— the failure §1a of the ratified decision exists to prevent.

---

## 4. The colon law — (d), and it is ruled here, not by the implementer

`sessions:notes.md` is a **legal filename on this machine today** (verified: § 11.3), and
`wire::Path` does not validate — its own doc says *"this newtype does not validate, it names"*
(`crates/wire/src/lib.rs:29-31`). `path_confined` (`crates/wire-serve/src/write.rs:1984`) rejects only
empty / leading-`/` / `.` / `..` segments. So there is no `:`-before-path validation anywhere, and
one string has two readings. **This document states which reading wins.**

### 4.1 The law

Consider the **head** of an address: the text before the first `/` and before the first `#`.

> **The root reading wins, unconditionally, and there is no fallback to the literal reading.**
>
> - **Zero `:` in the head** → the address has **no root**. It resolves in the ambient root. This is
> the overwhelming majority of refs and is unchanged.
> - **Exactly one `:` in the head** → that colon **is the root separator**. The text before it must
> be a well-formed `MountName`; the text after it must be non-empty. If either fails, the address
> is **REFUSED** — it is *never* reinterpreted as a literal path.
> - **Two or more `:` in the head** → **REFUSED**. Exactly one colon may act as a separator.
>
> After the first `/`, a `:` is an ordinary path byte and carries no meaning.

**Why root-wins and not a charset-dependent branch.** A rule where the reading flips on whether the
prefix happens to be a legal `MountName` is deterministic but subtle, and subtlety here becomes an
improvisation at sixteen call sites. One rule with three arms is checkable by reading it.

**Why no fallback.** A fallback would mean a typo'd root name (`session:notes.md` for
`sessions:notes.md`) silently degrades into a lookup for a file literally named
`session:notes.md` — a wrong **success** of exactly the cross-root misresolve defect's shape. Refusing is the only
outcome that cannot be mistaken for working.

### 4.2 The consequence, stated rather than discovered

A corpus-relative path whose **first segment contains `:`** cannot be named by any address.

> **The engine refuses to CREATE such a path**, and an existing one on disk renders **grey**, with
> the reason naming it unaddressable. It is never silently resolved, and never silently skipped —
> a document the corpus holds but no address can name must be visible as such.

**Amended 2026-08-18 (rooted-refs-everywhere — § 4.6).** The consequence above is about the
LITERAL path: a first-segment `:` on disk stays unaddressable (D10), and no address can spell one
into existence — every head-colon spelling IS an address (§ 4.1). What the amendment changed is
the DOOR: a `root:`-bearing spelling arriving at a page-taking door is no longer refused there —
it **resolves** as the address it is, root peeled, rel joined onto the named root's bound
workspace. D11's old outcome (refusal at the write door) is superseded; the superseded wording
and the ruling live in § 4.6. The wire is untouched: a raw head-colon `Path` ON THE WIRE still
refuses (`addr::confined`), because the wire speaks corpus paths and the address plane resolves
at the door.

### 4.3 `MountName`'s charset — ruled

`[a-z0-9-]`, non-empty. Lowercase only, and **an uppercase byte REFUSES rather than being silently
normalized**: a silent normalization would make two spellings one name, and the ratified law is one
name per root, used everywhere. The corpus index already lowercases its keys (`basename_lc`,
`crates/model/src/lib.rs`), so a normalizing `MountName` would be a second, invisible case rule on
the same address.

### 4.4 Law A-2 — the fragment is selector bytes to its end; `@fp` is off the name lane

**Amended 2026-08-07 (August team-e multi-root contract, Law A-2; supersedes
the previous § 4.4, which had `Addr::parse` split selector from fingerprint at
the first `@` in the fragment).**

The agent-plane grammar is **`[root:]path[#selector]`**. A fragment runs from
the first `#` to the end of the spelling, and **every byte of it is selector
bytes — `@` included.** Fingerprint pinning is its own field on whatever
surface carries one (a lock row's pin, a render-face decoration); it is never
an in-band suffix parsed out of the name lane. This is the packing law applied
one field to the right: heading text is an open charset, and an in-band
delimiter drawn from the charset it delimits is subject to the very collision
it exists to solve.

**The corpus motive, pinned.** 3,815 real headings across both roots contain
`@` (60 wiki-root; fence-aware at `field-notes@14bfc98bc` +
`field-notes-sessions@126d68cd5` — team-e census, register V11). Under the
superseded split, every one of them is unaddressable by its own spelling; and
for a constructible pair (`Deploy`, `Deploy @ prod`) a trimming resolver
resolves the wrong *real* section — the silent-wrong class, not a miss.

**What this narrows, stated rather than discovered.** A decorated render-face
spelling pasted back in — `page.md#Sec@green.b3…` — no longer has its
`@green.…` tail recognized and stripped. The whole fragment is selector
bytes, misses byte-exact, and lands in the Law A-3 teaching refusal, which
republishes the machine address (and the near candidate where the ranker
finds one). One taught round trip on that ingress, never a silent resolution.
No stored surface carries an in-band `@fp` for this to break: the engine
refuses an fp reaching stored bytes (S10 — *"a render-face decoration the
engine mints on read, never storable content"*,
`crates/wire-serve/src/write.rs:2356`), and the lock's pin is already its own
field. The old recognition served ingress only, and ingress now teaches.

**What this widens.** `#Deploy @ prod` resolves byte-exact to the real heading
`Deploy @ prod`. The 3,815 `@`-bearing headings become addressable by their
own spelling on the name lane.

**The type follows the law.** `addr::Addr` drops its `fp` field; `Addr::parse`
records the fragment verbatim as the selector; `Display` round-trips without
an `@` re-join (a selector containing `@` prints as its own bytes).

### 4.5 The negative cases, as negative cases

| # | Input | Required outcome | Refusal text / error class |
|---|---|---|---|
| D1 | `sessions:notes.md` | root `sessions`, path `notes.md` | parse OK — the mount lookup decides the rest |
| D2 | `notes.md` | no root, path `notes.md` | parse OK — ambient, unchanged |
| D3 | `dir/a:b.md` | no root, path `dir/a:b.md` | parse OK — the colon follows the first `/` |
| D4 | `sessions:24-01/notes.md#Design` | root `sessions`, path `24-01/notes.md`, selector `Design` | parse OK |
| D5 | `My Notes:draft.md` | **REFUSED** | `AddrError::BadMountName` — *"refused: 'My Notes' is not a canonical root name — root names are `[a-z0-9-]`. Fix: quote the path differently or rename the root; see [[address-grammar]]."* |
| D6 | `Sessions:notes.md` | **REFUSED** (uppercase, § 4.3) | `AddrError::BadMountName` |
| D7 | `:notes.md` | **REFUSED** | `AddrError::EmptyMountName` |
| D8 | `sessions:` | **REFUSED** | `AddrError::EmptyPath` |
| D9 | `a:b:c.md` | **REFUSED** (two colons in the head) | `AddrError::AmbiguousColon` |
| D10 | a corpus file at `sessions:notes.md` on disk | **grey**, unaddressable, named | `grey(unaddressable-path)` |
| D11 | a `create`/`splice` targeting `sessions:notes.md` | **RESOLVES through the rooted lane (§ 4.6)** — root `sessions`, rel `notes.md`, joined onto that root's bound workspace; the wire sees the rel half only | re-ruled 2026-08-18; a raw head-colon `Path` arriving ON THE WIRE still refuses `bad_path` |

**The acceptance half (parse-acceptance), asserted in the same breath:** D1–D4 must PARSE. A grammar proven
only by what it refuses is indistinguishable from one that refuses everything, and rows D2 and D3
are the ones that keep this law from swallowing the ordinary corpus.

> [!WARNING] D11 is re-ruled (rooted-refs-everywhere, ZT 2026-08-18) — the door RESOLVES what it used to refuse
> Superseded wording, preserved as the record (it lived here as D11's outcome and, mirrored, at
> the write door — `crates/wire-serve/src/write.rs`, `path_confined`'s doc): *"The head-colon arm
> is part of confinement because a `root:` prefix selects WHICH tree a path is joined onto: a
> `root:`-bearing spelling at a write door is an address, never a corpus path, and is refused
> rather than creating a document no address can name (§4.2, D11)."*
>
> The first half survives untouched — a `root:`-bearing spelling IS an address, never a corpus
> path; § 4.1's root-wins law is not amended. The refusal half is overturned, and its stated
> motive is engaged rather than dodged: the refusal existed because admitting the spelling as a
> PATH would have created a document no address can name. Resolving the spelling as the ADDRESS
> it is removes that hazard at its root — the created document is named by `sessions:notes.md`
> from everywhere and by `notes.md` inside its own tree, and the raw spelling never reaches a
> path join. The unnameable-document class cannot occur through a door that resolves. What still
> refuses: malformed heads (D5–D9, unchanged), an unbound or typo'd root (the rooted lane's
> teaching refusal — never a fallback to a literal reading), the literal first-segment-`:` path
> (D10 and § 4.2, unchanged), and a raw head-colon `Path` on the wire (an address that missed its
> door). Ruling, scope, and authority: § 4.6.

### 4.6 The rooted lane spans every page-taking door (rooted-refs-everywhere, ZT 2026-08-18)

**Ratified — ZT, session `1ed46083`, 2026-08-18, typed verbatim:** *"let's brief 8fb49fe6 and
ef7c6b05 to add root: resolves. including run / put / pin / realise / etc, anything that we can
take a page as refernece, we should be able to resolve it."* Motive — ZT verbatim (AUQ, session
`ef7c6b05`, same day): *"the root: ref is clear and no ambiguios, for mrd tool, the runtime cwd
should not be a factor to decide the behavior. this buys us durability."* Full record: llm-wiki
`decisions/2026-08-18-rooted-refs-everywhere.md`.

> **The law.** Every door at which the caller names a PAGE resolves the agent-plane
> `[root:]path[#selector]` spelling, through the ONE rooted lane (parse → confinement → mount
> table; the § 4.1 colon law, no fallback to a literal reading). No page-taking door refuses a
> well-formed rooted ref as such — with ONE stated exception, the preset lane below, which
> refuses with a teaching until it is converted — and none misreads it as a literal filename.
> The family is bound by the predicate, not by a list — the wire contract's door-family shape
> (`wire-contract.md` § 12.1, whose own dogfood showed an enumerated family is read as a subset)
> — so a door that is born taking a page is born inside this law.

Members at the amendment date (mirroring the impl door audit, ACKed 2026-08-18, corrected same
day): already rooted — `read`, `fingerprint`, `resolve`, `put --scope`. Converted by this ruling
— `run`, `walk`, `repair`, `realise`, `links`, `rules` (the read side); `put` (the write
TARGET), `rm`, `pin` (the PAGE position — the TARGET was already cross-root,
pin-cross-root/D-A), and `script --files` (each entry `root:path`-capable under the
one-declared-root convergence below). In the family but NOT YET converted — the preset lane
(`unfold`, `reconcile`, `new`): see the stated exception below. Outside the family by predicate
— arguments that name no page: `arm --at` (a directory scope, `armed-plane.md`), `test
--history` and `status --cwd` (explicit tree arguments), `test --corpus` (its SPEC is a plain
fixture FILE read from disk with cwd/absolute semantics, everything in it resolved relative to
the spec's own directory — a compiler-source-style argument, not a corpus page; measured at the
spec loader. The rule pages a spec names INSIDE itself are document positions, not door
arguments — the § 9.1 distinction — so they are outside too), `check`, `retire`, `skill`, `sql`
(no page; its `--root` is a new workspace selector, not a page ref). The enumeration is a
MEASURED snapshot, not a designed set — two members moved after the first ACK when their
resolution was read at the seam (`new` in, `test --corpus` out) — so the predicate stays the
authority and a later reader re-measures rather than trusting this list.

> **Authority: the page's tree governs — ZT's ruling by AUQ selection, 2026-08-18, session
> `ef7c6b05`.** Scope of assent, stated precisely (the DX-01 standard — ratification carries its
> receipt): ZT **selected** the option labelled *"Page's tree governs (Recommended)"* — a clicked
> label, not typed prose — and the option text he was shown and selected read, verbatim:
> *"`root:x` behaves exactly as if you had cd'd into that root: caps/conventions load from the
> PAGE's own tree, receipts land in the page's workspace. Closes the one live hazard — today's
> code loads the permission ceiling from where you stand, so resolving root: without this would
> let a read-only tree's bash tasks run under a looser caller tree's ceiling (permission bypass).
> Matches shipped code shape: authority already follows the declaring root."* The alternatives
> shown and not picked: the standing tree governs; both trees must permit. His own typed prose in
> the same answer independently carries the principle: *"for mrd tool, the runtime cwd should not
> be a factor to decide the behavior. this buys us durability."*
>
> *Editorial note on the quoted option (review finding, 2026-08-18 — not part of the assent):*
> the option's last sentence, "authority already follows the declaring root", is misleading as a
> description of shipped code. In shipped code the declaring root RESOLVES TO THE STANDING
> workspace — `run_cmd.rs` sets `declaring_root = answer.root()`, and `workspace::Answer::root`
> is the discovery ladder's answer (where the caller stands; `None` on a cwd default) — so it
> coincides with the page's tree only for ambient refs, which is exactly the coincidence this
> ruling breaks: the PAGE's tree now fills the declaring-root slot. The quote stands unaltered
> as the receipt; the option's own earlier sentence ("today's code loads the permission ceiling
> from where you stand") is the correct description, and the scope of assent is unharmed — a
> real hazard class was named and three real alternatives were shown (on the hazard's
> illustration, see the second note).
>
> *Second editorial note on the quoted option (reopened post-merge, 2026-08-18 — not part of
> the assent):* the option's bypass ILLUSTRATION — read-only tree's *bash* tasks running under
> a looser caller tree's ceiling — is not a mechanism that exists. **Caps do not apply to
> bash** (`crates/run/src/caps.rs` module doc, citing `laws.md` § Amendment):
> `resolve_authority` takes the bash branch FIRST and consults only the builtin,
> non-overridable `READ_ONLY_PATTERNS` (`check-*`/`verify-*` — those names refuse a bash fence
> at load); every other bash task resolves `Authority::Unsandboxed`, and the conventions
> argument is never read on that branch — so a tree's declaration never governs its bash
> tasks, and there is no bash ceiling for a caller's tree to be looser than. The REAL bypass
> this ruling closes lives on the STARLARK path: `resolve_caps` consults the conventions
> loaded from the declaring root, so a read-only `run.caps.*` ceiling over a task's `md.*`
> writes was cd-swappable for a looser tree's — that is the hazard the authority law removes,
> and the one the impl gate exercises. Scope of assent unaffected: the hazard class is real
> and three alternatives were shown; only the illustration inside the option was wrong.
> Provenance: the asking chain misread the exit-triad's "a bash fence under a read-only
> convention" — which names the BUILTIN name-keyed fence — as tree-declared policy, and the
> misreading traveled from the provisional brief through the option text into this ratified
> quote.
>
> In consequence, as law: a rooted op behaves exactly as if the caller had cd'd into the named
> root — conventions and caps load from the PAGE's own tree, receipts land in the page's
> workspace, and the standing workspace contributes nothing to the ceiling.

**The mechanism is the workspace jail, and it is law, not implementation detail.** One resident
daemon serves many workspaces; `hello` carries the workspace and pins it **exact-or-refuse, with
no ancestor walk** (`registry.rs` `pin_declared` — *"a declaration never widens to an enclosing
registered workspace"*), and the connection stays attached to that workspace for reads and writes
alike. A rooted door therefore resolves the root FIRST and dials the resolved workspace — which
is why this amendment needs **no wire change**: the wire carries the rel half only, and the § 1
`Path` law (`wire-contract.md`) with its head-colon confinement arm stands unchanged. A raw
head-colon `Path` arriving on the wire is an address that missed its door, and refuses.

> **One stated exception — the preset lane is NOT YET converted (2026-08-18).** `unfold`,
> `reconcile`, and `new` name a page (`new`'s def token is a page position — *"resolve def
> (presets/<KIND>.md or page path)"*, its own help) but write IN-PROCESS, with no daemon dial —
> a rooted preset op would therefore write into a foreign tree without that tree's armed gates
> ever firing. Under this amendment's own authority law those gates are the target tree's
> right, so a lane that structurally cannot fire them cannot resolve rooted refs yet: a rooted
> ref there **REFUSES with a teaching** naming exactly this reason and the remedy (run it from
> inside the target tree). Recorded as a lane AWAITING CONVERSION, in open tension with the
> ruling's motive — these three doors stay cwd-determined until then — never as a lane
> correctly cwd-bound forever. The named route is the preset lane riding the daemon write path;
> when it does, the family predicate already covers it.

**Convergence, not invention.** The engine's customer face (the ccc-statusd MCP `script` tool)
already ships this rule for its own multi-file door, verbatim: *"Every files[] entry resolves
through one root; that root is the workspace; in-program paths are relative to it"* — same ref
grammar as its read/put. A program binds ONE declared root and in-program paths are that
root-relative, which is also why cross-root reads inside one program do not arise as a question.
The mrd doors converge on that shipped law rather than minting a second one.

**Face grammars differ deliberately — do not harmonize.** The MCP face admits absolute and
session-relative refs beside `root:path`; the mrd CLI's § 1 path law forbids absolute paths and
`.`/`..` segments, and **this amendment does not touch that**. A later reader must not
"harmonize" mrd into accepting absolute paths because the customer face does.

**What stays outside the family.** A door argument that is not a page reference: the armed
plane's arm root (a workspace-relative DIRECTORY — `armed-plane.md`) keeps its head-colon
refusal, and `sql` names no page at all — its cwd-independence (an explicit `--root` selector,
ZT's conditional directive) is CLI surface owned by the decision page, not address law.

### 4.6a The root a spelling names — name first, then alias (root-alias, 2026-08-23)

The colon law (§ 4.1) says a head colon names a root. This says **which** root — the one question
the mount table answers, and the answer is an order:

> **The law.** A `root:` spelling resolves to the mount whose `name` it equals; if no mount is named
> that, to the mount whose `alias` it equals (`meridian-md-schema.md` § 5.1b); otherwise it refuses
> as an unbound root. One order, at every door of the § 4.6 family, through the one seam
> (`mrd::rooted::resolve_name` at the CLI, `addr::MountSet::canonical` on the link plane) — so two
> doors cannot hold two opinions of one spelling.

**The motive is a constant a consumer can spell.** The engine knows no root names — the no-baked-names
law (`laws.md`) — so a skill or daemon wanting to address "the sessions tree" has had nothing to
write. `alias:` maps ONE agreed constant, `sessions:`, onto whatever each machine calls that tree,
without the engine learning the name and without `primary:` acquiring a meaning it never had (ZT,
2026-08-23: `primary` is not consulted for `sessions`, or for anything else).

**Name-first is what removes the special case.** A machine whose root is already NAMED `sessions`
declares no alias and resolves `sessions:` anyway — a name is its own alias. The order is never a
tie-break, because a table where a name and an alias both answer one spelling does not load
(`alias-shadows-name`, whole-table refusal).

**The canonical spelling is the NAME, and this half is load-bearing.** An alias is a lookup spelling
only; nothing the engine writes down or echoes back carries one:

| Surface | Spelled with |
|---|---|
| receipts, pins, `mint {…}` paths, `sub` rows | the mount `name` |
| a door's canonical `root:path` echo (`mrd resolve`'s `ref:` row) | the mount `name` |
| the stored form's `vault=` (§ 3's three-way map) | the mount's vault, reached by canonicalizing the alias FIRST |
| `mrd resolve`'s `root:` row; the `alias` column of `mounts` and `mrd config` | the alias, where it EXPLAINS a resolution |

So `mrd resolve sessions:x` answers `root: field-notes-sessions (alias sessions)` and
`ref: field-notes-sessions:x`. Were an alias to reach stored bytes, one machine's private mapping would
travel into shared content and mean nothing — or, worse, something else — on the next machine to read
it. That is the same defect § 3's three-way translation exists to prevent, arriving by a new door.

**Declared, not bound, picks the refusal.** An alias onto a root this machine declares but cannot
read reaches THAT root's state (`grey(path-unseeable)` and its detail), never the undeclared
refusal — § 6's two causes with two remedies, unchanged: the unreadable root's prescribed fix is
already done. Pinned by `crates/mrd/tests/root_alias.rs` (the four acceptance arms end-to-end) and
`addr`'s own `an_alias_onto_an_unreachable_root_carries_that_roots_state`.

---

## 5. The basename fallback — (c), the cross-root misresolve defect, and no unit owned it

`crates/model/src/lib.rs:1736-1737`, inside `resolve_linkpath`:

```rust
let key = linkpath.trim().trim_end_matches(".md").to_lowercase();
let base = key.rsplit('/').next().unwrap_or(key.as_str()).to_string();
```

`"sessions:24-01-retro/notes.md"` → `rsplit('/')` → base `"notes"` → **matches the ambient root's
`notes.md`.** The `sessions:` prefix is never examined; it is discarded with the rest of the path.
Reproduced first-hand on the installed binary (§ 11.1).

**This is inside the owner, not at a door.** A perfectly-typed `Addr` arriving at `resolve_linkpath`
still basenames onto the wrong file unless the body learns to peel and refuse — implementation's retype reaches
this function without fixing it.

### 5.1 Ruled: peel and refuse

> **C-1 — the fallback is intra-root by construction.** The basename fallback applies only *within
> one root's corpus*. It is reached only after the root has been peeled and the mount lookup has
> selected the corpus to search.
>
> **C-2 — a rooted address never falls back to the ambient root.** If the named root is unmounted →
> **grey `unmounted`** (§ 6). If the named root is mounted but the path names nothing in *that*
> root's corpus → **`file_not_found` for that root**. Never, under any input, the ambient root's
> same-basename file.
>
> **C-3 — the body carries its own guard, because the retype does not reach it.**
> `resolve_linkpath`'s `linkpath` argument must contain no root separator. A `linkpath` whose head
> carries a `:` is a **programming error at that seam**: the function refuses (returns `None`) and
> **that refusal is asserted by a test**, so a future caller reintroducing a raw `&str` cannot
> reproduce the cross-root misresolve defect silently.
>
> **C-4 — one address, one answer.** The two spellings in § 11.1 disagree today. After implementation they must
> converge on the **same** answer, and that convergence is the assert — not merely that each is
> individually non-wrong.

### 5.2 The negative cases

| # | Input | Today (measured, § 11.1) | Required |
|---|---|---|---|
| F1 | `[[sessions:24-01-retro/notes.md#Design]]` in `claim.md`, ambient root holds `notes.md`, `sessions` **unbound** | `-> notes.md (1)`, **exit 0** — a wrong success | **grey `unmounted`**, the teaching refusal of § 6, exit 1 |
| F2 | `[[sessions:notes.md#Design]]` (the no-slash control), `sessions` **unbound** | `-> sessions:notes.md (1, unresolved)` | **grey `unmounted`** — the SAME answer as F1 (C-4) |
| F3 | same as F1, `sessions` **bound**, target exists in that root | `-> notes.md` (wrong root) | resolves to the file **in `sessions`**, and the bytes come from there |
| F4 | same as F1, `sessions` **bound**, target absent in that root | `-> notes.md` (wrong root) | **`file_not_found`** scoped to `sessions` — never the ambient file |
| F5 | a raw `&str` with a `:` head reaching `resolve_linkpath` directly | basenames onto the ambient root | **`None`**, asserted by a test (C-3) |

**Acceptance half:** F3 must RESOLVE, and criterion 3's standard applies to it — the resolved
**bytes must come from the target root**, proven by editing the file in that root and observing the
change. A build that renders everything grey satisfies F1, F2 and F4 and ships nothing.

---

## 6. The unmounted-root refusal — (f), the verbatim exemplar

Refusal text is specified nowhere in the plan: the rule is named five times and one string is
produced. The house pattern is a **pinned `const` exemplar asserted verbatim** — the shipped
instance is `model::selector::D1_TEACHING_REFUSAL_EXEMPLAR`
(`crates/model/src/selector.rs:569`), rendered by `render_ambiguity` and pinned by
`render_ambiguity_carries_d1_teaching_verbatim` (`crates/model/src/selector.rs:929-961`), which
asserts the rendered text carries the exemplar's teaching tail verbatim.

**This document produces the unmounted-root exemplar in that shape.** Placement follows the
precedent: the grey model already lives in `model::selector`, so the new reason and its exemplar
live beside `GreyReason`'s existing members.

```rust
// crates/model/src/selector.rs — beside GreyReason's existing members.

/// A cross-root address naming a root this machine does not bind. Grey, never
/// red: nothing drifted; the ledger cannot see from here (cross-root-addressing
/// §1a). Carries the missing name so the refusal can teach the fix (D8).
Unmounted { root: addr::MountName },

/// The unmounted-root teaching refusal, carried VERBATIM as the provenance
/// anchor (cross-root-addressing §1a). `render_unmounted` reproduces this
/// wording with the real root and address interpolated; this const pins the
/// exemplar so a drift in the wording is a visible test failure.
pub const GREY_UNMOUNTED_REFUSAL_EXEMPLAR: &str = "grey(unmounted): root 'assets' is not mounted — the address 'assets:domains/media/logo.md#Design' names a root this machine does not bind. Not red: nothing drifted, you just cannot see from here. Refs to mounted roots remain served. Fix: declare 'assets' in ~/MERIDIAN.md as a mount entry (name / path); see [[address-grammar]].";
```

**The teaching tail the pinning test asserts verbatim**, exactly as `TEACH_TAIL` does for D1:

```
. Not red: nothing drifted, you just cannot see from here. Refs to mounted roots remain served. Fix: declare '<root>' in ~/MERIDIAN.md as a mount entry (name / path); see [[address-grammar]].
```

It **names the missing mount** and **teaches the fix**, per D8, and it carries §1a's ratified
sentence — *"Not red — nothing drifted; you just cannot see from here"* — rather than paraphrasing
it.

### 6.0b One remedy per plane, and the `Fix:` answers the failure it is attached to (2026-08-09)

`docs/meridian-md-schema.md` § 8.3 requires a `Fix:` that **names the legal form**. Two failures at
the selector doors look similar and take opposite remedies, and the difference is what this section
exists to state:

| failure | what the caller holds | the remedy |
|---|---|---|
| **miss** — no node answers the selector | an address that does not resolve | **discovery**: list the sections, feed a row back |
| **ambiguity** — the selector resolved TWICE | an address that resolves too well | **disambiguation**: pin one occurrence, or make the duplicate ids distinct |

A discovery clause attached to an ambiguity sends the caller to look up an address **they already
typed**, so the sentence labelled `Fix:` is the one sentence in the message that cannot help. That is
a taught-recovery loop even when every other clause is correct — and it is worse than an absent
remedy, because the authoritative-looking clause is the circular one.

**Each ambiguity plane therefore publishes ONE remedy sentence, and every door renders those bytes:**
`selector::AMBIGUITY_FIX` (heading plane, carried inside `D1_TEACHING_REFUSAL_EXEMPLAR`) and
`selector::ANCHOR_AMBIGUITY_FIX` (anchor plane, inside `ANCHOR_AMBIGUITY_REFUSAL_EXEMPLAR`). The
constants exist because **two doors answer each of these failures** — the write door through the
exemplar renderers, the read face through its aggregate `Fix:` — and a remedy spelled twice is a
remedy that drifts once. Sharing the bytes puts both doors under the exemplar assertions that
already exist.

> **Measured receipt (dogfood pass 1, f05).** The write door rendered both exemplars verbatim while
> the read door derived its `Fix:` from the MISS vocabulary for **every** failure kind — so the
> duplicate-anchor AND duplicate-heading refusals both demoted their real remedy into a parenthetical
> and labelled a discovery step `Fix:`. The exemplars were asserted the whole time: the assertions
> covered the renderers, and **the read face did not call them.** An exemplar constrains only the
> doors that render it.

### 6.1 The vocabulary is grey-exit-1's, and it is not re-spelled here

> Grey **refuses**. It rides **exit 1**, with the distinct reason word **`grey(unmounted)`** — in
> the human line **and** in `--json`. **No fourth exit code.** `--force` is the escape, already
> ratified.

The sibling reason words this must not collide with, and must not be unified with:
`grey(cannot-assess)` (the `mrd check` verb-level state, cannot-assess) and `red(...)`. **D8a holds: two
subsystems, one shared meaning** — the unmounted-root grey routes through
`model::selector::Color`/`GreyReason` (the address plane); `cannot-assess` is a verb-level exit
state on the validity plane. What they share is the law — *outside sight never renders as verified*
— not the type.

### 6.2 Grey outranks red

> **R-3.** When an address names an unmounted root, the verdict is **grey**, whatever else is true
> of the target. A cross-root pin that was green and whose root is later unmounted becomes **grey,
> never red** — nothing drifted; the ledger simply stopped being able to measure.

The inverse is refused categorically for the reason grey-exit-1 gives: grey → exit 0 would make
unmounting a root a way to convert a red into a pass, through an edit to `~/MERIDIAN.md`, which
cannot itself be attested (canonicalize-at-bind ③).

---

## 7. Crate placement — (e), D4/D4a, and the fact that makes implementation and implementation buildable

### 7.1 Two new crates, and why the split is forced rather than chosen

| Crate | Position | Owns |
|---|---|---|
| **`crates/addr`** (NEW) | a **`std`-only leaf, zero dependencies, UPSTREAM of `syntax`** | `Addr`, `MountName`, `MountSet`, `AddrError`; the colon law (§ 4); the parse. **This is where an address becomes a value.** |
| **`crates/config`** (NEW) | **downstream of `model`** | the `MERIDIAN.md` parse; the mount TABLE (name ↔ vault name ↔ path); canonicalize-at-bind; the deny-ceiling inheritance; the equal-or-nested refusal; the declared-vs-bound check |

Neither is `crates/workspace` — its charter is *"a leaf, `std` + `cache` only"* (`laws.md`
§ Crate charters), and it is **read, not moved**: `config` calls `workspace::deny_reason`.

**Why `addr` must be upstream of `syntax`.** `model`'s dependencies are `syntax` + `blake3`
(`crates/model/Cargo.toml`), so any type living in a crate that depends on `model` is
**architecturally unreachable from `syntax`** — and `syntax::split_wikilink_target`
(`crates/syntax/src/lib.rs:424`) is the wikilink ingress where a cross-root address actually
arrives. A single address crate placed beside the markdown parsing would make implementation's ingress part
impossible.

### 7.2 The tension D4 and D4a create, and its resolution — read this before writing implementation

D4a rules that **the mount table is INJECTED as a parameter into `model::CorpusIndex::resolve_ref`**,
which already takes `docs: &BTreeMap<…>` — extending the existing owner rather than relocating it,
because moving resolution out of `model` breaks its charter (*"the single address law its two
dependents share"*).

**But `config` is downstream of `model`. `model` therefore cannot name a `config` type.** Taken
literally, D4 and D4a cannot both hold.

> **Ruled: the injected parameter is `&addr::MountSet`, defined in the upstream leaf.** `config`
> owns the binding and **projects** its bound names into a `MountSet`; `model::CorpusIndex::resolve_ref`
> names only `addr`, which is upstream of `syntax` and therefore upstream of `model`. Both D4 and
> D4a hold, unmodified.

`resolve_ref` needs exactly two things from the mount table, and `MountSet` carries exactly those:
**is this name bound**, and **which names are bound** (so the refusal of § 6 can teach). It needs no
paths — by the time resolution runs, `config`/`fs` have already loaded each root's documents into
the root-keyed corpus, which is `model`'s own type keyed by `addr::MountName`.

**`MountSet` is a concrete type, not a trait.** A trait invites a second implementation, and a
second implementation of an address fact is the exact defect D12 keeps producing: one question, two
answers. One owner, one type.

**This subsection is the single highest-value line in this document for implementation and implementation.** Without it,
the placement is discovered at compile time, in six crates at once, by a worker who then has to
re-decide D4a under time pressure.

### 7.3 The charter rows `laws.md` is owed

`laws.md`'s crate-charter table is **exhaustive per-crate**, so each new crate owes it a row.
implementation and implementation write these; the sentences are supplied here so they are not improvised:

| Crate | Charter |
|---|---|
| `addr` | The agent-plane address: `[root:]path[#selector]` parsed into a fallible type carrying an optional canonical root name, plus the resolution-facing bound-name projection every plane resolves through. A `std`-only leaf upstream of `syntax` — it is where an address becomes a value, so nothing downstream re-splits a string |
| `config` | The `MERIDIAN.md` plane: the one entry point parsed as content (a rev and a fingerprint like any page), and the mount table binding canonical root name ↔ Obsidian vault name ↔ local path — canonicalized at bind, passed through the `workspace` deny ceiling, refusing equal-or-nested mounts, failing loud with no partial table. Downstream of `model`; it projects the bound names into `addr::MountSet` so resolution stays `model`'s |

---

## 8. The mount-path law — (b), canonicalize-at-bind

### 8.1 The rules

> **B-1 — canonicalize at bind.** Every mount path is canonicalized (symlinks resolved, trailing
> separators normalized) **before** it is bound. `workspace::deny_reason` canonicalizes both sides
> of its comparison (`resolve_ref`, `crates/workspace/src/lib.rs:384`), so an uncanonicalized bind
> would be checked against a path that is not the one it binds.
>
> **B-2 — inherit the deny ceiling.** Every canonicalized mount path passes `workspace::deny_reason`
> before binding. A refused mount **fails the whole parse** — no partial mount table, no
> default-root fallback. The reasons are `workspace::DenyReason`'s existing six
> (`crates/workspace/src/lib.rs:301-314`): `FilesystemRoot`, `HomeDir`, `TempDir`, `XdgBaseDir`,
> `CacheRoot`, `MountPoint`. **The ceiling is reused, never re-implemented.**
>
> **B-3 — refuse equal-or-nested mounts, including through symlinks.** After canonicalization, no
> two bound paths may be equal, and none may be a **path-segment-boundary** prefix of another
> (INV-4). Comparison is on canonicalized paths, so a symlink cannot smuggle one tree in twice.

### 8.2 The measured motive — a read-mint bypass, found before the code existed

On this machine, verified (§ 11.2): `/Users/Shared/repos/field-notes` is a **symlink** to
`/Users/Shared/projects/field-notes`, while `CCC_LLM_WIKI_PATH` carries `/Users/Shared/projects/field-notes/`
— the real path, with a trailing slash.

A literal env-var inversion (implementation) therefore mounts **one tree twice, under two names**. Two canonical
refs then name one document with **identical `sec_rev`**, and **the read-mint recheck cannot
distinguish them: a receipt minted on ref A would gate a pin on ref B.** That is a read-mint bypass.
B-1 and B-3 are what prevent it, and B-1 alone is not enough — the trailing slash and the symlink
are two different ways to spell the same tree, and only canonicalization collapses both.

*(Amended 2026-08-16 — the read-mint ledger is deleted; pin proof rides the request,
wire-contract § A.3.)* The motive above is historical narration and stands as measured. The
bypass CLASS it names survives translation: the pin's proof compare must run against the
**resolved TARGET root's** live bytes — never against an ambient root's same-named file —
which the resolver guarantees through the same B-1/B-3 canonicalization. A same-named file
with different content now fails the compare on its own bytes; two spellings of one tree
collapse to one canonical target, exactly as B-1/B-3 rule. The mount laws of this section
are unchanged by the ledger's death.

### 8.3 The negative cases

| # | Input | Required outcome | Class |
|---|---|---|---|
| M1 | a mount path at `$HOME` | **whole parse fails loud**, naming the reason | `DenyReason::HomeDir` |
| M2 | a mount path at `/`, `/tmp`, an XDG base dir, or under the cache root | **whole parse fails loud** | the matching `DenyReason` |
| M3 | `/Users/Shared/repos/field-notes` (symlink) **and** `/Users/Shared/projects/field-notes/` bound under two names | **whole parse fails loud** — one tree, two names | `duplicate-mount-path` (INV-2, after B-1) |
| M4 | `/a/wiki` and `/a/wiki/sub` both bound | **whole parse fails loud** | `nested-mount` (INV-4) |
| M5 | `/a/wiki` and `/a/wiki-two` both bound | **BOUND — this is legal** | — (`wiki-two` is not a segment-boundary descendant of `wiki`) |
| M6 | a mount path that does not exist or is unreadable | **grey for that root**, the path named; the table stays loaded | `grey(unmounted)` family — **not** a parse failure |
| M7 | a mount path without a `vault:` leg (a plain folder) | **bound** | — (§ 9, row 7; the kind taxonomy is retired — see § 10.1) |

**M5 is the acceptance half of M4** and it is why the prefix test is segment-boundary rather than
string-prefix: a naive `starts_with` refuses M5, and a mount law that refuses legitimate sibling
roots is a guard that blocks everything.

**M6 is deliberately not a parse failure.** A root being absent from *this* machine is the topology
working as designed; failing the parse there would brick the CLI on every machine that does not hold
all six roots.

---

## 9. The positional grammar — (a), what implementation's transform may touch

### 9.1 The four positions, exhaustively

An agent-plane address occupies exactly these positions and no others:

1. **a wikilink target** — the `dest` of `[[…]]`, owned by `syntax::split_wikilink_target`
 (`crates/syntax/src/lib.rs:424`);
2. **a markdown link URL** — the URL of `[label](url)`;
3. **`meridian-lock` `ref:` values**;
4. **`meridian-lock` `objects:` keys**.

These four are positions inside a DOCUMENT — the stored-form translation's whole scope. They are
NOT argv positions: which CLI arguments admit a rooted spelling is the § 4.6 door family's
question, a different plane. (Stated 2026-08-18 because the overlap of the word "position" cost a
reader a wrong turn.)

### 9.2 The transform is positional, never a byte transform — and the motive is measured

> **A-1.** implementation's stored-form translation is **positional**. It identifies each address by its
> position in the candidate document and translates the ones in its owned positions. **A blanket
> byte transform over the token `root:` is forbidden.**

**Because `root:` is already a live YAML frontmatter key in the shipped preset/def grammar.**
`crates/preset/src/lib.rs:232` reads it — `fm_scalar(&doc, "root")` — and fixtures carry
`root: SESSION.md` verbatim at `crates/preset/tests/gates.rs:15`, `:321` and `:524`. A blanket transform
would rewrite that line, corrupt the def, and — because it changes bytes inside a span — **silently
invalidate every pin whose fingerprint covers it.** Frontmatter is not an address position, and the
shipped code already says so in a neighbouring refusal: *"frontmatter is not a claim-link position
(S10/R22)"* (`crates/wire-serve/src/write.rs:2521`).

> **A-2 — the precedent is `strip_fp_candidate`, copied structurally rather than by analogy.**
> `strip_fp_candidate` (`crates/wire-serve/src/write.rs:2476`) identifies each token **in the
> candidate**, attributes it to the payload that supplied it via `classify_fp`
> (`crates/wire-serve/src/write.rs:2311`), and **refuses what it cannot place** — it never
> blanket-strips. implementation does the same for addresses: identify by position, translate the owned ones,
> and **refuse** an address it cannot attribute to one payload rather than transforming it blind.
>
> **A-3 — the assertion, R25's shape.** After translation the candidate carries **zero** agent-plane
> `root:` spellings in positions 1 and 2 — asserted **on the candidate**, on every write path, dry
> and real alike. This is the artifact guard (D9), not a guard on the `splice` verb.

### 9.3 The transform is the IDENTITY on two of the four positions — state this or it becomes an improvisation

The four positions of § 9.1 are all **address** positions — implementation's type must reach every one. But
**implementation's stored-form translation applies to only two of them.**

> **A-4.** The `obsidian://` translation applies to **positions 1 and 2** (wikilink target, markdown
> link URL). It is the **identity** on **positions 3 and 4** (lock `ref:`, lock `objects:`), by
> ratified law: *"Lock `ref:` and `objects:` keys use the canonical `root:` form (agent plane),
> never the URI"* (stored form law, § 9).

A worker reading "the positional grammar implementation's transform may touch" without A-4 would translate the
lock and break the ratified stored form.

### 9.4 The negative cases

| # | Input (a candidate document) | Required outcome | Class |
|---|---|---|---|
| P1 | frontmatter line `root: SESSION.md` | **byte-identical, untouched** | — (not an address position) |
| P2 | `` example `root:page` address `` inside a code span or fenced block | **untouched** | — (the document law calls it a code sample) |
| P3 | wikilink `[[sessions:notes.md#Design]]` | translated to the `obsidian://` stored form carrying the **vault name** | — |
| P4 | markdown link `[x](sessions:notes.md)` | translated to the `obsidian://` stored form | — |
| P5 | lock `object: "[[sessions:notes.md]]"` | **kept in canonical `root:` form — identity** (A-4) | — |
| ~~P6~~ | ~~lock `objects:` key `sessions:assets/logo.png`~~ | **POSITION RETIRED (implementation)** — see below | — |
| P7 | an address the transform cannot attribute to one payload | **REFUSED** | `bad_request`, the `strip_fp_candidate` shape |
| P8 | an `@fp` token reaching a stored URI or a display field | **REFUSED** | criterion-4 machinery, reused at the candidate |
| P9 | a hand-edited, malformed `obsidian://` URI on read-back | **fails loudly**, never guesses | reverse-translation refusal |

**Acceptance half:** P3 and P4 must TRANSLATE, and the agent-plane form must round-trip through the
stored form byte-identically. **Round-trip identity alone is satisfied by never translating at all**
— an identity function round-trips perfectly and ships nothing — so P3/P4's positive assertion and
P1/P2/P5's untouched assertion are one gate, not two.

> [!WARNING] P6 is RETIRED, and P5 is re-spelled (implementation, R4 schema v2)
> **P6's position no longer exists.** R4 removed the lock's top-level `objects:` table (session
> `86449b4e`, the 17:20 ruling): the blob hash moved ONTO the pin row, so there is no `objects:`
> key for the transform to leave alone. A row asserting an outcome for a position that cannot
> occur is not a conservative extra check — it is a law about nothing, and the next reader spends
> real time deciding whether the transform is missing a case.
>
> **P5 survives as a POSITION and changed only its SPELLING.** The lock's address key is now
> `object: "[[sessions:notes.md]]"` — a wiki link whose inner text is carried verbatim — where v1
> wrote `ref: 'sessions:notes.md#Design'`. The A-4 ruling is untouched: the lock's address stays in
> the canonical `root:` form and the translation there is the IDENTITY. Only the key's name and the
> selector's home moved (the selector is now the sibling `path` / `properties` array).
>
> This is a spelling correction inside a surviving row, not a new ruling — no position gained or
> lost an outcome. The retirement of P6 is the only change of law here.

---

## 10. Plan §6's edge-case table, walked row by row (Quality Gate 2)

Every row of the plan's shadow-path table, answered by this grammar. **No row is left to a
charitable reading, and rows this document does not own say who does.**

| # | Plan §6 row | Answered by | Answer |
|---|---|---|---|
| 1 | `root:` naming an unmounted root | § 6 | grey, exit 1, reason word `grey(unmounted)`, the § 6 exemplar naming the missing mount and teaching the fix |
| 2 | `root:` naming a mounted root, file missing | § 5.1 C-2, row F4 | `file_not_found` **scoped to that root** — a distinct class from grey, never conflated |
| 3 | A bare path (no `root:`) | § 4.1 zero-colon arm, row D2 | resolves in the ambient root, unchanged; this is the majority case and it is the acceptance half of the colon law |
| 4 | **A path that literally contains `:`** — *"must be ruled in implementation"* | **§ 4 in full** | root-wins on the single head colon, no fallback; ≥2 head colons refuse; a first-segment colon on disk is `grey(unaddressable-path)`; a write door targeting one refuses `bad_path`. Rows D1–D11 |
| 5 | Two roots declaring the same canonical name | § 3 INV-1, row T1 | parse fails loud, no partial mount table |
| 6 | One root mounted at two paths | § 3 INV-2 + § 8 B-1/B-3, rows T2/M3 | parse fails loud **after canonicalization**, so a symlink cannot smuggle it past |
| 7 | a root without a `vault:` leg (formerly `git-folder`) | § 10.1 below | **selectors are legal on every mounted root** — G-1 is RETIRED with the kind taxonomy (kind-sweep, ZT 2026-08-13); see § 10.1 |
| 8 | Cross-root pin whose target root is later unmounted | § 6.2 R-3 | **grey, never red** — nothing drifted, the ledger stopped being able to measure |
| 9 | A stored `obsidian://` URI hand-edited by a human | § 9.4 row P9 | read-back translation **fails loudly**, never guesses |
| 10 | `MERIDIAN.md` pins a root it declares, and that root drifts | **not this document's** | **Implementation owns it** (mount-as-claim, canonicalize-at-bind ③ — load-bearing, since the fence's only bypass is an edit to `~/MERIDIAN.md`). The address grammar has no part in it |
| 11 | Hook placed, `mrd` later uninstalled | **not this document's** | **Implementation owns it** — fail closed with teaching, `--no-verify` named in the message |
| 12 | Two worktrees, one hook dir, different meridian workspaces | **not this document's** | **Implementation owns it** (D11 — placed per git common dir; the workspace-root ≠ worktree-top-level case is a stated refusal in `mrd skill hook`'s document) |
| 13 | A subprocess forked while `DrawerLock` is held | **not this document's** | **Implementation owns it** — explicit `LOCK_UN` in `Drop` (R19) |

**No row of plan §6 is unanswerable by this grammar.** Rows 10–13 are answered by naming their real
owner rather than by this document inventing an address-plane answer they do not have — which is the
honest reading of Quality Gate 2, not an evasion of it.

### 10.1 G-1 is RETIRED (kind-sweep, ZT 2026-08-13) — kept as the record

G-1 read: *a `git-folder` root has no parse and no sections, so an address into one must not carry
a `#selector`* — refused `AddrError::SelectorOnOpaqueRoot` at resolution time.

Retired WITH the kind taxonomy, and not merely because its discriminator left the schema: the
rule's premise was false at the data level. The corpus loader built the same parsed index for both
kinds (`model::RootedCorpus::with_root` never branched on kind), so the refusal refused work the
corpus could do. With `kind` gone from `meridian-md-schema.md` §5.1, every mounted root is the same
shape to the resolver, selectors resolve on any of them, and `SelectorOnOpaqueRoot` is deleted from
the closed error set. The extinction pin is `crates/model/tests/kind_gate_extinction.rs` — a
re-added kind gate fails there first, mirroring the daemon's mountgate pin.

---

## 11. The measurements the laws rest on, and when they were run

Each measurement below was run on this machine, first-hand, and is dated. All
three pre-date the implementation they motivated — § 5.1's peel-and-refuse is
now the C-3 guard at `crates/model/src/lib.rs:1733-1734`, and the mount-aware
resolver is `resolve_ref` (`crates/model/src/lib.rs:1776`) — so re-running the
§ 11.1 reproducer today reaches the guard's refusal (asserted on the § 11.1
input verbatim by `crates/model/tests/u11_c3_linkpath_peels_and_refuses.rs`).
That is the ruling working, not the measurement being wrong: these record the
defect the laws were written against.

### 11.1 The cross-root misresolve, reproduced first-hand, with its control

Measured 2026-07-25, installed binary `/Users/caoer115/.local/bin/mrd`, in a
fresh git workspace at `/tmp/u3repro/ws`:

```
$ mrd links claim.md            # [[sessions:24-01-retro/notes.md#Design]]
workspace /private/tmp/u3repro/ws
  source: daemon
  claim.md
    -> notes.md (1)             ← RESOLVED, to the ambient root's file.  exit 0

$ mrd links claim2.md           # [[sessions:notes.md#Design]]  (the control)
workspace /private/tmp/u3repro/ws
  source: daemon
  claim2.md
    -> sessions:notes.md (1, unresolved)                                  exit 0
```

The defect **needs the slash**, and one address grammar gave two different
answers on the same plane. That divergence is what § 5.1 C-4 makes the assert.

### 11.2 The symlink topology § 8 is written against

```
$ readlink /Users/Shared/repos/field-notes
/Users/Shared/projects/field-notes
$ echo $CCC_LLM_WIKI_PATH
/Users/Shared/projects/field-notes/
```

A symlinked spelling and a real path with a trailing slash — one tree, two
spellings. § 8's B-1/B-3 are written against this measurement. Re-verified
unchanged on this machine 2026-08-12.

### 11.3 A `:`-bearing filename is legal on this machine

```
$ printf 'x\n' > 'sessions:notes.md' && ls
sessions:notes.md
```

Created successfully (re-verified 2026-08-12). Together with `wire::Path`'s
own doc — *"this newtype does not validate, it names"* — and `path_confined`'s
segment checks (citations in § 4), this confirms there is no `:`-before-path
validation anywhere, and that § 4 rules a live ambiguity rather than a
hypothetical one.

---

## 12. Implementer self-check

**The claim:** an implementer holding only this document (and `laws.md` for crate placement) can produce the type without inventing grammar.

| implementation needs | Ruled here |
|---|---|
| the type's name and shape | § 2.2 — `addr::Addr`, `addr::MountName`, `addr::MountSet`, `addr::AddrError` |
| its crate and position in the graph | § 7.1 — `crates/addr`, `std`-only leaf, zero deps, upstream of `syntax` |
| whether construction is fallible, and the only way in | § 2.2 — `Addr::parse` is the sole constructor; no `from_parts` |
| how the prefix is separated from the path | § 4.1 — the colon law, three arms, no fallback |
| the root-name charset and its case rule | § 4.3 — `[a-z0-9-]`, non-empty; uppercase refuses, never normalizes |
| what happens to `@` in a fragment | § 4.4 — selector bytes to the end; fingerprint pinning is its own field, never in-band (Law A-2) |
| the closed error set | § 4.5 — `BadMountName`, `EmptyMountName`, `EmptyPath`, `AmbiguousColon` (`SelectorOnOpaqueRoot` retired with G-1, § 10.1) |
| the parse/resolve split | § 2.2 — parse never reads the mount table; unmounted is grey, not a parse error |
| what `resolve_ref` receives | § 7.2 — `&addr::MountSet`, defined upstream so D4 and D4a both hold |
| the three ingress classes the compiler cannot reach | § 9.1's four positions |
| what the body-level guard must be | § 5.1 C-3 — `resolve_linkpath` refuses a `:`-bearing head, asserted by a test |
| the refusal wording to copy | § 6 — the pinned `const` exemplar, verbatim, with its teaching tail |
| which doors resolve a rooted ref, and under whose authority | § 4.6 — every page-taking door, predicate-bound; the page's tree governs conventions and receipts; the wire carries the rel half only |

**Decisions deliberately left open, and who owns each:**

1. **The `MERIDIAN.md` in-file syntax of a mount entry** — the block grammar and key names for
 name / path / vault name. **`meridian-md-schema.md` owns it**; this document constrains only
 the *invariants* the entries must satisfy (§ 3, § 8), never their spelling.
2. **The `obsidian://` URI's exact construction and percent-encoding**, and the round-trip identity
 gate over it. **the wire-serve stored-form seam owns it** (see § 9); § 9 rules only which positions it may touch and where the guard
 lands.
3. **`MountSet`'s API surface beyond `is_bound` and `bound_names`.** **The mount-table implementation owns it**, once the § 6
 refusal render is written and its real needs are known. Adding speculative methods now would be
 over-completion (do not invent API beyond stated needs).
4. **Whether a root's self-declaration lives in a frontmatter key or a named block.** **`meridian-md-schema.md` + config bind own the check** (declared-vs-bound, INV-5); this document rules only that a
 mismatch fails loud and an absence is grey (D7).

None of the four is required to write the type.

---

