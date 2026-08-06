---
type: contract
status: standing
updated: 2026-08-06
description: Standing wire constitution. One document. Docs define law; code may lag.
---

# Wire contract

> **Standing law.** One wire constitution — not a v2/v3 stack.  
> **Docs-first:** change this text before code when design moves. **Doc correct > code correct.**  
> **Always on:** (A) mint address = segments only `{"hpath":[{"h":"Goals"},{"h":"Q3"}]}`;  
> (B) receipt armed facts on the wire are normative — no second path;  
> (C) DuckDB / `view_path` are not agent-core wire.  
> See `README.md`. Hash algorithms: `node-rev-merkle-spec.md`, `fingerprint-norm-spec.md`.

**Content-hash noun:** workspace content hash is **`fingerprint`** (`b3:…`). Workspace directory ≠ that noun.  
**Worked values:** hashes, spans, and counts in §§0.3–§12 are recomputed from the §0.3 fixture bytes (blake3).  
**Notation:** `[[…]]` inside the fixture fence are data bytes (what `resolve` / `links` operate on), not doc links.

## §0 Reading frame

### §0.1 Ordered goals this schema is designed TO

Intents here are **ordered goals with direction**, never co-equal absolute laws. A lower goal never overrides a higher one, and no rank adjudication is ever left to an agent.

**GOAL 1 — works-for-us (primary).** The ruled grammar (§2.1), the geography law (§5.3), the rulings enumerated below, and the foundational design law come first:

1. The interface ruling: five conceptual verbs (`toc`/`cat`/`edit`/`append`/`resolve`), sections-as-files, Edit-exact old/new scoped to a section, **requests never require revs / receipts always return them**, no byte offsets in the interface, mandatoriness = host policy ratchet, resolve = walk plane.
2. The design-selection and amendment rulings: the fix-at-freeze list resolved in the deviation ledger (§18), a single block-id charset (§2.4), and node-grain deltas (§7.4).
3. Foundations: Starlark as rules evaluator (§11.4). View organ optional and wire-agnostic (§10.3–§10.4).
4. The review checklist A1–A14 and the ratified convergence items (mint partition, 16-hex rev, md-only, two-stage resolve, app-oracle GT).
5. The foundational design law: delta noun, receipts, actor/now as wire inputs, rules-as-data, view topology, honest limits. Collisions with this law resolve TO it, surfaced never silent.

**GOAL 2 — must-work-in-Obsidian (the compatibility floor).** Everything we mint and emit works in the Obsidian app. This is one-way: we never promise to reproduce the app's behavior on inputs outside our own grammar. `resolve` walks the app's grammar best-effort (§4.5); out-of-grammar inputs (e.g. `_`-bearing anchors) refuse loudly. Goal 2 never overrides Goal 1 — where they meet, the ruled grammar wins, surfaced never silent.

### §0.2 Formerly open gates — all RULED (nothing silently resolved)

| Item | Ruling | Where in this contract |
|---|---|---|
| Delta grain at birth | node-grain | §7.4 |
| Starlark ratification | ratified | §11.4 |
| Rung-5 view organ | optional; no engine-named wire elements | §10.3–§10.4 |
| `_` block-id charset | two-plane split VETOED — one app-exact charset | §2.4 |

The base design carried each of these as a conditional with both outcomes designed; the ruled outcome is now the contract text and the not-taken branches live in the decision records, not here. Deviations and waivers: §18.

### §0.3 The worked fixture (all examples run against this)

Workspace `wsfix/` — three timeline states. State **S0**:

```
notes/plan.md            136 bytes   file_rev e3c4acaceb75b907
receipts/2026-07-18.md    26 bytes   file_rev 920a40c4ee23d37c
.github/README.md         11 bytes   (md, but OUTSIDE the hash domain — default ignore, §12)
meridian/domain.md                   (standing domain declaration; md, inside hash domain when present, §12)
```

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

The remaining fixture bytes (every file this document hashes is printed in this section):

- `receipts/2026-07-18.md` at S0, exact bytes (26 B — the `—` is 3-byte UTF-8): `# Receipts — 2026-07-18` + LF.
- `.github/README.md`, exact bytes (11 B): `# CI notes` + LF.
- `drafts/tmp.md` (appears only in the §12.3 domain-bump example), exact bytes (8 B): `scratch` + LF.
- `meridian/domain.md` at v0/v1 declares the custom ignore list and domain `version` (§12.3). It is markdown and participates in the hash domain when present.

Timeline: **S0** →(E3 edit)→ **S1** (`plan.md` 139 B, receipts 249 B) →(E4 append)→ **S2** (`plan.md` 150 B, receipts 474 B). Roots:

| State | Fingerprint (full width, never truncated) |
|---|---|
| R0 (S0) | `b3:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9` |
| R1 (S1) | `b3:10769ae1c77f5646750f3f52df2d055156b411145a02b8361ecd32af1357a1b7` |
| R2 (S2) | `b3:83b4ba591c0291d9f2a05428cac38e5820858fbb9c47720ab352344ddccc8f68` |

## §1 The five wire nouns

The wire has exactly five nouns. Four carry forward from `crates/wire` vocabulary (`Path`, `Span`, `NodeRev`, and the content-hash type still often named `Root` in code — design noun **fingerprint** — `crates/wire/src/lib.rs`, newtypes, serde-only); the fifth — **Delta** — is born here, at contract birth.

