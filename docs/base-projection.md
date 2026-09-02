---
type: spec
id: base-projection
status: standing
description: How `.base` (Obsidian Bases) files project into the sql face — membership, relations, references, and the two-witness freshness frame. View-lane only; no wire surface.
owns: [the base projection relations, the base membership rule, the base_fold witness, link.exclusion_path]
---

# `.base` projection — Bases files as first-class projection citizens

> **Standing:** Design law is `wire-contract.md` (one contract). DuckDB / `view_path` / SQL boards are **not** agent core (README standing correction C). **Doc correct > code correct; docs first.** See `README.md`.

Status: normative for the `.base` relations of the sql projection (both lanes: the
ephemeral `:memory:` build and the `sql.duckdb` cache). Law also:
`wire-contract.md` §10.3–§10.4 (view topology), §12 (hash domain);
`node-rev-merkle-spec.md` §4 (leaf/interior encoding); `laws.md` § crate charters.

A `.base` file is effectively a YAML file — a markdown file with only
frontmatter — and it defines important queries for different uses; this spec
is that understanding. The exclusion rule (bare-name fallback + `dangling …
AND exclusion IS NULL`) is what keeps the dangling census honest, and its
mechanics stand untouched (§5.1).

## §1 What a `.base` file is

A `.base` file is an **Obsidian Bases view definition**: one YAML document that
declares saved queries over the vault. It has no body, no headings, no
markdown — structurally it is a frontmatter-shaped file whose keys ARE the
content. Exactly four top-level keys occur, all optional:

| Key | Shape | Meaning |
|---|---|---|
| `filters` | boolean tree (`and:` / `or:` / `not:` over expression strings) | file-level filter every view inherits |
| `formulas` | map `name → expression` | computed columns |
| `properties` | map `property → display config` | display metadata (`displayName`, …) |
| `views` | list of view objects (`type`, `name`, own `filters`, `groupBy`, `order`, `sort`, `limit`, `columnSize`, …) | the saved views themselves |

Census on the two measuring corpora: 832 member `.base` files (812 in a large
template-stamped tree, 20 in a hand-authored vault). The two numbers do
different jobs and are labeled so: the 812 is mostly four standing templates
instantiated per directory (`TASKS.base` / `FLEET.base` / `DECISIONS.base` /
`BOARD.base` — distinct content is roughly two dozen files), so it is the
WALK-COST number (§9) and the embed-join population (§5.1), while the 20
hand-authored bases and the template set are the definition-variety evidence.
Key frequencies across every `.base` found (snapshots included, 1521 files):
`views` 1509, `filters` 1495, `formulas` 1085, `properties` 491.
The census also found **aliens**: files carrying the `.base` extension that are
not Bases YAML at all — shell scripts (`gpurun.base`), backup-suffix markdown
(`AGENTS.md.base`). The extension is not proof of the format, and §4.4 is
where that honesty lands. (`abc.BASE` is a different animal — a link-target
TYPO naming no file at all; it stays dangling, §3/§5.1.)

Two corpus facts drive the reference design (§5):

- **Bases are embedded, parameterized by context.** `![[TAG-FILES.base]]`
  appears 367 times on the measuring corpus (the dangling census);
  the embedded file filters by `this.note["tag"]` — the SAME file means a
  DIFFERENT query at every embed site. A base's meaning is not a property of
  the file alone.
- **References inside `.base` files are expression text, not wikilinks.** The
  census finds `file.inFolder(this.file.folder)`, `file.hasTag("type/task")`,
  `….linksTo(file)` — and zero literal `[[…]]` occurrences. There is nothing
  wikilink-shaped inside a Bases file to resolve.

## §2 The two laws this design lives under

1. **The hash domain does not move** (`wire-contract.md` §12.1). The md-only
   floor is structural; `.base` bytes never enter the workspace
   fingerprint, no prefix bumps, no pin/receipt/attestation surface changes.
   This projection is a **read-model** of `.base` content, not an admission of
   `.base` into the attested corpus.
