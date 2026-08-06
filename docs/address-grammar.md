# The address grammar — the cross-root address type and the mount-table law

> **Standing:** Design law is `wire-contract.md` (one contract). Mint addresses = segments only. Receipts = armed wire facts. DuckDB/`view_path` not agent core. **Doc correct > code correct; docs first.** See `README.md`.

**Status: normative SPEC** (ships no engine code). Where this document rules, the implementer has no design decision left; open points are named in § 10 with an owner surface inside this tree.

**Scope (read before §2).** This document owns the **cross-root / mount** grammar:
`[root:]path[#selector][@fp]`, the `addr::Addr` type, mount-table invariants, and
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
| `fs::WorkspaceRoot` | **One on-disk directory** — the single workspace every path is joined onto today | `crates/fs/src/lib.rs:38` | `PathBuf` |
| **`addr::MountName`** | **A canonical root NAME** — the mount-table key a cross-root address carries (`sessions`, `assets`) | `crates/addr` (NEW, § 7) | a lowercase name, never a path and never a hash |
| `root:` the frontmatter key | A **preset-def property** naming the root RECORD a session preset instantiates | `crates/preset/src/lib.rs:217`, fixtures at `crates/preset/tests/gates.rs:26` and `:368` | an ordinary YAML scalar (`root: SESSION.md`) |

**`addr::MountName` collides with none of them**, and the collision it avoids is deliberate: it is
neither a hash (`wire::Root`) nor a directory (`fs::WorkspaceRoot`) nor a document property
(`root:`), and the word **Name** says so at every call site.

One adjacent name to keep apart: **`run::address::AddressError`** (`crates/run/src/address.rs:45`)
is the run plane's *task-binding* parser for same-file block refs `[[#^id]]` — a different grammar
with a different owner. This document's error type is **`addr::AddrError`**; the two never meet.

---

## 2. The type — D3 and D5 ruled

### 2.1 Neither D12 story as written

- **Story B — RIDE** (`model/src/lib.rs:1626-1628`) says the prefix *"rides inside the spelling and
 resolves by the same three rules."* Its three rules are `docs.contains_key(s)`,
 `docs.contains_key(s + ".md")` and `resolve_linkpath(...)` — **all lookups into one
 `BTreeMap<String, Document>`.** example `root:page` address misses all three and renders
 `red selector-unresolved`. Unimplementable as written.
- **Story A — PEEL** (`view/src/read_face.rs:833-844`) peels a leading root in `split_lock_ref` —
 and then **discards it**: `LockItem` (`view/src/read_face.rs:405`) has `declared_ref` / `to_path`
 / `to_sel` and no root field.

**Ruled: the address becomes a fallible TYPE carrying an optional root, and the resolver takes a
root-keyed corpus.** Construction is fallible, so the compiler produces the door list (R5): a
string convention that 16 sites re-parse is exactly R5's *"boolean helper a caller may ignore"*.

### 2.2 The type

