---
type: spec
id: addr
status: standing
description: Normative spec for cross-root addressing, the mount table, and the `addr::Addr` type. Ships no engine code.
owns: [cross-root addressing, mounts, "addr::Addr"]
---

# The address grammar — the cross-root address type and the mount-table law

> Standing law: `README.md` (process and standing corrections) and `wire-contract.md` (the wire contract).

**Status: normative spec**; ships no engine code. Where it rules, the implementer has no design decision left. Open points are in § 10, each with an owner surface in this tree.

**Scope.** The cross-root (mount) grammar `[root:]path[#selector]`, the `addr::Addr` type, the mount-table invariants, and the stored-form translation positions. Wire section addressing stays `wire-contract.md` §2.1's: segment objects (`{"hpath":[{"h":"Goals"},{"h":"Q3"}]}`, `{"anchor":"…"}`, `{"fm_key":"…"}`), never a joined path (`Goals>Q3`, `Goals/Q3`, a sanitized slug) as the writeable form. An `Addr`'s `#selector` is an ingress/host-face slot; its machine-canonical resolution is still segment or anchor form.

**Law fixed in this tree** (not reopened by citing out-of-tree files): the agent/stored split (`root:` vs the `obsidian://` stored form); name ownership (the root declares, `MERIDIAN.md` binds — `meridian-md-schema.md`); the grey rule (unmounted ≠ red missing); canonicalize-at-bind; refusal of equal or nested mounts; grey refusals on exit 1 with a distinct reason word.

---

## 1. The four senses of "root"

`Root` is already a wire type, so this document's type is `MountName`.

| Name | What it is | Where it lives | Shape |
|---|---|---|---|
| `wire::Root` (type name) / **fingerprint** (design noun) | Merkle content-hash cursor the world-guard compares; design name `fingerprint` (`wire-contract.md`); Rust type still `Root` (code lag) | `crates/wire` | `"b3:" + 64 hex`, opaque, equality only |
| `fs::WorkspaceRoot` | The one on-disk workspace directory every path joins onto today | `crates/fs/src/lib.rs:33` | `PathBuf` |
| **`addr::MountName`** | A canonical root name: the mount-table key a cross-root address carries (`sessions`, `assets`) | `crates/addr` (§ 7) | a lowercase name, never a path or a hash |
| `root:` the frontmatter key | A preset-def property naming the root record a session preset instantiates | `crates/preset/src/lib.rs:232`; fixtures `crates/preset/tests/gates.rs:15`, `:321`, `:524` | an ordinary YAML scalar (`root: SESSION.md`) |

`run::address::AddressError` (`crates/run/src/address.rs:45`) parses the run plane's same-file block refs `[[#^id]]`, a different grammar and owner; it never meets `addr::AddrError`.

---

## 2. The type — D3 and D5 ruled

### 2.1 Neither D12 story as written

- **Story B (RIDE)** — keep the prefix in the spelling; resolve by `three_rules` (`model/src/lib.rs:1856-1871`: `docs.contains_key(spelling)`, `docs.contains_key(spelling + ".md")`, `resolve_linkpath(...)`). Rejected: all three rules look up one root's `BTreeMap<String, Document>`, whose keys carry no root, so `root:page` renders `red selector-unresolved`. Shipped, they run after the root is peeled (`resolve_ref`, `model/src/lib.rs:1767-1782`).
- **Story A (PEEL)** — strip the root textually at the lock face and discard it. Rejected: no such splitter exists; a discarded root has nowhere to go. Shipped, the root stays on the spelling until resolution, in `LockItem.to_root: Option<addr::MountName>` (`view/src/read_face.rs:294-334`, :326, :398-400, :840-841; `declared_addr` :847).

**Ruled: the address is a fallible type carrying an optional root; the resolver takes a root-keyed corpus.** Fallible construction makes the compiler list the doors; a string convention re-parsed at 16 sites is the "boolean helper a caller may ignore" defect.

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

