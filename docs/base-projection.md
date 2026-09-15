---
type: spec
id: base-projection
status: standing
description: How `.base` (Obsidian Bases) files project into the sql face — membership, relations, references, and the two-witness freshness frame. View-lane only; no wire surface.
owns: [the base projection relations, the base membership rule, the base_fold witness, link.exclusion_path]
---

# `.base` projection

> Standing law: `README.md` (process and standing corrections; the sql face is not agent core, correction C) and `wire-contract.md` (the wire contract).

Status: normative for the `.base` relations of the sql projection, both lanes
(the `:memory:` build and the `sql.duckdb` cache). Law also:
`wire-contract.md` §10.3–§10.4 (view topology), §12 (hash domain);
`node-rev-merkle-spec.md` §4 (leaf/interior encoding); `laws.md` § crate
charters. The exclusion rule (bare-name fallback +
`dangling … AND exclusion IS NULL`) is unchanged (§5.1).

## §1 What a `.base` file is

A `.base` file is an **Obsidian Bases view definition**: one YAML document of
saved queries, with no body, headings, or markdown. Exactly four top-level
keys occur, all optional:

| Key | Shape | Meaning |
|---|---|---|
| `filters` | boolean tree (`and:` / `or:` / `not:` over expression strings) | file-level filter every view inherits |
| `formulas` | map `name → expression` | computed columns |
| `properties` | map `property → display config` | display metadata (`displayName`, …) |
| `views` | list of view objects (`type`, `name`, own `filters`, `groupBy`, `order`, `sort`, `limit`, `columnSize`, …) | the saved views themselves |

Census: 832 member files on two corpora — 812 in a template-stamped tree
(mostly `TASKS.base` / `FLEET.base` / `DECISIONS.base` / `BOARD.base` per
directory, about two dozen distinct), 20 hand-authored. The 812 is the
walk-cost number (§9) and the embed-join population (§5.1). Key counts over
all 1521 `.base` files found (snapshots included): `views` 1509, `filters`
1495, `formulas` 1085, `properties` 491.

**Aliens** exist: `.base` files that are not Bases YAML (a shell script
`gpurun.base`, backup markdown `AGENTS.md.base`); §4.4 gives them rows.
`abc.BASE` is a link-target typo naming no file; it stays dangling (§3, §5.1).

Two corpus facts drive §5: **Bases are embedded and parameterized by context**
— `![[TAG-FILES.base]]` appears 367 times and filters by `this.note["tag"]`
(one file, a different query per embed site). **References inside `.base`
files are expression text, not wikilinks** —
`file.inFolder(this.file.folder)`, `file.hasTag("type/task")`,
`….linksTo(file)`, and zero literal `[[…]]`.

## §2 The two laws

1. **The hash domain does not move** (`wire-contract.md` §12.1). `.base` bytes
   never enter the workspace fingerprint; no prefix bumps; no pin, receipt, or
   attestation change. The projection is a **read-model**, not admission into
   the attested corpus.
2. **No wire surface** (`wire-contract.md` §10.3–§10.4). The relations appear
   in the sql face and nowhere else; no wire op, field, or error names them,
   and the doors (`toc` / `cat` / `read` / `splice` / `links` / …) answer for
   a `.base` path what they answer today.

Because `wire-contract.md` §12.1 forbids rows under an `as_of` fingerprint
stamp that the fingerprint does not cover, the base relations ride their **own
witness**, `base_fold`, in the same `_meridian_view` row, and each base row
carries its own `file_rev` (§6).

## §3 Membership

A file is a member of the base projection iff:

1. its final extension is exactly `.base`, **case-exact** against the name
   read from the directory (`abc.BASE` is not a member), because case-folding
   would canonize typos on APFS;
2. it passes the hash domain's ignore rules: the dot-segment floor and the
   `meridian/domain.md` custom ignore list.

**The base domain is the hash domain's rules with the floor swapped from
`*.md` to `*.base`**, so membership moves with `meridian/domain.md` and there
is no second rule surface.

Paths come from directory enumeration, so they are on-disk spellings. A
directory that cannot be enumerated reads as absence; a member whose bytes
cannot be read is **not** absence and gets a §4.4 error row. Non-UTF-8 paths
cannot match `.base` and are skipped; non-UTF-8 content is §4.4's problem.