```rust
// crates/addr — a std-only leaf, zero dependencies, UPSTREAM of `syntax`.

/// A canonical root name: the mount-table key. Lowercase `[a-z0-9-]`, non-empty.
pub struct MountName(String);

/// The agent-plane address `[root:]path[#selector][@fp]`, parsed.
pub struct Addr {
 root: Option<MountName>, // None = the ambient root
 path: String, // never carries a root prefix, by construction
 selector: Option<String>, // verbatim after the first `#`, caret kept
 fp: Option<String>, // a render-face decoration; never part of identity (§ 4.4)
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
(`crates/wire/src/lib.rs:66`). `path_confined` (`crates/wire-serve/src/write.rs:1833`) rejects only
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

### 4.3 `MountName`'s charset — ruled

`[a-z0-9-]`, non-empty. Lowercase only, and **an uppercase byte REFUSES rather than being silently
normalized**: a silent normalization would make two spellings one name, and the ratified law is one
name per root, used everywhere. The corpus index already lowercases its keys (`basename_lc`,
`crates/model/src/lib.rs`), so a normalizing `MountName` would be a second, invisible case rule on
the same address.

### 4.4 `@fp` — parsed, never identity

The `@fp` suffix is a render-face decoration the engine mints on read and **never stores** — the
shipped refusal says so verbatim: *"`@green.…` after a block ref is a render-face decoration the
engine mints on read, never storable content (S10)"* (`crates/wire-serve/src/write.rs:2259`).

> `Addr::parse` **accepts and records** an `@fp` suffix, so a decorated address arriving from the
> render face is recognized rather than mis-parsed into the selector. **The fp is never part of the
> identity a corpus lookup uses** — resolution reads the fp-free projection. This is the same law
> `read::to_model_ref` already follows when it strips a `SecRef::Anchor` before the mint guard:
> *"an address is compared, never stored"* (`crates/wire-serve/src/write.rs:2481-2486`).

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
| D11 | a `create`/`splice` targeting `sessions:notes.md` | **REFUSED at the write door** | `bad_path`, echoing the offending path |

**The acceptance half (parse-acceptance), asserted in the same breath:** D1–D4 must PARSE. A grammar proven
only by what it refuses is indistinguishable from one that refuses everything, and rows D2 and D3
are the ones that keep this law from swallowing the ordinary corpus.

---

## 5. The basename fallback — (c), the cross-root misresolve defect, and no unit owned it

`crates/model/src/lib.rs:1569-1571`, inside `resolve_linkpath`:

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
(`crates/model/src/selector.rs:472`), rendered by `render_ambiguity` and pinned by
`render_ambiguity_carries_d1_teaching_verbatim` (`crates/model/src/selector.rs:679-691`), which
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
pub const GREY_UNMOUNTED_REFUSAL_EXEMPLAR: &str = "grey(unmounted): root 'assets' is not mounted — the address 'assets:domains/media/logo.md#Design' names a root this machine does not bind. Not red: nothing drifted, you just cannot see from here. Refs to mounted roots remain served. Fix: declare 'assets' in ~/MERIDIAN.md as a mount entry (name / path / kind); see [[address-grammar]].";
```

**The teaching tail the pinning test asserts verbatim**, exactly as `TEACH_TAIL` does for D1:

```
. Not red: nothing drifted, you just cannot see from here. Refs to mounted roots remain served. Fix: declare '<root>' in ~/MERIDIAN.md as a mount entry (name / path / kind); see [[address-grammar]].
```

It **names the missing mount** and **teaches the fix**, per D8, and it carries §1a's ratified
sentence — *"Not red — nothing drifted; you just cannot see from here"* — rather than paraphrasing
it.

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
| **`crates/config`** (NEW) | **downstream of `model`** | the `MERIDIAN.md` parse; the mount TABLE (name ↔ vault name ↔ path + kind); canonicalize-at-bind; the deny-ceiling inheritance; the equal-or-nested refusal; the declared-vs-bound check |

Neither is `crates/workspace` — its charter is *"a leaf, `std` + `cache` only"* (`laws.md`
§ Crate charters), and it is **read, not moved**: `config` calls `workspace::deny_reason`.

**Why `addr` must be upstream of `syntax`.** `model`'s dependencies are `syntax` + `blake3`
(`crates/model/Cargo.toml`), so any type living in a crate that depends on `model` is
**architecturally unreachable from `syntax`** — and `syntax::split_wikilink_target`
(`crates/syntax/src/lib.rs:435`) is the wikilink ingress where a cross-root address actually
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
| `addr` | The agent-plane address: `[root:]path[#selector][@fp]` parsed into a fallible type carrying an optional canonical root name, plus the resolution-facing bound-name projection every plane resolves through. A `std`-only leaf upstream of `syntax` — it is where an address becomes a value, so nothing downstream re-splits a string |
| `config` | The `MERIDIAN.md` plane: the one entry point parsed as content (a rev and a fingerprint like any page), and the mount table binding canonical root name ↔ Obsidian vault name ↔ local path (+ kind) — canonicalized at bind, passed through the `workspace` deny ceiling, refusing equal-or-nested mounts, failing loud with no partial table. Downstream of `model`; it projects the bound names into `addr::MountSet` so resolution stays `model`'s |

---

## 8. The mount-path law — (b), canonicalize-at-bind

### 8.1 The rules

> **B-1 — canonicalize at bind.** Every mount path is canonicalized (symlinks resolved, trailing
> separators normalized) **before** it is bound. `workspace::deny_reason` canonicalizes both sides
> of its comparison (`resolve_ref`, `crates/workspace/src/lib.rs:337`), so an uncanonicalized bind
> would be checked against a path that is not the one it binds.
>
> **B-2 — inherit the deny ceiling.** Every canonicalized mount path passes `workspace::deny_reason`
> before binding. A refused mount **fails the whole parse** — no partial mount table, no
> default-root fallback. The reasons are `workspace::DenyReason`'s existing six
> (`crates/workspace/src/lib.rs:253-266`): `FilesystemRoot`, `HomeDir`, `TempDir`, `XdgBaseDir`,
> `CacheRoot`, `MountPoint`. **The ceiling is reused, never re-implemented.**
>
> **B-3 — refuse equal-or-nested mounts, including through symlinks.** After canonicalization, no
> two bound paths may be equal, and none may be a **path-segment-boundary** prefix of another
> (INV-4). Comparison is on canonicalized paths, so a symlink cannot smuggle one tree in twice.

### 8.2 The measured motive — a read-mint bypass, found before the code existed

On this machine, verified (§ 11.2): `«local-path»` is a **symlink** to
`«local-path»`, while `CCC_LLM_WIKI_PATH` carries `«local-path»`
— the real path, with a trailing slash.

A literal env-var inversion (implementation) therefore mounts **one tree twice, under two names**. Two canonical
refs then name one document with **identical `sec_rev`**, and **the read-mint recheck cannot
distinguish them: a receipt minted on ref A would gate a pin on ref B.** That is a read-mint bypass.
B-1 and B-3 are what prevent it, and B-1 alone is not enough — the trailing slash and the symlink
are two different ways to spell the same tree, and only canonicalization collapses both.

### 8.3 The negative cases

| # | Input | Required outcome | Class |
|---|---|---|---|
| M1 | a mount path at `$HOME` | **whole parse fails loud**, naming the reason | `DenyReason::HomeDir` |
| M2 | a mount path at `/`, `/tmp`, an XDG base dir, or under the cache root | **whole parse fails loud** | the matching `DenyReason` |
| M3 | `«local-path»` (symlink) **and** `«local-path»` bound under two names | **whole parse fails loud** — one tree, two names | `duplicate-mount-path` (INV-2, after B-1) |
| M4 | `/a/wiki` and `/a/wiki/sub` both bound | **whole parse fails loud** | `nested-mount` (INV-4) |
| M5 | `/a/wiki` and `/a/wiki-two` both bound | **BOUND — this is legal** | — (`wiki-two` is not a segment-boundary descendant of `wiki`) |
| M6 | a mount path that does not exist or is unreadable | **grey for that root**, the path named; the table stays loaded | `grey(unmounted)` family — **not** a parse failure |
| M7 | a mount path that is a `git-folder`, not a vault | **bound**, kind `git-folder` | — (§ 9, row 7) |

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
 (`crates/syntax/src/lib.rs:435`);
2. **a markdown link URL** — the URL of `[label](url)`;
3. **`meridian-lock` `ref:` values**;
4. **`meridian-lock` `objects:` keys**.

### 9.2 The transform is positional, never a byte transform — and the motive is measured

> **A-1.** implementation's stored-form translation is **positional**. It identifies each address by its
> position in the candidate document and translates the ones in its owned positions. **A blanket
> byte transform over the token `root:` is forbidden.**

**Because `root:` is already a live YAML frontmatter key in the shipped preset/def grammar.**
`crates/preset/src/lib.rs:217` reads it — `fm_scalar(&doc, "root")` — and fixtures carry
`root: SESSION.md` verbatim at `crates/preset/tests/gates.rs:26` and `:368`. A blanket transform
would rewrite that line, corrupt the def, and — because it changes bytes inside a span — **silently
invalidate every pin whose fingerprint covers it.** Frontmatter is not an address position, and the
shipped code already says so in a neighbouring refusal: *"frontmatter is not a claim-link position
(S10/R22)"* (`crates/wire-serve/src/write.rs:2389`).

> **A-2 — the precedent is `strip_fp_candidate`, copied structurally rather than by analogy.**
> `strip_fp_candidate` (`crates/wire-serve/src/write.rs:2345`) identifies each token **in the
> candidate**, attributes it to the payload that supplied it via `classify_fp`
> (`crates/wire-serve/src/write.rs:2190`), and **refuses what it cannot place** — it never
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
| 7 | `git-folder` root — no parse, no sections | § 10.1 below | **an address into a `git-folder` root MUST NOT carry a `#selector`** — refused `AddrError::SelectorOnOpaqueRoot`. Pin grain is the file; the fingerprint is a raw CID of the bytes |
| 8 | Cross-root pin whose target root is later unmounted | § 6.2 R-3 | **grey, never red** — nothing drifted, the ledger stopped being able to measure |
| 9 | A stored `obsidian://` URI hand-edited by a human | § 9.4 row P9 | read-back translation **fails loudly**, never guesses |
| 10 | `MERIDIAN.md` pins a root it declares, and that root drifts | **not this document's** | **Implementation owns it** (mount-as-claim, canonicalize-at-bind ③ — load-bearing, since the fence's only bypass is an edit to `~/MERIDIAN.md`). The address grammar has no part in it |
| 11 | Hook placed, `mrd` later uninstalled | **not this document's** | **Implementation owns it** — fail closed with teaching, `--no-verify` named in the message |
| 12 | Two worktrees, one hook dir, different meridian workspaces | **not this document's** | **Implementation owns it** (D11 — placed per git common dir; the workspace-root ≠ worktree-top-level case is a stated refusal in `mrd skill hook`'s document) |
| 13 | A subprocess forked while `DrawerLock` is held | **not this document's** | **Implementation owns it** — explicit `LOCK_UN` in `Drop` (R19) |

**No row of plan §6 is unanswerable by this grammar.** Rows 10–13 are answered by naming their real
owner rather than by this document inventing an address-plane answer they do not have — which is the
honest reading of Quality Gate 2, not an evasion of it.

### 10.1 The one rule row 7 required that no document stated

Row 7 forced a grammar rule the plan names nowhere:

> **G-1.** A `git-folder` root has **no parse and no sections**. An address naming such a root
> therefore **MUST NOT carry a `#selector`** — one is refused with `AddrError::SelectorOnOpaqueRoot`,
> naming the root and its kind. `assets:media/logo.png` is legal; `assets:media/logo.png#Design` is
> refused.
>
> **The refusal is a RESOLUTION-time refusal, not a parse-time one**, because `Addr::parse` does not
> read the mount table (§ 2.2) and the root's *kind* is a mount-table fact. This keeps the parse/
> resolve split of § 2.2 intact.

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
| what happens to `@fp` | § 4.4 — parsed and recorded, never part of resolution identity |
| the closed error set | § 4.5 + § 10.1 — `BadMountName`, `EmptyMountName`, `EmptyPath`, `AmbiguousColon`, `SelectorOnOpaqueRoot` |
| the parse/resolve split | § 2.2 — parse never reads the mount table; unmounted is grey, not a parse error |
| what `resolve_ref` receives | § 7.2 — `&addr::MountSet`, defined upstream so D4 and D4a both hold |
| the three ingress classes the compiler cannot reach | § 9.1's four positions |
| what the body-level guard must be | § 5.1 C-3 — `resolve_linkpath` refuses a `:`-bearing head, asserted by a test |
| the refusal wording to copy | § 6 — the pinned `const` exemplar, verbatim, with its teaching tail |

**Decisions deliberately left open, and who owns each:**

1. **The `MERIDIAN.md` in-file syntax of a mount entry** — the block grammar and key names for
 name / path / vault name / kind. **`meridian-md-schema.md` owns it**; this document constrains only
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