2. **No wire surface** (`wire-contract.md` §10.3–§10.4). The sql face is a
   non-wire operator face; these relations appear in it and nowhere else. No
   wire op, field, or error names them; the doors (`toc` / `cat` / `read` /
   `splice` / `links` / …) are untouched by this spec — whatever a door answers
   for a `.base` path today, it answers tomorrow.

The tension both laws create is the design's center: §12.1 rules that an
enumeration stamped `as_of` a fingerprint must not carry rows that fingerprint
does not cover — *"carrying such a row under that stamp would publish a claim
the stamp does not cover."* So the base relations do not ride under
`as_of_fingerprint`: they ride under their **own witness**, `base_fold`,
published in the same `_meridian_view` row, and every base row additionally
carries its own `file_rev`. One stamp row, two witnesses, each naming exactly
what it covers (§6). The enumerator law's other half — never exclude silently —
is met the same way it is for unserved members: an alien `.base` is a **named
row** (§4.4), not an absence.

## §3 Membership — the base floor

A file is a member of the base projection iff:

1. its final extension is exactly `.base`, **case-exact** against the name
   read from the directory (`abc.BASE` is not a member — the case law the
   probe's fallback arm applies, applied here for
   the same reason: a case-folding match would canonize typos on APFS);
2. it passes the SAME ignore rules the hash domain applies — the dot-segment
   floor and the `meridian/domain.md` custom ignore list — with the md-only
   floor swapped for the `.base`-only floor above.

One sentence teaches it: **the base domain is the hash domain's rules with the
floor swapped from `*.md` to `*.base`.** Membership therefore moves when
`meridian/domain.md` moves, exactly as md membership does; there is no second
rule surface to maintain. Walk posture: membership comes from directory
enumeration, so paths are always on-disk spellings; a directory that cannot be
enumerated reads as absence (its paths are unknowable, the standing posture);
a MEMBER whose bytes cannot be read is NOT absence — the walk saw it, and
§4.4 gives it an error row. Non-UTF-8 paths cannot match `.base` and are
skipped; non-UTF-8 CONTENT is §4.4's problem, not membership's.

The walk lives in `fs` (charter: disk read/walk into the model) beside
`domain_snapshot`, returning raw bytes per member plus the fold of §6.2. It is
a distinct walk from the probe's fallback index — that index deliberately does
NOT prune custom-ignored directories (excluded files are exactly what it
exists to find), while base membership honors them.

## §4 The relations

Three tables, view-lane only, in both lanes. DDL is the contract, as for every
projection table (`crates/view/src/schema.rs` mirrors this section):

```sql
-- The base relations ride the base_fold witness (§6.2), NEVER
-- as_of_fingerprint: their bytes cannot move the workspace fingerprint
-- (§12.1 md-only floor), so their coverage claim is base_fold's alone.
CREATE TABLE base (
    path       TEXT     PRIMARY KEY,   -- workspace-relative ON-DISK spelling (§3 membership)
    file_rev   TEXT,                   -- blake3(whole file)[:16] — leaf-shaped, in NO fingerprint (§6.1); NULL only on an unreadable member
    bytes      UBIGINT,                -- NULL only with file_rev NULL (unreadable member)
    error      TEXT,                   -- NULL = parsed as a YAML mapping; else the parser's or the read's own message
    filters    TEXT,                   -- file-level filters subtree, compact JSON (§4.2); NULL when absent
    properties TEXT,                   -- display-config subtree, compact JSON; NULL when absent
    extra      TEXT,                   -- compact JSON OBJECT: every top-level key §4.5 does not lift, subtree intact; NULL when none
    CHECK (error IS NULL OR (filters IS NULL AND properties IS NULL AND extra IS NULL)),
    CHECK ((file_rev IS NULL) = (bytes IS NULL)),
    CHECK (error IS NOT NULL OR file_rev IS NOT NULL)  -- an unreadable member always says why
);
CREATE TABLE base_view (                -- rides base_fold (see base)
    path    TEXT     NOT NULL REFERENCES base(path),
    ord     UBIGINT  NOT NULL,         -- 0-based document order within views:
    name    TEXT,                      -- lifted when the entry's name is a string (§4.5); else NULL
    type    TEXT,                      -- lifted when a string ('table', 'cards', …) — OPEN SET, no CHECK (§4.3)
    filters TEXT,                      -- view-level filters subtree, compact JSON; NULL when absent
    config  TEXT,                      -- remaining view keys as one compact JSON object in written order, or the whole entry when it is not a mapping (§4.5); NULL when none
    PRIMARY KEY (path, ord)
);
CREATE TABLE base_formula (             -- rides base_fold (see base)
    path TEXT     NOT NULL REFERENCES base(path),
    ord  UBIGINT  NOT NULL,            -- document order within formulas: (unconstrained beside the PK — the frontmatter precedent)
    name TEXT     NOT NULL,
    expr TEXT     NOT NULL,            -- the expression, verbatim scalar or compact JSON of a non-scalar — never interpreted (§4.3)
    PRIMARY KEY (path, name)           -- guaranteed by the PARSER, not by YAML: the pinned parser refuses duplicate mapping keys (§4.4)
);
```