The walk lives in `fs` beside `domain_snapshot`, returns raw bytes per member
plus the §6.2 fold, and honors custom-ignored directories. It is a distinct
walk from the link-target probe's fallback index (§5.1), which does not prune
them.

## §4 The relations

Three tables, view-lane only, in both lanes. The DDL is the contract;
`crates/view/src/schema.rs` mirrors it:

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

One `base` row per member file, one `base_view` row per `views:` entry, one
`base_formula` row per formula; *which views/filters does TASKS.base define*
is one SELECT (§11). Filter trees stay **one value on their owner**
(file-level on `base`, view-level on `base_view`), not rows: the leaves are
opaque expression strings anyway.

### §4.2 The encoding — compact JSON

YAML subtrees project as **compact JSON, written order preserved,
structure-preserving**: mappings → objects, sequences → arrays, scalars → JSON
scalars, expression strings → JSON strings byte-for-byte. Nothing is
normalized, sorted, defaulted, or interpreted. JSON rather than raw YAML
because DuckDB ships JSON operators (`filters->'and'`, `json_array_length`,
`LIKE`).

This rule governs; §1's table only describes. A `filters:` that is one bare
expression string projects as a JSON string, not an error. Only `views:` and
`formulas:` make rows; §4.5 rules when their shape does not hold.

### §4.3 Structure is modeled; expressions are not

The projection models the Bases **format** (the four keys, the view list, the
filter tree shape) and serves the Bases **language** (filter and formula
expressions, the view `type` vocabulary) as verbatim text, because the
language is Obsidian's, unversioned and evolving (the mechanism-vs-flow law).
So `type` has no CHECK enum, `config` carries unmodeled view keys as written,
and `extra` carries unmodeled top-level keys **subtree intact**; a fifth
Obsidian key is carried on day one, and a schema amendment is a later choice,
not a prerequisite.

### §4.4 Aliens are rows, not absences

A member that cannot be read as a YAML mapping projects as a `base` row with
`error` carrying the message of whatever refused and every content column
NULL. The classes:

- a shell script or markdown wearing `.base`; non-UTF-8 bytes; YAML whose root
  is not a mapping — the parser's message;
- **duplicate mapping keys anywhere in the document** — the pinned parser's
  rule, not YAML's: serde_yaml refuses the whole document, so one duplicate
  `groupBy:` makes an alien, not last-key-wins; a §10.1 fixture pins the
  verdict;
- **a member whose bytes could not be read** — `error` carries the I/O
  message; `file_rev` and `bytes` are NULL. In the cache delta (§7) a NULL
  `file_rev` compares unequal to everything, so the member re-reads at every
  sync until it heals.

An error row has **zero children**.
`SELECT path FROM base WHERE error IS NOT NULL` is the census of format rot: 3
aliens on the measuring corpus, not counting the duplicate-key class (pinned
by fixture, not census).

### §4.5 The lifting law

A column is lifted only when the value has the modeled shape; everything else
is **carried** as §4.2 JSON in the nearest carrier column:

- `views:` not a sequence, or `formulas:` not a mapping → the subtree lands
  intact in `extra` and makes no rows; the file is not an alien, and a good
  `filters:` beside it still projects.
- a `views:` entry that is not a mapping → a `base_view` row at its `ord`,
  `name`/`type`/`filters` NULL, `config` carrying the entry.
- `name`/`type` present but not strings → carried in `config`; column NULL.
- a formula value that is not a scalar → `expr` carries its compact JSON.

**Alien (`error`) is reserved for §4.4's classes.** Modeling never destroys
data it declines to lift.

## §5 References

### §5.1 md → `.base`: the probe's path

The exclusion rule is unchanged: a wikilink or embed whose target is a real
`.base` file on disk stamps `exclusion = 'non-md'` (literal arm, or case-exact
bare-name fallback with the shortest-path-then-lexicographic tie-break), and
`dangling` excludes explained rows. This spec adds **which** file the probe
resolved:

```sql
-- on link:
    exclusion_path TEXT,               -- the file the probe resolved (either arm), workspace-relative
    CHECK ((exclusion IS NULL) = (exclusion_path IS NULL))
```

