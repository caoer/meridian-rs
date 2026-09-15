---
type: contract
id: wire
status: standing
description: Standing wire constitution. One document. Docs define law; code may lag.
owns: [the wire constitution — nouns, ops, guards, receipts, errors]
---

# Wire contract

> **Standing law.** One wire constitution, no v2/v3 stack; **doc correct > code correct** (process: `README.md`; hashes: `node-rev-merkle-spec.md`, `fingerprint-norm-spec.md`).  
> **Always on:** (A) mint address = segments only `{"hpath":[{"h":"Goals"},{"h":"Q3"}]}`; (B) receipt armed facts on the wire are normative, no second path; (C) DuckDB / `view_path` are not agent-core wire.

**Content-hash noun:** `fingerprint` (`b3:…`), not the workspace directory. **Worked values:** hashes, spans and counts in §§0.3–§12 are blake3 over the §0.3 fixture bytes. **Notation:** `[[…]]` in the fixture fence are data bytes for `resolve` / `links`, not doc links.

## §0 Reading frame

### §0.1 Ordered goals

Goals are ranked: a lower never overrides a higher; no agent adjudicates rank.

**GOAL 1 — works-for-us (primary):** the ruled grammar (§2.1), the geography law (§5.3), and:

1. Interface: five verbs (`toc`/`cat`/`edit`/`append`/`resolve`); sections-as-files; Edit-exact old/new scoped to a section; **requests never require revs, receipts always return them**; no byte offsets in the interface; mandatoriness = host policy ratchet; resolve = walk plane.
2. Amendments: the fix-at-freeze list (§18), one block-id charset (§2.4), node-grain deltas (§7.4).
3. Foundations: Starlark evaluator (§11.4); optional, wire-agnostic view organ (§10.3–§10.4).
4. Review checklist A1–A14; ratified convergence items: mint partition, 16-hex rev, md-only, two-stage resolve, app-oracle GT.
5. Design law: delta noun, receipts, actor/now as wire inputs, rules-as-data, view topology, honest limits. Collisions resolve to it, never silently.

**GOAL 2 — must-work-in-Obsidian (the floor):** everything minted and emitted works in Obsidian, one-way: no promise to reproduce the app outside our grammar, and out-of-grammar input (e.g. `_`-bearing anchors) refuses loudly; `resolve` walks the app's grammar best-effort (§4.5).

### §0.2 Ruled gates

| Item | Ruling | Where |
|---|---|---|
| Delta grain at birth | node-grain | §7.4 |
| Starlark ratification | ratified | §11.4 |
| Rung-5 view organ | optional; no engine-named wire elements | §10.3–§10.4 |
| `_` block-id charset | one app-exact charset (two-plane split vetoed) | §2.4 |

Deviations and waivers: §18.

### §0.3 The worked fixture

Every example runs against workspace `wsfix/`, three timeline states. **S0** is exactly these three files:

```
notes/plan.md            136 bytes   file_rev e3c4acaceb75b907
receipts/2026-07-18.md    26 bytes   file_rev 920a40c4ee23d37c
.github/README.md         11 bytes   (md, but OUTSIDE the hash domain — default ignore, §12)
```

`meridian/domain.md` is **absent at S0**: R0 covers `notes/plan.md` and `receipts/2026-07-18.md` alone. When present it hashes itself (§12.1 rule 3; `crates/fs/src/domain.rs`), so writing it moves the fingerprint off R0; only the §12.3 example writes it, in the two forms printed below.

`notes/plan.md` at S0, exact bytes (LF endings, trailing newline):

```markdown
---
title: Plan
---
# Goals

Ship the contract.

## Q3

ship by August

## Q4

- item one
- see [[2026-07-18]]
- blocked on [[roadmap]]
```

Other fixture bytes (S1/S2 receipt entries: §6.3):

- `receipts/2026-07-18.md` at S0, 26 B (`—` is 3-byte UTF-8): `# Receipts — 2026-07-18` + LF.
- `.github/README.md`, 11 B: `# CI notes` + LF.
- `drafts/tmp.md` (§12.3 only), 8 B: `scratch` + LF.

`meridian/domain.md` **v0**, 33 B (a `version`, no custom ignore list):

```markdown
---
version: 0
---
# Hash domain
```

`meridian/domain.md` **v1**, 57 B (custom ignore list):

```markdown
---
version: 1
ignore:
  - "drafts/**"
---
# Hash domain
```

Timeline: **S0** →(E3 edit)→ **S1** (`plan.md` 139 B, receipts 287 B) →(E4 append)→ **S2** (`plan.md` 150 B, receipts 550 B). Roots:

| State | Fingerprint (full width, never truncated) |
|---|---|
| R0 (S0) | `b3:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9` |
| R1 (S1) | `b3:7f3b44376c719be236279e168c22fa2f4d346cd6e5da5bcf0784adb72e7c1f12` |
| R2 (S2) | `b3:6e866e13b5e65ef9961c050f8a621cf1980b00ee293be650deef5f4dbc6823f0` |

## §1 The five wire nouns

Five nouns: `Path`, `Span`, `NodeRev`, the content-hash type (`Root` in code; design noun **fingerprint**) from `crates/wire/src/lib.rs`, and **Delta**, born here.

| Noun | Shape | Law |
|---|---|---|
| `Path` | string, `/`-separated, workspace-relative, UTF-8 | never absolute, no `..`; root ambient (`fs::WorkspaceRoot`); agent-plane `[root:]path` resolves at the door, the wire carries the rel half (§ A.12) |
| `Span` | `[start, end)` byte pair, u64, serialized `[s,e]` | UTF-8 **bytes** on raw disk content, never chars or UTF-16 (`Loc.offset` UTF-16 conversion: conformance harness only) |
| `NodeRev` | 16 lowercase hex = `blake3-256(span bytes)[:16]` | opaque, equality-only; threat §13.1 |
| `Fingerprint` | `"b3:" + 64 hex`, full width | algorithm+domain prefix, bumped on domain-rule change (§12.3); never truncated |
| `Delta` | change-fact object, §7 | node-grain at birth; stable shape; replay ≡ live |

**Span sub-laws:** a section node is heading-inclusive, newline-inclusive, no trim, running to the next heading of level ≤ its own (else EOF) and containing deeper headings (node-rev-merkle-spec §5 pins stay live); a leaf block node excludes its final line terminator; an inline node includes its delimiters; a span splitting a multi-byte character refuses `bad_request` (guarantor: parser token discipline on reads, the reparse gate on writes; §15).

**Rev sub-laws:** `node_rev` hashes the **full span bytes**, heading-inclusive, so a heading rename invalidates the token. `content_span` (first byte after the heading line's terminator to the section span's end) mints **no** rev: one rev per node, the full-span hash (CAS comparison: §5.1). `file_rev` = `blake3(whole file bytes)[:16]`, same family and width.

**One hash family:** BLAKE3-256 for rev, file_rev, leaf, interior and fingerprint (this schema is the rung-2 wire amendment deciding it; node-rev-merkle-spec §1).

### §1.1 Replaceability test

Can a consumer outside the reference host replace each convention without engine changes?

| Noun/field | Verdict |
|---|---|
| `Path` | any UTF-8 relative path; no host naming |
| `Span` | raw byte math; no convention |
| `NodeRev`/`Fingerprint` | opaque tokens; algorithm swap = domain-prefix bump (§12.3), engine unchanged |
| `Delta` | generic node-change facts; no host vocabulary |
| `actor` | opaque, never parsed; `agent:b0864fb2` is only a host convention (§9) |
| `now` | caller-supplied RFC 3339; format validated, never generated (§9) |
| `receipt` address | any md path + anchor; rendering is a default template, the armed wire facts are normative (§6.4) |
| rule packs | pack data behind a generic manifest; no evaluator hard-coded (§11) |

## §2 One address grammar, two planes

The strict mint plane and the Obsidian walk are different verbs with different response types, and the walk response has no rev field, so a ref cannot arm a write: the mint partition.

### §2.1 The strict mint plane

Write targets and strict reads (`cat`/`splice` targets; echoes in `toc` rows, receipts, deltas, verdicts) name nodes by **exact name only**, in three forms:

| Form | Shape | Semantics |
|---|---|---|
| hpath | `{"hpath":[{"h":"Goals"},{"h":"Q3"}]}` | per-segment **byte-equality** on the real containment tree; optional occurrence `{"h":"Beta","n":2}` (1-based, document order among identical raw texts at that position). No join string: `#A#a/b` vs `#A#a#b` is unrepresentable. **Zero segments (`[]`) address the document node** (file span `0..len`, parent of every top-level heading) — the create door's empty `parent_hpath` |
| anchor | `{"anchor":"r-000042"}` | block id, exact match; node = the id's host block (Obsidian attachment law, `model::anchor_host_span`): a tail id keys its enclosing block (paragraph run, callout, table, list item line, heading line); an own-line id attaches to the nearest preceding block through blanks or joins an adjacent paragraph/list item; a document-start orphan or frontmatter caret keeps its own line. Duplicate id in one file: mint plane refuses `ambiguous_ref`, walk plane follows the app (last wins, silent) |
| fm_key | `{"fm_key":"title"}` | top-level frontmatter key; the node is the full key line (frontmatter is nodes, never ref grammar; `#:key` is dead) |

A stale name refuses `ref_not_found`.

### §2.2 The walk plane

`resolve` alone accepts the Obsidian ref algebra: raw linktext `path#sub`, `#sub`, `path#^id`, no brackets (the interface strips `[[…|alias]]` sugar; one wire spelling). It walks the app's loose grammar best-effort (§4.5) and returns location facts only; an interop ref pays one `toc`/`cat` hop to become a write target.

### §2.3 Layering

The Obsidian algebra is a syntactically disjoint, read-only input to the one strict grammar; revs, fingerprints, occurrence index and domain config never appear in it. `[[##`/`[[^^` search syntaxes are UI, out of scope.

### §2.4 The block-id charset

Block ids match `[A-Za-z0-9-]+` (Obsidian app-exact) on **both** planes. No `_` in newly minted ids; a `_`-bearing anchor is outside the strict-plane grammar (`bad_request`). No corpus-wide re-id migration: a mint-guard enforces the charset going forward, with a frozen-fixture exemption owned by the implementation (§13.8). The `_`-bearing probe stays frozen in the `obsidian-compat@1.12.7` pack, pinning the app's real treatment of legacy `_` ids.

## §3 Frame layer, correlation, discovery

### §3.1 Frames

NDJSON, one JSON object per line, on the daemon's unix socket (§3.3): frames only, logs to stderr, so `echo '{"id":1,"op":"hello",…}' | nc -U "$SOCKET"` works by contract. Three frame types, classified by the **raw** `id` key: present → Request/Response (correlated); absent → Notification (§7 deltas ride here).

**Raw-lexeme id law:** classification and id validation use the raw JSON `id` lexeme **before** typed decode. Valid ids are JSON integer lexemes in `[0, 2^53)`:

| raw lexeme | verdict |
|---|---|
| `7` | valid |
| `"7"` | `bad_request` (string) |
| `3.5`, `-1` | `bad_request` |
| `3e0` | `bad_request` (not an integer lexeme, though JSON-equal to 3) |
| `9007199254740991` (2^53−1) | valid |
| `9007199254740992` (2^53) | `bad_request` |
| `18446744073709551616` (2^64) | `bad_request`, never a Notification |

A non-conforming id is never echoed: the error frame carries `id:null` and the lexeme verbatim in `id_raw` (string). A pipelining client treats any `id:null` frame as corruption (fail all outstanding, respawn); a single-shot client reads `id_raw`.

**Correlation:** one response per request, id echoed by value, in-flight ids unique. NDJSON lines, no length prefix (`crates/transport` NdjsonCodec), no frame-size bound; a line-length cap is open (§18 row 14).

### §3.2 hello / caps

```json
{"id":1,"op":"hello","proto":1,"client":"md-cli/0.3"}
{"id":1,"ok":true,"body":{"proto":1,"server":"meridian-daemon/1.0.0",
  "caps":["toc","cat","extract","resolve","resolve.content","links","links.require_fingerprint",
          "splice","splice.if_node_rev","splice.if_fingerprint","splice.dry","splice.receipt",
          "splice.verdicts","fingerprint","diff","sub"],
  "fingerprint":"b3:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9"}}
```

`caps` is the complete set; no version sniffing. Shown: the sixteen-cap core; the daemon's base set (`crates/registry/src/server.rs` `CAPS`) adds `splice.fields`, `run.fields`, `run.ambient` — nineteen (v2 spelling identical minus the fingerprint renames). A negotiated v3 session is pushed twenty-three more: `read`, `check_write`, `splice.plan_edits`, `splice.pin`, `pin-cross-root`, `splice.pin.proof`, `splice.set`, `splice.fields`, `splice.create_rev`, `create`, `remove`, `mounts`, `mounts.primary`, `mounts.alias`, `hello.identity`, `script`, `run`, `run.mode`, `run.input`, `walk`, `sql`, `splice.remove`, `scoped-guards` (§A.3/§A.5/§A.6.6/§A.7/§A.8/§A.10/§A.11/§5.4). `wire-serve/src/rev.rs` is the authority; a count here that disagrees is this sentence's defect.