**Construction is the only way in.** `Addr::parse(&str) -> Result<Addr, AddrError>` is the sole constructor; no `Addr::from_parts` exists, so `Addr.path` never carries a root prefix (what makes § 5's body-level guard checkable).

**Parse is not resolve.** `Addr::parse` never touches the mount table; an unbound root is the resolver's grey answer (§ 6), never a parse error, so it is never mistaken for a malformed address.

---

## 3. The three-way translation invariant

The mount table is the single authority for the map canonical root name ↔ Obsidian vault name ↔ local path:

> **INV-1 (name is a key).** No two entries share a `MountName`.
> **INV-2 (path is a key).** No two entries share a canonicalized local path.
> **INV-3 (vault name is a key).** No two entries share an Obsidian vault name.
> **INV-4 (no containment).** After canonicalization, no bound path equals, or is a path-segment-boundary prefix of, another.
> **INV-5 (declared = bound).** A root's self-declared name equals the name `MERIDIAN.md` binds it to, or the parse fails loud. No declaration is grey, not a mismatch (D7 — *"MERIDIAN.md binds, it doesn't baptize"*).

| # | Input | Required outcome | Class |
|---|---|---|---|
| T1 | two entries named `sessions` | **parse fails loud, no partial mount table** | `duplicate-mount-name` |
| T2 | two entries with the same canonicalized path | **parse fails loud** | `duplicate-mount-path` |
| T3 | two entries with the same vault name | **parse fails loud** | `duplicate-vault-name` |
| T4 | `/a/wiki` and `/a/wiki/sub` both bound | **parse fails loud** (INV-4) | `nested-mount` |
| T5 | root declares `wiki`, table binds it as `field-notes` | **parse fails loud**, naming both spellings | `declared-bound-mismatch` |
| T6 | root declares no name | **grey for that root**, the missing declaration named | `grey(undeclared)` |

INV-1…INV-3 make the map a bijection; a silent pick would make stored links machine-dependent.

---

## 4. The colon law — (d), ruled here

`sessions:notes.md` is a legal filename (§ 11.3); `wire::Path` does not validate (*"this newtype does not validate, it names"*, `crates/wire/src/lib.rs:29-31`); `path_confined` (`crates/wire-serve/src/write.rs:1984`) rejects only empty, leading-`/`, `.` and `..` segments. So one string has two readings.

### 4.1 The law

The **head** of an address is the text before the first `/` and before the first `#`.

> **The root reading wins, unconditionally; there is no fallback to the literal reading.**
>
> - **Zero `:` in the head** → no root; resolves in the ambient root.
> - **Exactly one `:` in the head** → the root separator. Before it a well-formed `MountName`, after it non-empty text; otherwise **refused**, never reinterpreted as a literal path.
> - **Two or more `:` in the head** → **refused**.
>
> After the first `/`, a `:` is an ordinary path byte.

Rejected: a reading that flips on whether the prefix is a legal `MountName`. No fallback, because a typo (`session:notes.md` for `sessions:notes.md`) would otherwise succeed silently as a literal lookup.

### 4.2 The consequence

A corpus-relative path whose first segment contains `:` cannot be named by any address.

> The engine **refuses to create** such a path. An existing one on disk renders **grey**, the reason naming it unaddressable — never silently resolved or skipped.

This rules the literal path only (D10); a `root:` spelling resolves at every page-taking door (§ 4.6, D11), and a raw head-colon `Path` on the wire refuses (`addr::confined`).

### 4.3 `MountName`'s charset

`[a-z0-9-]`, non-empty, lowercase only. An uppercase byte **refuses**, never normalizes: the corpus index already lowercases (`basename_lc`, `crates/model/src/lib.rs`); a second, invisible case rule would break one name per root.

### 4.4 Law A-2 — the fragment is selector bytes to its end; `@fp` is off the name lane

The fragment runs from the first `#` to the end of the spelling; **every byte of it is selector bytes, `@` included**. Fingerprint pinning is its own field on its surface (a lock row's pin, a render-face decoration), never an in-band suffix in the name lane (the packing law: a delimiter drawn from an open charset collides with it).

- **Motive.** 3,815 real headings across two roots contain `@`. An in-band `@` split would make each unaddressable by its own spelling, and for (`Deploy`, `Deploy @ prod`) a trimming resolver would resolve the wrong real section.
- **Narrows.** A pasted render-face spelling (`page.md#Sec@green.b3…`) keeps its `@green.…` tail, misses byte-exact, and lands in the Law A-3 teaching refusal. No stored surface carries an in-band `@fp`: the engine refuses an fp reaching stored bytes (*"a render-face decoration the engine mints on read, never storable content"*, `crates/wire-serve/src/write.rs`).
- **Widens.** `#Deploy @ prod` resolves byte-exact to the heading `Deploy @ prod`; every `@`-bearing heading is addressable by its own spelling.
- **Type.** `addr::Addr` has no `fp` field; `Addr::parse` records the fragment verbatim; `Display` round-trips without an `@` re-join.

### 4.5 The negative cases

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
| D11 | a `create`/`splice` targeting `sessions:notes.md` | **RESOLVES through the rooted lane (§ 4.6)** — root `sessions`, rel `notes.md`, joined onto that root's bound workspace; the wire sees the rel half only | a raw head-colon `Path` arriving ON THE WIRE refuses `bad_path` |

D1–D4 must **parse**; D2 and D3 keep this law from swallowing the ordinary corpus.

> [!NOTE] D11 — why the door resolves rather than refuses
> A `root:` spelling is an address (§ 4.1); admitted as a path it would create a document no address can name. Resolved, the document is `sessions:notes.md` from everywhere and `notes.md` inside its own tree. What refuses: malformed heads (D5–D9), an unbound or typo'd root (a teaching refusal, never a literal fallback), the literal path (D10), and a raw head-colon `Path` on the wire (§ 4.6).

### 4.6 The rooted lane spans every page-taking door (rooted-refs-everywhere)

> **The law.** Every door at which the caller names a page resolves `[root:]path[#selector]` through the one rooted lane (parse → confinement → mount table, § 4.1, no literal fallback), so `mrd` does not depend on the runtime cwd. No page-taking door refuses a well-formed rooted ref as such or misreads it as a filename (one exception: the preset lane below). The family is bound by the predicate "names a page", not a list (`wire-contract.md` § 12.1).

Members, a measured snapshot (the predicate is the authority; re-measure at the seam):

- **Rooted:** `read`, `fingerprint`, `resolve`, `put --scope`; `run`, `walk`, `repair`, `realise`, `links`, `rules` (read side); `put` (write target), `rm`, `pin` (page position; the target is cross-root too); `script --files` (each entry `root:path`-capable).
- **Not yet converted:** the preset lane (`unfold`, `reconcile`, `new`).
- **Outside, no page named:** `arm --at` (a workspace-relative directory, `armed-plane.md`; keeps its head-colon refusal); `test --history`, `status --cwd` (explicit tree arguments); `test --corpus` (its spec is a fixture file read relative to the spec's directory with cwd/absolute semantics; rule pages it names are document positions, § 9.1); `check`, `retire`, `skill`; `sql` (`--root` selects a workspace, not a page: CLI surface, not address law).

> **Authority: the page's tree governs.** `root:x` behaves exactly as if the caller had cd'd into that root: conventions and caps load from the page's tree, receipts land in the page's workspace, and the standing workspace contributes nothing to the ceiling. Rejected: the standing tree governs; both trees must permit. Closed hazard: a ceiling loaded from where the caller stands (`workspace::Answer::root`, `None` on a cwd default).
>
> The hazard is Starlark-only — **caps do not apply to bash** (`crates/run/src/caps.rs` module doc, citing `laws.md` § Amendment): `resolve_authority` takes the bash branch first, consults only the builtin, non-overridable `READ_ONLY_PATTERNS` (`check-*`/`verify-*`, which refuse a bash fence at load), and resolves every other bash task `Authority::Unsandboxed` without reading the conventions. The bypass closed: `resolve_caps` reads the declaring root's conventions, so a read-only `run.caps.*` ceiling over `md.*` writes was cd-swappable for a looser tree's.

**The mechanism is the workspace jail, and it is law.** One daemon serves many workspaces; `hello` pins the workspace exact-or-refuse, no ancestor walk (`registry.rs` `pin_declared`: *"a declaration never widens to an enclosing registered workspace"*), and the connection stays on it. A rooted door resolves the root, then dials that workspace; the wire carries the rel half only, and the § 1 `Path` law (`wire-contract.md`) keeps its head-colon confinement arm.

> **The one exception: the preset lane is not yet converted.** `unfold`, `reconcile` and `new` name a page (`new`'s def token: *"resolve def (presets/<KIND>.md or page path)"*) but write in-process with no daemon dial, so a rooted op would bypass the target tree's armed gates. A rooted ref there **refuses with a teaching** naming this reason and the remedy (run it from inside the target tree). Until it rides the daemon write path the lane is cwd-determined, not correctly cwd-bound forever.

**Convergence.** A client's multi-file `script` face states the same rule: *"Every files[] entry resolves through one root; that root is the workspace; in-program paths are relative to it"*. A program binds one declared root, so cross-root reads inside one program do not arise.

**Face grammars differ; do not harmonize.** A client face may admit absolute and client-relative refs beside `root:path`; the `mrd` CLI's § 1 path law still forbids absolute paths and `.`/`..` segments.

### 4.6a The root a spelling names — name first, then alias (root-alias)

> **The law.** A `root:` spelling resolves to the mount whose `name` it equals; if none, to the mount whose `alias` it equals (`meridian-md-schema.md` § 5.1b); otherwise it refuses as an unbound root. One order at every § 4.6 door, one seam (`mrd::rooted::resolve_name` at the CLI, `addr::MountSet::canonical` on the link plane).

**Motive.** The engine knows no root names (the no-baked-names law, `laws.md`); `alias:` maps one agreed constant, `sessions:`, onto whatever each machine calls that tree, without the engine learning that name. `primary:` is not consulted for `sessions` or anything else.

**Default (schema §5.1c).** Where no mount is named or aliased `sessions`, the bound table gains the implicit default mount `sessions` at `$HOME/.local/share/ucc/sessions`, only when it binds clean — found by `name`, first rung, same seam. A declared name or alias suppresses it ("defaulted in code at most").

**Name first, never a tie-break.** A root named `sessions` resolves `sessions:` with no alias — a name is its own alias. A table where a name and an alias both answer one spelling does not load (`alias-shadows-name`, whole-table refusal).

**The canonical spelling is the name.** An alias is a lookup spelling only; nothing the engine writes or echoes carries one:

| Surface | Spelled with |
|---|---|
| receipts, pins, `mint {…}` paths, `sub` rows | the mount `name` |
| a door's canonical `root:path` echo (`mrd resolve`'s `ref:` row) | the mount `name` |
| the stored form's `vault=` (§ 3's three-way map) | the mount's vault, reached by canonicalizing the alias first |
| `mrd resolve`'s `root:` row; the `alias` column of `mounts` and `mrd config` | the alias, where it explains a resolution |

So `mrd resolve sessions:x` answers `root: field-notes-sessions (alias sessions)` and `ref: field-notes-sessions:x`; an alias in stored bytes would be one machine's private mapping (§ 3's defect).

**Declared, not bound, picks the refusal.** An alias onto a root this machine declares but cannot read reaches that root's state (`grey(path-unseeable)` and its detail), never the undeclared refusal — § 6's two causes keep their two remedies. Pinned by `crates/mrd/tests/root_alias.rs` (four acceptance arms) and `addr`'s `an_alias_onto_an_unreachable_root_carries_that_roots_state`.

---

## 5. The basename fallback — (c)

`resolve_linkpath` (`crates/model/src/lib.rs:1736-1737`) falls back to a basename match:

```rust
let key = linkpath.trim().trim_end_matches(".md").to_lowercase();
let base = key.rsplit('/').next().unwrap_or(key.as_str()).to_string();
```

So `"sessions:24-01-retro/notes.md"` matches the ambient root's `notes.md`; the `sessions:`
prefix is discarded unread (§ 11.1). The defect is inside the owner, not at a door.

### 5.1 Ruled: peel and refuse

- **C-1.** The fallback is intra-root: it runs only after the root is peeled and the mount
  lookup has chosen that root's corpus.
- **C-2.** A rooted address never falls back to the ambient root. Root unmounted: **grey
  `unmounted`** (§ 6). Root mounted, path absent there: **`file_not_found` for that root**.
  Never the ambient root's same-basename file.
- **C-3.** The body guards itself, because the retype does not reach it: `linkpath` must carry
  no root separator. On a head `:` (a programming error) `resolve_linkpath` returns `None`; a
  test asserts it, so a raw `&str` caller cannot revive the defect silently.
- **C-4.** One address, one answer: the two spellings of § 11.1 must converge, and the test
  asserts the convergence.

### 5.2 The negative cases

| # | Input | Today (measured, § 11.1) | Required |
|---|---|---|---|
| F1 | `[[sessions:24-01-retro/notes.md#Design]]` in `claim.md`; ambient `notes.md` exists; `sessions` unbound | `-> notes.md (1)`, exit 0 (wrong success) | **grey `unmounted`** (§ 6), exit 1 |
| F2 | `[[sessions:notes.md#Design]]` (no-slash control); `sessions` unbound | `-> sessions:notes.md (1, unresolved)` | **grey `unmounted`**, same as F1 (C-4) |
| F3 | as F1; `sessions` bound; target present | `-> notes.md` (wrong root) | the file **in `sessions`**, bytes from there |
| F4 | as F1; `sessions` bound; target absent | `-> notes.md` (wrong root) | **`file_not_found`** scoped to `sessions`, never the ambient file |
| F5 | a raw `&str` with a `:` head reaching `resolve_linkpath` | basenames onto the ambient root | **`None`**, asserted by a test (C-3) |

**Acceptance half:** F3 must resolve, with bytes from the target root (criterion 3), proven by
editing the file there. All-grey passes F1, F2 and F4 and ships nothing.

---

## 6. The unmounted-root refusal — (f), the verbatim exemplar

The refusal text is a **pinned `const` exemplar asserted verbatim**, like
`model::selector::D1_TEACHING_REFUSAL_EXEMPLAR` (`crates/model/src/selector.rs:569`; rendered by
`render_ambiguity`, pinned by `render_ambiguity_carries_d1_teaching_verbatim`,
`crates/model/src/selector.rs:929-961`). It lives beside `GreyReason`'s members:

```rust
// crates/model/src/selector.rs — beside GreyReason's existing members.

/// A cross-root address naming a root this machine does not bind. Grey, never
/// red: nothing drifted; the ledger cannot see from here (the grey rule).
/// Carries the missing name so the refusal can teach the fix (D8).
Unmounted { root: addr::MountName },

/// The unmounted-root teaching refusal, carried VERBATIM as the provenance
/// anchor (the grey rule). `render_unmounted` reproduces this
/// wording with the real root and address interpolated; this const pins the
/// exemplar so a drift in the wording is a visible test failure.
pub const GREY_UNMOUNTED_REFUSAL_EXEMPLAR: &str = "grey(unmounted): root 'assets' is not mounted — the address 'assets:domains/media/logo.md#Design' names a root this machine does not bind. Not red: nothing drifted, you just cannot see from here. Refs to mounted roots remain served. Fix: declare 'assets' in ~/MERIDIAN.md as a mount entry (name / path); see [[address-grammar]].";
```

The pinning test asserts the teaching tail verbatim, as `TEACH_TAIL` does for D1:

```
. Not red: nothing drifted, you just cannot see from here. Refs to mounted roots remain served. Fix: declare '<root>' in ~/MERIDIAN.md as a mount entry (name / path); see [[address-grammar]].
```

It names the missing mount and teaches the fix (D8), carrying the grey rule's sentence verbatim.

### 6.0b One remedy per plane

`docs/meridian-md-schema.md` § 8.3 requires a `Fix:` naming the legal form. Two selector-door
failures take opposite remedies:

| failure | what the caller holds | the remedy |
|---|---|---|
| **miss**: no node answers the selector | an address that does not resolve | **discovery**: list the sections, feed a row back |
| **ambiguity**: the selector resolved twice | an address that resolves too well | **disambiguation**: pin one occurrence, or make the duplicate ids distinct |

A discovery `Fix:` on an ambiguity is a recovery loop: the caller already typed that address.

**Each ambiguity plane publishes one remedy sentence, and every door renders those bytes:**
`selector::AMBIGUITY_FIX` (heading plane, inside `D1_TEACHING_REFUSAL_EXEMPLAR`) and
`selector::ANCHOR_AMBIGUITY_FIX` (anchor plane, inside `ANCHOR_AMBIGUITY_REFUSAL_EXEMPLAR`). Both
the write door (exemplar renderers) and the read face (aggregate `Fix:`) answer each failure,
and an exemplar constrains only the doors that render it.

### 6.1 The vocabulary is grey-exit-1's

Grey **refuses**: **exit 1**, reason word **`grey(unmounted)`** in the human line and in
`--json`. **No fourth exit code.** `--force` is the escape.

The sibling words `grey(cannot-assess)` (the `mrd check` verb-level state, validity plane) and
`red(...)` neither collide nor unify with it (D8a: two subsystems, one shared meaning). The
unmounted grey routes through `model::selector::Color`/`GreyReason` (address plane); what is
shared is the law, *outside sight never renders as verified*, not the type.

### 6.2 Grey outranks red

**R-3.** An address naming an unmounted root is **grey**, whatever else is true of the target; a
green cross-root pin whose root is later unmounted becomes **grey, never red** (nothing drifted;
the ledger stopped measuring). Grey → exit 0 is refused: an edit to `~/MERIDIAN.md`, which cannot
be attested (`meridian-md-schema.md` § 9), could then turn a red into a pass.

---

## 7. Crate placement — (e), D4/D4a

### 7.1 Two crates

| Crate | Position | Owns |
|---|---|---|
| **`crates/addr`** | `std`-only leaf, zero dependencies, upstream of `syntax` | `Addr`, `MountName`, `MountSet`, `AddrError`; the colon law (§ 4); the parse (where an address becomes a value) |
| **`crates/config`** | downstream of `model` | the `MERIDIAN.md` parse; the mount table (name ↔ vault name ↔ path); canonicalize-at-bind; deny-ceiling inheritance; the equal-or-nested refusal; the declared-vs-bound check |

Neither is `crates/workspace` (*"a leaf, `std` + `cache` only"*, `laws.md` § Crate charters); it
is read, not moved: `config` calls `workspace::deny_reason`. `addr` sits upstream of `syntax`
because `model` depends on `syntax` + `blake3` (`crates/model/Cargo.toml`), so a type downstream
of `model` cannot reach the wikilink ingress `syntax::split_wikilink_target`
(`crates/syntax/src/lib.rs:424`).

### 7.2 The tension D4 and D4a create, resolved

D4a injects the mount table **as a parameter into `model::CorpusIndex::resolve_ref`** (which
already takes `docs: &BTreeMap<…>`); moving resolution out of `model` breaks its charter (*"the
single address law its two dependents share"*). But `config` is downstream of `model`, so
`model` cannot name a `config` type.

**Ruled: the injected parameter is `&addr::MountSet`, defined in the upstream leaf.** `config`
owns the binding and **projects** its bound names into a `MountSet`; `resolve_ref` names only
`addr`. D4 and D4a both hold.

`MountSet` carries only **is this name bound** and **which names are bound** (so the § 6 refusal
can teach), no paths: `config`/`fs` have already loaded each root's documents into the
root-keyed corpus, `model`'s type keyed by `addr::MountName`. It is **a concrete type, not a
trait**, since a trait invites a second implementation (D12: one question, two answers).

### 7.3 The charter rows `laws.md` carries

`laws.md`'s crate-charter table is exhaustive per crate; these sentences are the floor those rows
carry:

| Crate | Charter |
|---|---|
| `addr` | The agent-plane address: `[root:]path[#selector]` parsed into a fallible type carrying an optional canonical root name, plus the resolution-facing bound-name projection every plane resolves through. A `std`-only leaf upstream of `syntax` — it is where an address becomes a value, so nothing downstream re-splits a string |
| `config` | The `MERIDIAN.md` plane: the one entry point parsed as content (a rev and a fingerprint like any page), and the mount table binding canonical root name ↔ Obsidian vault name ↔ local path — canonicalized at bind, passed through the `workspace` deny ceiling, refusing equal-or-nested mounts, failing loud with no partial table. Downstream of `model`; it projects the bound names into `addr::MountSet` so resolution stays `model`'s |

---

## 8. The mount-path law — (b), canonicalize-at-bind

### 8.1 The rules

- **B-1: canonicalize at bind.** Every mount path is canonicalized (symlinks resolved, trailing
  separators normalized) **before** binding, because `workspace::deny_reason` canonicalizes both
  sides of its comparison (`resolve_ref`, `crates/workspace/src/lib.rs:384`).
- **B-2: inherit the deny ceiling.** Every canonicalized mount path passes
  `workspace::deny_reason` before binding; a refused mount **fails the whole parse** (no partial
  table, no default-root fallback). Reasons: `workspace::DenyReason`'s existing six
  (`crates/workspace/src/lib.rs:301-314`): `FilesystemRoot`, `HomeDir`, `TempDir`, `XdgBaseDir`,
  `CacheRoot`, `MountPoint`. **The ceiling is reused, never re-implemented.**
- **B-3: refuse equal-or-nested mounts, including through symlinks.** After canonicalization,
  no two bound paths may be equal, and none may be a **path-segment-boundary** prefix of another
  (INV-4), so a symlink cannot smuggle one tree in twice.

### 8.2 The motive: a pin-proof bypass

Measured (§ 11.2): `/path/to/repos/wiki` is a **symlink** to `/path/to/projects/wiki`, and
`CCC_LLM_WIKI_PATH` carries `/path/to/projects/wiki/` (real path, trailing slash). Mounted
literally, that is **one tree twice, under two names**: two canonical refs with **identical
`sec_rev`**, so proof gathered on ref A would gate a pin on ref B. B-1 and B-3 together close
this bypass; B-1 alone does not, since only canonicalization collapses both spellings.

Pin proof rides the request (wire-contract § A.3; no read-mint ledger), so the bypass class
survives translation: the proof compare must run against the **resolved target root's** live
bytes, never an ambient root's same-named file. The same B-1/B-3 canonicalization guarantees it.

### 8.3 The negative cases

| # | Input | Required outcome | Class |
|---|---|---|---|
| M1 | mount path at `$HOME` | **whole parse fails loud**, naming the reason | `DenyReason::HomeDir` |
| M2 | mount path at `/`, `/tmp`, an XDG base dir, or under the cache root | **whole parse fails loud** | the matching `DenyReason` |
| M3 | `/path/to/repos/wiki` (symlink) **and** `/path/to/projects/wiki/` under two names | **whole parse fails loud**: one tree, two names | `duplicate-mount-path` (INV-2, after B-1) |
| M4 | `/a/wiki` and `/a/wiki/sub` both bound | **whole parse fails loud** | `nested-mount` (INV-4) |
| M5 | `/a/wiki` and `/a/wiki-two` both bound | **bound; legal** | — (`wiki-two` is not a segment-boundary descendant of `wiki`) |
| M6 | mount path missing or unreadable | **grey for that root**, path named; the table stays loaded | `grey(unmounted)` family, **not** a parse failure |
| M7 | mount path without a `vault:` leg (a plain folder) | **bound** | — (§ 10 row 7, § 10.1) |

M5 is M4's acceptance half: the prefix test is segment-boundary, not string-prefix, since a naive
`starts_with` would refuse legitimate sibling roots. M6 is not a parse failure: a root absent
from *this* machine is by design, and failing there would brick the CLI on every machine lacking
a declared root.

---

## 9. The positional grammar — (a)

### 9.1 The four positions

An agent-plane address occupies exactly these positions:

1. **a wikilink target** — the `dest` of `[[…]]`, owned by `syntax::split_wikilink_target`
 (`crates/syntax/src/lib.rs:424`);
2. **a markdown link URL** — the URL of `[label](url)`;
3. **`meridian-lock` `ref:` values**;
4. **`meridian-lock` `objects:` keys**.

These are positions inside a document (the translation's whole scope), not argv positions; which
CLI arguments admit a rooted spelling is § 4.6's question.

### 9.2 The transform is positional, never a byte transform

**A-1.** The stored-form translation is **positional**: it identifies each address by its
position in the candidate document and translates those in its owned positions. **A blanket byte
transform over the token `root:` is forbidden**: `root:` is a live YAML frontmatter key in the
preset/def grammar (`fm_scalar(&doc, "root")`, `crates/preset/src/lib.rs:232`; fixture
`root: SESSION.md` at `crates/preset/tests/gates.rs:15`, `:321`, `:524`), so a blanket
transform would corrupt the def and silently invalidate every pin whose fingerprint covers it.
Frontmatter is not an address position (*"frontmatter is not a claim-link position
(S10/R22)"*, `crates/wire-serve/src/write.rs`).

**A-2: the precedent is `strip_fp_candidate`** (`crates/wire-serve/src/write.rs:2476`), copied
structurally: it attributes each token in the candidate to its payload via `classify_fp`
(`crates/wire-serve/src/write.rs:2311`) and refuses what it cannot place, never
blanket-stripping. The address translation likewise **refuses** an address it cannot attribute
to one payload.

**A-3: the assertion.** After translation the candidate carries **zero** agent-plane `root:`
spellings in positions 1 and 2, asserted **on the candidate** on every write path, dry and real:
the artifact guard (D9), not a guard on `splice`.

### 9.3 The transform is the identity on positions 3 and 4

The type reaches all four positions; the translation touches two.

**A-4.** The `obsidian://` translation applies to **positions 1 and 2** (wikilink target,
markdown link URL) and is the **identity** on **positions 3 and 4** (lock `ref:`, lock
`objects:`), by the stored-form law: *"Lock `ref:` and `objects:` keys use the canonical `root:`
form (agent plane), never the URI"*.

### 9.4 The negative cases

| # | Input (a candidate document) | Required outcome | Class |
|---|---|---|---|
| P1 | frontmatter line `root: SESSION.md` | **byte-identical, untouched** | — (not an address position) |
| P2 | `` example `root:page` address `` in a code span or fence | **untouched** | — (a code sample, per the document law) |
| P3 | wikilink `[[sessions:notes.md#Design]]` | the `obsidian://` stored form, carrying the **vault name** | — |
| P4 | markdown link `[x](sessions:notes.md)` | the `obsidian://` stored form | — |
| P5 | lock `object: "[[sessions:notes.md]]"` | **canonical `root:` form kept; identity** (A-4) | — |
| P6 | a lock `objects:` key | **no such position** (see below) | — |
| P7 | an address the transform cannot attribute to one payload | **refused** | `bad_request`, the `strip_fp_candidate` shape |
| P8 | an `@fp` token reaching a stored URI or a display field | **refused** | criterion-4 machinery, reused at the candidate |
| P9 | a hand-edited, malformed `obsidian://` URI on read-back | **fails loudly**, never guesses | reverse-translation refusal |

**Acceptance half:** P3 and P4 must translate, and the agent-plane form must round-trip through
the stored form byte-identically; since an identity function round-trips perfectly and ships
nothing, P3/P4's positive assertion and P1/P2/P5's untouched assertion are one gate.

**P6 names no position; P5 is spelled the v2 lock's way.** The v2 lock has no top-level
`objects:` table (the blob hash rides the pin row), so P6 cannot occur. The lock's address key is
`object: "[[sessions:notes.md]]"`, a wiki link carried verbatim in canonical `root:` form (A-4,
identity); the selector lives in the sibling `path` / `properties` array.

---

## 10. The edge-case table

Every row is answered; rows 10–13 name their real owner.

| # | Row | Answered by | Answer |
|---|---|---|---|
| 1 | `root:` naming an unmounted root | § 6 | grey, exit 1, `grey(unmounted)`, the § 6 exemplar naming the missing mount and its fix |
| 2 | `root:` naming a mounted root, file missing | § 5.1 C-2, row F4 | `file_not_found` **scoped to that root**, distinct from grey |
| 3 | a bare path (no `root:`) | § 4.1 zero-colon arm, row D2 | the ambient root, unchanged; the colon law's acceptance half |
| 4 | **a path that literally contains `:`** | **§ 4 in full**, rows D1–D11 | root-wins on a single head colon, no fallback; ≥2 head colons refuse; a first-segment colon on disk is `grey(unaddressable-path)`; a write door targeting one refuses `bad_path` |
| 5 | two roots declaring the same canonical name | § 3 INV-1, row T1 | parse fails loud, no partial mount table |
| 6 | one root mounted at two paths | § 3 INV-2, § 8 B-1/B-3, rows T2/M3 | parse fails loud **after canonicalization**; a symlink cannot smuggle it past |
| 7 | a root without a `vault:` leg (a plain folder) | § 10.1 | **selectors are legal on every mounted root**; no kind taxonomy, no opaque-root refusal |
| 8 | a cross-root pin whose target root is later unmounted | § 6.2 R-3 | **grey, never red** |
| 9 | a stored `obsidian://` URI hand-edited by a human | § 9.4 row P9 | read-back **fails loudly**, never guesses |
| 10 | `MERIDIAN.md` pins a root it declares; that root drifts | not this document's | **Implementation owns it** (mount-as-claim, `meridian-md-schema.md` § 5.3; the fence's only bypass is an edit to `~/MERIDIAN.md`) |
| 11 | hook placed, `mrd` later uninstalled | not this document's | **Implementation owns it**: fail closed with teaching, `--no-verify` named in the message |
| 12 | two worktrees, one hook dir, different meridian workspaces | not this document's | **Implementation owns it** (D11: placed per git common dir; workspace-root ≠ worktree-top-level is a stated refusal in `mrd skill hook`'s document) |
| 13 | a subprocess forked while `DrawerLock` is held | not this document's | **Implementation owns it**: explicit `LOCK_UN` in `Drop` |

### 10.1 No opaque-root refusal

There is no mount `kind` and no refusal of a `#selector` on any mounted root. A rule *a
plain-folder root has no parse and no sections, so an address into one must not carry a
`#selector`* (`AddrError::SelectorOnOpaqueRoot` at resolution) is false at the data level:
`model::RootedCorpus::with_root` builds one parsed index for every root, never branching on
kind. `meridian-md-schema.md` §5.1 has no `kind`;
every mounted root is the same shape to the resolver, selectors resolve on any, and the closed
error set has no `SelectorOnOpaqueRoot`. Extinction pin:
`crates/model/tests/kind_gate_extinction.rs` (a re-added kind gate fails there first, mirroring
the daemon's mountgate pin).

---

## 11. The measurements

All three pre-date the fix and record the defect, not today's behaviour:
§ 5.1's peel-and-refuse is now the C-3 guard
(`crates/model/src/lib.rs:1733-1734`), the mount-aware resolver is
`resolve_ref` (`crates/model/src/lib.rs:1776`), and § 11.1 re-run now hits
that refusal (`crates/model/tests/u11_c3_linkpath_peels_and_refuses.rs`
asserts it on the verbatim input).

### 11.1 Cross-root misresolve

Pre-fix binary, fresh git workspace:

```
$ mrd links claim.md            # [[sessions:24-01-retro/notes.md#Design]]
workspace /path/to/repro/ws
  source: daemon
  claim.md
    -> notes.md (1)             ← RESOLVED, to the ambient root's file.  exit 0

$ mrd links claim2.md           # [[sessions:notes.md#Design]]  (the control)
workspace /path/to/repro/ws
  source: daemon
  claim2.md
    -> sessions:notes.md (1, unresolved)                                  exit 0
```

The defect **needs the slash**: one grammar, two answers on one plane. § 5.1
C-4 asserts against that divergence.

### 11.2 Symlink topology

```
$ readlink /path/to/repos/wiki
/path/to/projects/wiki
$ echo $CCC_LLM_WIKI_PATH
/path/to/projects/wiki/
```

One tree, two spellings: a symlink and a real path with a trailing slash.
§ 8 B-1/B-3 rest on this.

### 11.3 A `:`-bearing filename is legal

```
$ printf 'x\n' > 'sessions:notes.md' && ls
sessions:notes.md
```

The file is created. `wire::Path`'s own doc says *"this newtype does not
validate, it names"*, and `path_confined` checks only segments (cited in § 4):
nothing validates a `:` before a path, so § 4 rules a live ambiguity.

---

## 12. Implementer self-check

**The claim:** this document plus `laws.md` (crate placement) suffices to
write the type without inventing grammar.

| implementation needs | Ruled here |
|---|---|
| type | § 2.2: `addr::Addr`, `addr::MountName`, `addr::MountSet`, `addr::AddrError` |
| crate | § 7.1: `crates/addr`, `std`-only leaf, zero deps, upstream of `syntax` |
| constructor | § 2.2: `Addr::parse` only; no `from_parts` |
| prefix/path split | § 4.1: colon law, three arms, no fallback |
| root-name charset | § 4.3: `[a-z0-9-]`, non-empty; uppercase refuses, never normalizes |
| `@` in fragment | § 4.4 (Law A-2): selector bytes to the end; the fingerprint pin is its own field, never in-band |
| error set | § 4.5: `BadMountName`, `EmptyMountName`, `EmptyPath`, `AmbiguousColon`; no opaque-root variant (§ 10.1) |
| parse/resolve split | § 2.2: parse never reads the mount table; unmounted is grey, not a parse error |
| `resolve_ref` input | § 7.2: `&addr::MountSet`, upstream so D4 and D4a hold |
| three compiler-unreachable ingress classes | § 9.1: four positions |
| body-level guard | § 5.1 C-3: `resolve_linkpath` refuses a `:`-bearing head; test-asserted |
| refusal wording | § 6: pinned `const` exemplar, verbatim, with teaching tail |
| rooted-ref doors, authority | § 4.6: every page-taking door, predicate-bound; page's tree governs conventions and receipts; wire carries only the rel half |

**Left open**, with owner; none is needed to write the type:

1. **Mount-entry syntax in `MERIDIAN.md`** (block grammar; name, path,
   vault-name keys) — `meridian-md-schema.md`. Here only the invariants
   (§ 3, § 8), never the spelling.
2. **`obsidian://` URI construction, percent-encoding, round-trip identity
   gate** — the wire-serve stored-form seam. § 9 rules only which positions
   it may touch and where the guard lands.
3. **`MountSet` API beyond `is_bound` and `bound_names`** — the mount-table
   implementation, once the § 6 refusal render shows its real needs. No
   speculative methods.
4. **Root self-declaration: frontmatter key or named block** —
   `meridian-md-schema.md` + config bind own the check (declared-vs-bound,
   INV-5). Here only: mismatch fails loud, absence is grey (D7).

---