### §4.1 Grain

One `base` row per member file; one `base_view` row per entry of `views:`; one
`base_formula` row per formula. This is the grain the mandate's question has:
*which views/filters does TASKS.base define* is one SELECT (§11). Filter trees
stay **one value on their owner** (file-level on `base`, view-level on
`base_view`) rather than exploding into rows: a boolean tree in rows needs
parent-pointer reassembly for every read, and its leaves are opaque expression
strings either way — rows would model the part that carries no relational
value.

### §4.2 The encoding — compact JSON, structure-preserving

YAML subtrees project as **compact JSON, written order preserved,
structure-preserving**: mappings → objects, sequences → arrays, scalars →
JSON scalars, expression strings → JSON strings, byte-for-byte. Nothing is
normalized, sorted, defaulted, or interpreted — two spellings of the same
query stay two texts. JSON rather than raw YAML because the value plane of
this projection is DuckDB, and DuckDB ships JSON operators: the tree is
queryable (`filters->'and'`, `json_array_length`, `LIKE`) instead of being a
string only an external parser can open.

**This rule governs; §1's shape table describes.** The table says what the
corpus writes today; the encoding carries whatever a file actually holds — a
`filters:` that is one bare expression string projects as a JSON string, not
an error. Where a shape genuinely gates row-making (only `views:` and
`formulas:` make rows), §4.5 says what happens when it does not hold.

### §4.3 Structure is modeled; expressions are not

The projection models the Bases **format** (the four keys, the view list, the
filter tree shape) and serves the Bases **language** (filter and formula
expressions, view `type` vocabulary) as verbatim text. The language is
Obsidian's, unversioned and evolving; parsing it in-engine would hard-code a
moving vocabulary into the mechanism (the standing mechanism-vs-flow law), and
§5.2 shows interpretation would mint false facts even where parsing
succeeded. So `type` carries no CHECK enum, `config` carries unmodeled view
keys as written, and `extra` carries unmodeled top-level keys **subtree
intact** — when Obsidian grows a fifth key, the projection carries its data on
day one and a schema amendment is a later choice, not a prerequisite.

### §4.4 Aliens are rows, not absences

A member the projection cannot read as a YAML mapping — bytes unreadable, or
bytes that do not parse — projects as a `base` row with `error` carrying the
message of whatever refused, and every content column NULL (the DDL CHECK
makes the half-parsed state unrepresentable). The alien classes, named:

- a shell script or markdown wearing `.base`; non-UTF-8 bytes; YAML whose
  root is not a mapping — the parser's message;
- **duplicate mapping keys anywhere in the document** — the PINNED PARSER'S
  rule, not YAML's: serde_yaml refuses the whole document, so one duplicate
  `groupBy:` makes the file an alien rather than last-key-wins. Named
  deliberately: tolerating duplicates would need a hand-rolled event-stream
  walk for a case the live corpus has not produced; if it ever does, that is
  a design amendment, not a parser default to inherit silently. A §10.1
  fixture pins the verdict;