Field-only amendments are dotted `op.field` strings: `mounts.primary`/`mounts.alias` (the mounts row's primary designation and root alias, §A.5), `splice.fields` (the §A.2.1 middleware passthrough). `pin-cross-root` is a behavior cap on the existing `splice.pin` field (§A.3). `scoped-guards` (a behavior cap, same pattern) covers the whole scoped-premise family: `guards[]` entries' `scope`/`scope_bytes` pairs, the singular `scope` on `splice` (single and set form) and `script`, and the `fingerprint` op's mint arm with its own pair (§4.7, §5.4–§5.7); never pushed to a frozen v2 session, and un-negotiated use of any guard-family field refuses `bad_request` at the strict wall. Hello-body `fingerprint` is optional; when present it is the first ambient fingerprint.

**Hello is config-grade.** A workspace `hello` pins storage, validates the domain config (ambiguous or unreadable: `io_error{cause}` at the handshake) and binds the connection. It never walks the corpus, builds the engine or queues behind corpus-scoped work: `fingerprint` is present exactly when the resident fold is readable this instant, read without waiting (absent when cold or under lock contention), and the first corpus read starts the warm. Discovery ops (`hello`, `mounts` § A.5) answer at config cost.

**The cold build never blocks the read door for minutes.** With no resident engine, a warm-engine op (read family, `sql`, the `script` entry pass) starts the drawer rebuild in the background — one per workspace, however many callers — and waits a short bounded time whose value is engine-internal and unpublished (host deadlines are host knowledge, §8.1) and whose order is guaranteed: well under any sane op deadline.

- Drawer lands inside the wait: serves on first contact; an ordinary workspace never changes shape.
- Still rebuilding when the wait expires: `corpus_warming` (§8, retry); further corpus reads during the rebuild refuse the same in milliseconds; the first read after it lands serves.
- Rebuild fails: `io_error{cause}` (env) to the read that started it if the failure lands inside the wait, else to the next corpus read; a later read starts a fresh rebuild, so warming never masks a broken corpus.
- Warm workspace: the currency pass stays inline at every read at the vouched grade (`node-rev-merkle-spec.md` §6.7: O(dirty) via the event feed's cookie proof, extent-refresh floor O(domain) `stat`s / O(delta) parses on a named miss; `run-plane.md` § What an entry costs); only the cold whole-corpus build leaves the read door.
- In-process registries (CLI direct lane, test fixtures): inline build; no daemon, no deadline, blocking is honest.

**Rev-presence law:** `node_rev` is MUST on every `toc`/`cat`/`extract` node whenever `splice ∈ caps`.

**Evolution:** strict server (unknown request fields and enum values rejected loudly); tolerant client (unknown response fields and open-kind strings ignored). Server-first rollout.

**Grain of the strict wall:** "request fields" means **every object at every depth** — the op object, each `edits[]` edit, its edit-shape body (`match`/`put`), its `target`, each hpath segment. The wall refuses at each grain and names the legal field set it checked; else a nested `if_rev` (typo for `if_node_rev`) is dropped and the write silently unguarded. Binds every decoding door, including the CLI seam reading `edits` off stdin (§4.4).

### §3.3 Hosts — one wire door

The daemon's unix socket, speaking the §3.1 line dialogue, is the **only** wire door: one binary, one transport (hello's `server` names it, §3.2).

- `actor` rides each frame as data (§9); identity is not the door's job. One resident server, connection-scoped transport, per-frame identity.
- `daemon_only` (§8) is unmintable: every wire door is daemon-backed and has the resident corpus index.
- In-process paths (`mrd` over the engine crates) are out of wire scope (§ A.1); a CLI is not a wire door.

## §4 The op surface

Eleven ops. The § A.3 additions (`read`, `create`, `remove`) and § A.7 `script` land on top, not re-tabled; the five-verb interface maps 1:1 onto the original ten (§4.8). A read op belongs on the wire only when it feeds an action; orientation reads are dashboard-only.

| Op | Rung | Class |
|---|---|---|
| `hello` | 1 | discovery |
| `toc`, `cat`, `extract` | 2 | single-file facts, the **mint surface** |
| `resolve` | 2 | walk plane, never mints |
| `links` | 5 (view-shaped) | corpus fact, staleness triple (§10) |
| `splice` | 4 | the only write op, batch-only |
| `check_write` | 4 | write pre-flight: the splice verdict standalone, read-only (§ A.3) |
| `fingerprint` | 3 | integrity fact |
| `diff` | 3 (reserved shape, standing) | replay (§7) |
| `sub` | 5 | delta transport (§7), served at the daemon door (§4.7) |

No separate `Guard` op: the integrity surface is `fingerprint` + `splice.if_fingerprint` + `diff`, one grammar.

### §4.1 toc — the map, revs riding along

Request `{"id":2,"op":"toc","path":"notes/plan.md"}`; response at S0 (every value computed):

```json
{"id":2,"ok":true,"body":{
 "path":"notes/plan.md","file_rev":"e3c4acaceb75b907",
 "fingerprint":"b3:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9",
 "nodes":[
  {"kind":"frontmatter","span":[0,20],"node_rev":"26796ebec5d0bf1a",
   "text_prefix_16b":"---\ntitle: Plan\n","keys":["title"]},
  {"kind":"heading","level":1,"hpath":[{"h":"Goals"}],"span":[20,136],
   "content_span":[28,136],"node_rev":"a6665baff294bd04","text_prefix_16b":"# Goals\n\nShip th"},
  {"kind":"heading","level":2,"hpath":[{"h":"Goals"},{"h":"Q3"}],"span":[49,72],
   "content_span":[55,72],"node_rev":"33d5b0e1b27cb48b","text_prefix_16b":"## Q3\n\nship by A"},
  {"kind":"heading","level":2,"hpath":[{"h":"Goals"},{"h":"Q4"}],"span":[72,136],
   "content_span":[78,136],"node_rev":"4b8bc385a58da0e0","text_prefix_16b":"## Q4\n\n- item on"}]}}
```

`toc` is the complete write kit: `hpath` + `node_rev` per section, anchors with revs when present, frontmatter keys. The header `fingerprint` feeds a later `if_fingerprint` (the commit-guard idiom). `content_span` serves heading-preserving display and mints nothing (§1).

**Displayed bytes are not hashed bytes.** Q3 displays 17 content bytes; its rev hashes the 23-byte span. A rendered block line likewise drops the trailing ` ^id` the leaf span carries. Compare against `cat`'s full span bytes (§4.2), never a display column.

**The anchor toc row** (`receipts/2026-07-18.md` at S1, the fixture's only block-id-bearing file; `plan.md` has no block id, hence no anchor row above):

```json
{"id":4,"op":"toc","path":"receipts/2026-07-18.md"}
{"id":4,"ok":true,"body":{
 "path":"receipts/2026-07-18.md","file_rev":"51ad6428f5b5a898",
 "fingerprint":"b3:7f3b44376c719be236279e168c22fa2f4d346cd6e5da5bcf0784adb72e7c1f12",
 "nodes":[
  {"kind":"heading","level":1,"hpath":[{"h":"Receipts — 2026-07-18"}],"span":[0,287],
   "content_span":[26,287],"node_rev":"51ad6428f5b5a898","text_prefix_16b":"# Receipts — 2"},
  {"kind":"list_item","anchor":"r-000042","span":[26,286],
   "node_rev":"60bbee70d4a63a48","text_prefix_16b":"- splice notes/p"}]}}
```

The `^r-000042` block is a `list_item` node keyed by its `anchor` ref (§2.1), with its own `node_rev` over the block-leaf span, terminator excluded: `[26,286]`, the same bytes the §4.4 receipt facts arm (§6.3's E3 line is these 260 bytes). The lone top-level heading spans the whole file, so its `node_rev` equals `file_rev` (`51ad6428f5b5a898`). An anchor is a write target by the same one-hop path as a section.

### §4.2 cat — read one section, not the disk

`cat` returns the **full span bytes** (heading included); `node_rev` is blake3 of exactly those bytes. `sec` absent → whole file + `file_rev`.

```json
{"id":3,"op":"cat","path":"notes/plan.md","sec":{"hpath":[{"h":"Goals"},{"h":"Q3"}]}}
{"id":3,"ok":true,"body":{"span":[49,72],"node_rev":"33d5b0e1b27cb48b",
 "content":"## Q3\n\nship by August\n\n"}}
```

### §4.3 extract — the extract surface

`{"op":"extract","path":…,"kinds":[…]}` stands as specified in `crates/wire` §5: full node objects; an 11-variant kind enum whose declaration order is the sort tie-break ordinal; per-kind `info`; `text_prefix_16b` (implemented and tested in `crates/wire-map`); total node order (`span.start` asc, `span.end` desc, kind ordinal). **An unknown value in `kinds` refuses `bad_request{"unknown_kinds":[…]}`**: the strict-server law applied to values.

### §4.4 splice — the only write op (batch-only, one response shape)

The Edit-tool model is the wire write grammar: exact `old`/`new` replacement, no regex, no fuzz, uniqueness required, matched **server-side within the target's full span bytes**. No request carries a client span, so a wrong-offset write is unrepresentable.

```json
{"id":42,"op":"splice","path":"notes/plan.md",
 "actor":"agent:b0864fb2","now":"2026-07-18T20:31:04Z",
 "receipt":{"path":"receipts/2026-07-18.md","anchor":"r-000042"},
 "if_fingerprint":"b3:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9",
 "edits":[
  {"target":{"hpath":[{"h":"Goals"},{"h":"Q3"}]},
   "edit":{"match":{"old":"ship by August","new":"ship by September"}},
   "if_node_rev":"33d5b0e1b27cb48b"}]}
```

**The CLI seam reads the `edits` value, not this envelope.** `mrd put <PATH>` takes the bare array under `"edits"` on stdin (`id`, `op`, `path` are argv's) and refuses a whole request object, exit 2 before any engine contact: `the edits on stdin are not the §4.4 batch shape … stdin takes the BARE edits ARRAY, not the wire §4.4 request object: send the value of its "edits" field (id / op / path are argv's here)`.

**Edit shapes — exactly three.** `match` and `put` state a node's bytes; `remove` states that the node is not there (§ A.6.6).

| Shape | Semantics |
|---|---|
| `match{old,new}` | `old` occurs exactly once in the target's full span bytes and becomes `new`; zero → `no_match`, two+ → `not_unique{matches}` |
| `put{at,text}` | `at:"all"` replaces the full span, heading included; `at:"content"` the content span, heading preserved; `at:"end"` inserts `text` at the span-end byte (the append verb) by **raw byte concatenation, no synthesized separator** — a `text` that must start a new line carries its own leading `\n`, a terminator-less final line is the caller's to handle, and a result that loses containment refuses `would_corrupt`; `at:"upsert"` sets a frontmatter key, create-or-replace, `fm_key` targets only (§ A.6.3a) |
| `remove{}` | **`fm_key` targets only**: strikes the key line from the frontmatter block, moving the key to R4's absent state; no fields, no `text`. Absent key → `ref_not_found`; `hpath`/`anchor` target → `bad_request` (§ A.6.6) |

**Batch laws.** Batch-only; one response shape; every target and guard resolves against the **pre-batch** state. The batch commits atomically through one reparse, and an identity that does not survive it refuses `would_corrupt`. `dry:true` runs everything except disk: same response shape, `fingerprint_after:null`, no receipt written.

**Replaced regions** must be pairwise disjoint, else `bad_request{"overlap":…}` names the offending edits and a remedy. Region: `match` the matched bytes; `put at:"all"/"content"` that span; `put at:"end"` the zero-width insertion point; `remove` the key's grain span **plus its line terminator** (§ A.6.6). The grain is the region, not the target's full span, so nested *targets* compose when their regions differ: an append to a section plus a sibling-section birth under its parent is one batch, and a replace plus an `at:"end"` on one section composes, while two `replace_section`s on one section overlap. Zero-width regions at one byte are disjoint and apply in request order.

**The `would_corrupt` families.** One code; the body's `family` discriminates, and a caller dispatches on it, never on which extras are present.

| `family` | Extras | What died | Remedy the refusal teaches |
|---|---|---|---|
| `containment_lost` | `lost:[hpath…]` | a section **byte-disjoint from every edit** no longer resolves after the reparse | from `cause` (below) |
| `target_identity` | `target` (the edit's ref, §2.1 grammar) | the **edit's own target** no longer resolves, so its armed facts are unrepresentable | re-supply the identity the slot destroys (a section heading for `at:"all"`, a line-final block id for `at:"end"` on an anchor); to retire one, name it: `remove` on an `fm_key`, or the parent's content slot for a section or anchor |
| `transition_unrepresentable` | `target` (as above) | the edit **wrote past the span it named**: its bytes fall outside the target's post-batch span, so the node never received them and its `node_rev` cannot move | drop the trailing separator from `text`, or aim at the enclosing section, whose span contains the bytes |

**`transition_unrepresentable` is keyed on the mechanism, never on the `at:` scope.** `node_rev` is a function of the span bytes (node-rev-merkle-spec §2). An anchor block-leaf (§1) or `fm_key` leaf span ends before its line terminator, so a `text` carrying a separator writes a byte the node never covers; arming that would claim a false transition and silently disarm `if_node_rev` (two callers holding one rev would both succeed, neither told; repeated writes would pile up at one offset — appends in reverse order, a blank line per rewrite). The test: **a real byte change whose target's `node_rev` did not move is refused.** Containment is not the test: an `at:"end"` section append whose `text` opens a sibling heading places bytes outside the section yet moves its rev, and commits with a live guard. A caller may not hand a terminator-excluding leaf a `text` ending in a separator.

⚠️ Scope-keying would be a defect: the escape is reachable through `put{at:"end"}`, `put{at:"all"}`, `put{at:"content"}` **and `match`**, which is no `at:` scope (six such cells measured across the anchor-leaf and `fm_key` doors). Only the containment and identity families sit ahead of this one; all three are measured on one reparse, never inferred from the edit text.

**`remove` does not draw `target_identity`: a premise exemption, not a scope exemption.** Its armed facts are representable — `node_rev_before` the key line's rev, `node_rev_after` the no-node token `blake3("")[:16]`, `span_after` the zero-width point the line vacated — A.6.3a′'s create arm read backwards. **The token claims that no node stands at the address, never a direction; a consumer reads the op for direction.** The mechanism test stands for every shape with a post-batch rev; `remove`'s target has none. A future shape earns this exemption only by representing its own death.

`containment_lost` also carries `cause`, because two unlike mistakes lose the same hpath:

| `cause` | What the reparse shows | Remedy the refusal teaches |
|---|---|---|
| `heading_destroyed` | the lost section's heading line no longer parses as a heading | carry your own newlines: text that runs into a following heading must end with `\n` |
| `reparented` | the heading still parses at its level, but its ancestry moved, so its hpath no longer resolves | your text opened a heading at a level that adopts the following sections; deepen it, or aim the edit at the parent whose subtree you meant to rewrite |

`cause` is **absent** when the lost sections do not share one cause, and never appears in `target_identity`; without it the refusal names what would be lost and stops, teaching no remedy it did not measure. Both families are recovery class `fix` (§8); no dispatch-on-`recovery` path changes.

Response (S0→S1, all values computed):

```json
{"id":42,"ok":true,"body":{
 "armed":{"path":"notes/plan.md","edits":[
   {"target":{"hpath":[{"h":"Goals"},{"h":"Q3"}]},
    "node_rev_before":"33d5b0e1b27cb48b","node_rev_after":"41f643f034e5681f",
    "span_after":[49,75]}]},
 "receipt":{"path":"receipts/2026-07-18.md","anchor":"r-000042",
            "node_rev":"60bbee70d4a63a48","span_after":[26,286]},
 "fingerprint_before":"b3:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9",
 "fingerprint_after":"b3:7f3b44376c719be236279e168c22fa2f4d346cd6e5da5bcf0784adb72e7c1f12",
 "seq":1,"verdicts":[]}}
```

**`at:"end"` on a line-grain anchor host always refuses.** The block-leaf span excludes its terminator, so the insertion point sits on it; an end-append to an `{"anchor":id}` target whose host is one line (a list item, a heading line) lands one of two refusals, never a commit:

| the `text` you send | what the reparse measures | `family` |
|---|---|---|
| carries no newline (` tail`) | the bytes join the line; the id is no longer line-final, so the target stops resolving | `target_identity` |
| carries a newline (`\nX`) | the first newline ends the host line; the bytes land in a new line outside the node, so the target still resolves and its rev cannot move | `transition_unrepresentable` |

The host is the attached or enclosing **block**, not always one line, and the pair follows the span law, not the door: an end-append whose bytes stay inside the host block (a paragraph gaining a continuation line, a table gaining a row) commits. Remedy: re-supply the id line-final in your own `text`; an append that adds a line belongs to the enclosing section. Grain, never the host's kind, discriminates; `fm_key` targets share the escape.

The response carries what the write **armed** — target identities, rev transitions, spans after, the receipt fact, the fingerprint transition — never delivery claims. `verdicts` is the rules-as-data surface (§11). Spans appear freely in responses, never in argv.

**An armed row echoes the node's batch transition, never an intermediate.** Two edits on one node serve the same `node_rev_before`, `node_rev_after` and `span_after`; count distinct `node_rev_after` values, not rows.

The append verb is the same op:

```json
{"id":57,"op":"splice","path":"notes/plan.md",
 "actor":"agent:b0864fb2","now":"2026-07-18T20:33:41Z",
 "receipt":{"path":"receipts/2026-07-18.md","anchor":"r-000043"},
 "edits":[{"target":{"hpath":[{"h":"Goals"},{"h":"Q4"}]},
           "edit":{"put":{"at":"end","text":"- new item\n"}}}]}
{"id":57,"ok":true,"body":{
 "armed":{"path":"notes/plan.md","edits":[
   {"target":{"hpath":[{"h":"Goals"},{"h":"Q4"}]},
    "node_rev_before":"4b8bc385a58da0e0","node_rev_after":"f43203a1f0b4c9a3",
    "span_after":[75,150]}]},
 "receipt":{"path":"receipts/2026-07-18.md","anchor":"r-000043",
            "node_rev":"5c6ca7ec00ae279e","span_after":[287,549]},
 "fingerprint_before":"b3:7f3b44376c719be236279e168c22fa2f4d346cd6e5da5bcf0784adb72e7c1f12",
 "fingerprint_after":"b3:6e866e13b5e65ef9961c050f8a621cf1980b00ee293be650deef5f4dbc6823f0",
 "seq":2,"verdicts":[]}}
```

A guardless request is legal at the wire forever. Whether a scope *requires* `if_node_rev`/`if_fingerprint`/`actor` is host policy (§5.3), never wire schema.

Frontmatter-plane write, dry (the `fm_key` node is the full key line: span `[4,15]` = `title: Plan`):

```json
{"id":60,"op":"splice","path":"notes/plan.md","dry":true,
 "edits":[{"target":{"fm_key":"title"},
           "edit":{"match":{"old":"Plan","new":"Plan v2"}}}]}
{"id":60,"ok":true,"body":{
 "armed":{"path":"notes/plan.md","edits":[
   {"target":{"fm_key":"title"},
    "node_rev_before":"fa77480c79a853bc","node_rev_after":"fb49e9df2257fab8",
    "span_after":[4,18]}]},
 "fingerprint_before":"b3:6e866e13b5e65ef9961c050f8a621cf1980b00ee293be650deef5f4dbc6823f0",
 "fingerprint_after":null,"dry":true,"verdicts":[]}}
```

**The set form (dotted cap `splice.set`, v3-only; any v3 client holding the cap).** A request MAY carry `files:[{path, edits|plan_edits}, …]` instead of `path` + `edits`/`plan_edits`; strictly one form (`bad_request` at decode when both or neither appear).

- Two or more entries, paths pairwise distinct; one request-level `if_fingerprint` (or `guards[]`, §5.4), `actor`, `now`, `receipt`, `dry`, `force`; per-edit guards inside each entry unchanged; no `pin` (it rides the single form, whose `path` is the pinning page).
- Batch laws apply per file (one reparse per file). Guard law: §5.1 unchanged — a scope-less `if_fingerprint` is checked first and covers every entry; scoped premises answer the §5.5 Coverage Law at admission.
- **The commit is sealed across the set**: every entry validates before any byte lands; one fingerprint advance covers all files plus the receipt; one receipt entry (one anchor, §6.6 checked once) names every file; one Delta (§7.1) carries every file. A validation refusal anywhere lands nothing and names the entry (`files[i]` and its path).
- Response: `armed` is an array of per-file groups (`[{path, file_rev_after, edits:[…]}, …]`, request order); one `fingerprint_before`/`fingerprint_after` pair; one `seq`.
- Crash posture, including a failed in-memory restore: §6.5's set paragraph; no journal on any set path.
- The cap ships by the §3.2 evolution law; v2 and cap-less v3 sessions are byte-identical to today.

**Two ceilings, two audiences.** A limit that can refuse must be discoverable before it refuses; reading only the script budget under-builds the wire caller — the first real batch ran 103 files, so 64 would fail on contact at that door.

| ceiling | binds | value |
|---|---|---|
| **wire set cap** | any v3 client holding `splice.set` | **the corpus; no engine-minted numeric cap.** A set names existing, pairwise-distinct corpus files, so their count is the ceiling |
| **script arm budget** | the in-process script evaluator only (§A.7) | `max_armed_edits` = 64; arming past it faults the attempt, never a transport limit; a wire client passes through no evaluator |

**The price, measured end-to-end** (quiet Apple M4 Max/APFS, load 5.3–5.9, KB-scale members; the levels are machine claims, the shape claims are load-robust). Hold ≈ per-attempt term(corpus) + N × per-member marginal; no knee anywhere, so no finite N is minted.

- **Per-attempt term, O(corpus) not O(N):** ~57 ms at 200 docs, ~210 ms at 6696 (entry fold + post-commit re-fold + commit residue; the commit leg folds twice). Paid once per sealed set; the daemon never amortizes it across calls (single-form commit 236 ms wire vs 232 ms CLI at 6696 docs), so the set gains 16.6× at N=64 and 21.2× at N=1024 over N single commits.
- **Per-member marginal, corpus-independent and linear to N=1024:** 10.2–11.0 ms (fsync-dominated: validate ≈ 0.2–0.4 ms, stage + fsync + rename ≈ 10 ms); in-crate bench 12.3–13.1 ms/file; band ~11–13 ms/file.
- **Wall = hold** (~100% at every cell ≥1 s), so the denial window for other writers is the commit column: at 6696 docs N=1 ≈ 0.24 s · N=64 ≈ 0.91 s · N=256 ≈ 2.98 s · N=1024 ≈ 11.4 s; whole wiki ≈ 73 s extrapolated. Per-file at N=1024 holds ~242 s across 1024 interleavable windows; the set ~11.4 s in one.
- A competing writer is refused `workspace_busy` in ≤0.1 ms, no engine retry, no queue; waiting is the caller's policy. Hosts ratchet stricter (§5.3); the wire stays permissive.

**Sweep composition — what the set form does not seal.** Sealing is per attempt: a sweep too large for one attempt (the motivating workloads are 5827 and 2635 files) is k sealed sets plus at most one refusal, and **cross-set atomicity does not exist**.

1. **List-building is the caller's plane.** The wire serves no corpus-enumeration op, and content predicates ("docs missing `created_at`") are not glob-expressible, so `files[]` comes from § A.11 `sql` or the caller's own enumeration. Patterns in `files[]` (§ A.7) expand names, never contents.
2. **The script lane faults at arm 65.** A glob matching 200 files does not chunk itself.
3. **The sweep loop:** sets in sorted order; one `if_fingerprint` per set, chained from the previous `fingerprint_after`; stop at the first refusal; re-enumerate and resume. An idempotent predicate converges: a re-run's expansion is empty (`no_effect`, § A.7). A torn sweep leaves k sealed-set receipts plus one refusal, readable because one receipt entry names every file of its set.

**Visibility is a lane property, not a set property.** `seq` and a Delta are minted only where a daemon serves the write; an in-process lane has no seq sink, answers `seq: 0` and mints no Delta, so its sealed set is invisible to every watch-plane consumer (§18 row 12's debt at set grain; cross-lane catchup is diff-by-root, §4.7). A watched corpus writes sets through the daemon door.

### §4.5 resolve — the walk plane

`resolve` is **best-effort app-compatible walking**, two-stage: `parseLinktext` → stage 1 `getFirstLinkpathDest(linkpath, from)` (basename index, frontmatter aliases, case-insensitive, source-relative shortest-unambiguous, unresolved first-class) → stage 2 subpath walk (case-insensitive · first-match-wins on duplicates, silent · strictly-deeper-level · anywhere-after · generation-skipping). These properties are the **behavior spec**, a one-way compatibility floor (everything the engine mints and emits walks in the app), not two-way parity. The ruled grammar wins: an input outside it (a `_`-bearing anchor, §2.4) refuses `bad_request`, conforming, no deviation to ledger. The `obsidian-compat@1.12.7` pack (§13.4) pins the app's walk against version drift beside the six-probe walk law.

**The response type has no rev field**: the mint partition as a type-level fact.

```json
{"id":70,"op":"resolve","from":"notes/plan.md","ref":"plan#Goals#Q3"}
{"id":70,"ok":true,"body":{"dest":"notes/plan.md","span":[49,75]}}

{"id":71,"op":"resolve","from":"notes/plan.md","ref":"plan#goals#q3"}
{"id":71,"ok":true,"body":{"dest":"notes/plan.md","span":[49,75]}}

{"id":72,"op":"resolve","from":"notes/plan.md","ref":"2026-07-18"}
{"id":72,"ok":true,"body":{"dest":"receipts/2026-07-18.md","span":[0,550]}}

{"id":73,"op":"resolve","from":"notes/plan.md","ref":"plan#Goals#Q9"}
{"id":73,"ok":false,"error":{"code":"ref_not_found","recovery":"refresh",
 "stage":2,"dest":"notes/plan.md"}}

{"id":74,"op":"resolve","from":"notes/plan.md","ref":"roadmap"}
{"id":74,"ok":false,"error":{"code":"ref_not_found","recovery":"refresh","stage":1}}
```

Values at S2; ids 70/71 show case-insensitivity. `dest` rides every stage-2 outcome, so the failing stage is observable. `content:true` adds the fragment bytes, still no rev. The strict plane errors `ambiguous_ref` where the walk would silently pick; the walk itself mirrors the app, silence included. `from` is mandatory: resolution is source-relative, and the vault name/alias index arrives at rung 2.

### §4.6 links — the oracle audit

`read`-as-link-oracle, the dominant read pattern of corpus tooling, is a fact op; corpus-wide, it carries the staleness triple (§10):

```json
{"id":80,"op":"links","path":"notes/plan.md"}
{"id":80,"ok":true,"body":{
 "as_of_fingerprint":"b3:6e866e13b5e65ef9961c050f8a621cf1980b00ee293be650deef5f4dbc6823f0",
 "live_fingerprint":"b3:6e866e13b5e65ef9961c050f8a621cf1980b00ee293be650deef5f4dbc6823f0",
 "changes_seq":2,
 "files":{"notes/plan.md":{
   "resolved":{"receipts/2026-07-18.md":1},
   "unresolved":{"roadmap":1}}}}}
```

The shape mirrors the app's `resolvedLinks`/`unresolvedLinks`: per-edge counts, dangling refs first-class. `path` absent → whole-corpus edge map. Opt-in `require_fingerprint` → `stale_view` refusal (§10.2).

**An unresolved edge may carry `unresolved_reason`**, so the three §12.1 exclusion classes and a plain broken link do not collapse into one word and a deliberately unhashed file does not read as a typo. The map keys a subset of `unresolved` by the same linkpath with the §12.1 rule word: `non-md`, `dot-segment`, or `custom-ignore`.

```json
 "files":{"notes/plan.md":{
   "resolved":{"receipts/2026-07-18.md":1},
   "unresolved":{"roadmap":1,".private/secret":1},
   "unresolved_reason":{".private/secret":"dot-segment"}}}
```

1. **A genuine typo carries no reason.** A key appears only when a real file sits under the path the mint resolved (literal spelling, the `.md` append rule, or the bare-name fallback) and the domain excludes it. `[[.private/typo]]`, an excluded directory with no such file, stays bare.
2. **`resolved` and `unresolved` do not move**: `resolved` stays a bool and the app mirror is intact. The map is read beside the edge and omitted when empty.
3. **The word is minted once** (`fs::domain::LinkTargetProbe`); the `sql link` projection's `exclusion` column asks the same mint, so the two faces cannot disagree.

**The bare-name fallback.** A target with no `/` that misses the literal probe resolves by exact basename over the out-of-domain files (else every `[[TAG-FILES.base]]`-style link is a plain miss, an excluded file being absent from the corpus index). The match is **case-exact** (`abc.BASE` never matches `abc.base`); an ambiguous basename takes the **deterministic tie-break**, shortest path then lexicographic. A pathed spelling never falls back: `git/GIT.base` where only `sources/git/GIT.base` exists is genuine rot. Stated limit: a pathed spelling only a suffix walk could find stays bare.

**`path` present is a door; `path` absent is an enumeration (§12.1).** Named, the op serves the page even when the hash domain excludes it, edges resolved against the corpus; only a path with no file under the root is `file_not_found`. Absent, the op speaks for the whole corpus and carries **`excluded`**: the workspace-relative markdown under the root that the domain does not hold, absent from `files` (§12.1 enumerator clause); the key is omitted when empty.

### §4.7 fingerprint and diff — the integrity rung

```json
{"id":90,"op":"fingerprint"}
{"id":90,"ok":true,"body":{
 "fingerprint":"b3:6e866e13b5e65ef9961c050f8a621cf1980b00ee293be650deef5f4dbc6823f0","seq":2}}
```

**The scoped mint arm (`scoped-guards` cap, §5.4).** Under the cap `fingerprint` takes `scope` (a `Path`, §1) or `scope_bytes` (base64url over the raw path bytes, for names the UTF-8 `Path` noun cannot carry), exactly one. Both absent: the root mint above, byte-identical to v2. Both supplied: `bad_request`, a mint names one node (teaching text: §8.2's mint-pair text). The op mints the named node's token (root, folder, or file leaf), the one mint home for every premise the §5.4 guard family accepts. A lawful path with no node answers the reserved non-hex value `absent` (§5.6); an unlawful path refuses `scope_unresolved` (§5.6, §8). The response echoes the scope pair beside the token, `{fingerprint, seq, scope}` or `{fingerprint, seq, scope_bytes}`; `fingerprint: "absent"` is a legal body. No worked scoped hex is printed: the interior encoding moves under the width ruling (`node-rev-merkle-spec.md`).

`diff` is reserved at the integrity rung with its shape standing now:

```json
{"id":95,"op":"diff",
 "from_fingerprint":"b3:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9",
 "to_fingerprint":"b3:6e866e13b5e65ef9961c050f8a621cf1980b00ee293be650deef5f4dbc6823f0"}
{"id":95,"ok":true,"body":{"batches":[ /* Delta seq 1, Delta seq 2 — §7, byte-identical
                                          to the live notification frames */ ]}}
```

Replay ≡ live (§7.3). A range outside the retained history → `fingerprint_unknown` → full resync (bound: §13.5).

`sub` (rung 5) is **served** at the daemon door (`crates/registry/src/server.rs`). Its anchor carries cursor identity: ring `seq` is per tree instance (merkle spec §6.3), so a number alone never proves position.

- **Live subscribe:** `{"op":"sub"}`, no cursor → ack `{"root":…,"seq":N,"tree_instance":I}` (the baseline root, so the first push frame's `root_before` matches); the connection converts to push and carries Notification frames, one Delta batch each, starting after the acked `seq`. The ack is the resumption cursor `{tree_instance, seq}`; `seq` advances per delivered frame.
- **Resumption:** `{"op":"sub","tree_instance":I,"from_seq":N}`; the instance is evaluated before any sequence compare. A dead instance (daemon restart, idle reap) refuses `root_unknown` with the diff-by-root remedy, sequence never consulted. A live-instance `from_seq` outside the retained ring also refuses `root_unknown`.
- **Upgrade required:** `from_seq` without `tree_instance`, or `tree_instance` without `from_seq`, is half a cursor and refuses `bad_request` with the upgrade teaching.

A refused `sub` leaves an ordinary request channel. The delta stream is not actor-scoped: identities, revs, and spans only.

### §4.8 The five-verb interface, mapped

| Verb | Wire op | Notes |
|---|---|---|
| toc | `toc` | rows `level · span · rev · hpath` |
| cat (CLI: `mrd read` / section) | `cat` | `--sec` segments → hpath array verbatim |
| edit (CLI: `mrd put` + match) | `splice` with one `match` edit | `--if 9d3e…` → `if_node_rev`; receipt address defaulted by the client library, overridable |
| append (CLI: `mrd put` + put at end) | `splice` with one `put{at:"end"}` edit | |
| resolve (interop; interface strips `[[…]]`) | `resolve` | interface strips `[[…|alias]]`; wire takes raw linktext — one grammar |

Requests never require revs; receipts always return them, so the current rev is ambient after any read or write and `--if` costs ~10 tokens, zero extra calls. No byte offsets exist in the interface surface or in wire *requests* (§4.4).

## §5 Guards and the CAS law

### §5.1 The CAS rule

`if_node_rev` is compared against `blake3(target's full span bytes)[:16]`, re-derived at execution time from the pre-batch state — the derivation `toc`/`cat` serve as `node_rev`. No second rev derivation exists — no content-span rev (§1), no client span (§4.4) — so hashing the bytes about to be replaced is the only implementation.

Check order: `if_fingerprint` against the live workspace fingerprint first (world-grain; a mismatch fails the whole batch, `fingerprint_mismatch{expected,actual}` → re-plan), then per-edit `if_node_rev` (node-grain, `cas_mismatch{expected,actual}` → refresh). Merkle-spec §7 semantics: Rust computes hashes, hosts compare opaque tokens.

Under the `scoped-guards` cap (§5.4), `if_fingerprint` without `scope` is the root premise, byte-identical to v2 and sugar for `guards:[{scope?, fingerprint}]`. Full order: coverage at admission (§5.5) → every supplied premise (§5.4; a scoped refusal names it, `fingerprint_mismatch{expected,actual,scope}`) → per-edit `if_node_rev`. A premise refusal fails the whole batch before any byte lands.

### §5.2 The failure split

Worked failures against S2. Q3's rev at S2 is `41f643f034e5681f`: the E4 append touched only Q4, so Q3 is unchanged from S1.

```json
{"id":88,"op":"splice","path":"notes/plan.md","edits":[
  {"target":{"hpath":[{"h":"Goals"},{"h":"Q3"}]},
   "edit":{"match":{"old":"ship by September","new":"ship by October"}},
   "if_node_rev":"33d5b0e1b27cb48b"}]}
{"id":88,"ok":false,"error":{"code":"cas_mismatch","recovery":"refresh",
 "expected":"33d5b0e1b27cb48b","actual":"41f643f034e5681f"}}
```

The world moved — the client held S0's rev. One `cat` refreshes.

```json
{"id":89,"op":"splice","path":"notes/plan.md","edits":[
  {"target":{"hpath":[{"h":"Goals"},{"h":"Q3"}]},
   "edit":{"match":{"old":"ship by August","new":"ship by October"}},
   "if_node_rev":"41f643f034e5681f"}]}
{"id":89,"ok":false,"error":{"code":"no_match","recovery":"fix","matches":0}}
```

The rev passed and `old` did not match: provably the caller's typo — the diagnosis the `--if` flag buys.

```json
{"id":91,"op":"splice","path":"notes/plan.md","edits":[
  {"target":{"hpath":[{"h":"Goals"},{"h":"Q4"}]},
   "edit":{"match":{"old":"item","new":"entry"}}}]}
{"id":91,"ok":false,"error":{"code":"not_unique","recovery":"fix","matches":2}}
```

`item` occurs in both `- item one` and `- new item` (2 matches). Add context bytes to `old`. Without `if_node_rev`, `no_match` cannot tell a typo from a moved world.

### §5.3 Geography

The wire is permissive forever: unguarded, actor-less, receipt-less splices are legal frames. Requiredness ("shared scopes need `if_node_rev`", "this tree needs `actor`", "receipts mandatory under `reports/`") is host/client policy, never wire schema; tightening it after adoption is host work.

**Scope grammar (bound here):** a ratchet scope is a `Path`-set selector (path globs; config data that never rides the wire) plus, where it names nodes, strict-plane refs per §2.1. Policy config holds no second address grammar, and the §11.3 pack manifest carries no scope field (fix-at-freeze, §18 row 1).

### §5.4 Scoped guards

The family rides behind the `scoped-guards` cap; every refusal below teaches per §8.2.

**Any legal token in the tree is a legal guard.** A premise names any addressable path node — root, folder, file leaf — or the reserved value `absent` (§5.6). Interior sharding below a path node (the hash law's radix buckets) is never a scope.

**Shape: a list.** `guards:[{scope?, scope_bytes?, fingerprint}, …]`, because one plan holds several premises and their common ancestor over-covers: two literals under `a/` LCA to `a/`, so a neighbor creating `a/3.md` would refuse. Singular `if_fingerprint` (+ optional `scope`) is the one-premise sugar; neither `scope` nor `scope_bytes` means the root premise.

**`scope` is a JSON field beside the token, never a token encoding**: hosts bind policy to paths ("`results/**` requires a premise") and never parse tokens (§5.3). The `@`-form (`<token>@<scope>`) is a display spelling for faces only; no wire surface parses or emits it.

**Raw-byte names are addressable.** `scope_bytes` (base64url over the raw path bytes) rides beside the UTF-8 `scope`, exactly one of the two per premise; mint (§4.7) and guard serve both. No name is integrity-covered but unaddressable (`node-rev-merkle-spec.md` §9).

**The field matrix — one law at every strict wall.**

| field | where it rides |
|---|---|
| `if_fingerprint` (+ optional `scope`), the one-premise sugar | `splice` (single and set form) and `script` |
| `guards[]`, the premise list | `splice` (single and set form) and `script` |
| `scope_bytes` | top-level on **no** door: a `guards[]` entry, or the raw-byte mint on `fingerprint` (§4.7) |
| any guard-family field (`if_fingerprint`, `guards`, `scope`, `scope_bytes`) | never an effects door: on `run`, and beside `effects` on `script`, each refuses `bad_request` at the strict wall (§ A.7/§ A.8) |

The § A.7 field wall (12 → 14: `guards`, `scope`) is this matrix; no fifteenth field exists. Sugar and list together are legal: the sugar desugars to one more premise. Per premise `{scope?, scope_bytes?, fingerprint}`: exactly one of `scope`/`scope_bytes`, or neither for the root premise; `fingerprint` is required and holds a token or `absent` (§5.6). Pair violations — `scope` or `scope_bytes` without `fingerprint`, both spellings in one premise, sugar `scope` without `if_fingerprint` — refuse `bad_request`, validated atomically at the door so hash and path cannot desync; the mint door's own — both spellings on one `fingerprint` request — refuses `bad_request` at §4.7 with its own text.

**Guard-path freshness.** The engine refreshes the named premise's own extent and pays only that: one file, one folder, or the world. A refusal narrows because the premise narrows, never because the engine checked less hard.

**Negotiation.** A frozen v2 session is never pushed the cap, and un-negotiated use of any guard-family field refuses `bad_request` at the §3.2 strict wall, never silently.

### §5.5 The Coverage Law

Two questions, two laws:

- **Legality:** every addressable node — root, folder, file leaf — and `absent` is a legal premise (§5.4); the engine checks every premise supplied.
- **Sufficiency (the Coverage Law):** let `W` be the caller-authored write set (every path the caller's own edits publish) and `G` the caller's premise list (no engine premise ever enters `G`). Requiredness holds iff **for every `w` in `W` some `g` in `G` has a scope ancestor-or-self of `w`** (an exact-section or absence premise covering `w` suffices). A premise need not cover every target; one covering none is legal widening — checked, strictest wins, never sufficient alone. Failure refuses `scope_does_not_cover` naming the uncovered caller-authored targets; the engine never promotes to a common ancestor (LCA) or to root.
- **Placement:** at transaction/set admission, where complete `W` and `G` exist, before per-member validation or any byte move.
- **Door scope:** the doors under A.1's pure-write demand, which declare their write set in the request (`splice` in every form, `create`, `remove`). The script door (§ A.7) is admitted by its own law — its commit premise is the engine-computed touch set, which always contains the armed writes, so a guardless pure script passes with an empty `G`, and caller premises there are widening only. Effects doors hold no premise (§ A.8).
- **Engine-generated writes outside `W`** (the receipt rider, crossing scopes in one commit) are verified by the engine against the live tree at commit; `G` covers the caller-authored targets only.

A content edit's `if_node_rev` is such an exact-section premise, so A.1's demand is unchanged in effect; the set form's natural cover is each target file's own leaf token (one per file, membership-safe within it); a root premise covers everything (today's `if_fingerprint`).

### §5.6 `absent`

A lawful path with no node — never created, emptied, pruned — mints the reserved non-hex token `absent` (§4.7), and absence of the whole prefix is still `absent`: `a/b/c` with `a/` missing mints `absent`, not an error. Creation guards stand on this: an absence premise at the birth path refuses when anything now exists there. `absent` carries no algorithm/domain prefix because it names no fold; it compares node existence, not hashes. A path is unlawful — `scope_unresolved`, recovery `fix` — only where it escapes the root or conflicts in kind with an existing entry on its prefix, never for lawful absence.

### §5.7 The error split

Never one word for two facts:

| fact | code | recovery |
|---|---|---|
| the premise moved | `fingerprint_mismatch{expected,actual,scope?}` | `resync` — re-read that scope, re-plan |
| the premise cannot be evaluated at that path | `scope_unresolved` | `fix` — fix the path |
| the reference is too old (the cursor family: `fingerprint_unknown`, a dead instance, `fingerprint_version_retired`) | §8 | `resync` — re-derive and resume |

Version vocabulary stays split inside the cursor family (§12.3): a known retired hash-law family refuses `fingerprint_version_retired` with re-mint teaching, never `fingerprint_mismatch`; an unknown future family refuses `fingerprint_version_unsupported`. Texts: §8.2.

A fourth fact is the request, not the world: a premise value that is neither `absent` (§5.6) nor a grammatical `Root`-family token (merkle-spec §4.2: `b3` + bijective-base-26 suffix + `:` + 64 lowercase hex) refuses `bad_request` (recovery `fix`) at the premise rung, before any fold is compared, quoting the raw bytes debug-quoted so a leading space shows as a byte. `fingerprint_mismatch` therefore claims one thing only: a well-formed token differed from the live fold. This wall is value grammar, never a permission plane, and touches no version family (§12.3).

A well-formed token premise whose scope holds no live node refuses `scope_does_not_cover` (recovery `fix`, carrying `scope`; `uncovered` stays §5.5's target-set extra) — never `fingerprint_mismatch`, never `scope_unresolved` (§5.6 bars it). From `(token, absent)` the engine cannot tell removal-since-mint from a scope that never held a node, so no text narrates removal; a node-less scope's one lawful premise is `absent` (§5.6), and the remedy is the mint `fingerprint{scope}`, which says what the scope holds now; the caller re-pairs the premise or fixes the scope. A genuine post-mint removal draws the same refusal (text: §8.2). The reverse seam stands: an `absent` premise against a live node stays `fingerprint_mismatch` (§5.6's creation-guard collision).

## §6 Receipts — outcome as fact

### §6.1 The law

A write that names a receipt gives its address as `receipt:{path, anchor}` on the splice request. Receipts are per-request, never a wire requirement; whether a scope requires one is host policy (§5.3). When named, the engine appends the entry to that md file **in the same batch commit** as the content edit: one exchange, one reparse, one fingerprint advance covering both files. Receipts are ordinary markdown in the hash domain (§12): addressable, cat-able, resolvable via `#^anchor`, hashed into the fingerprint.

The armed response (§4.4) and the receipt entry carry the same facts — op, target identities, rev transitions, `fingerprint_before`, actor, now, request id — stating what was **armed**, never "delivered" or "succeeded downstream"; delivery is the host's business.

### §6.2 No-self-rooting law

A receipt **cannot contain the root it produces**: `fingerprint_after` covers the receipt file's bytes, so the hash would have to contain itself. Receipts carry `fingerprint_before`; `fingerprint_after` rides the wire response and the Delta — a limit, not worked around.

### §6.3 The worked receipt

**Normative content is the armed-fact set on the wire response** (§4.4), not any md line shape. Target identity is always §2.1 form, e.g. `{"hpath":[{"h":"Goals"},{"h":"Q3"}]}`, never a joined string.

The default receipt line, byte-exact (segment target only) — the fixture's own S1/S2 bytes:

```markdown
- splice notes/plan.md id=42 actor=agent:b0864fb2 now=2026-07-18T20:31:04Z fingerprint_before=b3:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9 edits=1 target.hpath=[{"h":"Goals"},{"h":"Q3"}] match 33d5b0e1b27cb48b->41f643f034e5681f ^r-000042
```

E4 append via `put{at:"end"}`:

```markdown
- splice notes/plan.md id=57 actor=agent:b0864fb2 now=2026-07-18T20:33:41Z fingerprint_before=b3:7f3b44376c719be236279e168c22fa2f4d346cd6e5da5bcf0784adb72e7c1f12 edits=1 target.hpath=[{"h":"Goals"},{"h":"Q4"}] put:end 4b8bc385a58da0e0->f43203a1f0b4c9a3 ^r-000043
```

**Do not re-teach** pretty joins (`Goals>Q3`); replaceability (§6.4) permits no second address grammar.

**Byte arithmetic.** The two lines measure **260 B** and **262 B** on the node basis (terminator excluded, per the leaf-block span law §1) — the widths required by the fixture's spans `[26,286]` and `[287,549]` (§4.4, §7.1) and the receipts file's 26 → 287 → 550 B growth (§0.3). R1, R2 and every S1/S2-anchored value are therefore reconstructable from this document (§18 row 10).

**The lane width.** Replaying E3 through the CLI lane (`mrd put`) under the same `actor` and `now` writes **254 B node bytes** (**255 B line**, terminator included); every width compared here is node bytes, terminator excluded (§1), and the line figure carries its own label because the two bases differ by exactly the terminator. **The 254 is a lane width, not the template's**: the same shipped template writes 260 B when the request carries an id, gated by `crates/receipt/tests/frozen_receipts.rs :: e3_receipt_line_byte_exact`, which renders `id: Some(42)` and asserts the E3 line above and `line.len() == 286 - 26`. A CLI invocation is no wire request and mints no request id, so under §9's absent-inputs law writing no `id=` token is correct. One gap remains, ruled rather than owed:

| gap | width | causes |
|---|---|---|
| shipped CLI lane 254 → fixture / printed 260 | **6 B** | `id=` alone — **ruled, not a defect** (§9, lane) |
| fixture 260 → the lines printed above 260 | **0 B** | the lines printed here are the fixture's bytes |

`254 + 6 = 260` closes to the byte. R1 and R2 are wire-lane values: a CLI-lane replay cannot reach them, since the receipt node differs by `id=` alone and a different node is a different fingerprint — a lane mismatch, not an engine shortfall.

**The target form is written by the template, and its escaping is §6.7's**: the punctuation of `target.hpath=…` is template text, only the heading text is interpolated (§6.7 rule 2's segment renderer), and `Goals`, `Q3` and `Q4` carry no `"`, so the escape is the identity and the widths above are unmoved.

### §6.4 Replaceability

The md rendering is a shipped default template any consumer replaces freely; the normative content is the armed-fact set of the wire response. The mechanism is generic: "append these facts at this address with this anchor". "No intent past-due without a receipt" is lintable — a rules pack can assert every splice-bearing transcript row has its receipt anchor resolvable (§11).

### §6.5 Crash honesty

The batch writes two files via tmp+fsync+rename each; a crash between the renames can land content without its receipt. Recovery is re-derive (cold rebuild → correct fingerprint, never wrong data), and the lint finds the missing receipt (§13.6).

**A set commit (§4.4 set form) widens the sequence and keeps the posture — in-memory rollback, no journal.** It stages every file, verifies every pre-image, then renames in member order with the receipt **last**. A rename failure mid-sequence (process alive) restores every already-renamed member from its held pre-image bytes, and the error names what failed and what was restored. Two limits:

- a crash mid-sequence can land a prefix of the set (each file still fully old or fully new, since atomic renames never tear; cold rebuild yields the correct fingerprint of whatever landed);
- the restore can itself fail; the error then lists which files hold the new bytes, so recovery is a statement, never a guess.

Because the receipt renames last, a resolvable receipt anchor implies the whole set landed, so the §6.6 collision door is the lost-answer probe for the whole set. This is the multi-file atomic commit.

### §6.6 The anchor is the caller's to mint

The anchor arrives from the caller and the engine appends what it is given (§6.4), so one obligation follows: **an anchor MUST be unique within the receipt file it names.** A repeated id publishes a receipt that is hashed but unaddressable: §2.1 resolves an anchor ref by exact block id and A.3 refuses `ambiguous` when a file carries it twice, against §6.1's promise of resolution via `#^anchor`.

**The engine polices the anchor of the write in front of it.** Anchors minted as ordinary content are never inspected — an append has no cross-invocation memory — but the requested receipt anchor is read in the same act as the write. **The splice door resolves the requested receipt anchor against the receipt file first, and refuses `bad_request` with zero bytes moved when the anchor already stands there.** The caller re-sends under an unused anchor; because nothing landed, the re-send appends its content exactly once. Resolving after the commit — `put --receipt <file>#<anchor>` on an anchor the file already carried — would publish the duplicate, then report *"committed receipt anchor did not resolve — receipt corrupt"* with two files already written, and its `recovery:"fix"` would duplicate the content on re-send.

For a host appending many receipts to one shared file across invocations: **derive the anchor from the invocation identity, never from a counter that restarts.** A per-invocation counter (`^r-000001`, `^r-000002`, …) is unique only within its process, so the second invocation re-mints the first id. Use a monotonic file-scoped counter (read the file, continue past its last id) or the invocation id plus an in-run sequence; the second costs no read and is what `mrd run` does (`r-<invocation-id>`).

A host deriving from a caller-supplied id inherits its charset duty: the mint routes through the block-id door (`[A-Za-z0-9-]`, §2.4) and refuses rather than publishing an unaddressable anchor.

### §6.7 The rendering law

The rendering is replaceable (§6.4) but not free. A receipt is ordinary markdown (§6.1), so a line that reads as a different structure than its facts misreports the write, and a receipt whose structure user content can move is not a receipt. Two rules decide which bytes are the template's and which the data's.

**Rule 1 — every interpolated value is escaped.** A value stands verbatim only if every character is *receipt-identifier text*: ASCII graphic, minus `[` and `]` (no wikilink or embed can form), minus the backtick and the backslash (spent as escape delimiters). The charset excludes whitespace and line endings, so no token or row boundary can be forged. Anything else renders as an inline code span with out-of-charset characters escaped `\u{…}` and `\` doubled — reversible, so the value is preserved exactly (§5.2).

**Rule 2 — where the template writes §2.1 JSON, the punctuation is template bytes.** In the segment form §6.3 mandates (`target.hpath=[{"h":"Goals"},{"h":"Q3"}]`), every `[`, `]`, `{`, `}`, `"`, `:` and `,` is template text. Only a segment's heading text and its occurrence index `n` come from the data, and the heading text goes through the **segment renderer**, a different renderer from rule 1's:

- it emits the body of a JSON string and never its quotes;
- a character stands verbatim only if it is receipt-identifier text and is not `"`;
- everything else — the double quote, the backslash, the brackets, the backtick, every space, every control character, every non-ASCII character — becomes a JSON `\uXXXX` escape (surrogate pairs above U+FFFF).

The output carries no `"` and no backslash that does not open a complete six-byte escape: the string cannot close early, no key can be forged, and no markdown structure can form inside it. A strict parser over the rendered array returns the original segments byte-for-byte.

Rule 1 alone cannot do this: it excludes `[` and `]` (so the punctuation must be the template's) yet permits `"`.

**This law is escape-only and byte-neutral on conforming text**: a heading of receipt-identifier text without `"` (§6.3's `Goals`, `Q3`) renders identically with or without it, so §6.3's arithmetic and §18 row 10 are unmoved. The escape exists before the JSON form is emitted, never alongside it.

## §7 The Delta noun

### §7.1 Shape

One Delta = one batch = one fingerprint advance. A §4.4 set commit mints one Delta whose `files[]` carries every content file plus the receipt — cardinality is data; never assume ≤2 files. E3's delta, every value computed:

```json
{"delta":{
 "seq":1,
 "fingerprint_before":"b3:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9",
 "fingerprint_after":"b3:7f3b44376c719be236279e168c22fa2f4d346cd6e5da5bcf0784adb72e7c1f12",
 "actor":"agent:b0864fb2","now":"2026-07-18T20:31:04Z",
 "files":[
  {"path":"notes/plan.md","change":"modified",
   "file_rev_before":"e3c4acaceb75b907","file_rev_after":"a9794a262e67ed02",
   "nodes":[{"hpath":[{"h":"Goals"},{"h":"Q3"}],"change":"edited",
             "node_rev_before":"33d5b0e1b27cb48b","node_rev_after":"41f643f034e5681f",
             "span_after":[49,75]}]},
  {"path":"receipts/2026-07-18.md","change":"modified",
   "file_rev_before":"920a40c4ee23d37c","file_rev_after":"51ad6428f5b5a898",
   "nodes":[{"anchor":"r-000042","change":"added",
             "node_rev_after":"60bbee70d4a63a48","span_after":[26,286]}]}]}}
```

E4's delta:

```json
{"delta":{
 "seq":2,
 "fingerprint_before":"b3:7f3b44376c719be236279e168c22fa2f4d346cd6e5da5bcf0784adb72e7c1f12",
 "fingerprint_after":"b3:6e866e13b5e65ef9961c050f8a621cf1980b00ee293be650deef5f4dbc6823f0",
 "actor":"agent:b0864fb2","now":"2026-07-18T20:33:41Z",
 "files":[
  {"path":"notes/plan.md","change":"modified",
   "file_rev_before":"a9794a262e67ed02","file_rev_after":"5f27a2814b517680",
   "nodes":[{"hpath":[{"h":"Goals"},{"h":"Q4"}],"change":"edited",
             "node_rev_before":"4b8bc385a58da0e0","node_rev_after":"f43203a1f0b4c9a3",
             "span_after":[75,150]}]},
  {"path":"receipts/2026-07-18.md","change":"modified",
   "file_rev_before":"51ad6428f5b5a898","file_rev_after":"6cb0e939ce2edf5a",
   "nodes":[{"anchor":"r-000043","change":"added",
             "node_rev_after":"5c6ca7ec00ae279e","span_after":[287,549]}]}]}}
```

Laws:

- `seq` is a monotone per-workspace batch counter (the `changes_seq` of §10), **per daemon epoch**: a restart resets it (memory is disposable, disk markdown-only, §14). So `from_seq`/`changes_seq` catchup is valid within one epoch only, cross-epoch catchup is diff-by-root, and the `sub` resumption anchor carries `{tree_instance, from_seq}`, instance evaluated before sequence (§4.7).
- File `change ∈ {created, modified, deleted, renamed, unattested}`. `renamed` carries `from_path`; `unattested` (§ A.9's re-scope word) means the file left the attested set while its bytes remain on disk — v3-only, demoted to `deleted` for a frozen v2 session.
- Node `change ∈ {added, edited, removed, anchored}`. `anchored` means the node moved solely by gaining an anchor id — a byte verdict, never an intent; content change plus a new anchor stays `edited`. v3-only, demoted to `edited` for a frozen v2 session.
- Node entries name the **deepest section containing each changed byte range**; ancestor section revs change implicitly (rev = span hash), are re-readable via `toc`, never duplicated into the delta.
- External changes (a human editing in Obsidian) produce deltas with `actor`/`now` **absent** — the engine never invents identity or time. `seq` is assigned at detection.

### §7.2 Node-grain at birth

Grain = the node entry above: identity (hpath/anchor/fm_key echo), rev transition, span_after — the vocabulary of toc rows and armed facts: one projection, three tenses (map / armed / changed).

### §7.3 Replay ≡ live

`diff(from_fingerprint, to_fingerprint)` returns the **byte-identical** Delta objects emitted (or that would have been emitted) as live notifications between those roots. There is no second diff dialect: catchup consumers and live subscribers parse one shape (`sub` carries Deltas in Notification frames, §4.7). Retention is the root-history ring, bound 256; a range outside it → `fingerprint_unknown` → full resync, never wrong data.

### §7.4 Delta grain

Deltas are node-grain at contract birth (the grain §7.1–§7.2 define); the question is ruled and no `keys:[…]` slot work ships now. Key-grain arrives later, if ever, only via the additive amendment path named here: node entries MAY gain an optional `keys:[{key, change, value_rev}]` sub-array once the frontmatter plane matures. Old consumers ignore the unknown field (tolerant-client law), replay stays shape-identical, `diff` needs no new op. Future-only.

## §8 Error taxonomy

Every error frame carries `code` + `recovery` from the closed six-class enum. Each code is statically bound to exactly one class; a client that does not recognize a code dispatches on `recovery` alone. Loud everything.

| class | meaning | codes |
|---|---|---|
| `fix` | your request is wrong; change it | `bad_request`, `unknown_op`, `bad_path`, `no_match`, `not_unique`, `would_corrupt{family,lost?,cause?,target?}`, `ambiguous_ref{candidates}`, `remove_refused{referrers}` (§ A.3 — unlink the named referrers, then resend), `scope_does_not_cover{uncovered}` (§5.5 — the extra names the uncovered target set; §5.7's arm mints it with `scope` alone), `scope_unresolved` (§5.6) |
| `env` | the world outside the workspace is wrong | `file_not_found`, `io_error{cause}`, `invalid_utf8{path,message}`, `daemon_only`, `mount_table_invalid{path,message}` |
| `refresh` | your picture of a node is stale; re-read one thing | `cas_mismatch{expected,actual}`, `ref_not_found{stage,dest?}` |
| `retry` | transient; same request may succeed | `lock_timeout`, `stale_view{required,as_of_fingerprint,live_fingerprint}`, `corpus_warming` (§3.2 — the drawer is rebuilding after a cold start) |
| `resync` | your picture of the world is stale; re-plan | `fingerprint_mismatch{expected,actual,scope?}`, `fingerprint_unknown`, `fingerprint_version_retired` (§5.7, §12.3 — re-mint at the same scope), `fingerprint_version_unsupported` (§5.7 — the family is newer than the serving law) |
| `respawn` | the channel itself is broken | `bad_frame`, `unsupported_proto`, `internal` |

- No bare `not_found`: `file_not_found` (env: the file is gone) differs from `ref_not_found` (refresh: the name dangles), and `io_error` carries its cause. `ref_not_found.stage` makes the two stages observable (1 = vault-namespace miss, no `dest`; 2 = subpath miss, `dest` present — §4.5).
- `budget_exceeded` is not here: it is a typed *finding* inside `verdicts` (§11), never a wire error.
- **`daemon_only`** (env) is unmintable (§3.3): it names a corpus-class rules pack — one whose WHEN needs the resident corpus name index, e.g. `link_resolves` (§11.2) — loaded with no resident index (the `BudgetClass::Corpus` law, §11.3). Every wire door is daemon-backed, so nothing mints it; the law gates nothing at the wire.
- Null-id frames: §3.1.
- Three declared deltas from the ruled table: `fingerprint_mismatch` is `resync`, not refresh (§5.1); `unsupported_proto` is `respawn`, not fix; no `bad_id` code (folded into `bad_request` + `id:null`/`id_raw`, §3.1). Deviation rows: no bare `not_found` (this table), unknown-`kinds` rejection (§4.3); the ledger is §18.
- The scoped-guard family (§5.4–§5.7) contributes four codes, bound as the table shows. `fingerprint_mismatch` carries optional `scope` (absent = the root premise, the exact v2 shape — §18 row 2); a retired-family token must never answer it — the law moved, not the premise (§5.7, §12.3).

### §8.1 The no-answer case

The six classes ride **error frames**, where the daemon answered. A request whose answer never arrives — the op deadline expired, or the connection died in flight — is a **transport loss**, not a wire error: no `recovery` class exists, and for a `splice` **persistence is unknown**, since the batch may have committed before the loss. A slow op is not a hung daemon.

Two consequences, both **client law** — the wire cannot rule on frames it never served:

- **The op deadline is a hang detector, never a safety mechanism.** Its value is host-chosen and MAY be op-class-aware (a host might bound ordinary ops at 10 s; `hello` is config-grade under §3.2 and never pays a cold whole-corpus build). The cold build never rides inside any op's deadline (§3.2). Warm-cost classes and load are host knowledge the engine does not publish (orientation is not a wire op, §10.3), and no finite value closes the window: any deadline can expire after the commit landed.
- **Re-read before retry.** After a lost `splice` answer, re-read the target and check whether the write **already landed** — content, not just tokens — before any re-send. The ordinary path (`cas_mismatch` → refresh → re-apply) is wrong here: it re-applies a write that may already be in the file, and a post-loss `no_match` reads as "provably your typo" (§5.2) when the truth is "your first send landed and consumed the anchor".

A **blind re-send without `force` cannot double-apply**: the wire-origin guard demand (A.1, A.3) refuses every arm — a guarded edit's token re-derived against post-commit bytes (`cas_mismatch`, §5.1), a birth whose subject now exists (`cas_mismatch`, absence guard), an unguarded content edit (`guard_required`). The refusal cannot say whose write landed, hence the read first, but nothing applies twice. **`force` strips the node-grain tokens (A.1) and reopens the double-apply; a post-loss re-send MUST NOT carry `force`.**

Reads are idempotent: after a lost answer, re-send freely.

### §8.2 Register-law refusal texts

Refusal teaching speaks the register law: **reason first, fitted remedy, never session rules.** The texts below are normative. They honor two constraints: the broken-premise-pair remedy admits the legal bare-root premise `{"fingerprint": …}` with no scope spelling (§5.4), and no text narrates a deletion the engine cannot know (§5.6, §5.7).

```
fingerprint_mismatch (scoped):
  "the premise at <scope> moved — expected <expected>, live is <actual>.
   Re-read under <scope> and re-plan. This refusal is about this premise
   only; it says nothing about what else was or was not checked."
scope_does_not_cover:
  "this write touches <uncovered targets> and no premise covers them —
   a premise must cover what it guards. Add a premise at each listed
   target's file or an ancestor (mint: fingerprint{scope: "<dir>"});
   premises beyond the cover are legal and also checked."
scope_unresolved:
  "<scope> cannot hold a token — it escapes the workspace, names a
   file/dir kind conflict with an existing entry, or is not encodable.
   A lawful path that simply has no node mints "absent" — that is a
   legal premise, not this error."
scope_does_not_cover — token premise at a node-less scope (§5.7):
  "the premise at <scope> holds a token, but no node lives at <scope> —
   a token premise cannot hold where there is no node (a node-less
   scope's one lawful premise is "absent", §5.6), so this premise covers
   nothing. Whether a node was removed since your mint or <scope> never
   held one, this refusal does not say — it cannot know. Mint at the
   scope (fingerprint{scope: "<scope>"}) to see what it holds now — a
   lawful empty scope answers "absent" — or fix <scope> if the token was
   minted elsewhere; then re-plan."
fingerprint_version_retired:
  "this token was minted under a retired hash law. The premise did not
   move — the law did. Re-mint at the same scope
   (fingerprint{scope: "<scope>"}) and re-plan once."
fingerprint_version_unsupported:
  "this token was minted under a hash law this engine does not know —
   the token is newer than the law being served. Re-mint at the same
   scope (fingerprint{scope: "<scope>"}) to proceed under the serving
   law; to keep the newer tokens, upgrade the engine, not the token."
bad_request — guard family, un-negotiated:
  "this session did not negotiate scoped-guards, so <field> cannot ride
   this request. Reconnect and negotiate the scoped-guards cap in hello,
   or drop the field — the v2 forms (root if_fingerprint, if_node_rev)
   are fully served without it."
bad_request — guard family, broken premise pair:
  "<detail — one of: <spelling> carries no fingerprint; both scope and
   scope_bytes in one premise; scope without if_fingerprint>. A premise
   is a token plus at most ONE scope spelling — the token is required,
   the spelling is not: a bare {fingerprint} is the legal root premise.
   To scope a premise, mint the pair together
   (fingerprint{scope: "<scope>"}) and send both; to guard the world,
   send the token alone."
bad_request — mint pair, both spellings on one fingerprint request:
  "this mint names its node twice — scope and scope_bytes in one
   fingerprint request. A mint names ONE node: keep the one spelling
   that names your path (scope for UTF-8 names, scope_bytes for raw
   bytes) and re-send; both absent mints the root."
bad_request — guard family, malformed premise value:
  "<the premise at <scope> | the world premise> holds <raw bytes,
   debug-quoted>, which is not a premise token — a premise holds an
   engine-minted b3…:<64-hex> token or the reserved "absent", and the
   quoted spelling shows every byte, whitespace included. The world was
   NOT compared: fix the spelling, not the plan. Paste the token exactly
   as the engine served it, or re-mint it (fingerprint{scope: "<scope>"},
   §4.7) and send that."
refused trace, recovery fix — script door, malformed entry pin
(engine-minted, so it rides the trace's fault triple with no §8 code —
§ A.7 response law):
  "the script entry pin holds <raw bytes, debug-quoted>, which is not an
   entry fingerprint — the pin holds an engine-minted b3…:<64-hex> token,
   and the quoted spelling shows every byte, whitespace included. The
   world was NOT compared: fix the spelling, not the plan. Paste the
   entry fingerprint exactly as the engine served it, or re-mint it
   (fingerprint{}, §4.7) and send that. The reserved "absent" is premise
   vocabulary (§5.6, guards[]), never an entry pin — a script evaluates
   against the world that exists."
bad_request — guard family, effects door:
  "<field> was supplied on an unguarded door: run and script-with-effects
   hold no premise — a guard here would promise what execution cannot
   keep (no-guard ruling). Drop the guard fields; to guard content
   writes, use splice, or script without effects (its commit premise is
   the engine-computed touch set)."
```

## §9 actor and now

Rust never reads a wall clock or env identity. `actor` (opaque string) and `now` (RFC 3339 string, format-validated, never generated) ride the wire as ordinary optional request fields on `splice` — recorded into receipts and Deltas — and as rule-evaluation inputs (§11: temporal predicates read the *given* `now`). Absent inputs produce absent facts (worked: external-change deltas, §7.1). Whether a scope requires them is host policy (§5.3). Hosts may mint `actor` from their own session machinery; the engine never invents one.

## §10 Staleness posture

### §10.1 The triple

Anything view-shaped — today `links`, tomorrow any corpus-wide fact op — declares three fields in every response: `as_of_fingerprint` (the fingerprint the answer was computed at), `live_fingerprint` (the fingerprint now), `changes_seq` (the Delta counter at `as_of_fingerprint`). **No lag bounds are promised, ever** — the corpus mutates while it is measured (honest-tense law).

### §10.2 The refusal

`require_fingerprint` is the opt-in strictness knob:

```json
{"id":81,"op":"links","path":"notes/plan.md",
 "require_fingerprint":"b3:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9"}
{"id":81,"ok":false,"error":{"code":"stale_view","recovery":"retry",
 "required":"b3:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9",
 "as_of_fingerprint":"b3:6e866e13b5e65ef9961c050f8a621cf1980b00ee293be650deef5f4dbc6823f0",
 "live_fingerprint":"b3:6e866e13b5e65ef9961c050f8a621cf1980b00ee293be650deef5f4dbc6823f0"}}
```

The client demanded R0; the world is at R2. `stale_view` is retryable, never silent.

### §10.3 View topology

Nothing on the agent path assumes SQL/DB access: every §4 op is served from the world model (parse + hash of disk bytes). The facts API (this wire, agents) and any view face (humans) are two faces of one projection; deciding from a view is a misclassified projection. Orientation surfaces (dashboards, counts, trees) are not wire ops: `debt`, `domains tree`, `status` are not ops (§16).

### §10.4 The rung-5 view organ

Nothing in this contract **names DuckDB** or any SQL engine; a view organ, if present, is implementation under §10.3: orientation is not a wire op.

No `view_path` op and no daemon-published `view.duckdb` file exist on the wire surface: such an op would put an engine-named artifact path and an orientation surface on the wire, and a synchronous rebuild cannot meet a request deadline on a real corpus (~6 min at 22k files, non-convergent under concurrent writes). Any view organ is a **non-wire face**, an operator surface over its own build — the daemonless `:memory:` build behind `mrd sql` is one, and carries no wire vocabulary.

## §11 Rules as data — the compatibility surface

### §11.1 Verdicts in write responses

Every splice response (dry or real) carries `verdicts:[]` — typed findings from the loaded packs:

```json
"verdicts":[{"rule":"blurb-required","severity":"warn","path":"notes/plan.md",
             "hpath":[{"h":"Goals"}],"span":[20,150],"node_rev":"5a8faa717fbcdb04",
             "message":"section has no blurb line"}]
```

Severity ∈ {`error`, `warn`, `info`}. On an **armed** workspace, block-severity verdicts and door-law violations **refuse the write in-engine** after CAS (`gate()`; § A. Armed plane); on a never-armed workspace they stay advisory. `budget_exceeded` is a finding here. The shape is `crates/policy`'s `Violation{rule, severity, path, span, node_rev, hpath, message}` verbatim.

### §11.2 WHEN/HOW partition (validation-enforced)

A rule's WHEN sees world-model facts only — nodes, revs, spans, links, the given `now`/`actor`. Its HOW is opaque data the engine never interprets (escalation, notification, remediation: host business). A WHEN outside that fact vocabulary fails compile (`UnsupportedVocab`, an existing `policy::CompileError` variant).

### §11.3 Pack manifest (generic, evaluator-free)

```yaml
id: wiki-hygiene
api: rulepack-api@1
budgets: { steps: 10000, mem: 4194304 }
fixtures: [fixtures/blurb-pass.md, fixtures/blurb-fail.md]
rules: [rules/blurb-required.md, …]
```

**Fixtures are the load gate:** a pack whose fixtures fail under its declared budgets is never admitted. Budgets are per-eval `{steps, mem}`, metered; exhaustion surfaces as `budget_exceeded`. The perf harness exists (`crates/perfsuite`, claims-as-data → verdicts).

### §11.4 The rule language — Starlark, ratified

Rule predicates are fenced Starlark in literate rule pages, evaluated in-engine via starlark-rust; dialect + injected API pinned as `rulepack-api@N` in the §11.3 manifest. The pinning is pack-level (`api:` and pack docs); the wire names no evaluator, so verdicts, WHEN/HOW, budgets and fixtures-as-load-gate read the same whatever a pack pins — an evaluator change is a pack change, never a wire amendment.

## §12 The hash domain — md-only + `meridian/domain.md`

### §12.1 The domain

Which files' bytes enter the workspace **fingerprint** (the merkle content hash):

1. **md-only floor** — only `*.md` files hash; non-md paths never enter.
2. **Default ignore (one rule)** — any path with a **dot-prefixed segment** is ignored (`.github/…`, `.obsidian/…`, `.trash/…`). Custom re-includes cannot lift this floor for non-md paths, nor the dot rule's intent on editor noise.
3. **Custom ignore** — optional rules on the **standing declaration page** `meridian/domain.md` (frontmatter: `version` + an `ignore` list; the body may explain). Patterns are gitignore-style — block list, last match wins, `!` re-includes — with the **trailing-slash law**: a pattern ending in `/` names a directory, so it excludes files beneath a matching directory segment, never a bare file whose basename fits the pattern body (`scratch*/` excludes `results/scratch-r4/venv.md`, not `tasks/scratch-cleanup.md`; `git check-ignore` rules the same pair).

**Hash domain ⊂ addressable domain — one answer at every door.** The filter gates hashing, not load. An out-of-domain path (an ignored `.md`, a dot-segment path) is still served by `toc`/`cat`/`read`/`extract`/`check_write`/`splice`: the read door serves its spans and mints its `file_rev` as for a member, the write door commits, and its bytes do not move the fingerprint (`fingerprint_before == fingerprint_after`). A door that refuses one is a door defect, and its CAS token (`node_rev_before` / `file_rev`) is mintable at the read door like any other. `file_not_found` means one thing at every door — **no such file under the workspace root** — never domain exclusion.

**The rule above binds a door family: every door the caller names a path at**: `links <PATH>`, `walk <PAGE>` and `repair <PAGE>` take a caller path as `cat` does. The same predicate bounds the rooted-ref lane: every door the caller names a page at resolves the agent-plane `[root:]path` spelling (§ A.12, `address-grammar.md` § 4.6).

**Enumerators are bound to say, not to admit.** A whole-corpus enumeration (`retire`'s sweep, the `sql` projection, `check`, bare `links`) MAY exclude what its `as_of` fingerprint cannot cover — **never silently: the exclusion is named in the output, and an enumeration that certifies absence either refuses or names what it did not see.**

**The verdict plane must say what it did not look at.** `walk`, `check` and `status` judge a pin's target — a path the caller never named — over the hash domain, so an out-of-domain target is absent only because **the engine did not look**. The door plane and the verdict plane both ask existence first, by reading the named path — the domain-independent read every named-path door owes, since existence is a disk fact and the domain a fingerprint fact.

| Named target | Verdict |
|---|---|
| absent from disk (in-domain or out) | red `file-not-found` |
| present, outside the hash domain | grey `outside-hash-domain` — grey outranks red |
| present, in the domain | green, or red on real drift or a real miss |

Grey outranks red because `pin` returns rc=0 on an out-of-domain path and writes its anchor to disk: a red would destroy a real attestation. The reason word `outside-hash-domain` says *seen but not hashed*, unlike the greys that mean *could not look*.

**A verdict may not assert a resolution that did not occur**, so an absent page never gets `selector-unresolved` (*the page resolved, the selector failed*) or `dangling-anchor`; `file-not-found` means *root reached, path genuinely absent*, and its recovery is to restore or re-pin the file, not fix a selector. Existence runs ahead of every address question, displacing the resolution reds for every selector class, the block class included.

Scope: the existence read runs where the domain arm runs, so it answers for the ambient root only (a mounted root's corpus is built by its own filter, which no face carries across). **A miss inside a mounted root is measured by resolution as before**: it renders `file-not-found`, naming its root; a face with no disk gets `cannot say` and keeps its resolution verdict.

**Standing surface:** `meridian/domain.md` only.  
**Legacy filename (not design):** `mdfs_config.yaml` may still be *read* when it is the **only** domain config present (old workspaces). Do not create it; do not teach it. Two at once is an error (ambiguous domain), not a precedence rule. See `crates/fs/src/domain.rs`.

Worked counterfactual — a wrong ignore implementation fails this fixture pair:

| Domain | Fingerprint (computed) |
|---|---|
| correct (`.github/` ignored) | `b3:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9` |
| wrong (`.github/README.md` included) | `b3:75a61c883e372102cfe7d75e94992b9be65e33fbe95956897a4cf2ea45bb8f1b` |

### §12.2 Interior encoding (merkle spec §4)

Leaf = blake3(whole raw file), full 32 B. Interior = blake3 over children sorted by raw name bytes, each `varint(len(name)) ‖ name ‖ type_byte(0x00 file / 0x01 dir) ‖ hash32`; empty dirs pruned; the workspace directory's own name never hashed. Full detail: `node-rev-merkle-spec.md`.

### §12.3 Domain-rule changes bump the prefix

An ignore-list (or algorithm) change re-defines the domain, so the token prefix advances: `b3:` → `b3a:` → … The `version` field rides **`meridian/domain.md`** with the ignore list, so definition and prefix travel together.

Worked at **S0**, writing `meridian/domain.md` in the forms §0.3 prints. `drafts/tmp.md` may be on disk for **rows 3–5 only**, which declare an ignore list covering it; **rows 1 and 2 require it absent**: with no ignore list a `drafts/` file joins the domain. The config is markdown, so **its own bytes are in the domain it declares** — why v0 and v1 differ in hex over one member set. Values are engine-measured:

| `meridian/domain.md` | Files hashed | Fingerprint |
|---|---|---|
| absent (S0 as printed in §0.3) | plan, receipts | `b3:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9` (= R0) |
| v0 — `version: 0`, no ignore list | plan, receipts, **domain.md** | `b3:23421037fa8d4a947aa7104941797b325e38c67878787773f76bc1009c63bab4` |
| v1 — ignore `drafts/**` | plan, receipts, **domain.md** | `b3a:48c0b314c7e0bf2d570936a302a4d5be4802a03187a988353efc5725b45067b1` |
| v1 — ignore `drafts/**`, `meridian/**` | plan, receipts | `b3a:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9` |
| v2 — same ignore list | plan, receipts | `b3b:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9` |

**The prefix tracks the domain rules, never the member set**: row 2 adds a file and stays at `b3:` (`version: 0` with no ignore list changes no rule); rows 3–5 advance because the rules moved. **The same surviving hex never compares equal across prefixes**: rows 1, 4 and 5 carry one 64-hex value under three tokens, so receipts cannot silently match a redefined domain. The table is computed over the standing `meridian/domain.md`, whose own bytes are in the domain it declares. The same pair computed through the legacy non-md `mdfs_config.yaml`, whose bytes stay out, would not close (§18 row 11).

**Hash-law retirement rides the same ladder.** The one-time interior-encoding cutover (fixed-256 radix child maps — `node-rev-merkle-spec.md`) changes the hash law, so the prefix advances and old tokens never compare equal. Three facts stay unflattened (§5.7, §8.2):

- A known retired family's token refuses `fingerprint_version_retired` with re-mint teaching, never `fingerprint_mismatch`: the law moved, not the premise.
- An unknown future family's token refuses `fingerprint_version_unsupported` — distinct, taught apart.
- Only a current-family unequal digest is the normal scoped mismatch.

**When retirement begins is the cutover's no-return boundary** (`node-rev-merkle-spec.md` §4.2.5): before it the old law serves and nothing refuses `fingerprint_version_retired`; the durable cutover record alone crosses it. No downgrade fence activates (no old-binary users) and no non-serving shadow build exists (pay once).

**No dual-hash serving window exists**: the engine never serves two hash laws at once. The price is one typed, taught re-plan event per workspace; `sub` re-baselines at the cutover with a labeled epoch boundary, never a silent break.

## §13 Threat and limit register (the honesty standard, throughout)

1. **16-hex rev truncation:** ≈2^32 birthday work on attacker-fed content forges a rev collision. Mitigation: the trusted-local boundary — this wire serves local, trusted workspaces, so any "adversary-proof" claim is dead. Full-width `fingerprint` and `file_rev`-over-whole-file are the escalation ladder.
2. **Staleness:** no lag bounds, ever (§10.1). `as_of_fingerprint` may trail `live_fingerprint` at any moment; `require_fingerprint` refuses, it does not synchronize.
3. **Storage re-probe posture:** on major dependency bumps (pulldown-cmark fork rebase, blake3 major, any ratified view engine's format), conformance packs and measured envelopes are re-probed, never assumed to carry — the app-oracle re-probe rule extended to every measured claim.
4. **Conformance unknowns are pack-pinned, never asserted:** stage-1 duplicate-basename tie-break, embed depth-cap constant, app version drift — pinned as `obsidian-compat@1.12.7` answers at default settings, re-generated per app version via the live oracle (`obsidian eval`). Pack and manual: `crates/testsuite/data/gt/obsidian-compat/`, replacing hand-frozen resolution fixtures.
5. **Root-history ring bound 256:** older ranges answer `fingerprint_unknown` → full resync. Re-derive, never serve wrong data.
6. **Crash window in two-file batches** (§6.5): content-without-receipt is possible; loud via lint, recovered via re-root.
7. **No-self-rooting** (§6.2): receipts structurally cannot carry their own `fingerprint_after`.
8. **`_` block ids:** the single ruled charset (§2.4) makes any `_`-bearing id unaddressable as a strict anchor (loud `bad_request`). No re-id migration ships; the implementation owns the mint-guard and the frozen-fixture exemption; the app's treatment of legacy ids stays pack-pinned.

## §14 Repo grounding — speced ON TOP of meridian-rs

Sequencing uses the **rung ladder** numbering (dialect → facts/check parity → integrity+diff → write/CAS → subscribe → policy packs); the repo's rung comments differ (write/CAS is repo rung 2, subscribe repo rung 4) — one numbering, said once.

| Crate seam | Stands | Changes under this schema |
|---|---|---|
| `crates/syntax` | single entry `parse(&str) -> Vec<DialectNode>`, 11-variant dialect vocabulary, fork-pin law | nothing — implement the `todo!()` |
| `crates/wire` | four noun newtypes; standing `Toc`/`Extract`/`Hello`; `Node` + kind ordinal; `ErrorBody` typed extras; strict/tolerant obligations | +`Delta`; +ops `cat`/`links`/`fingerprint`/`diff`/`sub`; `resolve`/`splice` reshaped by rung-freezing amendments; `ErrorCode` grows the §8 splits; no bare `not_found`; no `Guard` op |
| `crates/transport` | `NdjsonCodec` (implemented, tested), raw-id frame classification seam, null-id serialization test | raw-lexeme validation here, before typed decode; JSON-only — no length prefix, no frame-size bound (§18 row 14) |
| `crates/model` | `build`, richer NodeKind, no-serde law, **sealed `ValidatedSplice` capability discipline** (an unvalidated write cannot reach disk by construction), `merkle_root` seam | `SpliceRequest{span, if_node_rev, text}` reshaped to match-based (`target + match/put` — §4.4); `resolve` serves the strict plane only; hash algorithm = blake3 (the rung-2 wire amendment the model doc reserved — §1) |
| `crates/fs` | `load`/`walk`/`apply_splice` seams, tmp+fsync+rename, no-storage law (memory that cannot be thrown away is an architecture violation) | `apply_splice` takes the batch (content + receipt append, one commit — §6.1); walk gains the §12 filter |
| `crates/wire-map` | `prefix_16b` (implemented, contract examples as tests), the model+wire projection seam | `project` implements the superset-by-embedding predicates (§15) |
| `crates/policy` | `Violation`/`Severity`/`RulesetPin`/`CompiledRuleset`/`CompileError` — this schema's §11 shapes; `policy::gate` stays deferred off the splice path (actor is a wire input, not an engine gate — §9) | `evaluate` output rides splice responses as `verdicts` |
| `crates/query` | `backlinks` seam | serves `links` with the §10 triple |
| `crates/registry` | the daemon host, the one wire door (§3.3) | serves §4 over the unix socket |
| `crates/testsuite` / GT | consolidated test binary, GT pack + provenance | GT regenerated from this contract: lane pack demoted; resolution GT = app-generated `obsidian-compat@1.12.7`; every deviation row ships a fixture the deviated-from dialect fails (the discrimination law); `wsfix/` values join |

The downstream implementation plan is sequenced against this table, which is its contract.

## §15 Structural guarantees index I — construct-level guarantees

*Each claim, plus the § that guarantees it.*

- **Every dialect construct is wire-representable; no lossy projection** — `wire-map` superset-by-embedding, as four wire-observable predicates: every construct is representable (an 11-kind enum, `Comment` and `InlineCode` included); wikilink information is carried whole; an unterminated fence surfaces as `unterminated`; frontmatter key order is preserved in `keys` (§4.1). Divergence is a projection compile error, not runtime loss (§14).
- **Ids are validated as raw lexemes before typed decode**, full discrimination set worked (§3.1).
- **Ground truth is regenerated from this contract, not hand-authored** — resolution GT is app-oracle `obsidian-compat@1.12.7` (§14, §13.4).
- **One rev per node; client spans have no expressible form** (§5.1).
- **The mint/walk partition is a type law** — no rev field on the walk plane; stale names fail `ref_not_found` (§2, §4.5).
- **One grammar per plane, advertise-nothing-resolve-nothing** — one block-id charset (§2.4); extract emits ⇔ resolve resolves (§2.1).
- **No bare `not_found`; each error class binds one recovery** — `file_not_found` (env) ≠ `ref_not_found` (refresh) (§8).
- **Op discovery is complete; no version sniffing** — dotted `op.field` caps (§3.2).
- **The hash domain is md-only and config-declared**, with a counterfactual fingerprint pair worked and the prefix bumped on a rule change (§12).

**Foundational sweep — standing guarantees:** span law, newline-inclusive spans, merkle-§5 pins (§1); `node_rev` MUST when `splice ∈ caps` and proto retained (§3.2); out-of-set values loud-reject (§4.3); batch-only splice, one response shape (§4.4); Deltas as catchup entries, `#:key` dead (§7, §2.1); the 16-hex width with its threat (§13.1); the deviation-inventory law — "a rowless divergence is a contract bug" (§13.4, §8).

## §16 Structural guarantees index II — usage-pattern coverage

*Per usage pattern: **Matched** = served as a fact op; **Above-wire** = served by a consumer over named wire facts, not by a wire op; **Dead** = not served, by usage evidence.*

| Pattern | Disposition |
|---|---|
| `check` | Above-wire: verdicts (§11.1) + toc/extract facts; the check *engine* is a pack, not an op |
| `run` | Above-wire: actor capability; the wire serves fm-key facts for task blocks (§2.1) |
| `version` | Matched: `hello` proto + server + complete caps (§3.2) |
| `help` | Matched: `caps` complete set (§3.2) |
| `read` as oracle | Matched: `resolve` two-stage in-band (§4.5); audit shape is `links` (§4.6) |
| `#Heading` reads | Matched: `resolve … content:true` (§4.5); no silent pick — strict-plane `ambiguous_ref` (§2.1) |
| `skill render` | Above-wire: actor capability; wire serves cat/extract |
| `rules ls` | Above-wire: pack manifest is data (§11.3); pack listing is a consumer surface |
| `schema` | Above-wire; the write half is fm_key nodes (§4.4) |
| corpus-level `check` | Above-wire: procedure layer |
| `encode` | Above-wire: pure grammar library, no wire surface (§2) |
| `fix` | Above-wire mutation policy over `dry:true` + per-file batches (§4.4) |
| `debug` | Above-wire: rule dev tooling over §11 verdicts |
| `attest` | Above-wire effects layer; dry seam + fm_key handles (§4.4) |
| `mv` | Loudly alternativized: a composed consumer op — `links` (§4.6) + `fileToLinktext` emission algebra (the app's) + per-file splices; multi-file atomicity absent (§6.5) |
| `status` | Dead as op; liveness is the daemon's; the change feed is `sub` (§4.7) |
| `watch` | Dead as CLI; Delta + `sub` + recovery law serve it (§7) |
| `resolve` CLI | Matched: `resolve` op (§4.5) |
| `append` | Matched: `put{at:"end"}` with full receipts, worked (§4.4) |
| `toc` | Matched: `toc` with rev/write-kit per node (§4.1) |
| `pipe` | Dead; staged commit re-expressed as `dry` + atomic batch + `if_fingerprint` (§4.4, §5.1) |
| `edit-section` | Matched: the flagship — `match` edit + CAS; conflict recovery is one `cat` (§4.4, §5.2) |
| `set-prop` | Matched: fm_key splice (§4.4); `#:key` grammar dead (§2.1) |
| `chain promote` | Above-wire: effects layer |
| `debt` | Dead — orientation, not action (§10.3) |
| `domains tree` | Dead — orientation (§10.3) |
| `rules check` | Dead as op; fixtures-as-load-gate does it structurally (§11.3) |
| `def check/census` | Above-wire per vision; def-conformance = named amendment candidate riding `dry` |
| `domains show` | Dead — orientation |
| `def fix` | Dead |
| `cache stats/clean` | Dead as CLI; cache is implicit engine state (no-storage law, §14) |
| MCP `read`/`put`/`pipe` | Matched by vision: the MCP face is a wire client (§4.8 is face-agnostic) |
| friction: mass-mutation whole-tree write (~1,347 files) | Inexpressible: splices name explicit paths + targets; no glob/whole-tree grammar in §4.4 — the strongest gate, by omission |
| friction: JSON-arg dead ends | `caps` + loud echoing rejections (§3.2, §4.3) |
| friction: exit codes leak through pipes | In-band `ok` per correlated frame; no exit codes on the wire (§3.1) |
| friction: resolver/linter divergence | one resolver: the app's two-stage algebra, app-oracle GT; rules read the same facts (§4.5, §11.2) |
| friction: version drift mid-task | `hello` + complete caps in-band (§3.2) |
| friction: unregistered-check spam | An op is in `caps` or answers `unknown_op`; packs admit via fixtures or not at all (§3.2, §11.3) |
| friction: no machine-readable check output | Frames-only stdout, logs stderr (§3.1); verdicts typed (§11.1) |

**Cross-cutting facts honored:** low organic rev use is served by a permissive wire + ambient revs + a host policy ratchet (§4.4, §5.3); in-process consumers become wire clients (engine-linked ones are transition-tolerated only); zero-use flags are not carried; unknown request fields reject loudly (§3.2); heading reads ride `resolve.content`.

## §17 Structural guarantees index III — top-level requirement coverage

*Requirement labels are the review checklist item names.*

| Requirement | Where answered |
|---|---|
| Superset embedding + id/GT laws (A1) | §15 (nine guarantees + the sweep) |
| One grammar, two planes (A2) | §2 (layering per the parity ruling) |
| Convergence-item closure (A3) | mint partition §2/§4.5 · rev 16-hex + threat §1/§13.1 · md-only + config §12 · resolve `from` two-stage §4.5 · app-oracle GT §13.4 · nine items: raw-id-before-decode §3.1, dotted caps §3.2, error split §8, fm-plane §2.1, rev-MUST §3.2, batch-only §4.4, newline-inclusive §1, GT-regen §14, proto-retained §3.2 · hybrid interface §4.8 |
| Worked-value honesty (A4) | tool statement (header) + recompute from §0.3 fixture bytes |
| Usage-pattern coverage (A5) | §16 |
| Delta noun (A6) | §7 (+ `diff` reserved §4.7, replay ≡ live §7.3, node-grain §7.4) |
| Receipts (A7) | §6 |
| Actor/now as wire inputs (A8) | §9 + the §1.1 replaceability table |
| Rules-as-data (A9) | §11 (Starlark §11.4; evaluator-free wire) |
| View topology (A10) | §10 (no lag bounds; optional wire-agnostic view organ) |
| Honest limits (A11) | §13 (register of 8) |
| Repo grounding (A12) | §14 (stands/changes per crate seam) |
| Skill doc + HTML page (A13, A14) | Downstream deliverables bound to this contract; not claimed here |

**Rulings attest:** the §0.2 gates are ruled and folded — node-grain deltas (§7.4), Starlark (§11.4), optional view organ (§10.4), the single block-id charset (§2.4); deviations and waivers sit in §18. **Tool attest:** blake3 over the §0.3 fixture bytes; offsets from byte math; zero invented values.

## §18 Deviation & waiver ledger (fix-at-freeze)

Every reviewer-flagged debt is fixed or waived with a reason here, never silently. Rows 1–5 are the review fix list; rows 6–7 consolidate deviations declared in the body; rows 9–14 are measured against shipped artifacts and this document's arithmetic. Rows 1–7 record where this document departed from ruled law; rows 9–14 where a shipped artifact departed from this document, measured against the built binary. A deviation found without a row here is a contract bug (§15's assumption-audit law). **Declaring is not legislating:** the law a row names stays in force, unamended, and the row is the observation, never a licence.

| # | Item | Disposition |
|---|---|---|
| 1 | Policy-scope grammar unbound to the strict plane | **FIXED** — §5.3 binds it: `Path`-set selectors + strict-plane refs (§2.1), no second grammar; the manifest's missing scope field is deliberate |
| 2 | `fingerprint_mismatch` once named `expected/actual/scope/changed`; this contract ships `{expected,actual,scope?}` | **`scope` served, `changed` STRUCK.** `scope` rides the scoped world guard (§5.4): `fingerprint_mismatch{expected,actual,scope?}` (§5.7, §8); absent = the root premise. `changed` is minted by nothing — the caller cannot derive it, and the root-history ring is RAM-only per epoch, bounded at 256 with eviction, so no door can fill it when the caller is most stale. `resync` instructs the only honest recovery: a full re-read |
| 3 | The frontmatter node's span `[0,20]` is terminator-inclusive, against the §1 / merkle-spec §2 leaf-block law | **WAIVED, declared** — the frontmatter node is a fence-to-fence container, span-lawed with the section (newline-inclusive) family; its `fm_key` leaf (`[4,15]`, §4.4) excludes its terminator. All hashes stand |
| 4 | Two silent rebinds vs the ruled failure-class table, plus one dropped code | **FIXED, declared** — §8 declares all three: `fingerprint_mismatch`→`resync`, `unsupported_proto`→`respawn`, `bad_id` folded into `bad_request` + `id:null`/`id_raw`. Behavior-preserving |
| 5 | The A6 self-claim "replay ≡ live stated and tested" — nothing tests it | **FIXED, restated honestly** — executed: the fixture recomputation behind every worked value (§17 tool attest). Not executed: any replay ≡ live test or conformance-pack run, both implementation deliverables. "tested" is retracted |
| 6 | No bare `not_found` | Declared at §8 (split into `file_not_found` env / `ref_not_found` refresh) |
| 7 | "Unknown `kinds` match nothing" reversed to loud `bad_request` | Declared at §4.3 (the strict-server evolution law applied to values) |
| 9 | **The raw-lexeme id law (§3.1) at the daemon door**, on every op: on `mounts`, `{"id":5}` echoes while `{"id":"5"}` answers `id:null`. Nulling a non-conforming lexeme (`"1"`, `-1`, `1.5`, `true`, `null`) and serving it would deviate | **SERVED.** `transport::scan_id` scans the raw `id` lexeme at frame classification, before decode/dispatch, at the `crates/registry` door (§3.3); a bad lexeme refuses `bad_request` with `id:null` plus the verbatim lexeme in `id_raw`, never served, never reclassified as a notification (`2^64` pinned). `recovery` is `fix` (§8's binding; the respawn consequence stays client-side law keyed off the `id:null` frame header). §3.1 STANDS UNAMENDED, gated by `crates/registry/tests/v3_key_set_pins.rs`: `contract_3_1_a_non_integer_id_is_refused_with_id_raw`, `a_non_integer_id_is_refused_with_id_raw_at_the_daemon_door`, `an_out_of_range_id_is_refused_never_reclassified_as_notification` |
| 10 | **The fixture's S1/S2 receipt bytes must be printed** (§0.3): §6.3's receipt lines must be the fixture's own bytes, byte-exact | **CLOSED.** The two lines measure 260 B / 262 B on the node basis, matching spans `[26,286]` / `[287,549]` and the receipts file's 26 → 287 → 550 B growth; `root_before=` for §6.1's `fingerprint_before=` would move them **7 B**, a `Goals>Q3` join against the mandatory §2.1 form **31 B**. Receipts are markdown in the hash domain (§6.1), so a receipt-byte change moves the node rev and the fingerprint: R1 and R2 are RECOMPUTED, never re-typed, gated from the committed S0 bytes (`crates/testsuite/tests/pf_frozen_sweep.rs`). **Not owed:** the CLI lane writes **254 B** node (**255 B** line) because it mints no request id — `id=` is 6 B of §9's absent-inputs law, ruled not a defect — while the same template writes 260 B byte-for-byte when a request carries one (`crates/receipt/tests/frozen_receipts.rs :: e3_receipt_line_byte_exact`); live S1/S2 leave the published R1/R2 timeline by lane, not by shortfall. **The escaping is §6.7 rule 2**: the escape exists before the emission that creates the hazard |
| 11 | **§12.3's worked table must compute over the standing surface**, not the legacy non-md `mdfs_config.yaml` whose bytes stay out (`meridian/domain.md` self-hashes by design, `crates/fs/src/domain.rs`; that pair would contradict §0.3) | **HELD** — §12.3 computes over the standing `meridian/domain.md` with engine-measured values, and §0.3 prints its v0 and v1 bytes; the legacy filename stays do-not-create, do-not-teach. **The S0 file set is unmoved** — `meridian/domain.md` is absent at S0, R0 unchanged, and printing bytes never makes a member (absent → R0, v0 present → `b3:23421037…`, removed → R0 returns) |
| 12 | **CLI-lane commits advance the fingerprint and mint no Delta.** §7.1 laws one Delta per batch per fingerprint advance and §10.1's `changes_seq` is that counter; measured, `mrd put` moves the fingerprint while `changes_seq` reads 0 before and after | **DECLARED, not waived; §7.1 and §10.1 STAND UNAMENDED.** A consumer reading `changes_seq` as a change monotone misses every CLI-lane write silently. The fingerprint is the only monotone covering both lanes today, so cross-lane catchup is diff-by-root (§4.7), as for cross-epoch. The CLI-lane delta is owed |
| 13 | **`hello.identity.build` must state the `-dirty` half of §A.3's identity token** (`sha \| sha-dirty \| unknown`): a bare sha asserts a whole commit, so a build baked from `git rev-parse HEAD` with no cleanliness probe publishes one for a dirty worktree, and a foreign resident daemon then serves wrong results, no error | **SERVED.** The build stamp probes cleanliness and publishes `<sha>-dirty` where tracked content diverges from HEAD (`crates/mrd/build.rs`), and the socket law (§A.3) has the local client compare `hello.identity.build` at connect and refuse across builds, closing that class at the serve door. §A.3 STANDS as written |
| 14 | **No frame-size bound exists.** A `MAX_FRAME_BYTES` bound would be a length-prefix concept; the NDJSON seam (`crates/transport` NdjsonCodec) reads newline-delimited lines with no length prefix and no size cap | **HELD — nothing enforced**; a bound nothing serves is not law and this contract invents none (§3.1, §14). **Open question, deliberately not answered here:** whether the seam wants a line-length cap. If ruled later it amends §3.1 forward and lands in `crates/transport` with its tests |

Row 8 is **not used**: a `_`-bearing anchor refusing loudly (§2.4, §4.5) conforms under the one-way compatibility floor — parity is a floor, not law — so there is no walk-plane charset deviation. Nothing else here knowingly deviates from ruled law.


---

## § A. Standing additions (compact)

These are **current law**, not optional history. Implementation detail may lag; the shapes below are what agents and hosts must learn.

### A.1 Fingerprint-or-force (every pure write door)

Content-mutating writes on the **wire door** (the daemon socket, the only door, §3.3) require fingerprint match **or** `force`. Guard fields stay **schema-optional**: a guardless frame still **decodes**, and a write with neither fingerprint nor `force` is refused **after decode** as `guard_required` (recovery: `fix`) — a semantic refusal, not a frame rejection. `force` is any client's refuse→rewrite path; MCP is not a separate trust plane. In-process paths (`mrd` without the wire door) are out of scope, not out of trust.

The satisfying set is the §5.4 premise vocabulary: any legal tree token, judged by the Coverage Law at admission. `guard_required` means no premise and no `force`; premises that fail coverage refuse `scope_does_not_cover{uncovered}` (§5.5, §8.2). `if_node_rev` covers its own edit; `if_fingerprint` covers everything.

The demand binds every op that lands content by declaring its write set in the request: `splice` in every form with its composed fields (§ A.3/§ A.5), `create`, `remove`. It does not bind the effects lane — `run` (§ A.8) and `script` carrying `effects` (§ A.7) are unguarded by ruling (no CAS premise, no fingerprint requiredness, no synthesized touch-set guard), because a guard over execution mrd cannot bound promises what it cannot keep. A guard field there refuses `bad_request` at the §3.2 strict wall (inapplicable to the op — § A.8; teaching: §8.2), never `guard_required`, a pure-write-door refusal only. `script` without `effects` satisfies the demand by construction: its commit premise is the engine-computed touch set (§ A.7), caller premises legal as widening. The one-door transport law (§3.3) is untouched — the limit is op scope, not transport scope.

### A.2 Armed change plane (block is a feature)

When a workspace is **armed** (attested index present), after CAS and before bytes land the engine runs `gate()` over its own armed set. Block-severity verdicts and closed door-law violations **refuse the write**. Never-armed workspaces stay advisory. `--force` escapes an armed refusal and is loud (journaled and rendered). Detail and bootstrap ladder: `armed-plane.md`.

**Essential refusal recovery bindings** (subset of the closed §8 taxonomy; the attestation suite runs against these):

| `code` | `recovery` | Typical trigger |
|---|---|---|
| `ambiguous_ref{candidates}` | fix | write selector matches more than one node |
| `ref_not_found{stage,dest?}` | refresh | pinned or named ref dangles |
| `no_match` | fix | selector or match string resolves to nothing / zero occurrences |
| `guard_required` | fix | content-mutating write without fingerprint or force (A.1) |
| `convention_fault{index}` | env | armed index missing/corrupt on once-armed workspace |
| `armed_drift{armed_rev,report_rev}` | refresh | armed law drifted |
| `cas_mismatch{expected,actual}` | refresh | node or create/remove CAS failed |

#### A.2.1 Middleware on the write door

*(The armed plane's third kind. Full doctrine: `armed-plane.md` Part A2; only
the wire shape is ruled here.)*

- **`rules/middleware`** pages evaluate on the write door — after CAS and
  batch validation, before bytes land — in `id`-ascending order, mode
  `off|block`. They may `refuse`, transform this file or other files, and
  birth files; every disk emit joins the caller's write in **one sealed set**
  (validate-all-then-apply: all or nothing, one fingerprint advance, one Delta
  carrying every member). They may also emit `send` **intents**, never
  applied by the engine.
- **Request field `fields`** (splice single form + create): an optional
  `{string: string}` object, opaque to the engine — no key interpreted, none
  required — delivered to middleware verbatim as `ctx.fields`. Hosts put
  caller context here (creation stamp, session id, agent id);
  `actor`/`now` remain the §9 envelope inputs. Absent `fields` decodes as the
  empty map. The set form (`splice.set`) does not carry it: no middleware
  evaluates there in V1, so a `fields` key there refuses at the strict wall.
- **Response field `armed.intents`** (splice; `intents` top-level on the
  birth response, which has no `armed` group): on every non-dry successful
  write through a door that evaluates middleware, an array — possibly empty,
  never absent. V1 items are exactly:

  ```json
  { "kind": "send", "to": ["<recipient-or-channel>", …], "body": "<text>", "rule_id": "<middleware id>" }
  ```

  `kind` is closed (V1: `send` only). The engine never marks an intent
  delivered: realization is the host's, and a host must not return a bare
  success while a realization result is missing. An intent failure after
  commit names itself on the host's response; the disk set stays (send is not
  this write).
- **Response field `armed.set`** (splice, single form): on the same writes, an
  array — possibly empty, never absent — naming every other file the sealed
  set committed. The caller's own path never appears (its facts are the
  response's existing `armed` group). V1 rows are exactly:

  ```json
  { "path": "<root-relative>", "change": "modified" | "created",
    "file_rev_after": "<16hex>", "rules": ["<middleware id>", …] }
  ```

  `path`, `change` and `file_rev_after` are lifted from the commit's own
  Delta row — a repeated committed fact, never re-derived (`change` speaks
  the §7.1 vocabulary; V1 members are edits (`modified`) and births
  (`created`)). `rules` names the middleware id(s) whose emits compiled the
  member, first-touch order. Receipt appends and pin promotions are commit
  machinery, not members, and never appear.
- **Write responses carry no reaction envelopes** (`armed.effects` serializes
  empty here); `rules/hook` + `proto.send` ride the external-change detector
  only. A middleware refusal refuses `convention_fault` naming the rule id
  and its passing scenario, as a check refusal does; middleware armed-law
  faults (red / unloadable / unevaluable rows) fail closed under the same
  A.2 codes.

### A.3 Composed `read`, `check_write`, `mounts`, `plan_edits`, `pin`, `create`, `hello.identity`

| Surface | Role |
|---|---|
| `read` | Addressing, content, render and frontmatter props at one snapshot; section selectors are §2.1 segments (or anchor / dewey), never a joined string. |
| `check_write` | Write pre-flight: the splice verdict without writing. Read-only. |
| `mounts` | Mount-table discovery: the live root registry, machine-scoped. Read-only (§ A.5). |
| `splice.plan_edits` | Plan-level batch shapes; addresses are **segment arrays** — a heading path, or a `^id` block ref as the single segment. The lane resolves the read face's own anchor plane: a toc-listed anchor is writeable by its id, a host-excluded or absent id misses at both doors. That plane covers every body host Obsidian addresses (paragraph, list item, task, callout, table, fence, heading); the frontmatter caret is the one host-excluded miss. `match` edits block-leaf bytes; `replace_section` replaces block content, keeping the `^id` marker; `append` refuses toward the containing section. |
| `splice.pin` | Pin rides the write choke-point; selector is segments/anchor. |
| `pin-cross-root` | `splice.pin.target` admits the ruled `name:rel` rooted spelling (`address-grammar.md`): target loaded, gated, promoted and blob-written in the named mounted root under that root's own `LOCK_NB` write flock; the lock row's `object` = `name:rel` minus `.md`, verbatim; the proof compare (`splice.pin.proof`) runs on the target root's live bytes under that flock. Without the cap a face keeps its own taught refusal, not `pin_target_missing`. |
| `splice.pin.proof` | Pin proof rides the request — the proof law, below. |
| `create` | File birth through the guarded door; full body bytes, no `props` field (the starlark birth lane's `props=` dict rides this door, § A.8). `bad_path` on an engine machinery segment in the landing — `.git`, `.meridian`, `meridian`, `receipts` — at any depth, ASCII-case-insensitively, whatever the caps admit; the hash-domain config `meridian/domain.md` is the one carve-out (`run-plane.md` § machinery floor). |
| `remove` | File death through the guarded door; `remove_refused` while anything in the corpus references the record (below). |
| `hello.identity` | Optional `{build: sha \| sha-dirty \| unknown}` for deploy identity. `-dirty` rides the sha token (git-describe): `sha` = a whole commit, `sha-dirty` = a worktree diverging from it, `unknown` = nothing attributable. Match the whole token, never a substring — a decorated sha is a different build and must refuse (`docs/release.md` §5.1). Optional, v3-only |

**The socket law: a local client refuses a cross-build daemon.** One cache root
⇒ one socket ⇒ one resident daemon, whatever binary bound it first, so an
upgrade-in-place leaves a stale build serving every caller. Path
`$XDG_RUNTIME_DIR/mrd/<12hex>.sock` on Linux, else
`$HOME/.cache/mrd-run/<12hex>.sock`; `<12hex> = sha256(cache_root bytes)[:12]`;
pidfile `<12hex>.pid` beside it; state and registry at `<cache-root>/registry/`.
Injective per cache root; the short base keeps the path inside `sun_path`.

- **Scope: local clients only.** Dialing the derived socket on its **own cache
  root**, a client MUST compare `hello.identity.build` with its own baked build
  identity, whole token (the `hello.identity` row above), at connect:
  equal → serve, a different token or **no identity published** → refuse — an
  in-memory equality on the hello frame the one dial already parses, so no
  extra round trip.
  `hello.identity` stays **optional on the wire**: a remote peer, or a caller
  on a foreign cache root, is not bound.
- **The refusal is client-minted, one voice.** No §8 engine frame, no wire
  error code — the engine never sees the call. Stderr, CLI exit 2, minted
  before the operation is sent, in the skew grammar: both build identities, the
  `SKEW` verdict, the reason, fitted suggestions. No silent degrade to the
  in-process engine.
- **The remedy speaks the teaching register: reason first, then suggestions by
  applicability — never one demanded command.** Reason: the cache-root keying,
  and a resident surviving an upgrade until something restarts it. Then per
  condition: *you own the resident* → restart it (kill the pid in the named
  pidfile; the next call auto-starts the current build); *a pipeline manages
  it* → rerun its install step; *neither is yours* → report the skew to the
  operator, quoting both builds. Conditions state applicability, not authority.
- **Mechanism only.** No version ordering, no replace/supersede machinery, no
  restart duty: whoever installs a binary restarts the daemon, from the install
  pipeline (`justfile` `install`), never Rust. Known limit: two `unknown`
  tokens compare equal and assert nothing; the sha stamp (`build.rs`) keeps
  that rare.

**Pin proof rides the request.** In `put` the pin supplies `node_rev` or
fingerprint from the agent's own read. No server-side record of who read what
exists — no read-receipt ledger, no journal — and a read is identity-free and
side-effect-free: the composed `read` op has no `actor` field.

- **The read serves the token.** Each resolved section row of a sections-mode
  read carries that section's `fp1.…` fingerprint — the content-identity
  CID-token over the section's span, anchor-lines excluded, the token a pin of
  that section mints as `pin.fingerprint` — beside its `sec_rev`. The toc
  serves neither content nor fingerprints.
- **The pin spends it.** `splice.pin` takes `fingerprint` (the proof half) and
  `sec_rev` (the write-conflict half, optional, the section CAS token from that
  read). The live fingerprint is recomputed over the resolved target span under
  the write flock `splice` holds — the target root's flock for a cross-root
  pin — and compared; equal → the pin proceeds. The promotion marker is
  invisible to the compare (anchor removals), so a re-pin needs no refreshed
  token.
- **Requiredness follows the door.** A splice with a real session `actor`
  (daemon/MCP) must carry `fingerprint`; absent → `pin_proof_required` (fix
  class: read the selector in a sections read, carry the served `fingerprint`,
  pin again). The bare CLI door (`actor` absent) is local-operator-trusted and
  may pin proofless; a proof it does supply is still verified.
- **The mismatch split speaks two causes apart.** Supplied `sec_rev` differing
  from the live section's → `write_conflict{expected: caller's, actual: live}`
  (refresh class: re-read, pin again). `sec_rev` matching or absent →
  `pin_proof_required`, naming both possibilities (the content moved since the
  read, or the token is not from a read of this section), one round trip. Bad
  input is never spoken as a moved world (§ A.7 precedent).
- **Script verifies nothing:** no pin, no proof vocabulary; its commit premise
  is the engine-computed touch set (§ A.7).
- **Proof is content-bound, not identity-bound:** identity decides only whether
  proof is required, never whose read satisfies it.

**Composed-`read` selector resolution:**

- More than one match → `ambiguous_ref` naming each candidate's machine address
  (its `n`-carrying segment array); never a silent first match, never
  `ref_not_found` (§2.1's never-silently-picks, for strict reads as for `cat`
  and `splice`).
- All selectors fail → the refusal names **every** failed selector with its own
  reason (no match / ambiguous), as the partial-read `notice` does.
- **Remedies speak the operation, not one host's tool name**: the recovery
  clause names the toc read in each surface's own dialect (MCP: a read with
  `sections[]` omitted; CLI: `--section`-less read), never a binary the caller
  may not have — MCP spelling first, optionally a labeled CLI alternative.

**The `toc` scope:**

- The whole-call subtree scope is the `toc` field: **one tagged §2.1
  selector**, not a segment array. No `frag` field (it refuses at the strict
  decode, like any unknown field) and no `#fragment` on the wire: `path` =
  which file, `sections` = which content, `toc` = which subtree map.
- Resolution precedes serving, through the same `selector_matches` the sections
  plane uses: a heading path or a **dewey ordinal** resolves to one row, and
  the scope is that row's subtree-inclusive span. Rows and the `anchors` plane
  are bounded by byte containment, never by segment prefix, which would merge
  same-named siblings.
- The **anchor arm refuses** `bad_request`: a block has no subtree; the refusal
  teaches the `sections` lane.
- A **bare duplicate refuses** `ambiguous_ref`, naming each candidate's
  `n`-carrying machine address with the published ambiguity remedy; a dewey
  miss refuses `ref_not_found` in the dewey lane's own voice, remedied by the
  bare read that lists the ordinals.
- `toc` beside `sections` refuses `bad_request` **"pass one"**.

**The read plane's own budget:**

- A `sections[]` read carries **two bounds of its own**, both enforced in
  `wire-serve`'s `composed_read` so the wire and CLI doors answer identically:
  at most **20 000 words served per call** (`READ_MAX_WORDS`) and at most
  **64 distinct selectors per call** (`READ_MAX_SELECTORS`). Over either → `bad_request`,
  **refused, never truncated**, nothing read and no rev minted, naming the
  measured number, the ceiling and its `→` recovery.
- **The unit is words, not bytes, and that is the discoverability half of the
  face-honesty law (clause 2):** `words_total` and a `words` on every toc row
  and served section price a section before it is asked for. One named
  constant, sized to fire before an MCP host clips (near ~25k tokens ≈ 18–19k
  words).
- **The section map is never word-bounded**: it is the recovery the size
  refusal points at, and a refusal must point at a door that answers (clause
  3).
- **Repeated identical selectors are collapsed, and the collapse is stated**
  (clause 1): a repeat resolves to the same node, row, bytes and `sec_rev`.
  Identity is the selector's serialized spelling — two *different* spellings
  landing on one node stay two rows, since each row carries the caller's own
  `sel` back. The 64 ceiling applies to the collapsed set.

**Door symmetry over duplicate headings:**

- An `n`-less address matching more than one node refuses `ambiguous_ref`-class
  at **every** door, read and write alike (`splice.plan_edits`, and any host
  lowering onto it): the write-door refusal also names each candidate's machine
  address and teaches `n`. No door resolves what another refuses as ambiguous.
- Published addresses carry `n` exactly where the document is ambiguous, so
  read → verbatim address → write always lands.
- **The prose is symmetric too.** Both doors speak one remedy: **pin one
  occurrence by its `n`-carrying machine address, or by its dewey ordinal from
  the toc** — never *"by block id or node index"*, which omits `n` and
  prescribes minting an id on a heading the caller may not own. Renaming a
  duplicate heading stays a legitimate, secondary fix and is named as one.
- ⛔ **Neither ambiguity refusal ends in a wikilink** — `[[selector-grammar]]`
  is a vault-local address a machine caller cannot dereference, and both
  remedies are self-contained (an `n` address, a dewey ordinal, a distinct
  block id). *Scoped to the ambiguity pair: the `see [[address-grammar]]` tail
  on the `crates/addr` refusals is the same class, not swept here — a recorded
  finding.*

**Door symmetry over duplicate block ids:**

- An anchor selector (`^id`) whose id appears on more than one block refuses
  `ambiguous_ref` at **every** strict-plane door — the composed read's
  `sections[]`, `cat`, `pin`, `splice` (§2.1's duplicate-anchor row). Never a
  silent first match: a read picking one would hand back a `sec_rev` the write
  door refuses.
- The anchor grammar carries no occurrence index (`n` disambiguates hpath
  segments; `{"anchor":id}` has no `n` slot), so **no machine address exists
  per candidate**: `candidates` stays `[]` and the message names how many
  blocks carry the id. `toc`'s `anchors[]` publishes every occurrence with its
  span, duplicates included.
- The remedy **speaks the anchor grammar, never the heading one**: give each
  duplicate block a distinct id (a block id addresses exactly one block in its
  file), or address the enclosing section by heading path. "Rename one heading"
  never appears on an anchor refusal.

**Teaching row — the anchor host-kind gate is a read-face law, and the write
door does not carry it:** `unaddressable_host` and `anchors[]` cover only the
block kinds this read face addresses — every body host Obsidian's block
references cover, the host span being the attached block
(`model::anchor_host_span`) — leaving the frontmatter caret unpublished. The
native write door has no host-kind gate: a frontmatter-hosted `^id` the read
door refuses is still a legal `{"anchor":id}` splice target on the strict plane
and arms a rev transition normally, while the plan lane resolves against the
face plane (door symmetry, A.3). Every address the map publishes it serves, but
absence from `anchors[]` is no evidence that a native write will refuse.

**`check_write` — the standalone pre-flight (a host consumes this op on every
guarded put):**

- Request `{"op":"check_write","path":…,"target":…,"actor":…,"now":…,"edits":[…]}`,
  strict-decoded, v3-only at dispatch (`crates/wire-serve/src/decode.rs`),
  advertised in v3 `caps` (`crates/wire-serve/src/rev.rs`). Each edit is
  `{op, at, find?, body?, rev?, all?}`; `at` is the §2.1 segment array
  (`{h, n?}`) the committer takes, single-segment forms carrying a block `^id`
  or a frontmatter key (`crates/wire/src/lib.rs`). `path` addresses the file
  under the workspace root; `target` is the raw host path labelling refusal
  strings.
- Reply `{"refuse":…?, "repairs":[…], "forced":[…]}` (`crates/wire/src/lib.rs`).
  `refuse` absent = the write may proceed. `refuse` is
  `{class, code, message, remedy?}`; `class` picks the host's render template —
  `rebuild` (candidate unbuildable) vs `verdict` (severity ladder refused).
  `repairs` are `{key, value}` autofill property sets the host folds into the
  same atomic write; `forced` echoes overridden warn rule-ids. Both always
  serialize.
- Read-only over the warm engine (`crates/registry/src/server.rs`); no file
  under the workspace root → `file_not_found`. A real file outside the hash
  domain is served from disk on the same snapshot (§12.1 addressability): corpus
  residency is not the admission test. Mandatoriness stays host policy (§5.3) —
  the engine computes the verdict, the host decides. `splice` re-runs the same
  verdict inside its own flock (`crates/wire-serve/src/write.rs`), closing the
  check→apply TOCTOU gap: this op is the host's pre-flight and error-rendering
  surface, the splice-internal run is the law.

**Law A-1 at the create door — `create.rev`:**

- Shape `{"create":{"parent_hpath":[…],"title":…,"body":…,"rev":…?}}`. `rev` is
  the **parent's** node-grain token: the `node_rev` read for the section the
  birth appends under, or the document's when `parent_hpath` is empty (§2.1
  zero-segment hpath). A section create lowers to a parent-append, so the
  parent's rev is the honest grain; `rev` threads to the lowered edit's
  `if_node_rev`, compared per §5.1 and re-derived at execution from the
  pre-batch state — one derivation, no second comparison rule. The
  child-absence guard is unchanged: an already-born subject refuses
  `cas_mismatch`.
- **Schema-optional, like every guard field (§ A.1):** a rev-less `create`
  frame still decodes; the demand is a semantic refusal after decode, never a
  frame rejection.
- **The demand (the engine-side create-door law):** on a wire-origin splice, a
  `create` whose `parent_hpath` carries any `n`-bearing segment demands `rev`
  or `force`; absent both → `guard_required` (fix class), teaching the slot
  (`create.rev`) and the toc read that mints the token. `{h, n}` binds by
  *position among identical texts*, which a same-titled sibling insert re-binds
  invisibly to the child-absence CAS; a guard that mints its own token proves
  nothing (Law A-1).
- `force` bypasses it as it bypasses every fingerprint-plane demand (§ A.1):
  loud, bypassed plane named against the parent subject. No new force
  semantics.
- Dotted cap `splice.create_rev` (v3 projection; frozen v2 caps byte-identical).
  On occurrence-free parents an offered `rev` is honored (CAS); demanding it
  there is the §5.3 ratchet, a named future-amendment candidate. Do not widen
  the demand here.
- **The armed fact names the birth (A.6.3a′ is the precedent; the law is
  §6.1/§7.1):**
  a `create` row's armed edit carries the **born section**: `target` = the
  address the read face publishes for it (`n` where the document is ambiguous),
  `node_rev_before` = the empty-input hash (`af1349b9f5f9a1a6`, A.6.3a′'s
  born-from-nothing token, not a claim that an empty section existed),
  `node_rev_after`/`span_after` = the born node's facts from the one post-batch
  reparse. The parent's rev guards the CAS (`create.rev` above); the parent's
  own transition stays implicit, re-readable via `toc` like every ancestor rev
  (§7.1). The receipt renders the same facts with op `create` (§6.4). The born
  node is identified by the position the sealed batch placed it, never by
  counting siblings; a reparse leaving no section heading there refuses
  `would_corrupt{target_identity}`, the §4.4 family for armed facts that cannot
  be represented. The native `put:end` door is untouched: a native append
  addressed the parent and keeps naming it.

- **Empty `parent_hpath` is the top-level birth door:** `create` with
  `parent_hpath: []`
  births a level-1 heading at document end, running the § A.3 hygiene
  composition over the document's full span — `put{at:"end"}` for a pure
  extension (insert at EOF), `put{at:"content"}` when trailing whitespace at
  EOF must collapse — with the document as native target (`hpath: []`); the
  armed fact still names the born section. `rev`, when offered, is the
  document's node-grain token (the file span; equal to `file_rev`); a section
  rev CAS-mismatches. Schema-optional, no occurrence demand (no `n` on an empty
  parent). Absence guard: a same-title top-level heading present refuses
  `cas_mismatch`.
- **Why not parent-append on the last heading.** A last section whose span
  already includes its trailing blank does not receive the sibling's bytes, so
  its `node_rev` does not move and `would_corrupt{transition_unrepresentable}`
  refuses (`# Notes\n\n` then append `# Edges…`; `# Notes\n` writes, hygiene
  growing the last section by the separator) — that empty-tail + blank line
  shape is the regression. The document receives the bytes. `append` opening a
  heading at a non-final section stays `would_corrupt{containment_lost}` — use
  `create`.

**The remove door — guarded file death:**

- **Request** `{"id":…,"op":"remove","path":…,"if_file_rev":…,"actor":…?,
  "now":…?,"if_fingerprint":…?,"dry":…?}`: strict-decoded, v3-only at dispatch,
  advertised as cap `remove` at op grain (no dotted `remove.<field>`; frozen v2
  caps byte-identical). `now` is RFC 3339, validated never generated (§9).
- **`if_file_rev` is the mandatory guard — remove-what-you-read:** the record's
  own whole-file rev from the caller's read. Schema-optional (§ A.1); absent →
  `guard_required` (fix) after decode, teaching the slot and the read that
  mints the token; stale → `cas_mismatch{expected,actual}` (refresh). The
  world-grain `if_fingerprint` is honored when present (`fingerprint_mismatch`,
  resync), never demanded: the referential check is recomputed inside the
  critical section (scope-key law: node-grain wherever a node token exists).
- **The referential guard — refuse while referenced.** Inside the unlink's own
  write flock, after the door-entry observation (`root_before` / the world
  guard): wikilinks and embeds through `query::backlinks` (link-plane
  resolution, walk stage 1), ambient `meridian-lock` pins through
  `query::lock_pin_referrers` (the walk plane's Down predicate at corpus
  grain) — never a new index, never a merkle fold. The door lists the hash
  domain (`fs::hash_domain`) and reads member bytes for `fs::build_corpus`
  only, never `domain_snapshot`: fingerprint and referential parse are
  different reads. Any inbound edge → `remove_refused` (fix), unlink never
  runs. Self-edges are excluded; a dangling inbound spelling does not block.
- **The refusal names the referrers:** `referrers:[{path, kind, count}]` — each
  referring file, its edge kind (`wikilink` / `embed` / `pin`), its edge count,
  path-lex sorted — then the teaching register: these records still reference
  this one and removing it would strand them dangling, so unlink or retarget
  each named edge, then resend.
- **Check and unlink are one critical section:** check, `if_file_rev` CAS and
  unlink all run under the workspace write flock, so no cooperating writer
  lands a link in a TOCTOU window. Editors that never take the flock (a
  human's editor, raw `rm`) stay outside every write door's serialization — the
  residual every door carries.
- **No `force`, by ruling.** No `force` field; strict decode refuses a frame
  carrying one. Deletion is the one irreversible op, so the one door with no
  escape hatch — the forced-birth precedent applied to death. A stale rev →
  re-read and resend; a fresh rev plus a referentially-empty record always
  lands.
- **No tombstone, by ruling.** The file ceases; the terminal fact is the death
  Delta the response and the delta plane carry — `change:"deleted"`,
  `file_rev_before` present, `file_rev_after` absent (§7.1, unchanged) — plus
  the fingerprint advance. `sub` transports it live with `actor`/`now`
  attribution, `diff` replays it within the ring, and past the ring the path is
  absent from the re-derived world (`fingerprint_unknown` → full resync). Disk
  is markdown only; git history is the archaeology. Hash law is untouched — a
  removed leaf leaves the tree under the existing merkle composition
  (`node-rev-merkle-spec.md` records the ruling).
- **Response** `{"path","file_rev_before","fingerprint_before",
  "fingerprint_after","seq","dry"?,"verdicts"}`: what died (its confirmed rev),
  the world transition, the Delta's seq. `dry:true` runs everything except disk
  (guards, referential check, verdicts) and carries `fingerprint_after:null`.
- **Refusal codes, complete:** `guard_required` (fix — no `if_file_rev`) ·
  `cas_mismatch{expected,actual}` (refresh — drifted from the read rev) ·
  `remove_refused{referrers}` (fix — inbound references exist) ·
  `fingerprint_mismatch{expected,actual}` (resync — stale world guard) ·
  `file_not_found` (env) · `bad_path` (fix — escapes the workspace). The
  armed-plane gate runs over the death's before-state (`ChangeOp::Remove`) as
  at every door; the index-integrity floor (the armed index and once-armed
  marker refuse removal) is unchanged and predates this door.
- **Stated limits** (§13 register, not silent): (1) cross-root inbound pins are
  invisible — the guard enumerates this workspace's own corpus, so a remove
  here can strand a pin held in a different root red (the walk plane colors it
  broken on the pinning side); (2) the guard reads the attested corpus (§12
  hash domain), so references in files outside the domain do not block; (3) a
  concurrent corpus reader can see a path vanish mid-read and refuses whole
  (no-partial-load) — vanished-path reader behavior is priced on the run-plane
  lane, not changed here.

**The `replace_section` containment law:**

- **The invariant:** after `replace_section(target)`, every byte outside the
  target's subtree is identical and the subtree is exactly the payload. A
  payload that would restructure the document refuses whole — never demoted,
  never clamped. So a `replace_section` armed fact or receipt line
  (`wrote §target rev:a→b`) can never describe bytes that landed outside the
  target's subtree.
- **The gate:** a payload heading at or above the target's own level refuses
  `bad_request` (fix class) with the `payload_escapes_section` grammar, naming
  the offending body line, the payload heading's level, the target's level and
  the honest alternative: restructuring is a write to the parent, so target the
  parent section or use `create_section`. No `allow_escape` flag.
- **Judged on the parsed payload, never line-regex:** the dialect parse's own
  heading law applies (ATX only, ≤3 indent), and `#`-lines inside a fenced code
  block are code. A setext underline is not a dialect heading, so a
  setext-shaped payload splices contained as body text; that engine/CommonMark
  divergence (Obsidian renders an h2) is known, and the engine-side definition
  is pending, not this law.
- **The one normalization:** a payload whose first line echoes the target's own
  heading — same level, same title — has that line stripped silently, and the
  remainder splices under the rules above; an echo-only payload normalizes to
  an empty section. First-line-only: the same heading later refuses as a
  duplicate sibling, and a same-titled deeper heading is ordinary content,
  never normalized.
- **One refusal carries both facts:** the gate runs at plan lowering, before
  the §5.1 CAS comparison, against the same flocked pre-image CAS reads. A
  payload that escapes and a stale `rev` produce one refusal, carrying the
  containment teaching and the stale-rev fact (current rev inline, resend token
  included).
- **Scope:** this law binds the plan door's `replace_section`. The native
  `edits` face stays byte-exact Edit-model (§4.4 unchanged, including the
  truthful-transition law for sibling-opening appends). The same containment is
  expected for the plan door's `append` and `create_section` bodies, untested
  there today; until that lands they rely on the §4.4 post-reparse families
  alone.

**Splice hygiene at the plan doors (the companion of the `replace_section`
containment law):**

- The plan-level body verbs — `append`, `replace_section`, `create` — compose
  their lowered bytes so every boundary the splice touches is canonical:
  **exactly one blank line at block and section boundaries** (between a
  section's heading line and its content, between adjacent blocks, before a
  following heading); a file ends on a single terminator. One exception, itself
  a boundary rule: a payload whose first content line is a list item, appended
  to a section whose last block is a list, joins that list flush — a blank line
  there splits one list into two (CommonMark loose-list). Interior payload bytes
  stay the caller's, verbatim; the payload's own leading and trailing blank
  lines collapse into the canonical separators.
- Mechanism selection is derived, never declared: the canonical result is
  compared against the section's current content bytes. Pure extension (the
  result starts with the existing content) → `put{at:"end"}`; a boundary
  needing surgery (a separator to remove or collapse, e.g. the trailing blank
  line that must not sit inside a joined list) → a content-span rewrite,
  `put{at:"content"}`, preserving every non-boundary byte. The armed fact and
  the receipt name the lowered shape; target and rev transition are identical
  either way.
- The native §4.4 ops are untouched: `at:"end"` stays raw byte concatenation
  and a native caller owns its separators. Hygiene is plan-door composition law
  only.

**Frontmatter-properties plane on the composed `read`:**

- The composed-read reply gains a `props` plane: the document's top-level
  frontmatter key facts, at the same engine snapshot as the rest of the body —
  one row per key, document order, first occurrence wins (the model's flat
  parse is the keys authority; the v2 toc `keys` echo serves the same order).
- Row shape `{key, value, span, prop_rev}`:
  - `key` — the top-level key, the string the §2.1 `fm_key` form addresses;
    the read→set loop closes off this row.
  - `value` — the key line's value **decoded through the § A.6 scalar law**:
    the colon remainder, whitespace-trimmed, then unquoted when it is a
    well-formed quoted scalar. A block value (indented continuation lines)
    serves the key line's own remainder, empty when it carries none. Quotes
    are never kept — a value, not source bytes (§ A.6).
  - `prop_rev` — the key's CAS token: blake3 over the full key grain span
    bytes (the key line plus its indented continuation lines), 16 hex — the
    same token `cat` on the `fm_key` node serves and `if_node_rev` compares
    (§5.1).
  - `span` — that grain span; intra-file byte offsets, root-independent.
- Emission law (the `anchors` precedent): always emitted — empty means "this
  document has no top-level frontmatter keys", never "ask again with a flag";
  decoding tolerant of older recorded frames, serialization unconditional.
- Document-grain, both modes, never `toc`-scoped: frontmatter belongs to the
  document, not to any subtree.
- v3-only by construction (the composed read is v3-only at dispatch): never on
  a v2 session, frozen v2 caps and bytes byte-identical, no new cap — a
  response-side additive field under the tolerant-client law (§3.2), the
  `words`/`anchors` precedent.
- Delta grain unchanged: the map tense of the frontmatter plane (§7.2 — one
  projection, three tenses); §7.4's ruled node-grain and its named future-only
  `keys` amendment path are untouched.

**Per-selector unresolved facts on the composed `read`:**

- The composed-read reply gains an `unresolved` plane: one row per section
  selector that resolved to no served section, in request order — the machine
  tense of the partial-read `notice` and the all-fail refusal (the A.3 symmetry
  law: three tenses of one fact set). `notice` and `truncated` stay unchanged
  beside it, from the same resolution pass, so they cannot disagree.
- Row shape `{sel, reason, candidates, count?, host?, nearest}`:
  - `sel` — the failed selector echoed in its own request grammar
    (`{"hpath":…}` / `{"n":…}` / `{"anchor":…}`): correlate by shape or
    position, not by parsing a display string.
  - `reason` — closed vocabulary, one per row: `no_match` (nothing carries the
    address) · `ambiguous` (a heading or dewey selector matched more than one
    node) · `duplicate_anchor` (more than one block carries the `^id`) ·
    `unaddressable_host` (the id exists, but its host is outside the face's
    anchor plane — the frontmatter caret alone, every body host being
    addressable; distinct from `no_match`, the remedy differs).
  - `candidates` — `ambiguous` only: each candidate's machine address as the
    §2.1 `n`-carrying segment array (actual arrays, never encoded strings), in
    the order the refusal names them. Always serialized; `[]` on every other
    reason, `duplicate_anchor` included (no per-candidate address exists).
  - `count` — `duplicate_anchor` only: how many blocks carry the id.
  - `host` — `unaddressable_host` only: the true host kind (`frontmatter` in
    practice; the same open string the toc anchor row echoes, never a
    fallback).
  - `nearest` — anchor-`no_match` only: the nearest live ids as
    `{anchor, kind}` rows. **The candidate pool spans every `^id` on the page,
    non-addressable hosts included**, `kind` being the host kind, so a render
    teaches the host-kind gate instead of implying absence. Empty when the page
    carries no `^id`. Always serialized.
- The prose teaching draws from the same widened pool: the miss clause names a
  non-addressable candidate with its host kind and the servable way in; the
  no-anchors clause claims a bare page only when no `^id` of any host kind is
  present.
- Emission and v3-only as for `props`: always emitted — empty means "every
  selector resolved" (a toc read trivially so), never behind a flag; decoding
  tolerant of older recorded frames; never on a v2 session, frozen v2 caps and
  bytes byte-identical, no new cap (§3.2).

**The counting law — one `words` number per rev, every face:**

- A `words` value is always `strings.Fields` over the raw bytes of the range
  it names, and is never assembled by summing other rows.
  - `words_total` (both modes, and the script toc face's `words`) names the
    file: fields over the whole document, frontmatter included — the number
    `wc -w` prints.
  - `toc[].words` names a section subtree, unchanged: fields over the
    heading-excluded, subtree-inclusive content span. Rows therefore do not sum
    to the banner; that is the law, not a defect.
  - `sections[].words` is that same section-grain count, off the same raw
    content bytes `sections[].content` carries, on the structured plane and in
    the rendered head. The projection may show less (engine-block elision) or
    more (claim-link decoration) than the section holds; what is shown never
    changes what is counted. `bytes` alone declares the served length.
- One derivation each, in code: `wire_map::facts::words_total` and
  `wire_map::facts::section_words`. A face that computes its own count is the
  defect this law names.

CLI inventory (descriptive): `status.md`. Cross-root agent address grammar: `address-grammar.md`. Config parse: `meridian-md-schema.md`.

### A.4 What this document does not teach as core

- Joined hpath strings as machine addresses  
- `mdfs_config.yaml` as the domain config (use `meridian/domain.md`)  
- SQL / `view_path` / DuckDB as agent path  
- Dual wire constitutions ("v2 vs v3" for agents)

### A.5 `mounts` — mount-table discovery

`mounts` serves the mount table `~/MERIDIAN.md` binds for the whole machine.
Read-only; machine-scoped; v3-only at dispatch, advertised as cap `mounts`
at op grain (no dotted `mounts.<field>` at birth); a v2 session answers
`unknown_op` and its frozen caps stay byte-identical. No parameters and **no
workspace binding required**: a bare `hello` connection may call it.

```json
{"id":7,"op":"mounts"}
{"id":7,"ok":true,"body":{
 "config_rev":"9f27a2814b517681",
 "mounts":[
  {"name":"wiki","state":"bound",
   "workspace":"/home/me/wiki"},
  {"name":"agent-sessions","state":"bound",
   "workspace":"/home/me/agent-sessions","primary":true,
   "alias":"sessions"},
  {"name":"assets","state":"grey(path-unseeable)"}]}}
```

Row shape `{name, state, workspace?, primary?, alias?}`; no `kind`, since
the config schema carries no mount taxonomy (`meridian-md-schema.md` §5.1).

| Field | Law |
|---|---|
| `name` | canonical `MountName`, lowercase `[a-z0-9-]` (`address-grammar.md` § 4.3) |
| `state` | the `MountState` word verbatim, one spelling on human line, `--json` and wire: `bound` · `grey(path-unseeable)` · `grey(undeclared)` · `grey(declaration-unreadable)` · `grey(claim-unverifiable)` · `red(content-drifted)`. All but `bound` refuse; a client gates on `state == "bound"` and treats an unknown word as not-bound (the set may grow) |
| `workspace` | canonical bound path, the handle `hello` returns as `workspace`; present exactly when the binding canonicalized, absent at least on `grey(path-unseeable)` |
| `primary` | literal `true` exactly on the designated row (`meridian-md-schema.md` §5.1a); two designations refuse the whole table (`duplicate-primary-designation` inside `mount_table_invalid`). A host role: the engine reports it, never acts on it |
| `alias` | a second spelling for the root (`meridian-md-schema.md` §5.1b), present exactly when the block declares `alias:`. Lookup only: `root:path` resolves by **name first, then alias** (`address-grammar.md` §4.6a); every canonical echo (`name` here, receipts, pins, `mint {…}` paths, `sub` rows) carries `name`. A client hard-codes `sessions:`, which the alias maps; `primary` is not consulted. An alias equal to any `name` or other `alias` refuses the whole table (`alias-shadows-name` inside `mount_table_invalid`) |

`primary` and `alias` are copied verbatim from the binding file; absence is
their only "not declared" spelling, as in the config grammar. Both are
field-only amendments, caps `mounts.primary` and `mounts.alias`; an unread
key is inert (tolerant-client law).

**The implicit default row (schema §5.1c).** If no mount is named or aliased
`sessions`, the table may carry the implicit default `sessions` at
`$HOME/.local/share/ucc/sessions`, only when it binds clean: a real bound
root, shape-identical to a declared row, unmarked on this wire (`mrd config`
marks it `(implicit default)`; `--json` `"implicit": true`). A declared
`sessions` name or alias suppresses it.

**Freshness — the config-hash rebind law.** Every call hashes
`~/MERIDIAN.md` (blake3 over the file bytes) and re-derives only on a
changed hash; no mtime, TTL, or hello-time snapshot. A mid-session mount
appears on the next call; a vanished root degrades to its grey row.
`config_rev` is that token: 16 lowercase hex, `file_rev` family
(`blake3(whole file bytes)[:16]`, §1 rev sub-laws), opaque, equality-only; a
client caches on it, never parses it.

**Changed-invalid refuses.** If the hash changed and the re-derive fails
(duplicate names, paths or vault names; nested mounts; closed-schema
refusals, `address-grammar.md` § 3), the op refuses; it never serves the
previous table:

```json
{"id":8,"ok":false,"error":{"code":"mount_table_invalid","recovery":"env",
 "path":"~/MERIDIAN.md",
 "message":"two mounts bind the canonical path /home/me/wiki (duplicate-mount-path)"}}
```

Class `env`: the caller must fix the binding file; the refusal names the
offending entry (scope and member). Per-root grey states are served rows,
not this refusal; only a table-level parse or bind failure refuses.

**The staleness triple does not apply.** `as_of`/`live`/`changes_seq`
(§10.1) belong to view answers that may trail the live fingerprint;
`mounts` re-derives per call, and `~/MERIDIAN.md` lies outside every
workspace's hash domain (`~/` is no workspace).

**Not a bare `read`.** An optional `read.ref` would turn a missing required
argument (today a loud error) into a success with unrequested content;
`read`'s arguments stay required, and discovery is its own op.

### A.6 The frontmatter scalar law — decode on read, encode on write

*One law, two directions: the engine publishes a property value as the decoded
string, and writes a property value as a YAML scalar that decodes back to
exactly the caller's string. Read and write are inverses; nothing between them
is quote-tolerant.*

**The defect this closes.** Every property value plane (`props[].value`, the
`fm_key` `cat` remainder, the `set_property` value) is `string`, never a YAML
node. Serving and writing source bytes fails both ways:

- **Read, fail-inert.** `owner: "3f9a1c07"` served as `"3f9a1c07"` (10 bytes,
  quotes included) compares false against `3f9a1c07`; no rule arms.
- **Write, fail-closed.** `set_property owner=[[b1892b5a]]` emitted
  `owner: [[b1892b5a]]`, a list-of-list the I4 substrate law refuses.

**A.6.1 Decode (every read seam).** A value is unquoted when, and only when, it
is a **well-formed** quoted scalar: `'…'` with interior `'` only as `''`, or
`"…"` with no unescaped interior `"`. Single-quoted resolves `''`→`'` and
nothing else; double-quoted resolves `\\ \" \n \t \r` and leaves any other
escape verbatim. Everything else is served verbatim after the whitespace trim:
plain scalars, flow collections (`[a, b]`), and malformed quoting, which no
reader may guess at. A quoted scalar is a string in every schema: the decode is
the quoting layer only, never type inference.

**A.6.1a Block scalars.** A key line may carry a YAML **block-scalar header**
instead of a value: `>` or `|`, an optional chomping indicator (`-` strip, `+`
keep, default clip) and an optional indentation digit, with the value on the
following indented lines. One reader, `model::fm_block_scalar`, decodes it for
both published faces (`read`'s `props[]`, `sql`'s `frontmatter`): folded
breaks become spaces, a run of *k* blank lines becomes *k* newlines, a break
next to a more-indented line is kept, and chomping owns the trailing breaks.

The compared-value seams — `preset`'s `^properties` rule check and both halves
of `realise`'s `FieldEquals` — publish through `model::fm_doc_publish`, the
one `Document`-grain door over `fm_publish`, never through
`model::scalar::text`, whose opening `value.trim()` eats the newlines and
leading spaces the block-scalar decode produced. A seam that publishes the
indicator byte (`">"`) mis-serves valid YAML — a decoder gap, not corpus
damage — and a root whose projection carried such rows rebuilds its drawer at
the accompanying `SCHEMA_SALT` bump.

Consequences: `props[].value` may carry `\n`, under both indicators (clip
chomping leaves one trailing newline on a folded scalar too); a one-line face
escapes it, the JSON plane carries it verbatim. The write plane does not
widen: § A.6.3 still refuses a newline (D11), so a block-scalar value cannot
be written back through `properties`/`set_property` unchanged.

The law binds every seam that publishes a frontmatter value or compares one
against a caller-supplied string, with one owner (`model::scalar`). Compared
seams decode both halves: `status: "done"` compared raw against `done`, or a
board predicate `owner = '3f9a1c07'` against raw `"3f9a1c07"`, is the
read-half silent false again.

| Seam | Plane |
|---|---|
| composed read `props[].value` (§ A.3) | published value |
| a script's `fm` dict (`fm_key` value) | published value |
| the run plane's frontmatter binding values — **§ A.6.1a does not reach this row**, see the carve-out below | published value |
| `preset`'s `^properties` rule check and its `type`/`defines`/`root`/`births` reads | compared value |
| `realise`'s `FieldEquals` — both halves: the page's declared `realise.expected` and the observed field | compared value |
| the view projection's `frontmatter.value` column — and the `record` pivot and B2 tag parse riding it | published value |

**A.6.1a carve-out: the run plane's binding values.** The binding row stays
under § A.6.1 (`task.build: "[[#^x]]"` unquotes), but § A.6.1a does not reach
it, and routing it through `model::fm_doc_publish` would be a regression. A
binding's grammar is `[[#^id]]`, so surrounding whitespace is never content;
`run::address::parse_binding_value` strips `[[` and `]]` as a matched pair
before the later `v.trim()`. A `>`-folded binding is stored decoded as
`"[[#^id]]\n"`: published verbatim, `strip_suffix("]]")` misses the newline,
the whole string reaches `split_once("#^")`, and the non-empty target `"[["`
refuses `AddressError::CrossFileRef`; trimmed, it resolves. A block scalar
that is not a block ref refuses `InvalidBinding`. The binding plane reads a
value and never publishes one: accepted or refused, never mis-served. Pinned
by `crates/run/tests/binding_block_scalar.rs` (both directions plus the
`PyYAML` reading of the fixture).

**A.6.1′ A list value is read off the block, for every key.** The flat map
behind the value seams keeps only the key line's remainder, and a YAML **block
sequence** puts every item on a following line, so `agents:` over indented
`- <id>` lines would publish `''`.

Rule: the view projection's `frontmatter.value` for a key whose line carries
no scalar and whose block sequence follows is that sequence **rendered as the
flow-style text it spells** — `[a, b]`, items verbatim, spelling kept
(`- "[[x]]"` renders `["[[x]]"]`). A multi-line flow sequence (`[a,` / `  b]`) joins the same way.
Comment and blank lines inside a sequence are skipped; an indented non-item
ends the walk (fail-closed). A bare key line with nothing below stays `''`;
the engine never invents `[]`. The rendering is text, never a YAML node; a
consumer splits it as `parse_alias_list` / `parse_tag_list` split a flow
list. One reader owns it (`model::fm_value`), and `fm_tags` rides the same
walk, so the tag lane and the value lane agree on where a sequence ends.

Named residual: the other seams in the table (`props[].value`, a script's
`fm`, `preset`'s `^properties` check, both halves of `realise`'s
`FieldEquals`) go through `model::fm_doc_publish`, which is block-scalar aware
but block-sequence blind, and still serve `''` for a block sequence.
`rule.key` and `realise.field` are arbitrary keys, so a rule over a
list-valued key (`tags:`, `aliases:`, `agents:` are commonly block sequences)
compares its def string against `''`. Pinned by
`crates/testsuite/tests/props_plane.rs`'s
`a_block_value_serves_the_key_line_remainder_and_the_full_grain` (the test
name, not a line range). The run plane's bindings read under the `[[#^id]]`
grammar, so a bare key line with a sequence below refuses on the grammar.

**What stays raw** (§ A.6.2, the stance `cat` takes): `lock` (guard tokens),
`policy::change`'s `diff_fields`, and the view's locator and rev columns
(`span_start`/`span_end`, `node_rev`, `file_rev`) answer questions about the
stored bytes, and a quoting-only edit is a change to the stored form.

**Named residual:** `policy::change`'s `DocFacts.frontmatter` — the
`(key, value)` pairs the effect kernel's `on_change(event)` receives — is a
published value plane and still serves stored bytes. It lands with the change
kernel's own contract work.

**A.6.2 The stored form stays raw where hashing is the point.** `prop_rev`,
`span`, the props fingerprint (`props1`) and every node rev are computed over
source bytes, untouched by this law. A guard token must distinguish
`owner: ""` from `owner:`: the R4 three-state law (absent ≠ null ≠ empty
string) lives in the stored form, and decoding at the hash grain would
collapse two states into one.

**A.6.3 Encode (every value-plane write door).** The emitted line is
`{key}: {encoded}`, and the encoding is the inverse of A.6.1: **emit the plain
form when the plain form decodes back to exactly the caller's string; otherwise
emit a double-quoted scalar** (`\` and `"` escaped). The quoted form is the
canonical one for hosts that write frontmatter beside this engine, so both
write the same bytes. A value is quoted when it:

- is empty, or a null spelling (`~`, `null`, `Null`, `NULL`) — the plane has
  no null;
- starts with `'` or `"`;
- would parse as a **map or nested collection** (`{…}`, `[[…]]`, an
  unterminated `[…]`);
- carries `: ` unquoted, starts with `#`, or carries ` #`;
- carries an interior TAB (`k: a<TAB>b` kills the whole block for PyYAML;
  `serde_yaml` reads it);
- would be typed by a YAML **1.1** resolver even where 1.2 leaves it a string:
  a digit run (`19895504`; `02146210` is octal 576 648 to PyYAML), the
  underscore grouping `1_000`, the underscore radix forms (`0x1_f` is 31,
  `0b1_010` is 10) and every `0b…`, the sexagesimal forms (`12:30` is 750,
  `1:02:03` is 3723, `1:30.5` the float; each group after the first is 0–59),
  and the word booleans `y`/`yes`/`no`/`on`/`off` in every case variant;
- is a 1.1 resolver tag, `<<` (merge) or `=` (value): plain, PyYAML refuses
  the whole block while serde_yaml reads both as strings.

Unchanged: a **one-level flow list** (`[a, b]`) emits verbatim — the only way
this string plane authors a non-string value — and a **timestamp**
(`2026-08-07`) emits plain, because `serde_yaml` reads it back as the caller's
string. A newline is refused, never sanitized: a single-line value cannot
carry one, and an escaped-scalar workaround leaks.

**No typed-scalar carve-out: `true` and `7` are quoted.** An 8-hex short id is
all digits about 2.5 % of the time (203 of 8 125 distinct ids on one live
root), and git shas share the shape: plain, `owner: 19895504` reads back as an
integer everywhere and `session: 02146210` as 576 648 in PyYAML. A caller's
string is written so that PyYAML, `serde_yaml` and `gopkg.in/yaml.v3` all read
it back. Residuals: no door authors the integer `7` through the value plane
(`create(props=…)`'s `PropValue::List` is the one typed arm), so a
def-declared `int`/`bool` property (`shape.rs` `SHAPE_INT`/`SHAPE_BOOL`) must
be born in the record's own body bytes; timestamps and dates are deliberately
left plain (below).

**The trigger list above is not closed.** The law is the bold sentence: the
last trigger asks `serde_yaml` whether the plain line reads back as exactly
the caller's string, and a door adds no trigger of its own.
The engine's classifier is more permissive than YAML: a value opening `- `,
`? `, `,`, `*`, `&`, `%`, `@`, `` ` ``, `]`, `}`, or carrying an unterminated
`[[a]] and [[b]]`, emitted plain, kills the whole frontmatter block for every
YAML parser; `!t`, `>` and `|` parse to something the caller never wrote. One
carve-out: a plain form that parses as a non-string is legal exactly when the
classifier reads it as a one-level flow list. Measured churn: 14 of 29 377
distinct plain-spelled values (0.048 %), all plain→quoted; a same-value
write-back stays byte-identical (§ A.6.3c).

**`serde_yaml` is not the whole oracle.** It resolves YAML **1.2**; PyYAML and
go-yaml (`gopkg.in/yaml.v3`, what Go hosts link) resolve **1.1**, so
`02146210` is the string `"02146210"` to `serde_yaml` and the integer 576 648
to PyYAML. The law is the **union** of the schemas; the 1.1 classes are the
trigger above. Measured churn for the union: 810 of 29 270 distinct
plain-spelled values (2.767 %) — 515 ints, 245 floats, 37 all-digit short
ids, 7 booleans, 1 sexagesimal, 1 interior tab, 4 radix / resolver-tag — all
plain→quoted, none refused, every one read back by PyYAML as the caller's
string. Measured, the rule adds zero no-op re-spelling: the same 90 files
move and 14 refuse with and without it, all under the § A.6.3c exclusions
(79 bare-key `null` → `""`, the rest stored block-scalar markers and `[[…]]`
nesting).

**The line is value corruption, not retyping.** Each reader run as a library
into an untyped target (`serde_yaml::Value`, PyYAML `safe_load`,
`gopkg.in/yaml.v3` into `interface{}`):

| plain value | serde_yaml (1.2) | PyYAML (1.1) | go-yaml `yaml.v3` (1.1) |
|---|---|---|---|
| `owner: 19895504` | int | int | int |
| `session: 02146210` | string `"02146210"` | **576648** (octal) | **576648** (octal) |
| `created: 2026-08-23` | string | `date` object | `time.Time` |
| `stamp: 2026-08-23T02:09:32-04:00` | string | `datetime` object | `time.Time` |
| `session: "02146210"` (the emit) | string | string | string |

- **Ids: closed.** Both 1.1 readers agree on **576648** while `serde_yaml`
  alone still sees the string: the join key is destroyed. § A.6.3c keeps the
  37 ids already on disk byte-stable until a write changes their value.
- **Dates: left plain.** Every reader agrees on the instant; only the carrier
  differs (text, or a `date` / `time.Time` object). Quoting the class would
  raise churn from 2.767 % to ~25 % of the plain population (+7 356 distinct
  spellings under `created`, `created_at`, `updated_at`) and un-type the
  `date` property Obsidian views sort and filter on.
- **Exposure: a date-object writer**, which reads a plain date into a
  `date` / `time.Time` value and re-emits it in its own formatting. None of
  the surveyed writers does: this engine through every § A.6.3a door
  (`serde_yaml`: 1.2 core schema, no timestamp type), a Go host (`yaml.v3`
  into typed `string` struct fields returns the source text,
  `created="2026-08-23"`; plus its own line-level field setter) and the armed
  rules, which write through the engine. Safety comes from the target type,
  so **a writer that unmarshals frontmatter into `interface{}` /
  `map[string]any` and re-emits would rewrite every date in the corpus.**
  **Obsidian's property editor also writes frontmatter and was not
  measured.** Measure a reader against the library a program links, not a
  CLI that wraps it.
- **go-yaml's 1.1 resolution is partial.** It resolves the integer classes
  and dates but keeps **word-bools** (`yes` / `no` / `on` / `off`) and
  **sexagesimals** (`12:30`) as strings where PyYAML resolves them. No
  predicate changes: the writer quotes the union.
- The **`yq` CLI** (mikefarah v4.53.3) answers `2146210` for the same line, a
  third number and neither library's.

**A.6.3′ The key half of the composed line.** An unvalidated key forges
frontmatter as an unvalidated value does. **A property key is dotted
segments of `[A-Za-z0-9_-]+`** — the flat dotted spelling run-plane.md
mandates for the task grammar (`task.index.caps`) and the birth door writes; a
patch face refusing `task.index` on the same page would be three surfaces
under two laws. A dot separates and is never a segment byte: `.`, a leading or
trailing dot, `..`, and any byte outside the segment charset are refused. The
owner is `policy::defs::yaml_safe_key`; `SafeKey` has no other constructor, so
a call site that does not discharge the `Result` does not compile. Both write
doors (the rebuild committer and the wire splice face) speak one refusal,
minted at `policy::defs::invalid_property_key_refusal`. The dot adds no
forgery surface (no `: `, no newline, no `---`).

**A.6.3a The write doors this encoder owns.** Five doors write a frontmatter
value, and all five encode:

| Door | Path |
|---|---|
| `set_property` (and the `check_write` candidate sharing its owner) | the splice plan lowering |
| `put{at:"upsert"}` on an `fm_key` target | the native wire write door |
| the preset birth door — a `^template` placeholder standing in a frontmatter value position | `preset::new_record` / `unfold` (run-plane Law 3.6) |
| `put{at:"end"}` on an `fm_key` target | the native wire write door, composed at the door |
| `match` on an `fm_key` target | the native wire write door, composed at the door |

The upsert door is a value-plane door: its `text` is a caller's flat string,
never a YAML node. **All five doors refuse a multi-line value** with the
encoder's `MultiLineValue` refusal; a newline is refused, never sanitized.

**Why the birth door is a value door.** `mrd new --actor $'me\nstatus: closed'`
against a template carrying `owner: {{actor}}` once interpolated the caller's
source bytes, so the born record carried `status:` twice; § A.3's props plane
and `fm_key` address the first occurrence, so disk said `closed` while every
read door served `open`, and no governed edit could reach the shadow line.
Run-plane Law 3.4 stamps `actor`/`now` exactly as given, so
sanitizing is forbidden; § A.6.3 refuses a multi-line value. Together:
**encode what is representable, refuse what is not, alter nothing.** A
multi-line `--actor` refuses the birth (`bad_request` / `fix`, the uniform
sentence plus the placeholder that carried the newline); a representable
value (`me: closed`, `[[b1892b5a]]`) is born quoted and decodes back exactly,
a quote trigger landing the canonical spelling (`owner: "[[b1892b5a]]"` where
the pin wrote `owner: [[b1892b5a]]`) through the shared encoder.

**The rule that decides every scope reaching an `fm_key`.** Without it
`at:"end"` and `match` write raw: `owner: seedhand: x` at exit 0, and
`hand #c` committing with the comment silently dropped. The line:

> **A caller-facing value scope on an `fm_key` target is value-grain — the
> engine owes the encode. An engine-internal lowering slot is line-grain — it
> carries a pre-composed line and stays raw.**

`at:"end"` and `match` are caller-facing: the input is a fragment of a value,
so the door composes stored + fragment, encodes the whole result, and lowers
it to a `put{at:"all"}` span replace (encoding only the fragment would yield
`owner: seed"hand: x"`). `at:"all"` and `at:"content"` are the lowering's own
slots — A.6.3a′ lowers `set_property` through `at:"all"` with an
already-encoded line — so they stay raw, and the kernel with them. The receipt
renders the caller's edit (`put:end`, `match`), not the lowering; armed facts
state true before/after revs regardless.

**Uniform means the words too.** Both doors carry the same refusal sentence:
the key by name, the v1 single-line rule, and the body-section escape
(*"frontmatter values are single-line in v1; put multi-line content in a body
section"*).

**A.6.3a′ One armed fact per key — the `set_property` create arm is the upsert
door.** The plan lowering emits one edit per key, each targeting its own
`fm_key`: an existing key as `put{at:"all"}` over its line, an **absent key as
`put{at:"upsert"}`**, the only shape that addresses a key the document does
not carry yet. It is the same door row above and encodes there;
`set_property`'s own multi-line refusal fires first, in the door's words.

The grain is law: armed facts carry op, target identities and rev transitions
(§6.1), a node entry names the deepest node containing each changed byte
range (§7.1), and facts are the normative receipt content (§6.4). Folding
every create onto the last existing key with `put{at:"end"}` — a batch
setting `owner` and `status` over frontmatter holding `title`, receipting
`title put:end <rev>-><same rev>` — states a wrong identity, op, transition
and count; a §11 lint asserting receipts against intents would false-negative.

- A created key lands at the upsert door's insertion point (first-key
  position), not after the last key. Key order inside the block is not a law
  of this contract; the auditable identity of the write is.
- The create arm's `node_rev_before` is `blake3("")[:16]`, the empty-input
  hash `af1349b9f5f9a1a6`, not a claim that an empty key existed. A.6.5 keeps
  absent and empty apart; armed facts do not, so a consumer reads the op
  (`put{at:"upsert"}` is the create arm), not the token. The same arm births
  a missing frontmatter block when the document carries none.
- The kernel below the doors stays raw-grain: `model::plan_fm_upsert`
  composes the value verbatim, because the run plane's `md.set_field` writes
  whole-value grains through it and their spelling must land as sent.

**A.6.3b The splice consumer reads the encoded value.** The def-plane
`rebuild` path (and the `check_write` candidate sharing its owner) splices the
value span of an existing key instead of composing a whole line. Its
separator guard — which inserts the space in `{key}: {value}` over a stored
bare `key:` line — must test the encoded bytes, not the caller's string: the
empty string encodes to `""`, and a guard on the caller's value emits
`note:""`, one malformed line that voids the whole block for yaml.v3, PyYAML
and Obsidian alike. The wire's
own `set_property` lowering composes the full line and never reaches this
guard, so coverage belongs at the `rebuild` door, mutation-proven there.

A create has one line shape, `{key}: {encoded}\n`. A bare `{key}:\n` for an
empty value would forge a YAML null, the type A.6.3 says this plane cannot
express; the encoder never returns empty bytes, so no special case is needed.

**A.6.3c Spelling preservation on a semantic no-op.** An update keeps the
stored value bytes verbatim when `decode(stored)` (§ A.6.1) equals the
caller's value and the stored spelling classifies as neither `Nested` nor
`Null`. A read-modify-write of an untouched value is therefore byte-stable —
`owner: "3f9a1c07"` reads as `3f9a1c07` and writes back as
`owner: "3f9a1c07"` — so `prop_rev`, `span`, the `props1` fingerprint and any
pin held over the key survive, and a host's quoted spelling and the engine's
plain emit are each fixed points under the other's write-back. One owner
implements the predicate, beside the encoder; every § A.6.3a door consults it
on update (the value-span splice keeps the span bytes, the line-composing
doors keep the value spelling inside the one `{key}: {spelling}` line shape).

Excluded, because a standing law outranks byte quiet:

- **A stored NULL spelling** (bare `key:`, `~`, `null`) re-encodes to the
  quoted string even when text-equal: R4 demands that the write of a string
  land a string, distinguishable from the null it replaces (§ A.6.3).
- **A stored nested spelling** is repaired to the quoted canonical form
  instead of standing under an `ok` write.
- **A multi-line caller value** is refused (D11) before preservation is
  consulted, so a stored escape spelling cannot smuggle a newline past the
  uniform refusal.

Preservation covers the value spelling, never the line geometry: a doubled
separator space normalizes once at a line-composing door and is byte-stable
after that.

**A.6.4 What conformance means here.** Round-trip is the test, per direction
and composed: a canonical quoted value reads back without its quote bytes, and
a `set_property` of an `[[id]]`-shaped value lands quoted and reads back as
the caller's string. A quote-tolerant comparison anywhere (host, caller, or a
second engine seam) is a defect against this law.

**A write-back that changes the value may re-spell; a semantic no-op may not**
(§ A.6.3c). Decode and encode are inverses on the value, not on the bytes: a
write that lands a different value emits the encoder's spelling, and
`prop_rev`, `span`, the `props1` fingerprint and any pin held over the key
move with it. A caller comparing across a read-modify-write
compares values, never tokens: the three exclusions re-spell, and a value
change never promises byte geometry.

**Round-trip alone is not the test.** A conformance test asserts the stored
line shape byte for byte, and only then the round trip. The engine's own
decode is tolerant by design, so a value-only assertion passes over bytes no
external parser accepts: `note:""` round-trips through this engine and voids
the frontmatter block for everyone else. The bytes are the contract.

**A.6.5 R4 binds the def plane too — the empty string is empty.** A.6.3 makes
every value-plane write door emit `key: ""` for an empty value, and the def
plane reads the typed frontmatter value, where that is a string, not the YAML
null. An emptiness predicate written against the null alone therefore reads a
released card as still set: `set_property(owner, "")` on a card whose def
marks `owner` required returned `ok` where the bare-null spelling refused,
and `closed_at: ""` satisfied the terminal biconditional, so a terminal card
carried no close time and the close-stamp autofill minted no repair.

The ruling: **a key is empty when it is absent, when it is the null, or when
it is the empty string.** This matches the refusal text (*"missing or
empty"*) and keeps R4's three states (absent ≠ empty ≠ set) readable at the
def grain. Emitting a bare null instead was rejected: it would forge the one
type this string plane cannot express (A.6.3) and reinstate the A.6.3b splice
geometry. Not empty: whitespace, `0`, `false`, an empty list. The predicate
is about absence, never truthiness.

**A.6.5a Every value-plane door sets; none of them removes.** `set_property`,
`put{at:"upsert"}`, `put{at:"end"}` and `match` on an `fm_key`, and the preset
birth door all compose `{key}: {encoded}`, and the encoder never returns empty
bytes (A.6.3b), so no value a caller can send makes the key line go away.
`properties {"k": ""}` and `put{at:"upsert"}` with `text:""` land `k: ""` — a
present key holding the empty string, R4's empty, which A.6.5 rules distinct
from absent. **Removal is § A.6.6's `remove` shape and nothing else.**

### A.6.6 `remove` — the identity-plane door on `fm_key`

**The gap.** R4 ratifies three frontmatter states, `absent ≠ empty ≠ set`; the
read face, `fingerprint.rs` (`=A` / `=N` / `=S`) and the def plane (A.6.5)
discriminate all three. At key grain the write plane reached `Scalar` through
any value-plane door but `Absent` only by rewriting the whole document, which
names no key. With no shape for "strip key K", callers emptied instead:
**emptying is what stripping degrades into.** One live corpus's `frontmatter`
projection held **5,700+** rows trimming to ``, `""` or `''` (`manifest`
2,222, `status` 1,957, `owner` 768) — an upper bound, since a template may
birth a key blank, but a count no absence test reads correctly.

**The parent slot is reachable.** §4.4's `target_identity` remedy retires a
node through its parent's content slot, and a key's parent is the document:
`{"hpath":[]}` (zero segments) is the root, and `put{at:"all"}` there, guarded
by the document's `file_rev` as `if_node_rev`, rewrites the whole file and
commits with the key gone. A section-grain `at:"all"` with a shortened block
refuses `transition_unrepresentable`, since the frontmatter lies outside the
section's span. A delete was unnameable, not impossible:

| What the root slot makes the caller do | Cost |
|---|---|
| resend the whole document to strike one line | every concurrent edit to any other node is silently reverted: whole-file last-writer-wins |
| guard at root grain | the key's `prop_rev` cannot guard the write that removes it; two agents stripping different keys from one record clobber each other and nothing refuses, since each moved the root truthfully |
| name no key | the armed fact reads `target:{"hpath":[]}` with the file's rev transition; nothing says `hooks` was removed |
| include-by-omission | no key to typo, so a mis-transcribed block cannot be refused; removal is a side effect of bytes left out |

The third row is the defect A.6.3a′ fixed on the create side: armed facts
carry op, target identities and rev transitions (§6.1), so a fact must name
the key that moved. `remove` gives the retire side that per-key identity; it
is not a new capability.

- **A `remove` on a `hpath` or `anchor` target refuses `bad_request`**, saying
  why: there a section retires through its parent's content slot naming that
  parent and an anchor through its enclosing section, so `remove` would be a
  second spelling of a capability that already carries its own identity.
- `op:"remove"` is the page-grain death of a whole document, a different
  object: a delta reports a key removal as a `removed` node entry, a page
  removal as a deleted file with no node entries.
- **Not a value-plane door.** § A.6.3a's five doors write a value and belong
  to the encoder; `remove` carries no `text`, so the multi-line refusal has
  nothing to refuse. The value plane answers *what does this key hold*,
  `remove` *is this key here*.
- **The replaced region is the grain span plus its line terminator**, since a
  leaf's span excludes its terminator (§1) and `fm_key`'s grain span covers
  the key line and every indented continuation line of a block value (§4.4);
  striking the span alone would leave a blank line in the block. It is the one
  shape whose region is wider than its target's span, and §4.4's disjointness
  runs over that region: a `remove` and an `upsert` naming one key overlap and
  refuse.
- **An absent key refuses `ref_not_found`; removal is not idempotent, on
  purpose.** Success-on-absent would return one frame for a typo'd key and a
  finished job. A caller sweeping N records reads first (revs and rows are
  ambient after any read) and removes only what it saw, as `rm` does for a
  missing page.
- **Removing the last key carries the fences.** `---\n---\n` is not
  frontmatter: `syntax::parse` mints a `Frontmatter` node only from a pulldown
  `MetadataBlock` event, and an empty block raises none. Measured:
  `at:"upsert"` on a file opening `---\n---\n` synthesized a second block at
  byte 0 above the bare fences (`plan_fm_upsert`'s blockless arm, run only
  when `find_frontmatter` returns `None`; an empty `props[]` cannot tell an
  empty block from none). Bare `---` lines are ordinary markdown that the next
  property write corrupts, so a removal that empties the block takes the
  fences too and leaves a legitimately blockless document — the mirror of
  A.6.3a′'s upsert, which births the block for a document without one. The blank line after the block
  was never the block's and is not carried.
- A shadowed duplicate key is a survivor: `fm_key` addresses the first
  occurrence (§ A.6.3a), so striking it un-shadows the second. Two
  byte-identical shadow lines need the premise-exemption twice: striking the
  first leaves an identical-bytes node at the same address, which the
  write-past-its-span guard would otherwise read as "the rev did not move"
  and refuse.

⚠️ **Stated limit: the two doors disagree about a blockless document, and
`remove` makes that reachable.** The plan lane refuses to set a property on a
document with no frontmatter (*"cannot set a new property — the file has no
frontmatter to anchor it"*, a pinned rule); the native `at:"upsert"` door
synthesizes the block. A document could already be born blockless; `remove`
adds a second path: strip the last key, and the plan lane cannot add one back,
though `edits[]` can. Not ruled here: A.6.3a's *"Uniform means the words too"* names
the class (one law refused in two dialects is two laws) and the divergence
predates `remove`. A test pins it.

**Armed facts.** `node_rev_before` is the key line's real pre-batch rev, so
`if_node_rev` guards a removal as it guards a set; `node_rev_after` is
`blake3("")[:16]`, the no-node token A.6.3a′ arms on the create arm;
`span_after` is the zero-width post-batch point the line vacated. The receipt
renders the caller's shape as `remove`. §4.4's `target_identity`
premise-exemption note says why this door does not draw that refusal.

**Delta.** No new case: `collect_removed`'s frontmatter arm mints a `removed`
node entry for a key that resolved before and not after — `node_rev_before`
set, both after-facts `null`, only on a proven `NotFound` (an `Ambiguous` is
dropped, not fabricated). A feed reader sees
`{"fm_key":"K","change":"removed","node_rev_before":"…"}`.

**Plan lane.** `plan_edits[]` carries `{"remove_property":{"key":…,"rev"?:…}}`
beside `{"set_property":{…}}`, the door of the `properties` map, `mrd script`'s
`put(props=)` and every host face. It is separate because that map is
`{key: string}` at every face (Starlark: `BTreeMap<String, String>`); removal
inside it would need a sentinel, and a sentinel for absence is the confusion
`remove` ends. A removal is named, or it is not expressed.

**Discovery.** v3 sessions carry `splice.remove` in `caps` (§3.2's dotted
`op.field` convention). No negotiation is needed: an engine without the shape
refuses it by name at §3.2's strict wall (*"unknown field `remove` in
`edit`"*). `scoped-guards` needed negotiation because it changes the meaning
of a field a client already sends.

### A.7 `script` — in-process script submission

*The run plane's script entry (`run-plane.md` § The script entry) rides the
wire: one request carries the whole program, the daemon evaluates it in-process
(Starlark, never a subprocess per call), and the body is the trace. The entry's
own semantics stay normative in `run-plane.md`.*

**Request** — the entry's inputs as one frame on the bound workspace (§3.2's
binding guard applies):

```json
{"id":7,"op":"script",
 "source":"card = read(\"notes/plan.md\")\n",
 "args":{"page":"notes/plan.md"},
 "files":["notes/plan.md"],
 "actor":"agent:b0864fb2","now":"2026-07-18T20:31:04Z",
 "receipt":{"path":"receipts/2026-07-18.md","anchor":"r-000099"},
 "dry":false,
 "if_fingerprint":"b3:…",
 "expect_armed":"armed-set-path-edit:sha256:…"}
```

- `source` required, all other fields optional, as at the CLI entry; strict
  decode at every grain (§3.2).
- `args`: the inert dict (string keys, string values).
- `files[]`: paths only — the wire serves no corpus-enumeration op, so
  enumeration stays the host's.
- **Call-order binding:** `files[i]` is the i-th path the caller named. One
  `{kind:"bound", index, path}` trace row per member.
- **Patterns in `files[]`:** a member containing `*` is a pattern in the one
  scope glob grammar (`**` spans whole segments, `*` a non-`/` run within one,
  everything else literal); other members are literal.
  - Expanded at entry against the entry world's hash-domain membership (the
    entry-fingerprint walk): deterministic within the attempt.
  - Expands **in place** to its sorted matches, never a global sort; a path
    bound earlier is dropped, first occurrence wins.
  - One `{kind:"expanded", pattern, matched:[…]}` trace row per pattern;
    replay replays the recording.
  - Zero matches contributes zero paths — data, not a refusal.
  - A pattern never names an out-of-domain path; a literal still may (§12.1).
  - `mrd script --files` forwards patterns through this op: one expansion
    semantics, never a CLI-private glob.
- **Literals first:** a pattern member before a literal member is refused at
  entry, dry and armed alike — zero evaluation, nothing armed, workspace
  unchanged (`files_member_order`, recovery `fix`). A literal's index is then
  its own member ordinal; inside the pattern region order stays the host's, and
  an over-long index there is an out-of-range fault, never a write to an
  unnamed document. All-literal and all-pattern lists are unchanged.
- `actor`/`now`: per §9, threaded to the commit splice verbatim; absent stays
  absent.
- `dry`, `if_fingerprint`, `expect_armed`, `receipt`: the CLI entry's
  semantics — rehearsal; pre-eval fast-fail plus §5.1 commit authority; the
  pre-splice armed-set gate; the §6 receipt address. Arm→commit (`run-plane.md`
  § the execution-model seam) is two ordinary calls: `dry`, then
  `if_fingerprint` + `expect_armed`.
- No budgets field; limits are the daemon's own (`EvalLimits` + the wall
  clock). An override would arrive as a dotted `script.<field>` cap through
  §3.2's evolution law.

**Malformed pin.** Per §5.7, an `if_fingerprint` that is not a grammatical
`Root`-family token (merkle-spec §4.2) refuses as a `refused` trace **before
any compare** — recovery `fix`, engine-minted fault triple, no §8 code — never
`conflict`. The text debug-quotes the raw bytes (§8.2); `guard_expected` stays
absent. The reserved `absent` (§5.6) is legal in `guards[]`, never as the entry
pin, which spells an engine-minted token. A grammatical retired or future
token is never malformed (§12.3). Both lanes refuse identically: the wall is
value grammar, not a permission plane.

**Response** — `ok:true` with the run-plane `ScriptTrace` as the body,
verbatim, whenever the entry ran. A fault, a refusal and a conflict are traces
in the trace's own closed vocabulary (`committed | no_effect | conflict |
fault | refused`), never §8 frames, which answer only what never reached the
entry:

| Frame | When |
|---|---|
| `bad_request` | strict-decode failure, no workspace bound |
| `unknown_op` | a v2 session — the op is v3-only at dispatch |
| `io_error` / env class | the entry pass itself failed before an entry fingerprint existed |
| `corpus_warming` / retry class | the workspace is cold: the entry pass would be the whole-corpus build — it rebuilds in the background and the entry never began (§3.2) |

An `ok:false` frame means the entry never began: nothing armed, no splice,
workspace unchanged. Once a trace answers, every claim in it is the engine's
own. §8.1 binds this op's clients unchanged.

**Dispatch:** v3-only; op-grain cap `script` (the `create`/`mounts` precedent —
no dotted fields at birth). A v2 session answers `unknown_op`; the frozen v2
caps stay byte-identical. The complete v3 push is §3.2's.

**The entry world** (this op's read law; detail in `run-plane.md`):

- The currency pass runs **once, at entry**: the daemon proves the corpus
  current — the corpus-grain proof every read op runs, Law A-3c's scope
  unchanged — and pins the entry fingerprint. Reads then serve that pin: zero
  wire trips, zero re-walks, no doc-grain narrowing.
- **Read-your-own-writes:** a read of an armed target serves the armed content
  (entry bytes plus the program's own edits, in arm order) and that content's
  own rev — §4.2, overlay included.
- **Foreign mid-program changes are invisible** inside the hash domain, the
  surface the entry pin covers: the program reads the pinned entry generation
  for the whole attempt (frozen view), disk moves only at commit, and churn
  outside the touch set (below) stays invisible mid-run and never refuses the
  commit. Every read in one attempt is thus consistent with one fingerprint.
- **Out-of-domain paths stay addressable and live** (§12.1: hash domain ⊂
  addressable domain, one answer at every door): a real file under the root
  that the domain does not hold serves from a single-file disk load, on this
  lane as on the wire-client lane. The stand-still guarantee does not cover it.

**Not the banned snapshot.** The entry world is attempt-scoped: born at entry,
dropped when the attempt answers, never retained across attempts, never shared
across connections, no version history, no as-of parameter, no MVCC. What
`run-plane.md` bans is daemon-held state across attempts; this is one attempt
reading the picture its own entry pass took. The commit's touch-set verify is
the only write authority.

**Containment (the eval boundary).** The kernel runs in the daemon under the
entry's own limits — fuel, memory cap, call depth, source bytes, the read and
armed-edit ceilings — plus a daemon-enforced wall clock checked at entry, at
every read builtin, and pre-commit. `catch_unwind` seals the boundary: a panic
answers a `fault` trace and the daemon serves its next frame. No workspace lock
is held during evaluation (entry pass and commit each take the serve path's own
locking), so a long evaluation never parks another connection's op.

**Zero delta everywhere else.** Every §4 op, every § A.3/§ A.5 addition, every
v2 byte: unchanged. The CLI subprocess entry (`mrd script`, wire-client mode)
stays functional and byte-compatible; its removal is a separate ruling. The
socket law (§ A.3) is untouched — same one door, same connect-time identity
comparison, CLI skew refusals unaffected. This lane's commit is issued
daemon-side, so it advances the delta ring like any wire splice: §18 row 12's
CLI-lane delta gap does not extend here.

**Corpus-grain at entry, not doc-grain per read.** Reads serve O(1) from the
pinned root; the non-script doors' refusal scope is untouched.

**Effects mode.** Two more request fields (the field wall grows 9 → 11):
`effects:[…]` and `invocation`. Absent `effects` = the pure script above,
byte-identical. Present = the **live program** model (`run-plane.md` § Effects
mode is normative):

- `read()` serves the live disk at call time.
- `put()` applies immediately through the wire splice door — write flock held,
  structural validation intact, the guard's own `force` bypass — no rev, no
  snapshot, no CAS.
- `run()` (admitted by naming it in the list) executes the § A.8 lane at call
  time and returns its row as a value — run-then-decide.

The principle: the rev leashes an agent's stale context, not writes, and
effects cannot be refused (out-of-world), so the transaction promise is
unkeepable there. Tradeoff on record: two effect-scripts can last-writer-wins
each other on one section; the flock keeps files structurally intact;
exclusivity belongs to the coordination layer (no `mutex()` builtin ships, by
ruling).

Combination walls, all `bad_request` at decode:

- `effects` beside `dry`, `if_fingerprint` or `expect_armed` — a live program
  cannot rehearse and holds no premise and no armed set;
- `effects: []` — name an effect builtin or omit the field;
- an unknown effect name — the closed set today is `run`, `token_count` (the
  effects registry lives here);
- `effects` without `invocation` (§9) — run identity is host-minted: per-call
  ids are `<invocation>-r<K>`, K the 0-based `run()` call ordinal.

**The `token_count` effect.** One more optional field (the field wall grows
11 → 12): `token_count_endpoint`, a unix-socket path. `effects:["token_count"]`
admits `token_count(text) -> int` — the text's real token cost, measured now.

- **One measurement law:** the argument string is measured verbatim (the tool
  face's `{text}` arm); the builtin resolves no refs and no sections, so the
  tool face's stored-vs-served split cannot enter it: a program measures what
  its own `read()` served, or what it built.
- The engine never counts tokens (no tokenizer, no credentials): the live host
  dials the endpoint per call as an NDJSON socket client, with the endpoint
  daemon's own `token_count` verb frame, identityless — the endpoint picks the
  instrument and answers for tokenizer provenance.
- Endpoint without the effect, or an explicit empty endpoint: `bad_request` at
  decode. Effect with no endpoint decodes; the builtin then faults "unbound" at
  call time — every lane without a harness (the pure/entry-world host, the CLI
  wire client).
- The endpoint's own refusal faults the program, its words carried whole. The
  dial deadline caps at the remaining wall clock.
- A measurement is not an act: no trace entry. A top-level
  `n = token_count(…)` rides the bindings echo like any computed name.

Response here: `outcome` is the new word `effects` when eval completed;
`trace[]` records the acts in call order — read entries as today, `wrote`
entries (a live `put()`: path, group facts, the splice's own after-facts),
`ran` entries (the § A.8 row, verbatim). A mid-program fault answers `fault`,
every prior act landed and recorded: no rollback. The vocabulary grows exactly
this one word; the pure path's five words and v2 are untouched. Live `put()`s
advance the delta ring through the wire choke-point like any splice; `run()`s
mint per committed batch through the run plane's delta sink (§ A.8).

**The commit premise is the touch set.** Commit authority is never a re-pinned
entry fingerprint: the engine computes the premise, the caller declares nothing.

- The pure lane records what it touches: `toc`/`cat` point reads, armed write
  targets, `files[]` literal and pattern expansions, sql reads. Leaf
  premises come from point reads and armed writes; set-premise folds from
  pattern and selector expansions; sql contributes the provenance regions it
  scanned (set-premise law: `node-rev-merkle-spec.md`). Commit verifies
  entry-vs-live at exactly those nodes — O(touch set), never O(corpus); churn
  outside it causes no retries. **Zero new caller fields on this door.**
- **A caller premise stays legal as widening** (strictest wins), never
  dropping write coverage: the touch-set floor always contains the armed
  writes. `if_fingerprint` (+ optional `scope`) and `guards[]` ride this op
  with §5.4's meaning under the `scoped-guards` cap; the field wall grows
  12 → 14 (`guards`, `scope`). `effects` excludes `guards`/`scope` as it
  excludes `if_fingerprint`.
- **Read visibility is frozen view** (above).
- `expect_armed` stays, orthogonal: it gates set identity (the host authorized
  *this* set), the touch-set verify set freshness (the world did not move).
- `dry` is untouched byte-for-byte; the dry trace also prints the recorded
  premise set.
- Retry budget: unchanged host policy, now spent only on genuine same-subtree
  contention.
- **Host requiredness:** no host-policy ratchet need make callers hand-copy a
  fingerprint token onto script doors; the touch-set premise guards what the
  script touched. A caller-passed token stays legal as a widening guard, and
  the engine mechanism is identical either way.

### A.8 `run` — page-task execution over the wire

*The run plane's task entry (`mrd run`, `run-plane.md`) rides the wire as a
list of targets, callable from `script()`. It is also the production arming
door: arming has no surface of its own — an activation task is a task, its
receipt the arming record. Plane semantics stay normative in `run-plane.md`.*

**Request** — a list of targets on the bound workspace (§3.2's binding guard
applies):

```json
{"id":9,"op":"run",
 "targets":[
  {"page":"rules/escalate.md","task":"arm","args":["--scope","team"],
   "env":{"HOME_WIKI":"/w"},"dry":false},
  {"page":"notes/plan.md"}],
 "actor":"agent:b0864fb2","now":"2026-08-13T16:02:11Z",
 "invocation":"run-1755100931421-4417-3"}
```

- `targets[]` required, 1..=64 entries. Per target: `page` required,
  workspace-relative; `task` optional (the plane's single-task default and
  several-tasks listing apply); `args[]` strings and `env{}` string→string
  optional, contract-validated; `dry` optional. Strict decode at every
  grain (§3.2's wall).
- `invocation` required: the host-minted, path-safe identity base; per-target
  ids are `<invocation>-t<index>`, zero-based position. The engine mints no
  identity (§9).
- `actor`/`now` per §9, optional; absent stays absent. A supplied actor
  threads into the run receipt's `actor` fact.
- `fields{}` string→string, optional (cap `run.fields`): the § A.2.1 opaque
  passthrough, verbatim as middleware `ctx.fields` on every `md.create` birth
  this run commits — the stamped lane for born-identity
  (`created`/`session`/`spawned-by`). The engine interprets no key. A host
  sends it only when hello advertises the cap; an older engine refuses it.
- `ambient` string, optional (cap `run.ambient`): the caller's ambient
  directory, workspace-relative, path-law-validated at the strict wall — a
  confined dir path, never a `root:` ref, never absolute. A bare `md.create`
  path born this run resolves under it. Precedence: descriptor `base` (a
  rooted ref or a confined dir) > frame `ambient` > workspace root; no
  `ambient` = the bare-door law, workspace-root-relative. Hosts resolve
  `ambient` per call from the caller's own identity, never from a
  page-hardcoded directory. The birth lane is
  the starlark kernel's `create(path=, body=, base=, message=, props=)`; bash
  has no effect channel.
  - **`props=` is the newborn's frontmatter as a dict — string keys to strings
    or lists of strings — and the door serializes it**: key grammar, value
    quoting and the one-line flow spelling of a list are the door's, on
    § A.6.3's encoders.
  - Keys land sorted. A props scalar that would read back as a collection,
    number or bool is quoted (`"7"` lands `"7"` — no typed-scalar carve-out at
    any door, § A.6.3). Two `bad_request` refusals, nothing landed: a newline
    in a value (D11, the § A.6.3a law verbatim); a `body` that already opens
    its own frontmatter fence while `props` is inhabited.
  - **The one deliberate asymmetry with the patch face** (§ A.6.3): no
    flow-list carve-out here, since this door has a typed list arm. `[a, b]`
    lands quoted through `props=`, plain through `properties`.
  - A rooted `base` must name the bound workspace (a foreign root refuses); a
    rooted spelling in `path` refuses, naming `base`. The wire `create` op
    carries **no** `props` field — there `body` is the whole document,
    frontmatter included. The door and the receipt judge the
    resolved landing, the capability glob the declared relative `path`, so
    `md.create:tasks/*.md` covers ambient, based and root boards alike.
- **Named absences.** No `receipt` field — run receipts are the plane's own,
  engine-appended under the per-target invocation anchor on both doors. No
  capability, timeout or code field — authority comes from the page plus
  declaring-root conventions, the timeout from the declaring root's config,
  and only corpus-declared task blocks run; the wire carries names, never
  code.

**The modes — `mode: load|fire` (caps `run.mode`, `run.input`).** One op, two
modes, one law:

> **`run` executes what the page declares: `task.<name>` in frontmatter or
> `declare()` in the block, never an undeclared block.**

- A `declare()` block is equally declared; a bare anchored fence with neither
  declaration is not a target, and a fire naming one refuses `not_declared`
  at the door.
- **`load`** evaluates a page's starlark blocks' top levels in a pure
  environment and answers each block's declarations. **`fire`** calls one
  declared block's frozen entry with a JSON `input` and answers its return as
  JSON plus the md effects it applied through the ordinary doors.
- **One addition at top level, six per target** — each optional, cap-gated,
  appended to the closed sets: a client that skipped negotiation is refused by
  name (`` unknown field `mode` on `targets[0]` of `run` ``), never silently
  downgraded:

  | Addition | Where | Cap | Carries |
  |---|---|---|---|
  | `prelude` string | top level, one per call | `run.mode` | load-phase source evaluated before each block's own top level; blake3-cached. A faulting source, **or one carrying consent material — a declaration or an `exec` value** — refuses `prelude_invalid` at the mode door, before any block loads, load and fire alike |
  | `mode` `"load"\|"fire"` | per target | `run.mode` | absent = the shipped task path |
  | `block` string | per target | `run.mode` | the `^id` anchor of a declared block (§2.4's charset); required with `mode:"fire"`, refused with `task` |
  | `input` any JSON | per target | `run.input` | the fire's one input channel, bound as a real starlark value (dict/list/str/int/float/bool/None) |
  | `timeout_ms` u64, `budget` `{steps,mem}` | per target | `run.mode` | caller ceilings: limit = min(declared, ceiling) |
  | `source` string | per target | `run.mode` | a draft's page bytes instead of `page`; **forces `dry`** |

- **Exclusion rules, each refusing `bad_request` by name at the strict wall:**
  `task` with `mode`; `args` on a mode-bearing target (a fire takes `input`);
  `env` on an evaluated-entry fire (an exec'd entry's fire takes `env` as its
  process overlay); `block`, `input` or `task` on `mode:"load"`;
  `timeout_ms`/`budget` on a task target;
  `block` absent on `mode:"fire"`. `source` replaces `page`, required on
  every other target.
- **Mixed batches are legal**: rows are independent and answered in request
  order, so one call may mix a task and a fire target.
  **Recording follows the declaration kind, never a caller switch.** A
  `task.<name>` row is a task run: receipts under `<invocation>-t<index>`,
  plus the plane's lock. A `declare()` row is a fire: **no receipt row, no
  task-path lock** (the applier's workspace lock still applies when the fire
  realizes md effects — run-plane.md § Recording by declaration kind).
- On a fire, `invocation` stays required but only labels: no receipt row
  exists, so no receipt anchor is minted; it names the exec log, and
  collisions are harmless. A host deriving one from a non-path-safe id maps
  every other byte to `.` itself; the engine validates, never rewrites.
- **`fields{}` extends its reach on a mode-bearing target** (widening
  § A.2.1's birth-lane scope): a fire's splice-door writes
  (`set_field`/`append_section`) carry `ctx.fields` too, not only its
  `md.create` births.
- **Response — `body.targets[]` gains two row kinds** beside the task row. A
  **fire row**:

  ```json
  {"result":"ok|fault|timeout|refused",
   "page":"HOOKS.md","block":"no-stash","rev":{"file":"5c7347b8…","block":"75692e87…"},
   "value":{"deny":"…"},
   "applied":[{"kind":"md.create","path":"…","result":"born|edited|refused|not_applied","file_rev":"…","class":"…","reason":"…"}],
   "exec":[{"block":"check","command":"…","exit":1,"stdout_sha256":"…","bytes":412,
            "log":".meridian/runs/…-t0.log","timed_out":false,"dry":false}],
   "process":{"interpreter":"bash","exit":0,"stdout_tail":"…","stderr_tail":"…",
              "stdout_bytes":10000,"stderr_bytes":0,"timed_out":false,
              "log":".meridian/runs/HOOKS.md/…-t0.log"},
   "fault":{"class":"parse|name_error|effect_at_load|declare_at_fire|declared_twice|impl_type|budget|reply_shape|runtime|no_block|not_declared|ambiguous_anchor|not_a_module|missing_entry|prelude_invalid|bad_path|corpus_race","reason":"…","line":7},
   "telemetry":{"steps":812,"mem":20480,"wall_ms":3}}
  ```

  `applied[]` words: `born` a birth, `edited` an edit, `refused` the one
  descriptor a door judged (with its `class`), `not_applied` its siblings —
  positionally, on the refusal's descriptor index. Births realize sequentially
  before the atomic page splice, so a create before the refused index reads
  `born` (it is on disk), one after it `not_applied`, every edit
  `not_applied`. No `exists` arm — an occupied path refuses at the create
  door. `value` is the program's return verbatim, absent for an exec'd entry,
  whose `process` carries the interpreter, raw exit code (1 and 2 distinct)
  and tails. `rev` is provenance: the page's `file_rev` and the block's `rev`.
  A `bash()` that ran before a fault stays in `exec[]`. The `fault` union is
  what the engine can emit — `runtime` the production catch-all,
  `corpus_race` the warm→pin race, `bad_path` the create door's (§ A.3),
  `impl_type` typed — and a `fault` carries no `applied` (all-or-nothing per
  row).

  **A door refusal is that effect's row and never the fire row's**: the effect
  row carries `result:"refused"` and the door's reason; the fire row keeps
  `result:"ok"` and its `value`, the never-veto law. Only a failure to carry
  the batch at all (lock, I/O, page load, a non-md or malformed descriptor)
  refuses the row. `result` on a fire row is an evaluation word, not a state
  word.

  **The tails are tails**: `stdout_tail`/`stderr_tail` carry the last **4096
  bytes** of each stream, `stdout_bytes`/`stderr_bytes` the true sizes, and
  `log` the out-of-tree file with all of it —
  `.meridian/runs/<page-path>/<invocation>-t<index>.log`, retained per page,
  absent under `dry`. An exec'd entry's program is a staged file run as
  `<interpreter> <file> <args…>`, never `-c`.

  **`declarations` is one dict or `null`, never `{}`** — what `declare()`
  collected, verbatim and uninterpreted. A block declaring nothing publishes
  `null`; `{}` is a `declare()` with no keys; the consumer law is presence (a
  non-null dict arms the block).

  A **load row**: `{page, rev:{file}, loaded:[{block, rev, result,
  declarations, entry_kind, fault?}]}`.
- **What the modes do not gain:** no `rev` to attest, no vocabulary list, no
  event shape, no armed check, and no `entry` parameter — the block
  self-describes.

**Execution.** Targets run sequentially in list order, each an independent
invocation: its own `run.lock` window, receipt and row. **Each target answers
for itself; no target's outcome halts, gates, or colors another's.**
Dependent sequencing composes runs inside `script()`.

**Response** — `ok:true` with per-target rows in request order whenever the
op reached the plane; no aggregate boolean exists in the body:

```json
{"id":9,"ok":true,"body":{"targets":[
 {"page":"rules/escalate.md","invocation":"run-1755100931421-4417-3-t0",
  "receipt":"receipts/run.md §^r-run-1755100931421-4417-3-t0",
  "dry":false, "task":"arm", "task_rev":"…", "guarantee":"hermetic",
  "state":"applied", "applied":[{"kind":"md.set_field","domain":"…"}],
  "unexecuted":[], "caps":{"effective":["md.edit:**/rules/*.md"],
  "source":"explicit","narrowed":[]}, "cap_reached":false,
  "out_of_band_delta":false},
 {"page":"notes/plan.md","invocation":"run-1755100931421-4417-3-t1",
  "refusal":{"class":"invocation","reason":"several tasks declared — name one",
   "declared_tasks":["check-links","fix-drift"]}}]}}
```

- A carried target answers the plane's report object verbatim
  (`run-plane.md` § the report) plus addressing (`page`, `invocation`,
  `receipt`, `dry`): `caps` absent on an unsandboxed row, `exec` facts and
  bash stdout as the plane's bounded record — this op streams nothing. The
  echoed `page`, on rows and in the receipt, is the resolved
  workspace-relative spelling (§2.1's grammar), never the request's bytes.
- A target the plane refused before a report existed answers a `refusal` row:
  `class:"invocation"` for the CLI's exit-2 family (addressing, contract
  violation, authoring faults — `declared_tasks[]` rides the several-tasks
  listing); `class:"run"` for the exit-1 family (workspace busy, timeout —
  execution refusals only, no foreign-edit or root-mismatch legs); `reason`
  verbatim from the plane's typed error.
- `dry` rows carry the plane's dry legs unchanged: a starlark dry answers the
  full effect set with `applied:false`; a bash dry answers the block source
  with `executed:false` and `effects:"undeclared"`. A dry target rehearses
  every pre-apply gate the live one enforces — addressing, contract (arity,
  env declarations), capability admission — so a gate-refused rehearsal
  answers the live call's refusal row byte-identically (`runner::rehearse`).
  A contract fault never reaches eval.
- §8 `ok:false` frames answer only what never reached the plane:
  `bad_request` (strict-decode failure, empty or oversize `targets[]`, no
  workspace bound), `unknown_op` (v2 session).

**Dispatch:** v3-only; op-grain cap `run`, plus four dotted field caps per
§3.2's convention: `run.fields` and `run.ambient` in the base set,
**`run.mode`** and **`run.input`** in the v3 push. A v2 session answers
`unknown_op`; the frozen v2 caps stay byte-identical. A client that has not
seen `run.mode` in the hello does not send it.

**Containment (what this door inherits, all of it the plane's own).** The wire
arm drives the same runner seam as the CLI: deny-by-default capability
resolution on starlark, the bash-fence convention refusals, the declaring
root's configured timeout with process-group kill past deadline, and the
inherited task environment — `run` must not strip the daemon's environment;
the step inherits it, declared contract pairs shadow inherited values, and the
plane's own `MERIDIAN_PROJECT_ROOT` shadows everything. On this op the task
step's working directory is the bound workspace root. A long-running target
parks only its own connection (§ A.7's containment
posture); `run.lock` refusals answer as `class:"run"` rows, never hangs.

**Delta honesty (the § A.8 half of §18 row 12 is discharged).** Run applies
here mint Deltas like every other daemon-side write:

- The executor commits through `fs::apply_batch` (unchanged); at each
  committed batch the serve arm's delta sink assembles one frame at the §7.3
  single constructor and advances the workspace ring **under the workspace
  write flock, held as a bracket around the commit and the mint**
  (`write.lock`, not `run.lock`, which does not exclude the detector).
  Because the detector reconciles under the same flock, no detect cycle sees a
  half-landed commit or an un-advanced ring.
- One committed batch = one fingerprint advance = one Delta (§7.1), the content page
  and the receipt file as two entries of one frame's `files`.
- Identity is §9's: a supplied `actor` threads verbatim; absent, the frame
  carries the plane's own `run:<task>` self-label, so `actor`-absent still
  means exactly "edited outside the face". `now` is the caller's or absent,
  never invented.
- A mid-run fault mints frames for the batches that committed and none for
  what refused (no rollback).
- The CLI entry (`mrd run`) is a separate process with no ring in reach: its
  commits stay under §18 row 12 as CLI-lane `put` commits do.

**Zero delta everywhere else.** Every §4 op, § A.3/§ A.5/§ A.7 and every v2
byte: unchanged. The CLI entry stays byte-compatible — same runner, same
receipts, its own host-minted identity. The socket law (§ A.3) is untouched.

**No guard on this door.** `run` is not guarded: no CAS premise, no
fingerprint requiredness, no synthesized touch-set guard — on execution whose
consequences mrd cannot bound, a guard would promise what it cannot keep.
Consequences:

- **A supplied guard field is rejected as inapplicable, never ceremonially
  checked.** `if_fingerprint`, `guards`, `scope` — and `scope_bytes`, a
  top-level field on no door (§5.4's field matrix) — are not in this op's
  field set, so the §3.2 strict wall refuses them `bad_request` at decode
  (teaching: §8.2).
- **`task_rev` is targeting, never CAS.** It chooses what to execute — which
  task bytes the plane resolved — never a world premise; no refusal here is a
  premise refusal.
- **The `class:"run"` family has no premise legs**: no self-pinned corpus root
  (root mismatch), no per-target pin-and-verify (foreign edit). A foreign
  advance re-derives and proceeds; a vanished unrelated record never fails
  another target. Normative detail: `run-plane.md` § the no-guard amendment.
- **Guard-free never means fold-invisible.** Every landed run write rides the
  same write choke-point, advances the resident folds other writers' premises
  compare against, and mints Deltas as above. Tree maintenance, not a guard.

### A.9 Re-scope honesty on the delta plane

**The defect this closes.** A domain re-scope (the §12 config changed —
`meridian/domain.md` landed on a live root) floods the feed with one batch of
`deleted` rows, 1,010 in the measured case, every one false: the files stay on
disk, they left the attested set. Three additive rules:

**1. The `unattested` file-change word (v3-only).** A path in the previous
attested set, absent from the current one, whose file still exists on disk
(any filesystem object at that path — probed at classification, never
inferred), mints `change:"unattested"`: `file_rev_before` present when the
departed bytes still parse, `file_rev_after` absent, no node entries.
`deleted` now means only that the path is gone from disk. So a still-on-disk
path is never claimed by the `renamed` pairing, and a frozen v2 session gets
such a row demoted to `deleted`, keeping v2's birth vocabulary.

**2. The `rescope` batch summary.** When the effective domain configuration
changed between the detector's baselines — compared as parsed scope rules
(config identity + `Domain` semantics), so a prose-only edit to
`meridian/domain.md` re-scopes nothing — the frame carries a root-level
sibling of `delta`:

```json
{"delta":{…},"rescope":{"cause":"meridian/domain.md","unattested":1010,"attested":2}}
```

`cause` names the config file whose change re-scoped the set (on a switch,
the file now in effect; on a removal, the departed one). Under a `rescope`:

- membership-only changes collapse into the counts, never per-file rows:
  `unattested` = paths that left the set but stay on disk, `attested` = paths
  that entered it. Whether an entering path is also new on disk is unknowable
  at the set grain — re-read, never guess.
- disk-true and content rows still ride: the cause file's row **first in the
  batch** (law), genuine `deleted`, `modified` in full node grain, `renamed`
  pairs.
- disposition is `resync` (§8): re-derive what you watch, then continue from
  the cursor. Replay ≡ live holds — the ring stores the collapsed frame,
  `diff` replays it byte-identical (§7.3).

**3. The `overflow` marker — the feed bounds itself before any transport
does.** A frame's file enumeration is bounded at assembly (`MAX_DELTA_FILES`,
128). Rows past the bound — deterministic order: cause first, then
path-sorted — are dropped and counted:

```json
{"delta":{…},"overflow":{"dropped":988}}
```

In an `overflow` frame the enumeration is a sample, the count complete, the
recovery re-read. Both new fields are v3-additive (`rev::V2_RESERVED_FIELDS`
rows at the notification root, stripped for v2 typed); tolerant consumers
ignore unknown fields (§7.4's law), and one
meeting an unknown `change` word treats it as "membership or content moved —
re-read".

The consumer-side half (a drain face bounding rows per answer with
partial-cursor semantics) is the consumer's own contract; the rules above govern
what the engine emits.

### A.10 `walk` — pin graph

`walk` reads the pin graph around one page: read-only, per query, never
stored, citing the doc revs read (§2.4). Workspace-bound, unlike § A.5
`mounts`.

- **Direction**: up (default) = what the page draws from, transitively;
  `down: true` = who pins it (dependents listing, dry-run blast radius).
- **Request** `{path, down?, depth?}`: workspace-relative page, direction
  toggle, hop bound (`1` = direct edges).
- **Caps**: v3-only at dispatch; cap `walk` at op grain, never dotted
  `walk.<field>`; v2 → `unknown_op`, frozen v2 caps byte-identical.
- **One computer** (`view::walk::walk_rooted`, shared mount-corpus assembly):
  one spelling of color/reason/detail on the wire and in `mrd walk --json`.

```json
{"id":12,"op":"walk","path":"a.md"}
{"id":12,"ok":true,"body":{
 "direction":"up","page":"a.md",
 "entries":[
  {"depth":1,"selector":"b.md","rev":"fp1.span2.b3.…","color":"green"},
  {"depth":2,"selector":"wiki:c.md","rev":"fp1.span2.b3.…","color":"grey",
   "reason":"unmounted","detail":"root 'wiki'","teaching":"grey(unmounted): …"}],
 "revs_read":[{"path":"a.md","doc_rev":"…"},{"path":"b.md","doc_rev":"…"}]}}
```

| Field | Law |
|---|---|
| `page` | the walked page, page grain; not `root` — that body key is the fingerprint slot |
| `depth_bound` | bound in effect; absent = unbounded |
| `entries[]` | BFS order: depth ascending, then discovery. `{depth, selector, rev?, color, reason?, detail?, teaching?}`. `selector`: lock row's canonical address, `root:`-qualified across roots; a section claim spells `path §selector`, never `path#selector` (`#` only on the stored/lock plane). `color`: tone `green`/`red`/`grey`. `reason`: stable word, absent exactly on green. `teaching`: only colors that teach, never invented |
| `revs_read[]` | `{path, doc_rev}`, path order — the docs the listing rests on, the only revs it is falsifiable against |
| `excluded[]` | §12.1 enumerator clause, down walks only: the markdown the hash domain left out. Omitted when empty and always on up — up drops nothing, naming an excluded ancestor at a red edge by its correct path |

**Doors and refusals.** A named page the hash domain excludes is served (§12.1
door-family clause). Missing root → `file_not_found`; unserved member →
`invalid_utf8`; in-snapshot cycle → `walk_cycle` (env class), naming the loop;
unreadable hash domain → `io_error`, never the default domain (a fail-open
§12.1 rules out).

**The staleness triple does not apply.** The walk serves the warm projection at
one borrow; per-row `rev` and `revs_read` carry falsifiability.

### A.11 `sql` — corpus SQL over the resident projection cache

- **Scope**: one SQL statement over the workspace's fingerprint-pinned,
  append-only `sql.duckdb` projection cache (`view::store`) — the one wire op
  over the view organ.
- **§10.4 holds**: the daemon alone owns and appends to the cache file; the
  wire carries **results, never a file path**.
- **Request** `{query}` only (strict field wall): cwd and row bounds are host
  concerns; faces bound results (the MCP face's `max_rows` + output-file law).
- **Execution**: one path, no profile split — as the CLI lane runs it,
  spill-bounded, nothing locked or disabled.
- **Caps**: workspace-bound; v3-only at dispatch, cap `sql` at op grain; v2 →
  `unknown_op`, frozen caps byte-identical.
- **Serve shape per call**: warm engine snapshot → pin check + delta append
  (O(changed files), cache-as-manifest) → always-rollback query
  (`BEGIN → statement → collect → ROLLBACK`) → post-result currency through
  the resident memo at the vouched grade (`node-rev-merkle-spec.md` §6.7; the
  leaf-memo stat sweep is the floor on a named miss), so `state` post-dates
  the rows (§10.1).

```json
{"id":13,"op":"sql","query":"SELECT path FROM doc ORDER BY path"}
{"id":13,"ok":true,"body":{
 "as_of_fingerprint":"b3b:…","live":"b3b:…","state":"FRESH_AT_SAMPLE",
 "columns":[{"name":"path","type":"VARCHAR"}],
 "rows":[["a.md"],["b.md"]],"row_count":2}}
```

| Field | Law |
|---|---|
| `as_of_fingerprint` | the projection pin the rows were computed at: the engine's warm corpus fold, verbatim |
| `live` | post-result currency fingerprint; absent exactly on UNVERIFIED |
| `state` | `FRESH_AT_SAMPLE` \| `STALE` \| `UNVERIFIED`; UNVERIFIED iff `error` is set |
| `columns[]` | `{name, type}` as `DuckDB` reports them — the pair the CLI `--json` frame carries |
| `rows[]` | row-major JSON cells; list cells are real arrays per row, never column dumps. Booleans and numerics are JSON bools/numbers; every other scalar family — timestamp (tz-aware columns marked `+00`), date, time, interval, decimal, enum, struct, map — is a string in `DuckDB`'s `::VARCHAR`, never a `Debug` repr; a union cell is its member's cell |
| `error` | a caller's failing SQL is a success body with the engine's words verbatim plus the arms below. Never a wire error: `ok:false` frames are door faults (`io_error`, `bad_request`, `unknown_op`) |

**Teaching arms on a refusal (reason first, then a suggestion that fits).**
Three arms extend or trim the engine's words:

1. **View-DML**: the remedy names the `hist.*` lane.
2. **A retired face name** names its replacement (`card` → `record`); with no
   compat alias, the refusal is the whole migration path.
3. **A Did-you-mean fitted to a catalog internal is dropped.** Edit-distance
   fits over the whole catalog can land on metadata (`card` → `pg_attrdef`,
   `board_drift` → `duckdb_constraints`): a `pg_`/`duckdb_`/`sqlite_` fit never
   answers a face question. Near-miss face fits (`records` → `record`) are
   untouched.

**DML law (ruled).** Latest-layer names (`doc`, `section`, …) are views over
append-only history; DML against them refuses with `DuckDB`'s own error plus
the `hist.*` remedy (arm 1). DML against `hist.*` executes, is visible to its
own statement, and dies at ROLLBACK: "writes nothing durable" on a persistent
file. Nothing else is guarded: trust posture, no statement classifier, no auth.

**The occurrence index is SERVED as a column, never re-derived.** The `section`
relation publishes `n`: the section's 1-based occurrence among same-parent,
same-raw-text siblings (§2.1's occurrence index), **NULL exactly where the
published address omits it** — so `n IS NOT NULL` is the ambiguity
predicate, and the column is the last segment of that row's `hpath`. `n` comes
from the `hpath` column's address owner, shared with the read face's toc
(`model`'s law), never the projector. It closes a silent wrong-target write:
an occurrence re-derived from `node_seq` — the document-order ordinal over
every section of the file, the row's identity — hits a real, different section
that guards fine; a class with no refusal.

⚠️ **`n` addresses a section inside one file, never which entry of a `files[]`
request a row belongs to** — see `files[i]`, §A.7 and the §4.4 set form.

**The CLI ladder.** `mrd sql` asks the resident daemon first (this op), then
the drawer file when unheld, then `:memory:`. One ladder, no profile
distinction; only `--rebuild` goes direct.

### A.12 Rooted refs at every page-taking door

Every agent-facing door that names a page resolves the agent-plane
`[root:]path` through the one rooted lane. The law, the door family, the
preset-lane exception, and the authority rule (the page's workspace governs
conventions, caps, receipts) live in `address-grammar.md` § 4.6. **On this contract the rooted lane changes nothing
structural**:

- **Resolution happens at the door; the wire carries the rel half only.** §1's
  `Path` law and its head-colon confinement arm (`addr::confined`) stand: a
  raw `root:` head refuses `bad_path`.
- **The mechanism is the shipped workspace jail.** `hello` pins the declared
  workspace exact-or-refuse, never widening to an enclosing one, and stays
  attached for reads and writes; a rooted door resolves the root, then dials
  that workspace.
- **The one in-band exception is unchanged:** `splice.pin.target` carries
  `name:rel` on the wire (pin-cross-root, § A.3).
- **`run` over the wire (§ A.8) follows the authority law:** it executes under
  the page's tree — conventions, caps ceiling, receipts.
- **The `script` lane's one-declared-root rule:** every `files[]` entry
  resolves through one root, which is the workspace; in-program paths are
  relative to it. An MCP face may admit absolute and session-relative refs;
  the §1 path law does not.

---

## § B. Process

1. Edit this file (or the relevant spec under `docs/`) **before** code.  
2. Do not reintroduce versioned contract files or amendment piles.  
3. **UNVERIFIED** when evidence is missing.