`LinkTargetProbe::resolution` returns `(path, reason)`; the projection now
keeps the path. `.base` targets join `base.path` **exactly**, with no basename
re-derivation in SQL; every other excluded class (`.svg`, `.xlsx`,
dot-segment, custom-ignore) gets the same. *Who embeds this base* is a join
(§11).

**Mint rule, added by this spec: a stamp carries the on-disk spelling, or it
does not stamp.** The fallback arm already holds this. The literal arm today
returns the caller's spelling after an `is_file` probe, and on a
case-insensitive filesystem that probe answers true through case-folding:
`[[bases/tasks.base]]` over on-disk `bases/TASKS.base` would stamp a path
`base` does not contain, so the join misses, and `[[abc.base]]` over on-disk
`abc.BASE` would stamp a typo as deliberate. So the literal arm verifies its
final segment against the parent directory's entries case-exactly; a spelling
reached only through case-folding stays unstamped and dangling. The probe is
the shared mint, so the `exclusion` word sharpens wherever it is served
(§10.3, served-content motion).

No new vocabulary word and no change to `dangling`'s definition; stamping
narrows only where case-folding lied. The wire `links` door
(`wire-contract.md` §4.6 `unresolved_reason`) stays **word-only**; widening a
wire map is a wire amendment, out of scope (§10.4).

### §5.2 `.base` → corpus: no edges

The projection mints **zero `link` rows from `.base` content**:

1. **A parameterized base names no fixed target** (§1: 367 embed sites, 367
   queries), so an edge from its text would publish a fact the file does not
   state. The stable file-alone reading (which folders, tags, and properties
   the expressions mention) is servable by text —
   `WHERE filters LIKE '%hasTag(%'` or a JSON walk (§4.2) — and the reader,
   not the engine, interprets a mention.
2. **Nothing wikilink-shaped exists to resolve** (§1): Bases expressions are
   function calls in Obsidian's language, which §4.3 rules opaque.

So `.base` content cannot move the dangling census.

## §6 Attestation and freshness

### §6.1 `file_rev`

`base.file_rev` is the `node-rev-merkle-spec.md` §4 leaf truncated to 16 hex,
the same shape as `doc.file_rev`. It enters **no** merkle interior,
fingerprint, pin, or receipt: it is a staleness and identity witness for one
projection row only.

### §6.2 `base_fold`

`base_fold` is the stamp's second witness, one column on `_meridian_view`:

```sql
-- on _meridian_view:
    base_fold VARCHAR,                 -- 'bf:'+blake3-hex over the member list (below); NULL = the base walk did not run
```

`base_fold` = `bf:` + lowercase hex of blake3 over the member sequence:
members sorted by path byte order, each contributing
`varint(len(path)) ‖ path ‖ 0x00 ‖ leaf32` (`leaf32` = the full 32-byte leaf
of `node-rev-merkle-spec.md` §4, that section's interior recipe with the
workspace-relative path as the name). Zero members fold the empty sequence.
`NULL` means the build was handed no base walk (a docs-only `build_memory`
caller): "not asked", never "empty".

**An unreadable member (§4.4) contributes
`varint(len(path)) ‖ path ‖ 0x01 ‖ [0u8; 32]`**, keeping three states
distinct: readable (`0x00` + leaf), unreadable-but-seen (`0x01` + zeroes),
absent (no contribution). The zero leaf is not a content claim.

`bf:` is **a staleness witness, not an attestation**: compared only against a
re-walk of the same workspace in the same face, never across contexts or on
the wire; `wire-contract.md` §12.3's domain-version laddering does not apply,
and the prefix never advances. A `bf:` value never compares equal to a `b3…:`
`fingerprint`.

`base_fold` comes from the same `fs` walk as membership (§3); `view` folds
nothing itself, so the `build_memory` rule against locally-computed folds
(`crates/view/src/lib.rs`) is untouched: that rule guards the fingerprint,
whose fold must take the domain filter and `version` prefix from
`fs::domain_snapshot`.

### §6.3 The freshness frame names the plane

The sql face's honest-tense frame (`crates/mrd/src/sql.rs`: sample live last)
covers both witnesses: fold the md corpus, re-walk the base members, compare
each against the stamp. A stale verdict **names the plane that moved**,
because the remedies differ.