- **a member whose bytes could not be read** — `error` carries the I/O
  error's message and `file_rev`/`bytes` are NULL (the walk SAW the path;
  dropping it would certify an absence nobody measured). For the delta grain
  a NULL `file_rev` compares unequal to everything, so the member re-reads at
  every sync until it heals.

An error row has **zero children**: no `base_view`, no `base_formula` rows.
`SELECT path FROM base WHERE error IS NOT NULL` is the census of format rot —
3 aliens on the measuring corpus today (census run with a duplicate-tolerant
parser, so the duplicate-key class is pinned by fixture, not by census).

### §4.5 The lifting law — columns are lifted, everything else is carried

A column is lifted only when the value has the modeled shape; everything else
is CARRIED as §4.2 JSON in the nearest carrier column. Concretely: `views:`
not a sequence, or `formulas:` not a mapping → the subtree lands intact in
`extra` and makes no rows (the file is NOT an alien; a good `filters:` beside
it still projects). A `views:` entry that is not a mapping → a `base_view`
row at its `ord` with `name`/`type`/`filters` NULL and `config` carrying the
entry. `name`/`type` present but not strings → carried in `config`, column
NULL. A formula value that is not a scalar → `expr` carries its compact JSON.
**Alien (`error`) is reserved for §4.4's classes — a file that cannot be read
as a YAML mapping at all.** Modeling never destroys data it declines to lift.

## §5 References

### §5.1 md → `.base`: the probe's path, projected

The exclusion rule stands unchanged: a wikilink/embed whose target is a
real `.base` file on disk stamps `exclusion = 'non-md'` (literal arm, or
case-exact bare-name fallback with the shortest-path-then-lexicographic
tie-break), and `dangling` excludes explained rows. This spec adds the fact
the probe already computes and then discards — WHICH file it resolved:

```sql
-- on link:
    exclusion_path TEXT,               -- the file the probe resolved (either arm), workspace-relative
    CHECK ((exclusion IS NULL) = (exclusion_path IS NULL))
```

`LinkTargetProbe::resolution` returns `(path, reason)` today; the projection
stops truncating it to the word. Every stamped row gains the resolved path —
`.base` targets join `base.path` **exactly**, with no basename re-derivation
in SQL, and every other excluded class (`.svg`, `.xlsx`, dot-segment,
custom-ignore) gets the same honesty for free. *Who embeds this base* becomes
a join (§11).

**The exactness is earned by a mint rule this spec adds: a stamp carries the
ON-DISK spelling, or it does not stamp.** The fallback arm already holds this
(its candidates come from directory enumeration). The literal arm today
returns the CALLER'S spelling after an `is_file` probe, and on a
case-insensitive filesystem that probe answers true through case-folding — so
`[[bases/tasks.base]]` over on-disk `bases/TASKS.base` would stamp a path
`base` does not contain (a join key that misses), and `[[abc.base]]` over
on-disk `abc.BASE` would stamp a genuine typo as deliberate — the exact
canonization the case-exact guard exists to prevent, on
the arm the guard did not yet reach. So the literal arm verifies its final
segment against the parent directory's entries case-exactly; a spelling that
reaches bytes only through filesystem case-folding is NOT verified, stays
unstamped, and remains honestly dangling. The probe is the shared
mint, so the `exclusion` WORD inherits the same sharpening wherever it is
served — one mint, carried to its stated intent — and §10.3 names it as
served-content motion.

No new vocabulary word, no change to `dangling`'s definition, and stamping
narrows only where case-folding was lying. The wire `links` door
(`wire-contract.md` §4.6 `unresolved_reason`) stays **word-only**: widening a
wire map is a wire amendment, out of this spec's scope and named in §10.4.

### §5.2 `.base` → corpus: no edges, deliberately

The projection mints **zero `link` rows from `.base` content**. Two grounds:

1. **A parameterized base names no fixed target.** `TAG-FILES.base` filters by
   `this.note["tag"]`: at 367 embed sites it is 367 different queries. An edge
   minted from its text would publish a fact the file does not state. The
   file-alone reading that IS stable — which folders, tags, and properties the
   expressions mention — is servable today by text: `WHERE filters LIKE
   '%hasTag(%'` or a JSON walk (§4.2), with the reader, not the engine,
   deciding what an expression mention means.
2. **There is nothing wikilink-shaped to resolve** (§1 census): references
   inside Bases expressions are function calls in Obsidian's language, and
   §4.3 already rules that language opaque.

So the answer to *where do references inside `.base` land* is: **in the text
columns, verbatim** — and the dangling census is structurally untouchable by
`.base` content, which is what the original noise complaint asked for.

## §6 Attestation and freshness — the two-witness frame

### §6.1 What `file_rev` is and is not

`base.file_rev` is the merkle-spec §4 leaf truncated to 16 hex — the SAME
shape as `doc.file_rev`, so operators compare like with like. It participates
in **no** merkle interior, no fingerprint, no pin, no receipt: it is a
staleness and identity witness for one projection row, full stop.

### §6.2 `base_fold` — the second stamp witness

```sql
-- on _meridian_view:
    base_fold VARCHAR,                 -- 'bf:'+blake3-hex over the member list (below); NULL = the base walk did not run
```

`base_fold` = `bf:` + lowercase hex of blake3 over the member sequence:
members sorted by path byte order, each contributing
`varint(len(path)) ‖ path ‖ 0x00 ‖ leaf32` (`leaf32` = the full 32-byte §4
leaf — the interior recipe of `node-rev-merkle-spec.md` §4 reused with the
workspace-relative path as the name). Zero members fold the empty sequence;
`NULL` means the build was handed no base walk (a docs-only `build_memory`
caller), which is "not asked", never "empty".

**An UNREADABLE member (§4.4) contributes
`varint(len(path)) ‖ path ‖ 0x01 ‖ [0u8; 32]`** — the same slot, the §4 type
byte flipped, a zero leaf where no leaf exists (the recipe above covers only
members whose bytes were read). It is the one encoding that keeps the three
states DISTINCT in the
witness: readable (`0x00` + its leaf), unreadable-but-seen (`0x01` + zeroes),
and absent (no contribution at all). Omitting the member instead would fold an
unreadable member and a deleted one to the same value — the §12.1 absence lie
this spec forecloses everywhere else, arriving through the witness. The zero
leaf is not a content claim: `0x01` already says no content was read, and the
row's own `file_rev` is NULL beside it.

The `bf:` token is **a staleness witness, not an attestation**: it is compared
only against a re-walk of the same workspace within the same face, never
across contexts, never on the wire — so §12.3's domain-version laddering has
no job here and the prefix never advances. It shares no prefix space with
`fingerprint` by construction: a `bf:` value can never compare equal to a
`b3…:` value.

This does not conflict with `build_memory`'s own doc comment
(`crates/view/src/lib.rs`), which forbids locally-computed folds: that rule guards
the FINGERPRINT, whose fold must ride the domain filter and `version` prefix
from `fs::domain_snapshot`. `base_fold` is computed by the SAME `fs` walk
that defines base membership (§3) and handed in beside the bytes; `view`
still folds nothing itself, and no prefix exists to get wrong.

### §6.3 The freshness frame names the plane

The sql face's honest-tense frame (order of operations per
`crates/mrd/src/sql.rs`: sample live LAST) extends to both witnesses: fold
the md corpus and re-walk the base members, compare each against the stamp,
and a stale verdict **names the plane that moved** — "the corpus moved" and
"the base plane moved" are different sentences because their remedies differ
(a caller who just wrote markdown should not be told their Bases changed).

The state space, enumerated so no implementer invents a cell (md live-fold ×
base live-walk; "not asked" = the stamp's `base_fold` is NULL — the build was
handed no walk):