| Noun | Shape | Law |
|---|---|---|
| `Path` | string, `/`-separated, workspace-relative, UTF-8 | never absolute; no `..`; the workspace root is ambient (`fs::WorkspaceRoot`) |
| `Span` | `[start, end)` byte pair, u64, serialized `[s,e]` | UTF-8 **bytes** on raw disk content — never chars, never UTF-16 (conversion to Obsidian's UTF-16 `Loc.offset` lives ONLY in the conformance harness) |
| `NodeRev` | 16 lowercase hex = `blake3-256(span bytes)[:16]` | opaque to clients, equality-only; honest threat: §13.1 |
| `Fingerprint` | `"b3:" + 64 hex`, full width | algorithm+domain prefixed; prefix bumps on domain-rule change (§12.3); never truncated |
| `Delta` | the change-fact object, §7 | node-grain at birth; stable shape; replay ≡ live |

**Span sub-laws (carried from contract v1 §2):** section (heading) nodes are newline-inclusive to the next boundary — a section's span ends at the next heading of level ≤ its own (else EOF); strictly-deeper headings are contained within it — heading-inclusive, no trim (the node-rev-merkle-spec §5 fixture pins stay live); leaf block nodes exclude the final line terminator; inline nodes include their delimiters; a request naming a span that splits a multi-byte character is refused (`bad_request`) — the guarantor is parser token discipline on reads and the reparse gate on writes (§15).

**Rev sub-laws:** `node_rev` is minted over the node's **full span bytes** — heading-inclusive for sections, so a heading rename deliberately invalidates the token. `content_span` starts at the first byte after the heading line's terminator and runs to the section span's end; it mints **no** rev — there is exactly one rev per node and it is the full-span hash (§5.1 states the CAS comparison rule). File-grain `file_rev` = `blake3(whole file bytes)[:16]`, same family, same width.

**One hash family:** BLAKE3-256 everywhere (rev, file_rev, leaf, interior, fingerprint). The repo deliberately left the algorithm open (`model` crate doc: "algorithm is a rung-2 wire amendment"); **this schema is that amendment** — it decides blake3, consistent with the fingerprint spelling (`"b3:…"`) and node-rev-merkle-spec §1.

### §1.1 Replaceability test (zero consumer concepts)

Per noun and per field: *can a non-ccc consumer replace the convention without touching engine code?*

| Noun/field | Verdict |
|---|---|
| `Path` | any UTF-8 relative path; no ccc naming baked in |
| `Span` | raw byte math; no convention at all |
| `NodeRev`/`Fingerprint` | opaque tokens; algorithm swap = domain-prefix bump (§12.3), engine mechanism unchanged for consumers |
| `Delta` | generic node-change facts; no ccc vocabulary in any field |
| `actor` | opaque string, engine never parses it — `agent:b0864fb2` is a ccc *convention*, the field takes anything (§9) |
| `now` | RFC 3339 string supplied by the caller; engine validates format, never generates (§9) |
| `receipt` address | any md path + anchor; the receipt *rendering* is a shipped default template, the armed facts on the wire are the normative content (§6.4) |
| rule packs | pack data behind a generic manifest; no evaluator hard-coded (§11) |


## §2 One address grammar, two planes — and the verb IS the plane

This contract requires one fleet grammar. This schema enforces it **structurally**: the strict mint plane and the Obsidian interop walk are carried by *different verbs with different response types*, and the walk-plane response type **has no rev field to return**. A ref cannot arm a write because the op that accepts refs is incapable of minting — the mint partition.

### §2.1 The strict mint plane (the ONE fleet grammar)

Write targets and strict reads name nodes by **exact name only** — three forms, used in `cat`, `splice`, and echoed in `toc`/receipts/deltas:

| Form | Shape | Semantics |
|---|---|---|
| hpath | `{"hpath":[{"h":"Goals"},{"h":"Q3"}]}` | per-segment **byte-equality** against the real containment tree; optional occurrence `{"h":"Beta","n":2}` (1-based, document order among identical raw texts at that position). No join string exists — the `#A#a/b` vs `#A#a#b` ambiguity is unrepresentable |
| anchor | `{"anchor":"r-000042"}` | block id, exact match. Duplicate id in one file → the mint plane refuses `ambiguous_ref` (loud), while the walk plane follows the app (last wins, silent) — the silent-last-wins mint death mode is closed |
| fm_key | `{"fm_key":"title"}` | top-level frontmatter key; the node is the full key line (frontmatter plane is nodes, never ref grammar — `#:key` is dead) |

Stale names fail loud (`ref_not_found`); every ref-carrying wire surface — `cat`/`splice` targets and the echoes in `toc` rows, receipts, deltas, and verdicts — uses this grammar and no other.

### §2.2 The walk plane (read-only interop INPUT)

`resolve` alone accepts the Obsidian ref algebra — the raw linktext `path#sub`, `#sub`, `path#^id` (no brackets on the wire; the interface strips `[[…|alias]]` sugar before the wire, so exactly one wire spelling exists). It walks the app's loose grammar best-effort (§4.5); its output is location facts only. Interop refs pay one `toc`/`cat` hop to become write targets.

### §2.3 Layering, not collision

The strict grammar is THE fleet grammar; the Obsidian algebra is a syntactically disjoint, read-only compatibility input. Our extensions (revs, fingerprints, occurrence index, domain config) never appear inside the walk grammar; `[[##`/`[[^^` search syntaxes are UI, out of scope.

### §2.4 The block-id charset — ONE charset, both planes

Block ids match `[A-Za-z0-9-]+` — Obsidian app-exact — on BOTH planes. This is the single normative statement of the charset; every other section references it. No `_` in newly minted block ids anywhere; a `_`-bearing anchor is outside the strict-plane grammar (`bad_request`). No organic live `_` block ids exist in any fleet corpus — an empirical corpus finding, true as of the survey behind it, never a standing invariant — so a corpus-wide re-id migration costs ZERO and none ships; what remains is a mint-guard enforcing this charset going forward plus a frozen-fixture exemption, **owned by the phase-2 impl-taskpack, not this document** (§13.8). The `_`-bearing probe stays frozen in the `obsidian-compat@1.12.7` pack so the app's actual treatment of legacy `_` ids is pinned, not assumed.

## §3 Frame layer, correlation, discovery

### §3.1 Frames

NDJSON, one JSON object per line; stdout carries frames only, logs go to stderr (`echo '{"id":1,"op":"hello",…}' | sidecar` debuggability is a contract property). Three frame types, classified by the **raw** `id` key:

- key `id` present → Request/Response (correlated)
- key `id` absent → Notification (§7 deltas ride here)

**Raw-lexeme id law:** classification and id validation happen on the **raw JSON `id` lexeme BEFORE typed decode**. Valid ids are JSON integer lexemes in `[0, 2^53)`. The discrimination set:

| raw lexeme | verdict |
|---|---|
| `7` | valid request |
| `"7"` | `bad_request` (string, not integer) |
| `3.5`, `-1` | `bad_request` |
| `3e0` | `bad_request` — raw-lexeme law: JSON-equal to 3 is irrelevant, the lexeme is not an integer lexeme |
| `9007199254740991` (2^53−1) | valid |
| `9007199254740992` (2^53) | `bad_request` |
| `18446744073709551616` (2^64) | `bad_request` — never misclassified as Notification |

A non-conforming id cannot be echoed as a valid id: the error frame carries `id:null` plus the offending lexeme verbatim in `id_raw` (string). Under the null-id corruption law a pipelining client treats any `id:null` frame as corruption — fail all outstanding, respawn — which is the *correct* outcome for a client whose id generation is broken; single-shot clients read `id_raw`.

Correlation: one response per request, id echoed by value; in-flight uniqueness required. `MAX_FRAME_BYTES = 256 MiB` (corruption bound, stands from `crates/transport-proto`).

### §3.2 hello / caps (proto-1 retained)

```json
{"id":1,"op":"hello","proto":1,"client":"md-cli/0.3"}
{"id":1,"ok":true,"body":{"proto":1,"server":"meridian-sidecar/2.0",
  "caps":["toc","cat","extract","resolve","resolve.content","links","links.require_fingerprint",
          "splice","splice.if_node_rev","splice.if_fingerprint","splice.dry","splice.receipt",
          "splice.verdicts","fingerprint","diff","sub"],
  "fingerprint":"b3:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9"}}
```

`caps` is the complete set — no version sniffing, ever. Field-only amendments ship as dotted `op.field` strings. `fingerprint` in the hello body is optional (the engine may not have walked yet); when present it is the first ambient fingerprint.

**Rev-presence law:** `node_rev` is MUST on every `toc`/`cat`/`extract` node whenever `splice ∈ caps`.

**Evolution:** strict server — unknown request fields and unknown enum values in requests are rejected loudly; tolerant client — unknown response fields and unknown open-kind strings are ignored. Server-first rollout.


## §4 The op surface

Ten ops. The five-verb interface maps onto them 1:1 (§4.8). Read ops are classified by the wire-op criterion: feeds-an-action → wire fact op; feeds-orientation → dashboard-only, NOT on this wire.

| Op | Rung (panel ladder) | Class |
|---|---|---|
| `hello` | 1 | discovery |
| `toc`, `cat`, `extract` | 2 | single-file facts — the **mint surface** |
| `resolve` | 2 | walk plane — never mints |
| `links` | 5 (view-shaped) | corpus fact — staleness triple (§10) |
| `splice` | 4 | the ONLY write op, batch-only |
| `fingerprint` | 3 | integrity fact |
| `diff` | 3 (reserved shape, standing) | replay (§7) |
| `sub` | 5 (reserved shape, standing) | delta transport (§7) |

The v1 §6.4 `Guard{root,path}` reserved op is **dropped**: the integrity surface is `root` + `splice.if_fingerprint` + `diff` — the mined commit-guard idiom gets its wire story without a second guard grammar: one construct, one grammar.

### §4.1 toc — the map, revs riding along free

Request: `{"id":2,"op":"toc","path":"notes/plan.md"}` — response at S0 (every value computed):

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

toc is the complete write kit: hpath + `node_rev` per section, anchors with their revs when present, frontmatter keys. The header `fingerprint` makes the commit-guard idiom ambient — read a toc, later pass `if_fingerprint`. `content_span` is served for interface use (heading-preserving display) but mints nothing (§1).

**The anchor toc row, worked** (`receipts/2026-07-18.md` at S1 — the only block-id-bearing file in the fixture; every value computed): the plan.md toc above shows no anchor row because no plan.md section carried a block id at toc time, so the "anchors with their revs" clause is worked here instead.

```json
{"id":4,"op":"toc","path":"receipts/2026-07-18.md"}
{"id":4,"ok":true,"body":{
 "path":"receipts/2026-07-18.md","file_rev":"2731acfa39bbb92c",
 "fingerprint":"b3:10769ae1c77f5646750f3f52df2d055156b411145a02b8361ecd32af1357a1b7",
 "nodes":[
  {"kind":"heading","level":1,"hpath":[{"h":"Receipts — 2026-07-18"}],"span":[0,249],
   "content_span":[26,249],"node_rev":"2731acfa39bbb92c","text_prefix_16b":"# Receipts — 2"},
  {"kind":"list_item","anchor":"r-000042","span":[26,248],
   "node_rev":"639a2dca46f6fcc8","text_prefix_16b":"- splice notes/p"}]}}
```

The `^r-000042` block echoes as a `list_item` node keyed by its `anchor` ref (§2.1) carrying its own `node_rev` over the block-leaf span (terminator excluded — `[26,248]`, byte-identical to the receipt facts armed in §4.4 and printed in §6.3); the lone top-level heading spans the whole file, so its `node_rev` equals `file_rev` (`2731acfa39bbb92c`). An anchor becomes a write target by the same one-hop path as a section.

### §4.2 cat — read one section, not the disk

What you read is exactly what is hashed: `cat` returns the **full span bytes** (heading-inclusive) and the rev is blake3 of precisely those bytes — the ambient-rev property with zero fine print. `sec` absent → whole file + `file_rev`.

```json
{"id":3,"op":"cat","path":"notes/plan.md","sec":{"hpath":[{"h":"Goals"},{"h":"Q3"}]}}
{"id":3,"ok":true,"body":{"span":[49,72],"node_rev":"33d5b0e1b27cb48b",
 "content":"## Q3\n\nship by August\n\n"}}
```

### §4.3 extract — the extract surface, stands

`{"op":"extract","path":…,"kinds":[…]}` stands as specified (`crates/wire` §5): full node objects, 11-variant kind enum whose declaration order is the sort-tiebreak ordinal, per-kind `info`, `text_prefix_16b` (implemented + tested in `crates/wire-map`), total node order (span.start asc, span.end desc, kind ordinal). One decision this schema makes: **an unknown value in `kinds` is `bad_request{"unknown_kinds":[…]}`, loud** — the strict-server evolution law applied to values, killing the typo-silently-returns-nothing trap. This diverges from v1's "not an error"; decided once, here.

### §4.4 splice — the only write op (batch-only, one response shape)

The Edit-tool semantic model IS the wire write grammar: exact `old`/`new` replacement, no regex, no fuzz, uniqueness required, matched **server-side within the target's full span bytes**. There is no client span field anywhere in a request: a client *cannot* supply a byte offset, so the class of wrong-offset writes is unrepresentable — spans are the wire's business.

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

**The CLI seam reads the `edits` VALUE, not this envelope.** `mrd put <PATH>`
takes a BARE array on stdin — exactly what stands under `"edits"` above —
because `id`, `op` and `path` are argv's on that face. Sending the whole
request object is refused (`invalid type: map, expected a sequence`). The
request shape shown here is the WIRE's; it is not a stdin template.

Edit shapes — exactly two:

| Shape | Semantics |
|---|---|
| `match{old,new}` | Edit-exact: `old` must occur exactly once in the target's full span bytes; replaced by `new`. Zero occurrences → `no_match`; two+ → `not_unique{matches}` |
| `put{at,text}` | whole-slot writes: `at:"all"` (replace full span, heading included), `at:"content"` (replace content span, heading preserved), `at:"end"` (insert `text` at the span-end byte — the append verb; **raw byte concatenation, no synthesized separator**, Edit-model exact, so `text` that must begin a new line carries its own leading `\n` — against a terminator-less final line a separator-less `text` is the caller's to get right, and a result that loses containment refuses `would_corrupt`, batch laws below) |

Batch laws: batch-only, ONE response shape; all targets and guards resolve against the **pre-batch** state; the edits' **replaced regions** must be pairwise disjoint (`bad_request{"overlap":…}` otherwise, and the refusal names the offending edits and a remedy). The replaced region is what the edit rewrites — `match` the matched bytes, `put at:"all"/"content"` that span, `put at:"end"` the zero-width insertion point — so edits whose *targets* nest compose legally when their regions touch different bytes: an append to a section plus a sibling-section birth under its parent is ONE batch. Zero-width regions at the same byte are disjoint and apply in request order. The batch commits atomically through one reparse — a post-apply parse that would lose containment refuses `would_corrupt{"lost":[hpath…]}`. `dry:true` runs everything except disk: same response shape, `fingerprint_after:null`, no receipt written. *(Amended 2026-08-06: the disjointness grain was previously the target's full span, containment included — which refused any mixed append + section-birth batch under one tree, contradicting the batch-only law's own premise. The overlap refusal's `bad_request{"overlap":…}` extra is unchanged.)*

Response (S0→S1, all values computed):

```json
{"id":42,"ok":true,"body":{
 "armed":{"path":"notes/plan.md","edits":[
   {"target":{"hpath":[{"h":"Goals"},{"h":"Q3"}]},
    "node_rev_before":"33d5b0e1b27cb48b","node_rev_after":"41f643f034e5681f",
    "span_after":[49,75]}]},
 "receipt":{"path":"receipts/2026-07-18.md","anchor":"r-000042",
            "node_rev":"639a2dca46f6fcc8","span_after":[26,248]},
 "fingerprint_before":"b3:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9",
 "fingerprint_after":"b3:10769ae1c77f5646750f3f52df2d055156b411145a02b8361ecd32af1357a1b7",
 "seq":1,"verdicts":[]}}
```

The response carries what the write **ARMED** — target identities, rev transitions, spans after, the receipt fact, the root transition — never delivery claims. `verdicts` is the rules-as-data surface (§11). Spans appear in *responses* freely: the wire's business, never argv's.

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
            "node_rev":"c912d4578883f288","span_after":[249,473]},
 "fingerprint_before":"b3:10769ae1c77f5646750f3f52df2d055156b411145a02b8361ecd32af1357a1b7",
 "fingerprint_after":"b3:83b4ba591c0291d9f2a05428cac38e5820858fbb9c47720ab352344ddccc8f68",
 "seq":2,"verdicts":[]}}