State space (md live-fold × base live-walk; "not asked" = the stamp's
`base_fold` is NULL):

| md fold | base walk | verdict |
|---|---|---|
| ok, matched | ok, matched | fresh |
| ok, moved | ok, any | stale — "the corpus moved" (+ "and the base plane moved" when both) |
| ok, matched | ok, moved | stale — "the base plane moved" |
| ok | failed | md tense as computed; base plane: **cannot say** (walk failed, error named) |
| failed | any | live source `none`; both planes cannot say (the standing posture) |
| any | stamp not asked | base plane: **"not walked"**, said in the frame; an empty `base` table under a NULL `base_fold` is "not measured", never "measured empty" (`wire-contract.md` §12.1 forbids silence) |

## §7 The cache lane (`sql.duckdb`)

The append-only cache carries the base relations under the protocol of every
other projection table:

- **`hist.base`** (§4 columns + `gen BIGINT` + `tombstone BOOLEAN`),
  **`hist.base_view`**, **`hist.base_formula`** (+ `gen`); latest views
  `main.base` / `main.base_view` / `main.base_formula` pick each path's newest
  generation and drop tombstones (the `hist.doc` QUALIFY-window pattern;
  children by `(path, gen)` semi-join).
- **The cache stays its own manifest.** The latest `base(path, file_rev)` map
  is diffed against the live base walk, as `doc(path, file_rev)` is against
  the live parsed corpus: added/changed/removed append rows and tombstones.
  **An append triggers on either delta**, so base motion appends even when the
  fingerprint did not move; `hist.pin` gains `base_fold` beside the
  fingerprint, the cache's `_meridian_view` VIEW (over that pin ledger, unlike
  the `:memory:` singleton table) selects it, and the no-op check reads one
  row.
- **The affected-set rule covers all link rows, not only dangling ones.** An
  appearing member can newly stamp unresolved rows; a removal must un-stamp
  already-stamped rows (deleting `bases/TAG-FILES.base` un-stamps its 367
  embed rows); a shorter-or-earlier same-basename member must re-point every
  stamped bare row whose old winner is not itself in the delta. Predicate: the
  delta contributes each added/changed/removed member's basename and full
  path, **case-exact**, and a doc re-projects when any of its link rows
  matches on the target's own key (bare basename or literal path) or on its
  `exclusion_path` (full path or basename). These keys stay separate from the
  deliberately lowercased md affected-set keys (§3 and §5.1 forbid
  case-folding).
- **The disclosed approximation narrows.** `.base` motion leaves the store's
  "moves no fingerprint, triggers no append" list; `link.exclusion` /
  `exclusion_path` are consistent with `base` as of the append's own two
  witnesses (no lag bounds promised: the honest-tense law). Other non-md files
  entering or leaving the exclusion domain (`.svg`, `.xlsx`, …) stay
  approximated in the cache lane; rebuild is the repair. The `:memory:` lane
  re-walks everything per query and has no approximation.

## §8 Alternatives held