| md fold | base walk | verdict |
|---|---|---|
| ok, matched | ok, matched | fresh |
| ok, moved | ok, any | stale — "the corpus moved" (+ "and the base plane moved" when both) |
| ok, matched | ok, moved | stale — "the base plane moved" |
| ok | failed | md tense as computed; base plane: **cannot say** (walk failed, error named) |
| failed | any | live source `none`; both planes cannot say (the standing posture) |
| any | stamp not asked | base plane: **"not walked"**, said in the frame — an empty `base` table under a NULL `base_fold` is "not measured", never "measured empty"; silence is the one option the §12.1 absence rule forecloses |

## §7 The cache lane (`sql.duckdb`)

The append-only cache carries the base relations under the same protocol as
every other projection table:

- **`hist.base`** (columns of §4 + `gen BIGINT` + `tombstone BOOLEAN`),
  **`hist.base_view`**, **`hist.base_formula`** (+ `gen`); latest views
  `main.base` / `main.base_view` / `main.base_formula` pick each path's newest
  generation and drop tombstones — the `hist.doc` QUALIFY-window pattern
  verbatim, children by `(path, gen)` semi-join.
- **The cache stays its own manifest.** The latest `base(path, file_rev)` map
  is diffed against the live base walk exactly as `doc(path, file_rev)` is
  diffed against the live parsed corpus; added/changed/removed append rows and
  tombstones. **An append triggers on either delta** — base motion appends
  even when the fingerprint did not move, so the pin ledger (`hist.pin`) gains
  `base_fold` beside the fingerprint, the cache's `_meridian_view` VIEW (a
  view over the pin ledger, unlike the `:memory:` singleton table) selects it,
  and the no-op check reads one row.
- **The affected-set rule extends to the probe's inputs — over ALL link rows,
  not the dangling ones.** The rows a base delta can move divide in two, and
  the second class is by definition NOT dangling: (i) unresolved, UNEXPLAINED
  rows an appearing member can newly stamp; (ii) already-STAMPED rows whose
  stamp a removal must clear or a tie-break shift must re-point (deleting
  `bases/TAG-FILES.base` must un-stamp its 367 embed rows — under a
  dangling-only predicate they would stay stamped forever while the cache
  reported itself fresh, repairable only by rebuild; adding a
  shorter-or-earlier same-basename member must re-point every stamped bare
  row whose old winner is NOT itself in the delta). So the predicate: the
  delta contributes the name-keys of every added/changed/removed member — its
  basename and its full path, **case-exact** — and a doc re-projects when any
  of its link rows matches on EITHER the target's own key (bare basename, or
  literal path) OR the row's `exclusion_path` (full path, or its basename).
  This key set is SEPARATE from the existing md affected-set keys, which are
  deliberately lowercased; folding the base keys into that set would
  reintroduce through the back door exactly the case-folding §3 and §5.1
  forbid.
- **The disclosed approximation narrows and the remainder stays disclosed:**
  `.base` motion leaves the store's "moves no fingerprint, triggers no append"
  list, and `link.exclusion` / `exclusion_path` are consistent with `base` as
  of the append's own two witnesses (no lag bounds are promised — the
  honest-tense law). Every OTHER non-md file entering or leaving the exclusion
  domain (`.svg`, `.xlsx`, …) remains approximated in the cache lane — no
  snapshot of those exists to diff — and rebuild remains the repair. The
  `:memory:` lane has no approximation: it re-walks everything per query.

## §8 Alternatives held, and why they lose

- **Frontmatter-table reuse** (the mandate's "md file with only frontmatter"
  intuition, taken literally). Three structural mismatches: (1) `frontmatter`
  FKs into `doc`, so `.base` rows would put non-members into the md-corpus
  tables and under the fingerprint stamp — the §12.1 coverage lie; (2) the
  frontmatter value plane is the § A.6 FLAT-SCALAR law — `views:` is a nested
  list of maps, and one opaque blob per key cannot answer *which views does
  this file define*; (3) `frontmatter` rows carry CAS tokens (`node_rev`,
  `prop_rev`) whose purpose is the `fm_key` splice door — `.base` has no
  splice door, and serving CAS-shaped tokens on unwritable rows teaches an
  affordance that refuses. The intuition is honored at the right grain
  instead: a frontmatter-SHAPED file gets its own relations, the way
  frontmatter itself got `frontmatter`/`frontmatter_tag` rather than being
  crammed into `section`.