```

Note the guardless request: legal at the wire forever — the mined call record shows zero organic rev use, and guards stay optional at the wire by design. Whether a scope *requires* `if_node_rev`/`if_fingerprint`/`actor` is host policy (§5.3), never wire schema.

Frontmatter-plane write, dry (fm_key node = the full key line, computed: span `[4,15]` = `title: Plan`):

```json
{"id":60,"op":"splice","path":"notes/plan.md","dry":true,
 "edits":[{"target":{"fm_key":"title"},
           "edit":{"match":{"old":"Plan","new":"Plan v2"}}}]}
{"id":60,"ok":true,"body":{
 "armed":{"path":"notes/plan.md","edits":[
   {"target":{"fm_key":"title"},
    "node_rev_before":"fa77480c79a853bc","node_rev_after":"fb49e9df2257fab8",
    "span_after":[4,18]}]},
 "fingerprint_before":"b3:83b4ba591c0291d9f2a05428cac38e5820858fbb9c47720ab352344ddccc8f68",
 "fingerprint_after":null,"dry":true,"verdicts":[]}}
```

### §4.5 resolve — the walk plane: where things are, never a handle

`resolve` is **best-effort app-compatible walking**, two-stage: within the ruled grammar it walks the way the app walks — `parseLinktext` → stage 1 `getFirstLinkpathDest(linkpath, from)` (basename index, frontmatter aliases, case-insensitive, source-relative shortest-unambiguous, unresolved first-class) → stage 2 subpath walk (case-insensitive · first-match-wins on duplicates, silent · strictly-deeper-level · anywhere-after · generation-skipping). These empirical walk-law properties are carried verbatim as the **behavior spec**; their ruled **status** is a one-way compatibility floor (everything we mint and emit walks in the app), not a binding two-way parity law. The ruled grammar always wins: an input outside it (e.g. a `_`-bearing anchor, §2.4) refuses loudly (`bad_request`) — conforming behavior, never a deviation to ledger, because we never promised to reproduce the app's walk on inputs outside our own grammar. The `obsidian-compat@1.12.7` pack (§13.4) is the **regression fixture set** that pins the app's actual walk against version drift, alongside the six-probe walk law; hand-frozen resolution fixtures are dead.

**The response type has no rev field.** This is the mint partition as a type-level fact, not a discipline.

```json
{"id":70,"op":"resolve","from":"notes/plan.md","ref":"plan#Goals#Q3"}
{"id":70,"ok":true,"body":{"dest":"notes/plan.md","span":[49,75]}}

{"id":71,"op":"resolve","from":"notes/plan.md","ref":"plan#goals#q3"}
{"id":71,"ok":true,"body":{"dest":"notes/plan.md","span":[49,75]}}

{"id":72,"op":"resolve","from":"notes/plan.md","ref":"2026-07-18"}
{"id":72,"ok":true,"body":{"dest":"receipts/2026-07-18.md","span":[0,474]}}

{"id":73,"op":"resolve","from":"notes/plan.md","ref":"plan#Goals#Q9"}
{"id":73,"ok":false,"error":{"code":"ref_not_found","recovery":"refresh",
 "stage":2,"dest":"notes/plan.md"}}

{"id":74,"op":"resolve","from":"notes/plan.md","ref":"roadmap"}
{"id":74,"ok":false,"error":{"code":"ref_not_found","recovery":"refresh","stage":1}}
```

(Values at S2; ids 70/71 demonstrate case-insensitivity on computed spans.) `dest` rides every stage-2 outcome, success or failure — the failing stage is observable in every transcript. `content:true` additionally returns the fragment bytes — still no rev. The strict plane errors `ambiguous_ref` where the walk would silently pick (never-silently-picks on the extension plane; the walk itself mirrors the app best-effort, silence included). `from` is mandatory: resolution is source-relative, and the vault name/alias index this implies arrives at rung 2.

### §4.6 links — the 188-call oracle audit, one call per file

The mined record's biggest read pattern (`read`-as-oracle, 188×) becomes a fact op. Corpus-wide ⇒ it carries the staleness triple (§10):

```json
{"id":80,"op":"links","path":"notes/plan.md"}
{"id":80,"ok":true,"body":{
 "as_of_fingerprint":"b3:83b4ba591c0291d9f2a05428cac38e5820858fbb9c47720ab352344ddccc8f68",
 "live_fingerprint":"b3:83b4ba591c0291d9f2a05428cac38e5820858fbb9c47720ab352344ddccc8f68",
 "changes_seq":2,
 "files":{"notes/plan.md":{
   "resolved":{"receipts/2026-07-18.md":1},
   "unresolved":{"roadmap":1}}}}}
```

Shape mirrors the app's `resolvedLinks`/`unresolvedLinks` — per-edge counts; dangling refs first-class. `path` absent → whole-corpus edge map. Opt-in `require_fingerprint` → `stale_view` refusal (§10.2).

### §4.7 fingerprint and diff — the integrity rung

```json
{"id":90,"op":"fingerprint"}
{"id":90,"ok":true,"body":{
 "fingerprint":"b3:83b4ba591c0291d9f2a05428cac38e5820858fbb9c47720ab352344ddccc8f68","seq":2}}
```

`diff` is reserved AT the integrity rung with its shape standing now — the compound front door:

```json
{"id":95,"op":"diff",
 "from_fingerprint":"b3:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9",
 "to_fingerprint":"b3:83b4ba591c0291d9f2a05428cac38e5820858fbb9c47720ab352344ddccc8f68"}
{"id":95,"ok":true,"body":{"batches":[ /* Delta seq 1, Delta seq 2 — §7, byte-identical
                                          to the live notification frames */ ]}}