- **Frontmatter-table reuse** — rejected: `frontmatter` FKs into `doc` (the
  `wire-contract.md` §12.1 coverage lie), its flat-scalar values (that
  spec's § A.6) cannot answer nested `views:` per view, and its CAS tokens
  (`node_rev`, `prop_rev`) serve an `fm_key` splice door that `.base` lacks.
- **One `base` table, the document as one JSON column** — rejected: questions
  arrive per view, so `ord`/`name`/`type` earn columns
  (`WHERE type = 'table'`) and the cache keys child rows `(path, gen)`; the
  remainder stays JSON (`config`, `extra`).
- **Membership in `doc`** — rejected: it changes what `COUNT(*) FROM doc`
  means and forces a kind-filter into every query.
- **Admission into the hash domain** — rejected: the ask is queryability, not
  attestation. Attestation over `.base` would be its own `wire-contract.md`
  §12 ruling.
- **Parsing the expression language or minting edges** — rejected under §5.2.
- **`LIKE '%.base'` query-side filtering** — rejected: it hides genuine rot
  and varies per query.

## §9 Costs

- **`:memory:` lane, per query:** one extra member walk (812 on the measuring
  corpus) and one blake3 + serde_yaml parse per member; a small fraction of
  the md fold, which already hashes the whole md corpus per query. `mrd sql`
  stays a slow operator tool (README standing correction C).
- **Cache lane, per sync:** the base walk + map diff; only deltas append.
  Base-only motion costs one append transaction.
- **Schema:** `SCHEMA_VERSION` 5 → 6 and `CACHE_SCHEMA_VERSION` 5 → 6;
  delete-don't-migrate (a mismatched cache file cold-rebuilds).
- **Dependency:** the Bases parse needs real YAML (arbitrary nesting);
  `serde_yaml` enters `view` as a leaf module, `view`'s charter gains it, and
  the `yaml_confinement` permitted-taker set grows `config, policy` →
  `config, policy, view`, this bullet being the stated deviation. `model`
  stays serde-free (Law 1); `fs` stays YAML-free by charter and hands raw
  bytes up.

## §10 Rollout

### §10.1 Red tests to pin

Each test lands with its code, red first.

1. **Gate:** a `TASKS.base`-shaped fixture answers §11's SELECT with exact
   rows (views, types, file filter).
2. **Alien honesty:** a shell script `x.base` projects as an error-stamped
   row: content columns NULL, row present.
3. **Floor:** `abc.BASE`, dot-segment `.bases/X.base`, and a custom-ignored
   `.base` are not members.
4. **Fingerprint invariance:** adding, removing or editing a `.base` leaves
   the fingerprint byte-identical while `base_fold` moves (§2).
5. **Pairing:** `exclusion_path` is set iff `exclusion` is, carrying the
   tie-broken path for a bare-name `.base` target.
6. **Verbatim expressions:** `this.note["tag"]` survives byte-exact from YAML
   to `base.filters` JSON.
7. **Cache:** base-only motion appends (fingerprint pin unchanged, `base_fold`
   advanced); latest views equal a fresh build.
8. **On-disk spelling (§5.1):** on a case-insensitive volume a case-mismatched
   literal target does not stamp (row stays dangling); a stamped
   `.base`-target row's `exclusion_path` equals a `base.path` byte-for-byte.
9. **Duplicate keys (§4.4):** a duplicated mapping key makes an alien: `error`
   set, zero child rows.
10. **Removal clears the stamp (§7):** after a `.base` member is deleted under
    the cache lane, the next append un-stamps its embed rows (`exclusion` and
    `exclusion_path` NULL, rows back in `dangling`) with no rebuild.

### §10.2 Doc deltas

Authorized by this spec (docs-first): `crates/view/src/schema.rs` DDL + both
version consts; `crates/view/src/store.rs` module doc (hist tables, delta
grain, the narrowed approximation paragraph); `laws.md` `view` charter row
(+ serde_yaml, + base relations) and `fs` row (+ base walk); `status.md`
§ operator SQL face (descriptive, after the binary changes); the sql face's
table-teaching text (refusal/`SQL:` surface) gains the three relations.

### §10.3 Served-face note

- New tables and columns change the sql face's answer set; conformance
  re-records in the same landing (standing served-face rule).
- The §5.1 mint rule also moves served content: on a case-insensitive volume,
  `exclusion` words that stamped only through case-folding stop stamping;
  those rows return to `dangling` and leave the `links` door's map
  (`wire-contract.md` §4.6). Shape is unchanged; the landing flags it.

### §10.4 Out of scope

- The wire `links` door's map (`wire-contract.md` §4.6) stays word-only
  (§5.1); widening it is a wire amendment.
- Door behavior (`toc`/`cat`/`read`/`splice`/…) on `.base` paths: unchanged.
- No Bases evaluation: the engine projects definitions, never runs them;
  compiling Bases filters to SQL over this projection is a separate future
  design.
- Downstream clients' teaching surfaces follow the served face after landing.
- `hist` coverage for non-`.base` exclusion inputs stays the disclosed
  approximation (§7).

## §11 The worked gate

Target `bases/TASKS.base` on the measuring corpus:

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

The §5.1 join, *who embeds this base*, without basename re-derivation:

```sql
SELECT l.src_path, l.kind
FROM link l JOIN base b ON l.exclusion_path = b.path
WHERE b.path = 'bases/TAG-FILES.base';
```