- **One `base` table with the whole document as a single JSON column** — the
  nearest neighbor, since §4.2 already makes JSON the value plane. Rejected on
  the grain the questions actually arrive at: the mandate's own question is
  per-VIEW (*which views does this file define*), so view identity —
  `ord`/`name`/`type` — deserves column identity, answered by `WHERE
  type = 'table'` rather than a `json_each` unnest re-derived inside every
  query by every caller; the cache's delta protocol wants child rows keyed
  `(path, gen)` like every other child table; and the house precedent is
  exactly this split — `frontmatter_tag` exists because the queried grain
  earned rows even though `frontmatter` already carried the same bytes. The
  un-lifted remainder DOES stay JSON (`config`, `extra`) — the design is the
  precedent's two-layer shape, not a rejection of JSON.
- **Membership in `doc`.** `doc` is the parsed md corpus the fingerprint
  covers; admitting `.base` rows changes what `COUNT(*) FROM doc` means,
  forces a kind-filter into every existing query forever, and re-creates the
  coverage lie with a marker column as apology.
- **Admission into the hash domain.** Attestation over Bases is not the ask —
  queryability is — and the md-only floor is structural and
  prefix-laddered. Rejected without prejudice: an attestation ask over `.base`
  is its own ruling on §12, not a projection detail.
- **Parsing the expression language / minting edges.** §5.2. The `this.*`
  parameterization makes file-alone interpretation FALSE, not merely fragile.
- **`LIKE '%.base'` query-side filtering** is rejected (hides genuine rot,
  drifts per query); this spec is the structural continuation of the exclusion
  rule's direction.

## §9 Costs, named

- **`:memory:` lane, per query:** one extra walk over the tree for members
  (812 on the measuring corpus), one blake3 + one serde_yaml parse
  per member (Bases files are tens of lines). The md fold already walks and hashes
  the ENTIRE md corpus per query; the base walk is a small fraction of that
  standing cost. `mrd sql` remains, by design, a slow operator tool (README
  standing correction C).
- **Cache lane, per sync:** the base walk + map diff; appends only deltas.
  Base-only motion now costs one append transaction where it previously
  (wrongly) cost nothing.
- **Schema:** `SCHEMA_VERSION` 5 → 6 and `CACHE_SCHEMA_VERSION` 5 → 6;
  delete-don't-migrate handles both (a mismatched cache file cold-rebuilds).
- **Dependency:** the Bases parse needs real YAML (arbitrary nesting — the
  hand-rolled scanner argument that admitted `serde_yaml` into `config` for
  arbitrary user frontmatter applies verbatim). The parse lives beside its
  only consumer as a leaf module of `view`, whose charter gains it; the
  `yaml_confinement` instrument's permitted-taker set grows `config, policy`
  → `config, policy, view`, with this paragraph as the stated deviation.
  `model` stays serde-free (Law 1); `fs` stays YAML-free (its charter) — it
  hands raw bytes up.

## §10 Rollout

### §10.1 Red tests to pin (each lands with the code, red-first)

1. **The gate:** a `TASKS.base`-shaped fixture answers §11's SELECT — views,
   types, and the file filter, exact rows.
2. **Alien honesty:** a shell script named `x.base` projects as an
   error-stamped row, content columns NULL, row PRESENT.
3. **The floor:** `abc.BASE` and a dot-segment `.bases/X.base` are not
   members; a custom-ignored `.base` is not a member.
4. **Fingerprint invariance:** adding/removing/editing a `.base` file leaves
   the workspace fingerprint byte-identical while `base_fold` moves — the §2
   constitutional pin.