```

Replay ≡ live (§7.3). A fingerprint range outside the retained history → `fingerprint_unknown` → full resync (honest bound: §13.5). `sub` (rung 5) is a reserved shape — `{"op":"sub","from_seq":N}` → ok, then Notification frames each carrying one Delta batch; only the transport is deferred, the noun it carries is frozen here — rung 4/5 defers only transport.

### §4.8 The five-verb interface, mapped

| Verb | Wire op | Notes |
|---|---|---|
| toc | `toc` | rows `level · span · rev · hpath` — the map, revs riding free |
| cat (CLI: `mrd read` / section) | `cat` | `--sec` segments → hpath array verbatim |
| edit (CLI: `mrd put` + match) | `splice` with one `match` edit | `--if 9d3e…` → `if_node_rev`; receipt address defaulted by the client library, overridable |
| append (CLI: `mrd put` + put at end) | `splice` with one `put{at:"end"}` edit | |
| resolve (interop; interface strips `[[…]]`) | `resolve` | interface strips `[[…|alias]]`; wire takes raw linktext — one grammar |

Requests never require revs; receipts always return them — after any read or write the current rev is ambient, and `--if` costs ~10 tokens, zero extra calls. No byte offsets exist anywhere in the interface surface; none exist in wire *requests* either (§4.4).


## §5 Guards and the CAS law

### §5.1 The CAS rule, explicit

`if_node_rev` is compared against `blake3(target's full span bytes)[:16]` **re-derived at execution time from the pre-batch state** — the same bytes, the same hash, the same truncation that `toc`/`cat` served as `node_rev`. There is no second rev derivation anywhere: no content-span rev exists (§1), and no client span exists to disagree with a minted one (§4.4). The natural implementation — hash the bytes you are about to replace — is the *only* implementation, so the "fails 100% of content_span splices" trap is not fixed but **unrepresentable**.

`if_fingerprint` compares against the current workspace fingerprint and is checked FIRST — world-grain, cheapest, fails the whole batch (`fingerprint_mismatch{expected,actual,changed}` → re-plan); then per-edit `if_node_rev` (node-grain, `cas_mismatch{expected,actual}` → refresh). Merkle-spec §7 semantics carried; Rust computes hashes; hosts only compare opaque tokens.

### §5.2 The failure split (the adoption carrot, on the wire)

Worked failures against S2 (all values computed; Q3's rev at S2 is `41f643f034e5681f` — the E4 append touched only Q4, so Q3's bytes and rev are unchanged from S1, itself a demonstration of node-grain rev stability):

```json
{"id":88,"op":"splice","path":"notes/plan.md","edits":[
  {"target":{"hpath":[{"h":"Goals"},{"h":"Q3"}]},
   "edit":{"match":{"old":"ship by September","new":"ship by October"}},
   "if_node_rev":"33d5b0e1b27cb48b"}]}
{"id":88,"ok":false,"error":{"code":"cas_mismatch","recovery":"refresh",
 "expected":"33d5b0e1b27cb48b","actual":"41f643f034e5681f"}}
```

→ the world moved (the client held S0's rev). One `cat` refreshes.

```json
{"id":89,"op":"splice","path":"notes/plan.md","edits":[
  {"target":{"hpath":[{"h":"Goals"},{"h":"Q3"}]},
   "edit":{"match":{"old":"ship by August","new":"ship by October"}},
   "if_node_rev":"41f643f034e5681f"}]}
{"id":89,"ok":false,"error":{"code":"no_match","recovery":"fix","matches":0}}
```

→ rev PASSED and old-string didn't: **provably your typo** — the diagnosis the `--if` flag buys, now a wire-observable distinction.

```json
{"id":91,"op":"splice","path":"notes/plan.md","edits":[
  {"target":{"hpath":[{"h":"Goals"},{"h":"Q4"}]},
   "edit":{"match":{"old":"item","new":"entry"}}}]}
{"id":91,"ok":false,"error":{"code":"not_unique","recovery":"fix","matches":2}}
```

→ `item` occurs in both `- item one` and `- new item` (count computed: 2). Add context bytes to `old`, exactly as with the Edit tool.

Without `if_node_rev`, `no_match` is ambiguous between typo and moved world — stated honestly; the guard is what disambiguates, which is why agents adopt it.

### §5.3 Geography: where mandatoriness lives

The wire is permissive forever: unguarded, actor-less, receipt-less splices are legal wire frames. Requiredness — "shared scopes need `if_node_rev`", "this tree needs `actor`", "receipts mandatory under `results/`" — lives in **host/client policy** (the geography law), not in wire schema: the wire always accepts the frame. Tightening requiredness after adoption is host work.

**Scope grammar (bound here):** a ratchet scope is expressed in the fleet vocabulary and no other — a `Path`-set selector (path globs, config data that never rides the wire) plus, where a scope names nodes, strict-plane refs per §2.1. No second address grammar exists in policy config, and the §11.3 pack manifest carries no scope field by design: packs bind rules to the world model; the ratchet binds requiredness to scopes on the host side. (Fix-at-freeze, §18 row 1.)

## §6 Receipts — outcome as fact

### §6.1 The law

A write intent that names a receipt gives its address as `receipt:{path, anchor}` on the splice request — receipts are per-request, never a wire requirement (unguarded, actor-less, receipt-less splices are legal frames, §5.3; whether a scope *requires* one is host policy, §5.3, not wire schema). When named, the engine appends the receipt entry to that md file **in the same batch commit** as the content edit — one wire exchange, one reparse, ONE fingerprint advance covering both files. Receipts are ordinary markdown inside the hash domain (composes with the md-only domain law, §12): addressable, cat-able, resolvable via `#^anchor`, hashed into the workspace fingerprint like everything else.

The armed response (§4.4) and the receipt entry carry the same facts: op, target identities, rev transitions, `fingerprint_before`, actor, now, request id. Facts about what was **armed** — never "delivered", never "succeeded downstream" (outcome-as-fact; delivery is the host's business).

### §6.2 No-self-rooting law (honest limit)

A receipt **cannot contain the root it produces**: `fingerprint_after` covers the receipt file's bytes, so writing `fingerprint_after` into the receipt is structurally impossible (the hash would have to contain itself). Receipts therefore carry `fingerprint_before`; `fingerprint_after` rides the wire response and the Delta. This is stated as a limit, not worked around.

### §6.3 The worked receipt — one writeable form only

**Normative content is the armed-fact set on the wire response** (§4.4), not any particular md line shape. Target identity is always §2.1 form — e.g. `{"hpath":[{"h":"Goals"},{"h":"Q3"}]}` — never a joined string.

Illustrative default receipt line (segment target only):

```markdown
- splice notes/plan.md id=42 actor=agent:b0864fb2 now=2026-07-18T20:31:04Z fingerprint_before=b3:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9 edits=1 target.hpath=[{"h":"Goals"},{"h":"Q3"}] match 33d5b0e1b27cb48b->41f643f034e5681f ^r-000042
```

E4 append via `put{at:"end"}`:

```markdown
- splice notes/plan.md id=57 actor=agent:b0864fb2 now=2026-07-18T20:33:41Z fingerprint_before=b3:10769ae1c77f5646750f3f52df2d055156b411145a02b8361ecd32af1357a1b7 edits=1 target.hpath=[{"h":"Goals"},{"h":"Q4"}] put:end 4b8bc385a58da0e0->f43203a1f0b4c9a3 ^r-000043
```

**Do not re-teach** pretty joins (`Goals>Q3`). Template replaceability (D-C10) is not permission for a second address grammar.

### §6.4 Replaceability (D-C10 — facts are the contract)

The md *rendering* is a shipped default template; a non-ccc consumer replaces it freely — the normative receipt content is the armed-fact set, defined by the wire response shape. The engine mechanism is generic: "append these facts at this address with this anchor". "No intent past-due without a receipt" is lintable: a rules pack can assert every splice-bearing transcript row has its receipt anchor resolvable (§11).

### §6.5 Crash honesty

The batch writes two files via tmp+fsync+rename each; a crash between renames can land content without receipt. Recovery is re-derive (cold rebuild → correct root, never wrong data) and the missing receipt is exactly what the lint finds — the failure is loud in the world model, not hidden in engine state. Stated as a limit (§13.6); multi-file atomic commit is a rung-3 amendment candidate, not assumed.

## §7 The Delta noun — the fifth noun, stable at birth

### §7.1 Shape (stable)

One Delta = one batch = one fingerprint advance. E3's delta, every value computed:

```json
{"delta":{
 "seq":1,
 "fingerprint_before":"b3:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9",
 "fingerprint_after":"b3:10769ae1c77f5646750f3f52df2d055156b411145a02b8361ecd32af1357a1b7",
 "actor":"agent:b0864fb2","now":"2026-07-18T20:31:04Z",
 "files":[
  {"path":"notes/plan.md","change":"modified",
   "file_rev_before":"e3c4acaceb75b907","file_rev_after":"a9794a262e67ed02",
   "nodes":[{"hpath":[{"h":"Goals"},{"h":"Q3"}],"change":"edited",
             "node_rev_before":"33d5b0e1b27cb48b","node_rev_after":"41f643f034e5681f",
             "span_after":[49,75]}]},
  {"path":"receipts/2026-07-18.md","change":"modified",
   "file_rev_before":"920a40c4ee23d37c","file_rev_after":"2731acfa39bbb92c",
   "nodes":[{"anchor":"r-000042","change":"added",
             "node_rev_after":"639a2dca46f6fcc8","span_after":[26,248]}]}]}}
```

E4's delta, in full (every value computed):

```json
{"delta":{
 "seq":2,
 "fingerprint_before":"b3:10769ae1c77f5646750f3f52df2d055156b411145a02b8361ecd32af1357a1b7",
 "fingerprint_after":"b3:83b4ba591c0291d9f2a05428cac38e5820858fbb9c47720ab352344ddccc8f68",
 "actor":"agent:b0864fb2","now":"2026-07-18T20:33:41Z",
 "files":[
  {"path":"notes/plan.md","change":"modified",
   "file_rev_before":"a9794a262e67ed02","file_rev_after":"5f27a2814b517680",
   "nodes":[{"hpath":[{"h":"Goals"},{"h":"Q4"}],"change":"edited",
             "node_rev_before":"4b8bc385a58da0e0","node_rev_after":"f43203a1f0b4c9a3",
             "span_after":[75,150]}]},
  {"path":"receipts/2026-07-18.md","change":"modified",
   "file_rev_before":"2731acfa39bbb92c","file_rev_after":"9167b12b0eb13be6",
   "nodes":[{"anchor":"r-000043","change":"added",
             "node_rev_after":"c912d4578883f288","span_after":[249,473]}]}]}}
```

Laws: `seq` is a monotone per-workspace batch counter (the `changes_seq` of §10), **per-daemon-epoch** — a daemon restart resets it (memory is disposable and disk is markdown-only, §14, so no counter survives on disk to reload), which means `from_seq`/`changes_seq` catchup is valid only within one epoch and cross-epoch catchup is diff-by-root (§4.7), the root being the only restart-durable handle; file `change ∈ {created, modified, deleted, renamed}` (renamed carries `from_path`); node `change ∈ {added, edited, removed}`; node entries name the **deepest section containing each changed byte range** — ancestor section revs change implicitly (rev = span hash) and are re-readable via `toc`, never duplicated into the delta. External changes (a human editing in Obsidian) produce deltas with `actor`/`now` **absent** — the engine never invents identity or time it wasn't given; `seq` is assigned at detection.

### §7.2 Node-grain at birth

Grain = the node entry above: identity (hpath/anchor/fm_key echo), rev transition, span_after. This is the same vocabulary as toc rows and armed facts — one projection, three tenses (map / armed / changed).

### §7.3 Replay ≡ live

`diff(from_fingerprint, to_fingerprint)` returns the **byte-identical** Delta objects that were (or would have been) emitted as live notifications between those roots. There is no second diff dialect: catchup consumers and live subscribers parse one shape (`sub` carries Deltas in Notification frames, §4.7). History retention is the root-history ring, bound 256; a range outside it → `fingerprint_unknown` → full resync — degrades to re-derive, never to wrong data.

### §7.4 Delta grain — node-grain at birth (RULED)

Deltas are node-grain at contract birth — the grain §7.1–§7.2 define. The grain question is RULED; no `keys:[…]` slot work ships now. Key-grain arrives later, if ever, ONLY via the additive amendment path named here: node entries MAY gain an optional `keys:[{key, change, value_rev}]` sub-array once the frontmatter plane matures — old consumers ignore the unknown field (tolerant-client law), replay stays shape-identical, `diff` needs no new op. The path is named at birth precisely so the amendment stays additive; it is future-only.


## §8 Error taxonomy — six recovery classes

Every error frame carries `code` + `recovery` from the CLOSED six-class enum; each code is statically bound to exactly one class; a client that doesn't recognize a code dispatches on `recovery` alone. Loud everything.

| class | meaning | codes |
|---|---|---|
| `fix` | your request is wrong; change it | `bad_request`, `unknown_op`, `bad_path`, `no_match`, `not_unique`, `would_corrupt{lost}`, `ambiguous_ref{candidates}` |
| `env` | the world outside the workspace is wrong | `file_not_found`, `io_error{cause}`, `invalid_utf8`, `daemon_only` |
| `refresh` | your picture of a node is stale; re-read one thing | `cas_mismatch{expected,actual}`, `ref_not_found{stage,dest?}` |
| `retry` | transient; same request may succeed | `lock_timeout`, `stale_view{required,as_of_fingerprint,live_fingerprint}` |
| `resync` | your picture of the world is stale; re-plan | `fingerprint_mismatch{expected,actual,changed}`, `fingerprint_unknown` |
| `respawn` | the channel itself is broken | `bad_frame`, `unsupported_proto`, `internal` |

W4 dispositions: v1's `not_found` is **retired** — `file_not_found` (env: the file is gone) is distinct from `ref_not_found` (refresh: the name dangles), and `io_error` carries its cause. `ref_not_found.stage` makes the two-stage decomposition observable in every failure (1 = vault-namespace miss, no `dest`; 2 = subpath miss, `dest` present — §4.5). `budget_exceeded` is deliberately NOT here: it is a typed *finding* inside `verdicts` (§11), never a wire error. **`daemon_only`** (env class) is the one env code that names the engine's own deployment: it fires when a corpus-class rules pack — one whose WHEN needs the resident corpus name index (e.g. `link_resolves`, §11.2) — is loaded against a sidecar-mode engine that has no resident index, so the ruleset cannot run and is refused loud (the `BudgetClass::Corpus` law, §11.3); a single-file op never raises it, since every §4 op is served from disk bytes alone (§10.3). Null-id frames: §3.1. Three declared deltas from the ruled class table (previously undeclared, now fixed): `fingerprint_mismatch` rebound refresh→`resync` (a failed world guard invalidates the plan, not one node's picture — §5.1's split), `unsupported_proto` rebound fix→`respawn` (a protocol mismatch is a channel property; no request edit repairs it), and `bad_id` dropped (folded into `bad_request` + `id:null`/`id_raw`, §3.1 — one malformed-envelope code, not two). All three are behavior-preserving relabelings, now declared. Deviation-from-v1 rows: `not_found` retirement (this table), unknown-`kinds` rejection (§4.3) — each with its rationale at the cited section; the consolidated ledger is §18.

## §9 actor and now — wire inputs, never ambient

Rust never reads a wall clock and never reads env identity. `actor` (opaque string) and `now` (RFC 3339 string, format-validated, never generated) ride the wire as ordinary optional request fields on `splice` (recorded into receipts and Deltas) and as rule-evaluation inputs (§11 — temporal predicates read the *given* `now`). Absent inputs produce absent facts — the engine records nothing it wasn't told (worked: external-change deltas, §7.1). Whether a scope *requires* them is host policy (§5.3).

Ambient env identity is **not** engine law: hosts may mint `actor` from their own session machinery and place it on the wire; the engine never invents one.

## §10 Staleness posture — honest tense on every corpus read

### §10.1 The triple

Anything view-shaped — today `links`, tomorrow any corpus-wide fact op — declares three fields in every response: `as_of_fingerprint` (the fingerprint the answer was computed at), `live_fingerprint` (the fingerprint now), `changes_seq` (the Delta counter at `as_of_fingerprint`). The reader knows exactly what tense the answer is in; **no lag bounds are promised, ever** — the corpus mutates while you measure it (honest-tense law; +1 file mid-spike, observed).

### §10.2 The refusal (worked)

`require_fingerprint` is the opt-in strictness knob:

```json
{"id":81,"op":"links","path":"notes/plan.md",
 "require_fingerprint":"b3:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9"}
{"id":81,"ok":false,"error":{"code":"stale_view","recovery":"retry",
 "required":"b3:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9",
 "as_of_fingerprint":"b3:83b4ba591c0291d9f2a05428cac38e5820858fbb9c47720ab352344ddccc8f68",
 "live_fingerprint":"b3:83b4ba591c0291d9f2a05428cac38e5820858fbb9c47720ab352344ddccc8f68"}}
```

(The client demanded R0; the world is at R2.) `stale_view` is retryable, never silent.

### §10.3 View topology (structural)

Nothing on the agent path assumes SQL/DB access: every op in §4 is served from the world model (parse + hash of disk bytes). Facts API (this wire, agents) and any view face (humans) are two faces of ONE projection; deciding-from-a-view is a misclassified projection. Orientation surfaces (dashboards, counts, trees) are deliberately NOT wire ops — the mined kill/merge rows (`debt`, `domains tree`, `status`) stay dead (§16).

### §10.4 The rung-5 view organ — optional, wire-agnostic

Nothing on this contract **names DuckDB** (or any SQL engine). A view organ, if present, is implementation under §10.3: orientation is not a wire op. Live `Op::ViewPath` / `view.duckdb` paths are **implementation debt**, not design. Open: keep, reshape, or drop.

## §11 Rules as data — the compatibility surface

### §11.1 Verdicts in write responses

Every splice response (dry or real) carries `verdicts:[]` — typed findings from whatever rule packs are loaded:

```json
"verdicts":[{"rule":"blurb-required","severity":"warn","path":"notes/plan.md",
             "hpath":[{"h":"Goals"}],"span":[20,150],"node_rev":"5a8faa717fbcdb04",
             "message":"section has no blurb line"}]
```

Findings vs decisions: severity ∈ {error, warn, info}. On an **armed** workspace, block-severity verdicts and door-law violations **refuse the write in-engine** after CAS (`gate()`); see § A. Armed plane. On a never-armed workspace, verdicts stay advisory findings. `budget_exceeded` is a finding in this array. The shape is `crates/policy`'s `Violation{rule, severity, path, span, node_rev, hpath, message}` verbatim — the typed stub already matches this schema.

### §11.2 WHEN/HOW partition (validation-enforced)

A rule's WHEN sees world-model facts only — nodes, revs, spans, links, the given `now`/`actor`; its HOW is opaque data the engine never interprets (escalation, notification, remediation — host business). A pack whose WHEN references anything outside the fact vocabulary fails compile (`UnsupportedVocab`, existing `policy::CompileError` variant).

### §11.3 Pack manifest (generic, evaluator-free)

```yaml
id: wiki-hygiene
api: rulepack-api@1
budgets: { steps: 10000, mem: 4194304 }
fixtures: [fixtures/blurb-pass.md, fixtures/blurb-fail.md]
rules: [rules/blurb-required.md, …]
```

**Fixtures are the load gate:** a pack whose fixtures fail (under the declared budgets) is never admitted — a rule that cannot demonstrate itself does not run. Budgets are per-eval `{steps, mem}`, metered, exhaustion surfacing as the `budget_exceeded` finding. The perf harness for budget claims exists today (`crates/perfsuite`, claims-as-data → verdicts).

### §11.4 The rule language — Starlark, ratified

Rule predicates are fenced Starlark in literate rule pages, evaluated in-engine via starlark-rust; dialect + injected API pinned as `rulepack-api@N`; manifest as §11.3. All of that is **pack-level pinning** — it lives in `api:` and pack docs. The wire surface names no evaluator: verdicts, WHEN/HOW, budgets, and fixtures-as-load-gate are expressed identically whatever the pack pins — an evaluator change is a pack change, never a wire amendment.

## §12 The hash domain — md-only + `meridian/domain.md`

### §12.1 The domain

Which files' bytes enter the workspace **fingerprint** (merkle content hash):

1. **md-only floor** — only `*.md` files hash. Non-md paths never enter the domain.
2. **Default ignore (one rule)** — any path with a **dot-prefixed segment** is ignored (`.github/…`, `.obsidian/…`, `.trash/…`). Structural: custom re-includes cannot lift this floor for non-md or for the dot rule's intent on editor noise.
3. **Custom ignore** — optional rules on the **standing declaration page** `meridian/domain.md` (frontmatter carries `version` and `ignore` list; body may explain). Pattern semantics are gitignore-style (block list, last match wins, `!` re-includes).

**Hash domain ⊂ addressable domain:** an ignored `.md` path can still be `toc`/`cat`/`splice` by explicit path; its bytes simply do not move the fingerprint.

**Standing surface:** `meridian/domain.md` only.  
**Legacy filename (not design):** `mdfs_config.yaml` may still be *read* by the engine when it is the **only** domain config present (old workspaces). Do not create it; do not teach it. Two domain configs at once is an error (ambiguous domain), not a precedence rule. See `crates/fs/src/domain.rs`.

Worked counterfactual — a wrong ignore implementation cannot pass this fixture pair:

| Domain | Fingerprint (computed) |
|---|---|
| correct (`.github/` ignored) | `b3:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9` |
| wrong (`.github/README.md` included) | `b3:75a61c883e372102cfe7d75e94992b9be65e33fbe95956897a4cf2ea45bb8f1b` |

### §12.2 Interior encoding (merkle spec §4)

Leaf = blake3(whole raw file), full 32 B. Interior = blake3 over children sorted by raw name bytes, each `varint(len(name)) ‖ name ‖ type_byte(0x00 file / 0x01 dir) ‖ hash32`; empty dirs pruned; the workspace directory's own name never hashed. Full detail: `node-rev-merkle-spec.md`.

### §12.3 Domain-rule changes bump the prefix

An ignore-list (or algorithm) change re-defines the domain, so the token prefix advances: `b3:` → `b3a:` → … The domain `version` field rides **`meridian/domain.md`** with the ignore list so domain definition and prefix travel together. Worked at S2 with `drafts/tmp.md` present:

| Config | Fingerprint |
|---|---|
| v0 (drafts in domain) | `b3:05f0c6192308db5937c3e1352d1f9a6fc31b89b1a57175c8af6ce7903525aa4a` |
| v1 (ignore `drafts/**`) | `b3a:83b4ba591c0291d9f2a05428cac38e5820858fbb9c47720ab352344ddccc8f68` |

Same surviving hex can never compare equal across prefixes — receipts must not silently match a redefined domain.

## §13 Threat and limit register (the honesty standard, throughout)

1. **16-hex rev truncation:** ≈2^32 birthday work on attacker-fed content forges a rev collision. The mitigation is the trusted-local boundary — this wire serves local, trusted workspaces; any "adversary-proof" claim is dead. Full-width `fingerprint` and `file_rev`-over-whole-file are the honest escalation ladder for wider guarantees.
2. **Staleness:** no lag bounds, ever (§10.1). `as_of_fingerprint` may trail `live_fingerprint` at any moment; `require_fingerprint` is refusal, not synchronization.
3. **Storage re-probe posture:** on major dependency bumps (pulldown-cmark fork rebase, blake3 major, any ratified view engine's format), the conformance packs and measured envelopes are re-probed, never assumed to carry (extends the app-oracle re-probe rule to every measured claim).
4. **Conformance unknowns are pack-pinned, never asserted:** stage-1 duplicate-basename tie-break, embed depth-cap constant, app version drift — pinned as `obsidian-compat@1.12.7` answers at default settings, re-generated per app version via the live oracle (`obsidian eval`). The pack and its regeneration operating manual live in-repo at `crates/testsuite/data/gt/obsidian-compat/` (see pack notes under that path). Hand-frozen resolution fixtures are dead.
5. **Root-history ring bound 256:** older ranges answer `fingerprint_unknown` → full resync. Re-derive, never wrong data.
6. **Crash window in two-file batches** (§6.5): content-without-receipt is possible; loud via lint, recovered via re-root.
7. **No-self-rooting** (§6.2): receipts structurally cannot carry their own `fingerprint_after`.
8. **`_` block ids:** the ruled single charset (§2.4) makes any `_`-bearing id unaddressable as a strict anchor (loud `bad_request`). Empirically none exist in live corpora — the re-id migration is priced at zero; the phase-2 implementation package owns the forward-looking mint-guard plus the tournament-fixture exemption, and the app's own treatment of legacy-form ids stays pack-pinned, never assumed.

## §14 Repo grounding — speced ON TOP of meridian-rs

Sequencing below uses the **panel ladder** numbering (dialect → facts/check parity → integrity+diff → write/CAS → subscribe → policy packs); the repo's own rung comments differ (write/CAS is repo rung 2, subscribe repo rung 4) — one numbering, said once. What stands vs what changes, per crate:

| Crate seam | Stands | Changes under this schema |
|---|---|---|
| `crates/syntax` | the single entry `parse(&str) -> Vec<DialectNode>`, 11-variant dialect vocabulary, fork-pin law | nothing — implement the `todo!()` |
| `crates/wire` | all four noun newtypes; standing `Toc`/`Extract`/`Hello`; `Node` + kind ordinal; `ErrorBody` typed extras; strict/tolerant obligations | +`Delta` noun; +ops `cat`/`links`/`fingerprint`/`diff`/`sub` and reshaped `resolve`/`splice` via rung-freezing amendments; `ErrorCode` grows the §8 splits; `not_found` retired; §6.4 `Guard` op dropped |
| `crates/transport` | `NdjsonCodec` (implemented, tested), raw-id frame classification seam, null-id serialization test | raw-lexeme validation added at this seam (before typed decode) |
| `crates/transport-proto` | varint framing, `MAX_FRAME_BYTES`, wire-agreement drift pin | new oneofs per new ops (the agreement test forces this mechanically) |
| `crates/model` | `build`, richer NodeKind, no-serde law, **sealed `ValidatedSplice` capability discipline** (an unvalidated write cannot reach disk by construction), `merkle_root` seam | `SpliceRequest{span, if_node_rev, text}` reshaped to match-based (`target + match/put` — §4.4); `resolve` stays but serves the strict plane only; hash algorithm decided = blake3 (the "rung-2 wire amendment" the model doc reserved — §1) |
| `crates/fs` | `load`/`walk`/`apply_splice` seams, tmp+fsync+rename, no-storage law ("the moment memory can't be thrown away, the architecture has been violated") | `apply_splice` takes the batch (content + receipt append, one commit — §6.1); walk gains the §12 domain filter |
| `crates/wire-map` | `prefix_16b` (implemented, contract examples as passing tests), the one model+wire projection seam | `project` implements the superset-by-embedding predicates (§15) |
| `crates/policy` | `Violation`/`Severity`/`RulesetPin`/`CompiledRuleset`/`CompileError` — already this schema's §11 shapes; `authorize` stays deferred off the splice path (actor is a wire input, not an engine gate — §9) | `evaluate` output rides splice responses as `verdicts` |
| `crates/query` | `backlinks` seam | serves `links` with the §10 triple |
| `crates/sidecar` | `serve` loop (implemented), <300-line target | `dispatch` implements §4 |
| `crates/testsuite` / GT | consolidated test binary, GT pack + provenance | GT regenerated from THIS contract: lane pack demoted; resolution GT = app-generated `obsidian-compat@1.12.7`; every deviation row ships a fixture the deviated-from dialect FAILS (the discrimination law); `wsfix/` values join as fixtures |

The draft implementation plan sequenced against this table is a downstream deliverable; this section is its contract.

## §15 Structural guarantees index I — construct-level guarantees

*Every guarantee below is a standalone structural claim: what the contract guarantees, and the § where it is guaranteed. Which reviewer finding each one answers is recorded in the citation footnotes of the master.*

- **Every dialect construct is wire-representable, no lossy projection.** The `wire-map` projection seam is superset-by-embedding as law, restated as four wire-observable predicates: every dialect construct is representable (11-kind enum incl. Comment/InlineCode); wikilink information is carried whole; an unterminated fence surfaces as `unterminated` on the wire; frontmatter key order is preserved in `keys` (§4.1). Any divergence is a projection compile error, not a runtime loss (§14).
- **Ids are validated as raw lexemes before typed decode.** The raw-lexeme id law runs before typed decode, with the full discrimination set worked incl. `3e0` and the 2^53 boundary pair (§3.1).
- **Ground truth is regenerated from this contract, not hand-authored forever.** GT is regenerated from the contract; the lane pack is demoted; the discrimination law holds; all resolution GT is app-oracle `obsidian-compat@1.12.7` (§14, §13.4).
- **One rev per node, and client spans have no expressible form.** The CAS rule is explicit and singular: one rev per node, full-span bytes, re-derived at execution; `content_span` mints nothing; client spans are unrepresentable — the miswrite trap has no expressible form (§5.1).
- **The mint/walk partition is enforced as a type law.** The walk-plane response has no rev field; interop refs pay one toc hop; stale names fail `ref_not_found` (§2, §4.5).
- **One grammar per plane, advertise-nothing-resolve-nothing.** One block-id charset on both planes, stated once (§2.4); extract emits ⇔ resolve resolves; duplicate ids: mint refuses loud, walk stays app-silent (§2.1).
- **`not_found` is retired; each error class is statically bound to one recovery.** `file_not_found` (env) ≠ `ref_not_found` (refresh, +`stage`); `io_error{cause}` is split out (§8).
- **Op discovery is complete; there is no version sniffing.** Dotted `op.field` caps strings; the capability set is discovered whole; no version sniffing (§3.2).
- **The hash domain is md-only, config-declared, with a counterfactual root pair worked.** md-only domain + `meridian/domain.md`; one-rule default ignore; the hash domain is a subset of the addressable domain; a counterfactual root pair is computed; a domain-rule change bumps the root prefix to `b3a:` (§12).

**Foundational sweep — restated as standing guarantees:** span law is guarantor-restated as parser token discipline / reparse gate (§1); `node_rev` is MUST when `splice ∈ caps` (§3.2); the 16-hex width ships with the honest threat (§13.1); spans are newline-inclusive with no trim and the merkle-§5 pins live (§1); out-of-set values loud-reject (§4.3); splice is batch-only with one response shape (§4.4); per-kind info rides conformance receipts with open type strings (§3.2 tolerant-client; extract stands §4.3); the deviation-inventory law is adopted — "extract = what the app sees is TRUE except where a numbered row says otherwise; a rowless divergence is a contract bug" (§13.4 packs + §8 deviation rows); catchup entries are Deltas (handles + facts), never field-diff shapes, and the frontmatter plane is nodes with `#:key` dead (§7, §2.1).

## §16 Structural guarantees index II — usage-pattern coverage

*This index states, for every mined usage pattern, where the contract serves it structurally. **Matched** = this wire serves the pattern as a fact op; **Above-wire** = deliberately served by a consumer built on named wire facts, not by a wire op; **Dead** = removed, with the mined evidence as the justification. The originating usage inventory and its per-row counts are recorded in the citation footnotes of the master.*

| Pattern | Disposition |
|---|---|
| `check` | Above-wire: verdicts surface (§11.1) + toc/extract facts; the check *engine* is a rules pack, not an op |
| `run` | Above-wire: actor capability; wire serves fm-key facts for task blocks (§2.1) |
| `version` | Matched: `hello` proto + server + complete caps (§3.2) — binary-drift friction dead |
| `help` | Matched: `caps` complete set (§3.2) |
| `read` as oracle | Matched: `resolve` two-stage in-band (§4.5); the audit shape is `links` (§4.6) |
| `#Heading` reads | Matched: `resolve … content:true` (§4.5); never-silently-picks via strict-plane `ambiguous_ref` (§2.1) |
| `skill render` | Above-wire: actor capability; wire serves cat/extract |
| `rules ls` | Above-wire: pack manifest is data (§11.3); loaded-pack listing is a consumer surface |
| `schema` | Above-wire; the write half is fm_key nodes (§4.4 dry example) |
| `llm-wiki check` | Above-wire (SOP layer) |
| `encode` | Above-wire: pure grammar library; no wire surface (grammar defined once, §2) |
| `fix` | Above-wire mutation policy over `dry:true` + per-file batches (§4.4); see the mass-mutation friction row below |
| `debug` | Above-wire (rule dev tooling over §11 verdicts) |
| `attest` | Above-wire effects layer; dry seam + fm_key handles underneath (§4.4) |
| `mv` | Loudly alternativized: corpus move+link-rewrite is a composed consumer op — `links` (§4.6) + `fileToLinktext` emission algebra (the app's, not ours) + per-file splices; multi-file atomicity honestly absent (§6.5) |
| `status` | Dead as op; liveness is the daemon's; the change feed is `sub` (§4.7) |
| `watch` | Dead CLI; superseded by the Delta noun + `sub` reservation + recovery law (§7) |
| `resolve` CLI | Matched: `resolve` op (§4.5) |
| `append` | Matched: `put{at:"end"}` with full receipts — the worked append exchange (§4.4) |
| `toc` | Matched: `toc` with rev/write-kit per node (§4.1) |
| `pipe` | Dead; staged-commit value re-expressed as `dry` + atomic batch + `if_fingerprint` plan guard (§4.4, §5.1) |
| `edit-section` | Matched: THE flagship — `match` edit + CAS; conflict recovery costs one `cat` (§4.4, §5.2) |
| `set-prop` | Matched: fm_key splice (§4.4 dry example); `#:key` grammar dead (§2.1) |
| `chain promote` | Above-wire effects layer |
| `debt` | Dead (decaying, mined) — orientation, not action (§10.3) |
| `domains tree` | Dead — orientation (§10.3) |
| `rules check` | Dead as op; fixtures-as-load-gate does its job structurally (§11.3) |
| `def check/census` | Above-wire per vision; def-conformance = named amendment candidate riding `dry` |
| `domains show` | Dead — orientation |
| `def fix` | Dead |
| `cache stats/clean` | Dead as CLI; cache is implicit engine state (no-storage law, §14 fs row) |
| MCP `read`/`put`/`pipe` | Matched by vision: the MCP face is a wire client (§4.8 mapping is face-agnostic) |
| friction: mass-mutation whole-tree write (~1,347 files) | Structurally inexpressible: every splice names ONE path + explicit targets; no glob/whole-tree write grammar exists in §4.4 — the strongest mined gate, honored by omission |
| friction: JSON-arg dead ends | `caps` + loud echoing rejections (§3.2, §4.3) |
| friction: exit codes leak through pipes | In-band `ok` per correlated frame; the wire has no exit codes (§3.1) |
| friction: resolver/linter divergence | ONE resolver: the app's two-stage algebra, app-oracle GT; rules read the same facts (§4.5, §11.2) |
| friction: version drift mid-task | `hello` + complete caps in-band (§3.2) |
| friction: unregistered-check spam | An op is in `caps` or answers `unknown_op`; packs admit via fixtures or don't load — no ambient noise (§3.2, §11.3) |
| friction: no machine-readable check output | Frames-only stdout, logs stderr (§3.1); verdicts typed (§11.1) |

**Cross-cutting mined facts honored:** the zero-organic-rev reality is served by a permissive wire + ambient revs + a host policy ratchet (§4.4, §5.3); invisible in-process consumers become wire clients (the MCP row above; an existing engine-linked consumer is transition-tolerated only, ruled out by panel law); zero-use flags are not carried; unknown request fields reject loudly (§3.2) — with the corrected heading-read count honored via `resolve.content`.

## §17 Structural guarantees index III — top-level requirement coverage

*This index states where each top-level requirement of the contract is answered. The requirement labels are the last-gate checklist item names; their sourcing is recorded in the citation footnotes of the master.*

| Requirement | Where answered |
|---|---|
| Superset embedding + id/GT laws (A1) | §15 (nine construct-level guarantees + the foundational sweep) |
| One grammar, two planes (A2) | §2 (one grammar, two planes structurally; layering per the parity ruling) |
| Convergence-item closure (A3) | mint partition §2/§4.5 · rev 16-hex + threat §1/§13.1 · md-only + config §12 · resolve `from` two-stage §4.5 · app-oracle GT §13.4 · nine convergence items: raw-id-before-decode §3.1, dotted caps §3.2, error split §8, fm-plane §2.1, rev-MUST §3.2, batch-only §4.4, newline-inclusive §1, GT-regen §14, proto-retained §3.2 · hybrid interface §4.8 |
| Worked-value honesty (A4) | tool statement (header) + recompute from §0.3 fixture bytes; standing banner top |
| Usage-pattern coverage (A5) | §16 (the pattern index + cross-cutting facts) |
| Delta noun (A6) | §7 (delta at birth, stable shape, `diff` reserved §4.7, replay ≡ live §7.3, grain ruled node-grain §7.4) |
| Receipts (A7) | §6 (armed facts; intent names receipt address; receipts are md in the hash domain) |
| Actor/now as wire inputs (A8) | §9 + §1.1 replaceability table (per noun and field) |
| Rules-as-data (A9) | §11 (verdicts, WHEN/HOW, budgets+fixtures manifest, Starlark ratified §11.4, evaluator-free wire) |
| View topology (A10) | §10 (triple, `stale_view` worked, no lag bounds, optional wire-agnostic view organ — no engine-named wire elements) |
| Honest limits (A11) | §13 (register of 8) + honesty statements inline throughout |
| Repo grounding (A12) | §14 (stands/changes per crate seam; panel-ladder numbering declared; feeds the downstream impl-plan) |
| Skill doc + HTML page (A13, A14) | Downstream deliverables (skill doc, HTML page) — bound to this contract + this contract; not claimed here |

**Rulings attest:** the formerly open rows are ruled and folded — node-grain deltas (§7.4), Starlark (§11.4), optional wire-agnostic view organ (§10.4), the single block-id charset (§2.4); deviations and waivers are consolidated in §18. **Tool attest:** blake3 over this document's §0.3 fixture bytes; offsets from byte math; zero invented values.

## §18 Deviation & waiver ledger (fix-at-freeze)

The fix-at-freeze rule requires each reviewer-flagged debt fixed or waived with reason, here, never silently. Rows 1–5 are the winner-pick fix list; rows 6–7 consolidate the v1 deviations already declared in the body.

| # | Item | Disposition |
|---|---|---|
| 1 | Policy-scope grammar was unbound to the strict plane (ratchet scopes lived as host-side prose; the pack manifest has no scope field) | **FIXED** — §5.3 now binds scope grammar: `Path`-set selectors + strict-plane refs (§2.1), no second grammar; the manifest's scope-field absence is declared deliberate |
| 2 | The repo's reserved `fingerprint_mismatch` shape (`crates/wire` §6.5 reserved-codes note) carries extra fields `expected/actual/scope/changed`; this contract ships `{expected,actual,changed}` — the `scope` drop was unflagged | **WAIVED, declared** — the only world-grain guard is `if_fingerprint` (§5.1); no scoped-fingerprint construct exists for `scope` to describe. If a scoped world guard ever arrives by amendment, `scope` returns with it. The skill's ERRORS.md documents the same three-field shape — consistent |
| 3 | The frontmatter node's span `[0,20]` is terminator-inclusive, against the v1 §5.2 / merkle-spec §2 leaf-block law (exclude the final terminator) — previously undeclared | **WAIVED, declared** — the frontmatter node is a fence-to-fence container, span-lawed with the section (newline-inclusive) family, not the leaf-block family; the `fm_key` leaf inside it (`[4,15]`, §4.4) excludes its terminator, consistent with the leaf law. All hashes stand |
| 4 | Two silent rebinds vs the ruled failure-class table plus one dropped code | **FIXED, declared** — §8 now declares all three deltas with rationale: `fingerprint_mismatch`→`resync`, `unsupported_proto`→`respawn`, `bad_id` folded into `bad_request` + `id:null`/`id_raw`. Behavior-preserving |
| 5 | The base packet's A6 self-claim "replay ≡ live stated and tested" — nothing executable tests it today | **FIXED, restated honestly** — executed: the fixture recomputation behind every worked value (§17 tool attest). NOT executed: any replay ≡ live test, any conformance-pack run — both are impl-rung deliverables in the impl-plan (rung-4 test; GT regeneration). The word "tested" is retracted |
| 6 | v1 `not_found` retired | Declared at §8 with rationale (split into `file_not_found` env / `ref_not_found` refresh) |
| 7 | v1's frozen "unknown `kinds` match nothing" reversed to loud `bad_request` | Declared at §4.3 with rationale (the strict-server evolution law applied to values) |

The former row 8 (walk-plane charset/parity "collision") is **dissolved by the one-way-floor ruling:** parity is a one-way compatibility floor, not law, so a `_`-bearing anchor refusing loudly (§2.4, §4.5) is conforming behavior — there is no deviation to declare and no veto pending. Nothing else in this document knowingly deviates from ruled law; a deviation found without a row here is a contract bug (the assumption-audit law, §15).




---

## § A. Standing additions (compact)

These are **current law**, not optional history. Detail that only implements code may lag; the shapes below are what agents and hosts must learn.

### A.1 Fingerprint-or-force (every wire door)

Content-mutating writes on every **wire door** (daemon socket and sidecar stdio) require fingerprint match **or** `force`. Guard fields stay **schema-optional** (a guardless frame still **decodes**). A content-mutating write with neither fingerprint nor `force` is refused **after decode** as `guard_required` (recovery: `fix`) — semantic refusal, not a frame rejection. `force` is any client's refuse→rewrite path; MCP is not a separate trust plane. In-process paths (`mrd` without the wire door) are out of this ruling's reach by **scope**, not trust.

### A.2 Armed change plane (block is a feature)

When a workspace is **armed** (attested INDEX present), after CAS and before bytes land the engine runs `gate()` over the workspace's own armed set. Block-severity verdicts and closed door-law violations **refuse the write**. Never-armed workspaces stay advisory (verdicts as findings). `--force` escapes an armed refusal and is loud (journaled and rendered). Detail and bootstrap ladder: `armed-plane.md`.

**Essential refusal recovery bindings** (subset of the closed §8 taxonomy; full attestation suite is implemented against these classes):

| `code` | `recovery` | Typical trigger |
|---|---|---|
| `ambiguous_ref{candidates}` | fix | write selector matches more than one node |
| `ref_not_found{stage,dest?}` | refresh | pinned or named ref dangles |
| `no_match` | fix | selector or match string resolves to nothing / zero occurrences |
| `guard_required` | fix | content-mutating write without fingerprint or force (A.1) |
| `convention_fault{index}` | env | armed INDEX missing/corrupt on once-armed workspace |
| `armed_drift{armed_rev,report_rev}` | refresh | armed law drifted |
| `cas_mismatch{expected,actual}` | refresh | node or create/remove CAS failed |

### A.3 Composed `read`, `plan_edits`, `pin`, `create`, `hello.identity`

| Surface | Role |
|---|---|
| `read` | Addressing + content + render at one snapshot; section selectors use §2.1 segments (or anchor / dewey). Not a joined string address. |
| `splice.plan_edits` | Plan-level batch shapes; addresses are **segment arrays**. |
| `splice.pin` | Pin rides the write choke-point; selector is segments/anchor. |
| `create` | File birth through the guarded door; full body bytes. |
| `hello.identity` | Optional `{build: sha|unknown}` for deploy identity. |

**Composed-`read` selector resolution (2026-08-06, dogfood F4–F6):**

- A section selector matching **more than one** node refuses `ambiguous_ref` naming each candidate's machine address (its `n`-carrying segment array) — §2.1's "the strict plane never silently picks" applies to strict reads exactly as to `cat` and `splice`. Never a silent first match, never `ref_not_found`.
- When **all** selectors fail, the refusal names **every** failed selector with its own reason (no match / ambiguous), symmetric with the partial-read `notice`, which names them the same way.
- Refusal **remedies speak the operation, not one host's tool name**: the recovery clause names the toc read in each surface's own dialect (MCP `mode:"toc"`, CLI `--section`-less read) and never prescribes a binary the caller may not have.

**Door symmetry over duplicate headings (2026-08-06, fix-write-dup-symmetry):**

- An `n`-less address that matches more than one node refuses `ambiguous_ref`-class at **every** door — read and write alike (`splice.plan_edits`, and any host lowering onto it). No door may pick an occurrence the caller did not name: the write-door refusal names each candidate's machine address (its `n`-carrying segment array) and teaches `n`, the same evidence the read door gives. Two doors, one answer — a selector one door refuses as ambiguous, no other door resolves.
- The published loop is untouched: addresses the read face publishes carry `n` exactly where the document is ambiguous, so read → verbatim address → write always lands.

CLI inventory (descriptive): `status.md`. Cross-root agent address grammar: `address-grammar.md`. Config parse: `meridian-md-schema.md`.

### A.4 What this document does not teach as core

- Joined hpath strings as machine addresses  
- `mdfs_config.yaml` as the domain config (use `meridian/domain.md`)  
- SQL / `view_path` / DuckDB as agent path  
- Dual wire constitutions ("v2 vs v3" for agents)

---

## § B. Process

1. Edit this file (or the relevant SPEC under `docs/`) **before** code.  
2. Do not reintroduce versioned contract files or amendment piles.  
3. Optional history only: `worker-log.md` (deletable).  
4. **UNVERIFIED** when evidence is missing.