5. **Pairing:** `exclusion_path` is set iff `exclusion` is set, and carries
   the tie-broken path for a bare-name `.base` target.
6. **Verbatim expressions:** `this.note["tag"]` survives byte-exact from YAML
   to `base.filters` JSON.
7. **Cache:** base-only motion appends (fingerprint pin unchanged, `base_fold`
   advanced) and the latest views equal a fresh build — the store's standing
   invariant, extended.
8. **On-disk spelling (§5.1):** on a case-insensitive volume, a literal
   target whose case mismatches the on-disk name does NOT stamp — the row
   stays dangling — and a stamped row's `exclusion_path` always equals a
   `base.path` (for `.base` targets) byte-for-byte.
9. **Duplicate keys (§4.4):** a fixture with a duplicated mapping key is an
   alien — `error` set, zero child rows.
10. **Removal clears the stamp (§7):** delete a `.base` member under the
    cache lane; the next append un-stamps its embed rows (`exclusion` and
    `exclusion_path` NULL again, rows back in `dangling`) without a rebuild.

### §10.2 Doc deltas riding the code card (docs-first: this spec authorizes them)

`crates/view/src/schema.rs` DDL + both version consts; `crates/view/src/store.rs`
module doc (hist tables, delta grain, the narrowed approximation paragraph);
`laws.md` `view` charter row (+ serde_yaml, + base relations) and `fs` row
(+ base walk); `status.md` § operator SQL face (descriptive, after the binary
changes); the sql face's table-teaching text (refusal/`SQL:` surface) gains the
three relations.

### §10.3 Served-face note

Landing the code changes the sql face's answer set (new tables, new columns):
conformance re-records in the SAME landing per the standing served-face rule.
The §5.1 mint rule is also served-CONTENT motion beyond the sql face: on a
case-insensitive volume, `exclusion` words that previously stamped through
case-folding stop stamping (rows return to `dangling`, and the §4.6 door's
map shrinks by the same rows). Shape everywhere is unchanged; the content
motion is the case-exact guard reaching the literal arm, and it is
named here so the landing flags it rather than discovering it.

### §10.4 Out of scope, named

- The wire `links` door §4.6 map stays word-only (§5.1); widening it is a wire
  amendment of its own.
- Door behavior (`toc`/`cat`/`read`/`splice`/…) on `.base` paths: unchanged,
  whatever it is today.
- No Bases EVALUATION: the engine projects definitions; it does not run the
  queries. Compiling Bases filters to SQL over this projection is a real
  future direction and a separate design.
- Downstream clients' teaching surfaces follow the served face after landing.
- `hist` coverage for non-`.base` exclusion inputs: stays the disclosed
  approximation (§7).

## §11 The worked gate

Measured target (`bases/TASKS.base` on the measuring corpus):

```yaml
filters:
  and:
    - file.hasTag("type/task")
views:
  - type: table
    name: Board
    groupBy:
      property: status
      direction: ASC
    order:
      - file.name
      - status
      - file.folder
  - type: cards
    name: Kanban
    groupBy:
      property: status
      direction: ASC
    order:
      - file.name
      - file.folder
```

```sql
SELECT b.filters, v.ord, v.name, v.type, v.config
FROM base b JOIN base_view v USING (path)
WHERE b.path = 'bases/TASKS.base' ORDER BY v.ord;
```

| filters | ord | name | type | config |
|---|---|---|---|---|
| `{"and":["file.hasTag(\"type/task\")"]}` | 0 | Board | table | `{"groupBy":{"property":"status","direction":"ASC"},"order":["file.name","status","file.folder"]}` |
| `{"and":["file.hasTag(\"type/task\")"]}` | 1 | Kanban | cards | `{"groupBy":{"property":"status","direction":"ASC"},"order":["file.name","file.folder"]}` |

And the join §5.1 unlocks — *who embeds this base* — with no basename
re-derivation:

```sql
SELECT l.src_path, l.kind
FROM link l JOIN base b ON l.exclusion_path = b.path
WHERE b.path = 'bases/TAG-FILES.base';
```
