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

Workspace `wsfix/` — three timeline states. State **S0** is exactly these three files on disk:

```
notes/plan.md            136 bytes   file_rev e3c4acaceb75b907
receipts/2026-07-18.md    26 bytes   file_rev 920a40c4ee23d37c
.github/README.md         11 bytes   (md, but OUTSIDE the hash domain — default ignore, §12)
```

**`meridian/domain.md` is ABSENT at S0.** It is not a member of the S0 set and R0 does not cover it: R0 is the fingerprint of `notes/plan.md` and `receipts/2026-07-18.md` alone. The file is markdown and **hashes itself** when present (§12.1 rule 3; `crates/fs/src/domain.rs` — "the file that defines the attested surface is itself attested"), so writing it to disk moves the fingerprint off R0. The §12.3 domain-bump example is the only place this document puts it on disk, and its bytes are printed below **because §12.3 hashes them there**. Printing a file's bytes in this section never makes it an S0 member — the distinction is load-bearing: reading it the other way is a fingerprint change, not an editorial one.

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

The remaining fixture bytes (every file this document hashes is printed — at S0 in this section, and the S1/S2 receipt entries in §6.3, whose two lines ARE the fixture's bytes since the 2026-08-09 template rebaseline. The declared exception §18 row 10 carried is CLOSED; the row records how it closed):

- `receipts/2026-07-18.md` at S0, exact bytes (26 B — the `—` is 3-byte UTF-8): `# Receipts — 2026-07-18` + LF.
- `.github/README.md`, exact bytes (11 B): `# CI notes` + LF.
- `drafts/tmp.md` (appears only in the §12.3 domain-bump example), exact bytes (8 B): `scratch` + LF.
- `meridian/domain.md` — **absent at S0**, written only by the §12.3 example, in the two forms printed below.

`meridian/domain.md` **v0**, exact bytes (33 B) — a `version` and no custom ignore list:

```markdown
---
version: 0
---
# Hash domain
```

`meridian/domain.md` **v1**, exact bytes (57 B) — the same page carrying a custom ignore list:

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

NDJSON, one JSON object per line, on the one wire door — the daemon's unix socket (§3.3): the socket carries frames only, logs go to the daemon's stderr (`echo '{"id":1,"op":"hello",…}' | nc -U "$SOCKET"` debuggability is a contract property — the pipe test outlived the sidecar's DROP because the socket speaks the same line dialogue). Three frame types, classified by the **raw** `id` key:

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
{"id":1,"ok":true,"body":{"proto":1,"server":"meridian-daemon/0.1",
  "caps":["toc","cat","extract","resolve","resolve.content","links","links.require_fingerprint",
          "splice","splice.if_node_rev","splice.if_fingerprint","splice.dry","splice.receipt",
          "splice.verdicts","fingerprint","diff","sub"],
  "fingerprint":"b3:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9"}}
```

`caps` is the complete set — no version sniffing, ever. Field-only amendments ship as dotted `op.field` strings. `fingerprint` in the hello body is optional (the engine may not have walked yet); when present it is the first ambient fingerprint.

**Rev-presence law:** `node_rev` is MUST on every `toc`/`cat`/`extract` node whenever `splice ∈ caps`.

**Evolution:** strict server — unknown request fields and unknown enum values in requests are rejected loudly; tolerant client — unknown response fields and unknown open-kind strings are ignored. Server-first rollout.

**Grain of the strict wall:** "request fields" means **every object in the request, at every depth** — the top-level op object, each `edits[]` edit object, its edit-shape body (`match`/`put`), its `target`, and each hpath segment. The wall is loud at each grain and names the legal field set it checked against. This is not a nicety: a guard field lives on a *nested* object, so a decoder strict only at the top level drops `if_rev` (a typo for `if_node_rev`) and **silently converts a guarded write into an unguarded one** — the guard-you-believe-is-armed trap this law exists to kill. The rule binds every door that decodes a request, including the CLI seam that reads the `edits` value off stdin (§4.4).

### §3.3 Hosts — one wire door

**RULED — DROP (ZT, 2026-08-06, session `06-00-adhoc`).** The stdio sidecar host (`crates/sidecar`, deployed `ccc-sidecar`) is deleted; the daemon's unix socket is the **only** wire door. ZT, verbatim: *"there is no reason to have sidecar ever existed. debugability is lie, sending data over socket do the same job."* This executes R3b ("sidecar death row", session `05-19-meridian-socket-mcp-leg`), whose precondition — ccc-statusd's markdown ops moved off the exec'd sidecar onto the registry socket — shipped 2026-08-05.

- The socket speaks the same NDJSON line dialogue (§3.1), so pipe debuggability transfers whole; a second binary bought no observability the socket lacks.
- Identity was never the door's job: `actor` rides each frame as data (§9) on either transport, so removing a door removes no attribution. One resident server, connection-scoped transport, per-frame identity — the topology herdr (single UDS) and shellkit (identity injection at the server) already run.
- `daemon_only` (§8) retires with the host: every wire door is now daemon-backed, so no wire deployment lacks the resident corpus index.
- In-process paths (`mrd` over the engine crates) remain out of wire scope (§ A.1) — a CLI is not a wire door.

Consequences are threaded at §3.1 (pipe debuggability restated at the socket), §3.2 (hello `server` names the daemon), §8 (`daemon_only` retired), §13 (crate row), § A.1 (one-door enumeration).

## §4 The op surface

Eleven ops in this table (the § A.3 standing additions `read` and `create` land on top, not re-tabled here). The five-verb interface maps onto the original ten 1:1 (§4.8). Read ops are classified by the wire-op criterion: feeds-an-action → wire fact op; feeds-orientation → dashboard-only, NOT on this wire.

| Op | Rung (panel ladder) | Class |
|---|---|---|
| `hello` | 1 | discovery |
| `toc`, `cat`, `extract` | 2 | single-file facts — the **mint surface** |
| `resolve` | 2 | walk plane — never mints |
| `links` | 5 (view-shaped) | corpus fact — staleness triple (§10) |
| `splice` | 4 | the ONLY write op, batch-only |
| `check_write` | 4 | write pre-flight — the splice verdict standalone, read-only (§ A.3) |
| `fingerprint` | 3 | integrity fact |
| `diff` | 3 (reserved shape, standing) | replay (§7) |
| `sub` | 5 | delta transport (§7) — SERVED at the daemon door (§4.7) |

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

**Teaching row — displayed bytes are not hashed bytes.** A face rendering from `content_span` shows FEWER bytes than the row's `node_rev` covers: Q3 above displays 17 content bytes while its rev hashes the full 23-byte span. The same split runs at the anchor grain in the other direction — a rendered block line elides the trailing ` ^id` that the leaf span carries and the rev hashes. So a caller that diffs what it was shown against what was hashed mismatches by exactly the heading prefix or the id suffix. Compare against `cat`'s full span bytes (§4.2), never against a display column.

**The anchor toc row, worked** (`receipts/2026-07-18.md` at S1 — the only block-id-bearing file in the fixture; every value computed): the plan.md toc above shows no anchor row because no plan.md section carried a block id at toc time, so the "anchors with their revs" clause is worked here instead.

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

The `^r-000042` block echoes as a `list_item` node keyed by its `anchor` ref (§2.1) carrying its own `node_rev` over the block-leaf span (terminator excluded — `[26,286]`, byte-identical to the receipt facts armed in §4.4; §6.3's E3 line IS these 260 bytes since the 2026-08-09 rebaseline, no longer illustrative shape — §6.3, §18 row 10); the lone top-level heading spans the whole file, so its `node_rev` equals `file_rev` (`51ad6428f5b5a898`). An anchor becomes a write target by the same one-hop path as a section.

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

Batch laws: batch-only, ONE response shape; all targets and guards resolve against the **pre-batch** state; the edits' **replaced regions** must be pairwise disjoint (`bad_request{"overlap":…}` otherwise, and the refusal names the offending edits and a remedy). The replaced region is what the edit rewrites — `match` the matched bytes, `put at:"all"/"content"` that span, `put at:"end"` the zero-width insertion point — so edits whose *targets* nest compose legally when their regions touch different bytes: an append to a section plus a sibling-section birth under its parent is ONE batch. Zero-width regions at the same byte are disjoint and apply in request order. The batch commits atomically through one reparse — a post-apply parse in which some identity does not survive refuses `would_corrupt`, in the two families below. `dry:true` runs everything except disk: same response shape, `fingerprint_after:null`, no receipt written. *(Amended 2026-08-06: the disjointness grain was previously the target's full span, containment included — which refused any mixed append + section-birth batch under one tree, contradicting the batch-only law's own premise. The overlap refusal's `bad_request{"overlap":…}` extra is unchanged.)* *(Dogfood F8 disposition, verified 2026-08-06: a same-region double-replace — two `replace_section`s on one section — refuses overlap; the replace + `at:"end"` pair that sequenced is a zero-width insert at the region boundary composing legally — this grain working, not a missed refusal.)*

**The `would_corrupt` families (amended 2026-08-09).** One code covers the post-reparse armed-facts deaths, and the refusal body discriminates them with `family` — a caller dispatches on `family`, never on which extras happen to be present:

| `family` | Extras | What died | The remedy the refusal teaches |
|---|---|---|---|
| `containment_lost` | `lost:[hpath…]` | a section **byte-disjoint from every edit** no longer resolves after the reparse | computed from `cause` (below) |
| `target_identity` | `target` (the offending edit's ref, §2.1 grammar) | the **edit's own target** no longer resolves after the reparse, so its armed facts are unrepresentable | re-supply the identity the slot destroys — a section heading for `at:"all"`, a line-final block id for `at:"end"` on an anchor; to retire an identity, write through the parent's content slot |
| `transition_unrepresentable` | `target` (the offending edit's ref, §2.1 grammar) | the edit **wrote past the span it named** — bytes it placed fall outside the target's post-batch span, so the node the edit addressed never received them and its `node_rev` cannot move | drop the trailing separator from `text`, or aim the write at the enclosing section, whose span contains the bytes you meant to add |

**Why `transition_unrepresentable` is a corruption family and not a reporting nicety (added 2026-08-09).** `node_rev` is defined as a function of the node's span bytes (node-rev-merkle-spec §2), and §4.4 makes the target's span the region an edit rewrites. A node whose span **excludes its line terminator** — every anchor block-leaf (§1) and every `fm_key` leaf (§4.4) — therefore has its own extent END on that terminator, so a `text` carrying a separator there writes a byte the node never covers. Arming that commit states a transition that did not happen, and it silently disarms the caller's guard: `if_node_rev` then compares a value the write can never move, so two callers holding the same rev both write, both succeed, and neither is told. Repeated writes land at one fixed offset and accumulate — appends in REVERSE order, rewrites as a blank line per run.

**The family is keyed on the MECHANISM, never on the `at:` scope (ruling `decisions/0018`, 2026-08-09).** The test is one sentence: **a real byte change whose target's `node_rev` did not move is refused.** Containment — the bytes an edit writes lying within its target's post-batch span — is the EXPLANATION of that test and not the test itself, and the two are not equivalent. A write MAY place bytes outside its target and still move that target's rev: an `at:"end"` section append whose `text` opens a sibling heading shrinks the section and grows it by the separator, and that write is truthful, its guard is live, and nothing is owed. What cannot stand is a changed file over a rev that did not move — the node never received the bytes at all, `if_node_rev` then compares a constant, and two callers holding that rev both write, both succeed, and neither is told. A caller MAY NOT hand a terminator-excluding leaf a `text` that ends in a separator — the two readings that would have permitted it converge on this same refusal, because a rev whose bytes are untouched by construction cannot move, so "let it commit but move the rev" is not an executable outcome.

⚠️ **Scope-keying this family would be a defect, and that is measured rather than argued.** The same escape is reachable through `put{at:"end"}`, `put{at:"all"}`, `put{at:"content"}` **and `match`** — and `match` is not an `at:` scope at all, so any guard enumerating scopes misses it by construction. Six cells escaped at v1.0.0 across the anchor-leaf and `fm_key` doors; only the containment and identity families sit ahead of this one, and all three are measured on the same single reparse — never inferred from the edit text.

`containment_lost` carries one more discriminator, `cause`, because the containment refusal is the one place where two unlike mistakes produce the same lost hpath:

| `cause` | What the reparse shows | The remedy the refusal teaches |
|---|---|---|
| `heading_destroyed` | the lost section's heading line no longer parses as a heading at all | carry your own newlines — `at:"end"` is raw byte concatenation, so text that runs up against a following heading must end with `\n` |
| `reparented` | the heading still parses, at its own level, but its ancestry moved — so its hpath no longer resolves | the text you wrote introduces a heading at a level that adopts the following sections; deepen that heading's level, or aim the edit at the parent whose subtree you meant to rewrite |

`cause` is **absent** when the lost sections do not share one cause, and absent from the `target_identity` family entirely. **A refusal never teaches a remedy for a cause it did not measure:** with no `cause` the refusal names what would be lost and stops, rather than emitting a fixed remedy string that may misdiagnose. *(Amendment rationale, measured against the v1.0.0 artifact: `target_identity` previously served generic `bad_request` — one documented code for two families, indistinguishable at the caller — and the containment remedy was hardwired to the `heading_destroyed` cause, so a `reparented` refusal taught a fix that could not repair the batch that drew it. Both codes are recovery class `fix` (§8), so the discriminator refines within one class and no client's dispatch-on-`recovery` path changes. No §18 row is owed: the engine moves to this text in the same change, so nothing stands deviant.)*

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

**Teaching row — `at:"end"` on an anchor ALWAYS refuses; two families split which way it dies (family split measured 2026-08-09).** A block-leaf span excludes its terminator, so the insertion point sits on it, and every end-append to an `{"anchor":id}` target lands one of two refusals — never a commit:

| the `text` you send | what the reparse measures | `family` |
|---|---|---|
| carries no newline (` tail`) | the appended bytes join the line, so the id is no longer line-final: the target stops resolving | `target_identity` |
| carries a newline (`\nX`) | the first newline terminates the host line, so the bytes land in a NEW line outside the node: the target still resolves and its rev cannot move | `transition_unrepresentable` |

The remedy is not a repair to reach for occasionally — an append to an anchor must re-supply the id line-final in its own `text`, every time, and an append that means to add a LINE belongs to the enclosing section, not to the anchor. *(Correction rationale: this row previously read "the `target_identity` family is the WHOLE of `at:"end"` on an anchor". The refusal it required stands and is unweakened; its single-family attribution was wrong, and the newline half was measured COMMITTING at v1.0.0 `93184797` and at `b1fcc6e3` — silently, exit 0, with a null rev transition. The same escape was measured on `fm_key` targets, whose leaf span excludes its terminator by the same §4.4 law, so the family is defined over the SPAN LAW rather than over the anchor door. Host block kind is NOT the discriminator: paragraph, list-item and heading hosts all behaved identically once the `text` was held fixed.)*

The response carries what the write **ARMED** — target identities, rev transitions, spans after, the receipt fact, the root transition — never delivery claims. `verdicts` is the rules-as-data surface (§11). Spans appear in *responses* freely: the wire's business, never argv's.

**Teaching row — an armed row echoes the node's BATCH transition, never an intermediate.** Two edits on one node both serve the same `node_rev_before`, `node_rev_after` and `span_after`: the node's pre-batch and post-batch state, repeated per edit. Guards resolve against the pre-batch state, so this is the same law read from the response side. A consumer that counts distinct rev transitions by counting armed rows over-counts on a same-node batch; count distinct `node_rev_after` values instead.

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
 "fingerprint_before":"b3:6e866e13b5e65ef9961c050f8a621cf1980b00ee293be650deef5f4dbc6823f0",
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
{"id":72,"ok":true,"body":{"dest":"receipts/2026-07-18.md","span":[0,550]}}

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
 "as_of_fingerprint":"b3:6e866e13b5e65ef9961c050f8a621cf1980b00ee293be650deef5f4dbc6823f0",
 "live_fingerprint":"b3:6e866e13b5e65ef9961c050f8a621cf1980b00ee293be650deef5f4dbc6823f0",
 "changes_seq":2,
 "files":{"notes/plan.md":{
   "resolved":{"receipts/2026-07-18.md":1},
   "unresolved":{"roadmap":1}}}}}
```

Shape mirrors the app's `resolvedLinks`/`unresolvedLinks` — per-edge counts; dangling refs first-class. `path` absent → whole-corpus edge map. Opt-in `require_fingerprint` → `stale_view` refusal (§10.2).

**`path` present is a DOOR; `path` absent is an ENUMERATION, and they answer under different rules (§12.1).** Named, the op serves the page even when the hash domain excludes it — a real file outside the domain comes back with its edges resolved against the corpus it is not in, and only a path with no file under the root is `file_not_found`. Absent, the op speaks for the whole corpus and carries **`excluded`**: the workspace-relative markdown under the root that the hash domain does not hold, absent from `files` and named here rather than left to be inferred (§12.1 enumerator clause). The key is omitted when the list is empty, so a workspace whose domain is its whole md tree is unchanged on the wire.

### §4.7 fingerprint and diff — the integrity rung

```json
{"id":90,"op":"fingerprint"}
{"id":90,"ok":true,"body":{
 "fingerprint":"b3:6e866e13b5e65ef9961c050f8a621cf1980b00ee293be650deef5f4dbc6823f0","seq":2}}
```

`diff` is reserved AT the integrity rung with its shape standing now — the compound front door:

```json
{"id":95,"op":"diff",
 "from_fingerprint":"b3:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9",
 "to_fingerprint":"b3:6e866e13b5e65ef9961c050f8a621cf1980b00ee293be650deef5f4dbc6823f0"}
{"id":95,"ok":true,"body":{"batches":[ /* Delta seq 1, Delta seq 2 — §7, byte-identical
                                          to the live notification frames */ ]}}
```

Replay ≡ live (§7.3). A fingerprint range outside the retained history → `fingerprint_unknown` → full resync (honest bound: §13.5). `sub` (rung 5) is **served** at the daemon door (`crates/registry/src/server.rs:1193-1213`): `{"op":"sub","from_seq":N}` → ack `{"root":…,"seq":N}` — the baseline root, so the first push frame's `root_before` matches — then the connection converts to push and carries Notification frames, each one Delta batch. A `from_seq` outside the retained ring refuses `root_unknown` with the diff-by-root catch-up remedy; a refused `sub` leaves an ordinary request channel. The delta stream is not actor-scoped: identities, revs, and spans only.

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

The default receipt line, byte-exact (segment target only) — these are the fixture's own S1/S2 receipt bytes, not an illustration of them (rebaselined 2026-08-09; see the arithmetic below):

```markdown
- splice notes/plan.md id=42 actor=agent:b0864fb2 now=2026-07-18T20:31:04Z fingerprint_before=b3:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9 edits=1 target.hpath=[{"h":"Goals"},{"h":"Q3"}] match 33d5b0e1b27cb48b->41f643f034e5681f ^r-000042
```

E4 append via `put{at:"end"}`:

```markdown
- splice notes/plan.md id=57 actor=agent:b0864fb2 now=2026-07-18T20:33:41Z fingerprint_before=b3:7f3b44376c719be236279e168c22fa2f4d346cd6e5da5bcf0784adb72e7c1f12 edits=1 target.hpath=[{"h":"Goals"},{"h":"Q4"}] put:end 4b8bc385a58da0e0->f43203a1f0b4c9a3 ^r-000043
```

**Do not re-teach** pretty joins (`Goals>Q3`). Template replaceability (D-C10) is not permission for a second address grammar.

**Byte arithmetic, and the two lines above now close it (REBASELINED 2026-08-09).** The two lines measure **260 B** and **262 B** on the node basis, and those are the widths the fixture's own S1/S2 receipt entries carry: the spans `[26,286]` and `[287,549]` (§4.4, §7.1) and the receipts file's 26 → 287 → 550 B growth (§0.3) require exactly these numbers, terminator excluded per the leaf-block span law (§1). **The gap between this section's lines and the fixture's bytes is 0 B.** R1, R2 and every S1/S2-anchored value are therefore reconstructable from this document — the printing debt §18 row 10 declared is closed, and the row records how.

**What moved, so the closure is auditable rather than asserted.** Until this rebaseline the template wrote two deviations from this document's own law, worth 38 B per line: `root_before=` where §6.1's standing noun is `fingerprint_before=` (**7 B**), and the target joined as `Goals>Q3` where §2.1 form is mandatory (**31 B**) — the pretty join this section forbids in its own words two paragraphs up. Closing them moved receipt bytes, which is why it was a FIXTURE act and not an edit to this section: receipts are ordinary markdown inside the hash domain (§6.1), so the receipt node rev moved, the workspace fingerprint moved with it, and R1 and R2 were recomputed rather than re-typed. The superseded values are not scrubbed — they are the pre-rebaseline R1 `b3:10769ae1…` and R2 `b3:83b4ba59…`, recorded here and in §18 row 10.

**The lane width — what the shipped template writes with no request id (2026-08-09).** Replaying E3 **through the CLI lane (`mrd put`)** under this section's own `actor` and `now`, so deterministic, the template writes **254 B node bytes** (**255 B line**, terminator included — the form the dogfood step reports, s15-70). Every width COMPARED in this section — 260/262 and the 254 — is **node bytes, terminator excluded, per the leaf-block span law (§1)**; the line figure carries its own label because the two bases differ by exactly the terminator, and mixing them is what made this arithmetic wrong by 1 B until it was measured on both.

**The 254 is a LANE width, not the template's.** The **same shipped template writes the fixture's 260 B exactly** when the request carries an id — gated byte-for-byte, not inferred, by `crates/receipt/tests/frozen_receipts.rs :: e3_receipt_line_byte_exact`, which renders `id: Some(42)` and asserts both this section's E3 line and `line.len() == 286 - 26`. A CLI invocation is not a wire request and mints no request id, so §9's absent-inputs law makes writing no `id=` token the **correct** rendering, not a defect. **The 254 and the 260 are two lanes of one template, never two templates.**

One gap remains, and it is ruled rather than owed:

| gap | width | causes |
|---|---|---|
| shipped CLI lane 254 → fixture / this section 260 | **6 B** | `id=` alone — **ruled, not a defect** (§9, lane) |
| fixture 260 → this section's lines 260 | **0 B** | closed by the 2026-08-09 rebaseline (was 38 B) |

`254 + 6 = 260` closes to the byte. **The consequence of the surviving 6 B is unchanged and stays stated:** R1 and R2 are WIRE-lane values, so replaying the fixture through the CLI lane still cannot land on them — the receipt node differs by the `id=` token alone, and a different receipt node is a different fingerprint. That is a lane mismatch, not an engine shortfall, and it is why the CLI lane's own timeline is not the published one.

**The target form is written by the TEMPLATE, and its escaping is §6.7's.** Every `[`, `]`, `{`, `}`, `"`, `:` and `,` in `target.hpath=…` above is template text; only the heading text is interpolated, through the segment renderer §6.7 rule 2 mandates. `Goals`, `Q3` and `Q4` are receipt-identifier text carrying no `"`, so the escape is the identity on them and the widths above are unmoved by it.

### §6.4 Replaceability (D-C10 — facts are the contract)

The md *rendering* is a shipped default template; a non-ccc consumer replaces it freely — the normative receipt content is the armed-fact set, defined by the wire response shape. The engine mechanism is generic: "append these facts at this address with this anchor". "No intent past-due without a receipt" is lintable: a rules pack can assert every splice-bearing transcript row has its receipt anchor resolvable (§11).

### §6.5 Crash honesty

The batch writes two files via tmp+fsync+rename each; a crash between renames can land content without receipt. Recovery is re-derive (cold rebuild → correct root, never wrong data) and the missing receipt is exactly what the lint finds — the failure is loud in the world model, not hidden in engine state. Stated as a limit (§13.6); multi-file atomic commit is a rung-3 amendment candidate, not assumed.

### §6.6 The anchor is the caller's to mint, and a mint that collides is a defect (2026-08-09)

§6.4 states the mechanism — "append these facts at this address with this anchor" — so the anchor arrives from the caller and the engine appends what it is given. That leaves one obligation unstated until now, and a host met it wrongly in the field (dogfood 2026-08-09, s13): **an anchor MUST be unique within the receipt file it names.**

The obligation is not stylistic. §6.1 promises a receipt resolvable via `#^anchor`; §2.1 resolves an anchor ref by exact block id and A.3 refuses `ambiguous` when a file carries the id twice. A writer that repeats an id inside one file therefore publishes a receipt no strict door can address — the receipt exists, is hashed, and is unreachable. **Published-but-unusable, minted by the writer, in obedience to no rule it broke.**

**The engine polices the anchor of the write in front of it (amended 2026-08-09, dogfood pass 1 f02).** This section first said the engine polices nothing here, because an append carries no cross-invocation memory. That holds for anchors a caller mints as ordinary content — the engine has no memory of them and never inspects them. It does not hold for the REQUESTED receipt anchor: the receipt file is named by the request and read in the same act, so a collision between that anchor and the bytes on disk is visible before anything is written. **The splice door resolves the requested receipt anchor against the receipt file FIRST, and refuses `bad_request` with zero bytes moved when the anchor already stands there** — the byte-untouched shape every other target refusal at this door already has. The caller re-sends under an unused anchor; because nothing landed, the re-send appends its content exactly once.

The red this closes, measured at the release pin and again at `b1fcc6e3`: a `put --receipt <file>#<anchor>` naming an anchor the receipt file already carried committed BOTH files, minted the duplicate, then failed to resolve its own anchor and reported *"committed receipt anchor did not resolve — receipt corrupt"*. The refusal had already written — zero armed facts over two changed files — and the `recovery:"fix"` it taught duplicated the caller's content on the re-send. **The engine detected its own §6.6 violation after committing it and reported the symptom;** the ordering is the fix, not the message.

The derived rule for a host that appends many receipts to ONE shared file across invocations: **derive the anchor from the invocation identity, never from a counter that restarts.** A per-invocation counter (`^r-000001`, `^r-000002`, …) is unique only within its own process; the second invocation against the same file re-mints the first id. Derive from a monotonic file-scoped counter (read the file, continue past its last id) or from the caller's invocation id plus an in-run sequence — the second costs no read and is what `mrd run` already does (`r-<invocation-id>`).

A host that derives from a caller-supplied id inherits that id's charset duty: the mint routes through the block-id door (`[A-Za-z0-9-]`, §2.4) and refuses loudly rather than publishing an unaddressable anchor.

### §6.7 The rendering law — structure comes from the TEMPLATE, never from the data (2026-08-09)

§6.4 makes the md rendering replaceable. It does not make it free. Every template that renders the armed facts into markdown carries one obligation, and it is a §6.1 obligation rather than a stylistic one: a receipt is ordinary markdown inside the hash domain, so a line that READS as a different structure than the facts it carries is a receipt that misreports the write — the facts were armed, the bytes say something else. Heading text is user content; a receipt whose structure can be moved by user content is not a receipt.

Two rules discharge it, and both answer the same question: which bytes belong to the template, and which to the data.

**Rule 1 — every interpolated value is escaped.** A value stands verbatim only if every character is *receipt-identifier text*: ASCII graphic, minus `[` and `]` (no wikilink or embed can form), minus the backtick and the backslash (spent as escape delimiters). The charset excludes whitespace and line endings by construction, so no token boundary and no row boundary can be forged. Anything else renders as an inline code span with out-of-charset characters escaped `\u{…}` and `\` doubled — reversible, so the value is preserved exactly (§5.2).

**Rule 2 — where the template writes §2.1 JSON, the punctuation is TEMPLATE bytes.** §6.3 mandates the segment form for target identity (`target.hpath=[{"h":"Goals"},{"h":"Q3"}]`). Every `[`, `]`, `{`, `}`, `"`, `:` and `,` in that form is written by the template. Only a segment's heading text and its occurrence index `n` come from the data, and the heading text goes through the **segment renderer**, which is a different renderer from rule 1's:

- it emits the BODY of a JSON string and never its quotes;
- a character stands verbatim only if it is receipt-identifier text AND is not `"`;
- everything else — the double quote, the backslash, the brackets, the backtick, every space, every control character, every non-ASCII character — becomes a JSON `\uXXXX` escape (surrogate pairs above U+FFFF).

The output therefore carries no `"`, and no backslash that does not open a complete six-byte escape. The JSON string cannot be closed early, the object cannot be extended with forged keys, and no markdown structure can form inside it. The escape is JSON's own, so a strict parser over the rendered array returns the original segments byte-for-byte — the receipt's target stays machine-readable as the §2.1 address it echoes.

**Why the segment renderer must exist separately, stated so it is not re-derived as redundant.** Rule 1's charset excludes `[` and `]`, so routing a whole JSON array through rule 1 escapes the array itself into a code span — the address stops being an address. The brackets must come from the template, and that hands the template the quotes as well. But rule 1's charset **permits** `"`. A heading carrying a double quote, interpolated between template quotes, closes the string early and forges the object: the receipt then names a structurally different target than the write actually took, which is precisely the §6.1 misreport this section exists to forbid — minted by a heading any user is entitled to write.

**This law is escape-only and byte-neutral on conforming text.** A heading whose characters are all receipt-identifier text and not `"` renders identically with and without it — §6.3's `Goals` and `Q3` among them, so §6.3's byte arithmetic and §18 row 10's figures are unmoved by this section. Adopting the law changes no published byte; it bounds what an unpublished one can be.

**Sequencing (2026-08-09), and it held.** When this section was written the shipped default template did not write the §2.1 form at all — it wrote the pretty join §6.3 forbids, which was §18 row 10's 38 B debt and was carded separately as a fixture act. The segment renderer was that card's PREREQUISITE and landed ahead of it, at `b1fcc6e3`, because the forging hazard is created the moment the JSON form is emitted: the escape must exist before the emission, never alongside it. The emission lands in the commit carrying this sentence, so rule 2 is now law over shipped bytes rather than a rule waiting for its subject.

**A note on this paragraph's tense, because it was nearly written wrong.** The forward-looking version above was authored before either card landed, and an edit that flipped it to past tense was staged into the shared tree while HEAD still contained neither — which would have put a false history into the constitution itself. Law may be written ahead of its subject; a claim that the subject arrived may not, because that claim has an instrument and `git log` is it. The tense turns in the same commit that earns it.

## §7 The Delta noun — the fifth noun, stable at birth

### §7.1 Shape (stable)

One Delta = one batch = one fingerprint advance. E3's delta, every value computed:

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

E4's delta, in full (every value computed):

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
| `fix` | your request is wrong; change it | `bad_request`, `unknown_op`, `bad_path`, `no_match`, `not_unique`, `would_corrupt{family,lost?,cause?,target?}`, `ambiguous_ref{candidates}` |
| `env` | the world outside the workspace is wrong | `file_not_found`, `io_error{cause}`, `invalid_utf8{path,message}`, `daemon_only`, `mount_table_invalid{path,message}` |
| `refresh` | your picture of a node is stale; re-read one thing | `cas_mismatch{expected,actual}`, `ref_not_found{stage,dest?}` |
| `retry` | transient; same request may succeed | `lock_timeout`, `stale_view{required,as_of_fingerprint,live_fingerprint}` |
| `resync` | your picture of the world is stale; re-plan | `fingerprint_mismatch{expected,actual,changed}`, `fingerprint_unknown` |
| `respawn` | the channel itself is broken | `bad_frame`, `unsupported_proto`, `internal` |

W4 dispositions: v1's `not_found` is **retired** — `file_not_found` (env: the file is gone) is distinct from `ref_not_found` (refresh: the name dangles), and `io_error` carries its cause. `ref_not_found.stage` makes the two-stage decomposition observable in every failure (1 = vault-namespace miss, no `dest`; 2 = subpath miss, `dest` present — §4.5). `budget_exceeded` is deliberately NOT here: it is a typed *finding* inside `verdicts` (§11), never a wire error. **`daemon_only`** (env class) is **RETIRED** (hosts ruling, §3.3, 2026-08-06): it named the one deployment gap — a corpus-class rules pack, one whose WHEN needs the resident corpus name index (e.g. `link_resolves`, §11.2), loaded against a sidecar-mode engine with no resident index (the `BudgetClass::Corpus` law, §11.3). With the sidecar host deleted, every wire door is daemon-backed and the resident index is always reachable, so the code is unmintable; the `BudgetClass::Corpus` law stands, now gating nothing at the wire. Null-id frames: §3.1. Three declared deltas from the ruled class table (previously undeclared, now fixed): `fingerprint_mismatch` rebound refresh→`resync` (a failed world guard invalidates the plan, not one node's picture — §5.1's split), `unsupported_proto` rebound fix→`respawn` (a protocol mismatch is a channel property; no request edit repairs it), and `bad_id` dropped (folded into `bad_request` + `id:null`/`id_raw`, §3.1 — one malformed-envelope code, not two). All three are behavior-preserving relabelings, now declared. Deviation-from-v1 rows: `not_found` retirement (this table), unknown-`kinds` rejection (§4.3) — each with its rationale at the cited section; the consolidated ledger is §18.

### §8.1 The no-answer case — transport loss is not a class (RULED 2026-08-08)

The six classes ride **error frames** — cases where the daemon answered. A request whose answer never arrives — the client's own op deadline expired, or the connection died with the op in flight — is a **transport loss**, not a wire error: no frame arrived, so no `recovery` class exists, and for a `splice` **persistence is UNKNOWN**. The batch may have committed before the loss (observed 2026-08-08: a splice committed while the client's 10 s op deadline lapsed under a parallel build hold; the daemon answered a health probe in milliseconds — a slow op is not a hung daemon).

Two consequences, both **client law** — the wire cannot rule on frames it never served, so this subsection binds clients and hosts, not the engine:

- **The op deadline is a hang detector, never a safety mechanism.** Its value is host-chosen (ccc-statusd's D4 bound: 10 s per op), it does not scale with load — the engine publishes no load surface, and orientation is not a wire op (§10.3) — and no finite value closes the ambiguity window: any deadline can expire after the commit landed. Correctness comes from the retry discipline below, never from the number.
- **Re-read before retry.** After a lost `splice` answer the client's picture of the world is gone. Before any re-send, re-read the target and check whether the lost write **already landed** — checking content, not just tokens. The ordinary conflict path (`cas_mismatch` → refresh → re-apply with the fresh rev) is WRONG here: it re-applies a write that may already be in the file, and the ordinary teachings mislead — a post-loss `no_match` reads as "provably your typo" (§5.2) when the truth is "your first send landed and consumed the anchor".

A **blind re-send without `force` cannot double-apply** — the wire-origin guard demand (A.1, A.3) refuses every arm: a guarded edit's token re-derives against post-commit bytes (`cas_mismatch`, §5.1), a birth's subject now exists (`cas_mismatch`, absence guard), an unguarded content edit never reaches the write (`guard_required`). The refusal it draws is still ambiguous between "my lost write landed" and "a foreign write landed" — which is why the read comes first — but nothing applies twice while the client finds out. This is §5.2's adoption carrot extended: the guard demand is also what makes loss recovery safe. **`force` strips the node-grain tokens (A.1) and reopens the double-apply; a post-loss re-send MUST NOT carry `force`.**

Reads are idempotent: after a lost answer, re-send freely.

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
 "as_of_fingerprint":"b3:6e866e13b5e65ef9961c050f8a621cf1980b00ee293be650deef5f4dbc6823f0",
 "live_fingerprint":"b3:6e866e13b5e65ef9961c050f8a621cf1980b00ee293be650deef5f4dbc6823f0"}}
```

(The client demanded R0; the world is at R2.) `stale_view` is retryable, never silent.

### §10.3 View topology (structural)

Nothing on the agent path assumes SQL/DB access: every op in §4 is served from the world model (parse + hash of disk bytes). Facts API (this wire, agents) and any view face (humans) are two faces of ONE projection; deciding-from-a-view is a misclassified projection. Orientation surfaces (dashboards, counts, trees) are deliberately NOT wire ops — the mined kill/merge rows (`debt`, `domains tree`, `status`) stay dead (§16).

### §10.4 The rung-5 view organ — optional, wire-agnostic

Nothing on this contract **names DuckDB** (or any SQL engine). A view organ, if present, is implementation under §10.3: orientation is not a wire op.

**RULED — DROP (ZT, 2026-08-06, session `06-05-meridian-mcp-leg-2`).** The former "keep, reshape, or drop" question is closed. `Op::ViewPath`, its reply shape, and the daemon-published `view.duckdb` file are deleted from the wire surface and from both hosts — the op violated this contract twice (it returned an engine-named artifact path, and it put an orientation surface on the wire), and on a real corpus its synchronous rebuild could never meet a request deadline (measured ~6 min at 22k files, non-convergent while the fleet writes; session `05-19`, task `g5c`). Any future view organ returns as a **non-wire face** — an operator surface over its own build — never as a wire op. The daemonless `:memory:` build behind `mrd sql` is such a face and carries no wire vocabulary.

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

**Hash domain ⊂ addressable domain — one answer at every door.** A path outside the hash domain (an ignored `.md`, a dot-segment path) is still `toc`/`cat`/`read`/`extract`/`check_write`/`splice` by explicit path, at the READ door and the WRITE door alike: the read door serves its spans and mints its `file_rev` exactly as for a domain member, the write door commits to it, and its bytes simply do not move the fingerprint (`fingerprint_before == fingerprint_after` across such a write). The domain filter gates HASHING, not load — corpus residency is never a read admission test, so a door that refuses an out-of-domain path is a door defect, not domain law.

Two consequences, stated because a v1.0.0 build inverted exactly this (dogfood 2026-08-09, s10 — the warm read door refused what the write door committed): a guarded write's CAS token (`node_rev_before` / `file_rev`) for an out-of-domain page is mintable at the read door like any other, never only by the write door itself; and `file_not_found` means exactly one thing at every door — **no such file under the workspace root** — so its teaching must not offer domain exclusion as a second reading of the miss.

**The rule binds a DOOR FAMILY, and the family is every door the caller NAMES A PATH AT** — not the read/write pair the paragraph above happens to enumerate. `links <PATH>`, `walk <PAGE>` and `repair <PAGE>` take a path from the caller exactly as `cat` does, so each serves an out-of-domain path or is a door defect by the sentence above. Stated because a build served two of them and refused three, which reads as one law with an exception and is instead one law with three defects (dogfood 2026-08-09, f06 — the four-door reading of this paragraph was itself a subset of a nine-door family).

**Enumerators are the other half, and they are NOT bound to admit — they are bound to SAY** (ruled 2026-08-09, session decision 0017). A whole-corpus enumeration — `retire`'s sweep, the `sql` projection, `check`, bare `links` — stamps its answer `as_of` a fingerprint that an out-of-domain file's bytes cannot move, so carrying such a row under that stamp would publish a claim the stamp does not cover. An enumerator therefore MAY exclude what its attestation cannot reach, and **never silently: the exclusion is named in the output, and an enumeration that certifies ABSENCE either refuses or names what it did not see.** The engine already holds this shape for its neighbouring exclusion class — the unserved-member voice ("the file serves no spans/nodes … this scan does not see inside it") and `retire`'s refusal to certify over a partial corpus — and this rule is that reasoning carried to the domain-excluded case. The two halves compose: **a door that is asked about one path ADMITS; an enumeration that speaks for the whole corpus NAMES what it left out.**

**The VERDICT plane is the third half, and it is bound to say WHAT IT DID NOT LOOK AT** (ruled 2026-08-09, session decision 0034). The colour plane — `walk`, `check`, `status` — states a verdict about a pin's TARGET, a path the caller never named. Its corpus is the hash domain, so an out-of-domain target is absent from it for a reason that has nothing to do with the target: **the engine did not look.** A red there asserts evidence the engine does not hold — and because the taught response to a red is to fix the target or drop the pin, a false red drives a caller to destroy an attestation the engine itself minted (`pin` returns rc=0 on an out-of-domain path and writes its anchor to disk; §12.1's first paragraph is why). **So an out-of-domain target THAT EXISTS ON DISK renders `grey(outside-hash-domain)`, never red** — R-3, grey outranks red — and the reason word is what distinguishes *policy: seen but not hashed* from the greys that mean *blindness: could not look*. In-domain targets are untouched by this rule: a pin whose target the domain holds still colours green, or red on real drift or a real miss.

**The qualifier is load-bearing: "never red" binds a path that EXISTS but cannot be hashed** (ruled 2026-08-09, session decision 0049). **Absence outranks domain membership, because the order of questions is the order of facts: does the named path exist on disk, and only then, can the domain assess it.** Existence is a fact about the DISK; the domain filter is a fact about what the FINGERPRINT covers, and it is never a fact about what the disk holds. So the verdict plane answers the existence question by READING THE NAMED PATH — the same domain-independent read every named-path door owes (session decision 0045) — and **a named path that is absent from disk stays `red(file-not-found)` whether or not the domain would have excluded it.** This is the paragraph above about `file_not_found` meaning exactly one thing at every door, carried onto the verdict plane: a grey *"not in the hash domain"* over a file that is not there is a false sentence — the file is not anywhere, let alone outside the domain — and it fails in the certifying direction, because grey reads as intended exclusion and stops a reader looking. **The two states get two verdicts: out-of-domain and PRESENT is grey `outside-hash-domain`; out-of-domain and ABSENT is red `file-not-found`.**

Scope, stated because the two states are told apart by different mechanisms: the domain question is answerable only for the AMBIENT root today (a mounted root's corpus is built by its own workspace's filter and no face carries those filters across), so the existence read runs where the domain arm runs. A miss inside a MOUNTED root is measured by resolution as before and is unchanged by this rule. And **an IN-DOMAIN target that is absent keeps its existing verdict** — the domain arm never applied to it, so the address-resolution reds (`selector-unresolved`, `dangling-anchor`) still own that case; this rule moves only the verdicts the domain arm was answering.

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

An ignore-list (or algorithm) change re-defines the domain, so the token prefix advances: `b3:` → `b3a:` → … The domain `version` field rides **`meridian/domain.md`** with the ignore list so domain definition and prefix travel together.

Worked at **S0**, writing `meridian/domain.md` in the forms §0.3 prints. `drafts/tmp.md` may be on disk for **rows 3–5 only** — each of those declares an ignore list covering it, so its presence moves nothing there. **Rows 1 and 2 require it ABSENT**: they declare no ignore list, so a `drafts/` file joins the domain and neither row's value is served. The config is markdown, so **its own bytes are in the domain it declares** — that is why v0 and v1 differ in hex over the same member set. Every value is measured against the shipped engine, 2026-08-09:

| `meridian/domain.md` | Files hashed | Fingerprint |
|---|---|---|
| absent (S0 as printed in §0.3) | plan, receipts | `b3:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9` (= R0) |
| v0 — `version: 0`, no ignore list | plan, receipts, **domain.md** | `b3:23421037fa8d4a947aa7104941797b325e38c67878787773f76bc1009c63bab4` |
| v1 — ignore `drafts/**` | plan, receipts, **domain.md** | `b3a:48c0b314c7e0bf2d570936a302a4d5be4802a03187a988353efc5725b45067b1` |
| v1 — ignore `drafts/**`, `meridian/**` | plan, receipts | `b3a:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9` |
| v2 — same ignore list | plan, receipts | `b3b:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9` |

Two laws read straight off the table. **The prefix tracks the domain RULES, never the member set** — row 2 adds a file and stays at `b3:`, because declaring `version: 0` with no ignore list changes no rule; rows 3–5 advance because the rules moved. And **the same surviving hex can never compare equal across prefixes**: rows 1, 4 and 5 carry one 64-hex value under three different tokens, by design — receipts must not silently match a redefined domain.

**Amendment, 2026-08-09.** Before this date §12.3 published a pair anchored at S2 — v0 `b3:05f0c6192308db5937c3e1352d1f9a6fc31b89b1a57175c8af6ce7903525aa4a`, v1 `b3a:83b4ba591c0291d9f2a05428cac38e5820858fbb9c47720ab352344ddccc8f68`. Those values close arithmetically only if the domain config's own bytes stay OUT of the domain, which is true of the legacy non-md `mdfs_config.yaml` and false of the standing `meridian/domain.md`. The published worked example was therefore reproducible only through the surface §12.1 says do not create and do not teach. Ruled (advisor, 2026-08-09): the table recomputes over the standing surface. §12.1 stands untouched — the legacy filename stays forbidden — and the superseded values are printed here rather than scrubbed. §18 row 11.

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
| `crates/sidecar` | **DELETED** — hosts ruling (§3.3, 2026-08-06): the daemon socket is the one wire door; the registry host serves §4 | — |
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

The fix-at-freeze rule requires each reviewer-flagged debt fixed or waived with reason, here, never silently. Rows 1–5 are the winner-pick fix list; rows 6–7 consolidate the v1 deviations already declared in the body; rows 9–13 are measured against the shipped v1 artifact and this document's own arithmetic (dogfood, 2026-08-09).

**Rows 9 and 12 are a different KIND of row, and the difference is load-bearing.** Rows 1–7 and 11 record where this DOCUMENT departed from ruled law. Rows 9 and 12 record where the shipped ARTIFACT departs from this document — and **row 10 carried BOTH legs and is now CLOSED** (rebaselined 2026-08-09): a printing debt in this document, since discharged, and beside it the width the artifact actually writes — measured at the v1 cut against the built binary, not read off the source. Recording it is what §15's assumption-audit law demands: a deviation found without a row here is a contract bug, so the row FIXES that bug by this contract's own procedure. **Declaring is not legislating.** The law a row names stays in force, unamended, and the row is the observation — never a licence. (The contrast that fixes the rule: amending the constitution to legalize an implementation's behavior is breach; recording non-conformance while the law stands is this ledger working as designed.)

| # | Item | Disposition |
|---|---|---|
| 1 | Policy-scope grammar was unbound to the strict plane (ratchet scopes lived as host-side prose; the pack manifest has no scope field) | **FIXED** — §5.3 now binds scope grammar: `Path`-set selectors + strict-plane refs (§2.1), no second grammar; the manifest's scope-field absence is declared deliberate |
| 2 | The repo's reserved `fingerprint_mismatch` shape (`crates/wire` §6.5 reserved-codes note) carries extra fields `expected/actual/scope/changed`; this contract ships `{expected,actual,changed}` — the `scope` drop was unflagged | **WAIVED, declared** — the only world-grain guard is `if_fingerprint` (§5.1); no scoped-fingerprint construct exists for `scope` to describe. If a scoped world guard ever arrives by amendment, `scope` returns with it. The skill's ERRORS.md documents the same three-field shape — consistent |
| 3 | The frontmatter node's span `[0,20]` is terminator-inclusive, against the v1 §5.2 / merkle-spec §2 leaf-block law (exclude the final terminator) — previously undeclared | **WAIVED, declared** — the frontmatter node is a fence-to-fence container, span-lawed with the section (newline-inclusive) family, not the leaf-block family; the `fm_key` leaf inside it (`[4,15]`, §4.4) excludes its terminator, consistent with the leaf law. All hashes stand |
| 4 | Two silent rebinds vs the ruled failure-class table plus one dropped code | **FIXED, declared** — §8 now declares all three deltas with rationale: `fingerprint_mismatch`→`resync`, `unsupported_proto`→`respawn`, `bad_id` folded into `bad_request` + `id:null`/`id_raw`. Behavior-preserving |
| 5 | The base packet's A6 self-claim "replay ≡ live stated and tested" — nothing executable tests it today | **FIXED, restated honestly** — executed: the fixture recomputation behind every worked value (§17 tool attest). NOT executed: any replay ≡ live test, any conformance-pack run — both are impl-rung deliverables in the impl-plan (rung-4 test; GT regeneration). The word "tested" is retracted |
| 6 | v1 `not_found` retired | Declared at §8 with rationale (split into `file_not_found` env / `ref_not_found` refresh) |
| 7 | v1's frozen "unknown `kinds` match nothing" reversed to loud `bad_request` | Declared at §4.3 with rationale (the strict-server evolution law applied to values) |
| 9 | **The v1 artifact deviates from the raw-lexeme id law (§3.1).** §3.1 fixes valid ids as JSON integer lexemes in `[0, 2^53)` and requires a non-conforming lexeme to be refused with `id:null` plus the offending lexeme verbatim in `id_raw`. Measured at the v1 cut against the built release binary over a live socket: every non-conforming lexeme — `"1"`, `-1`, `1.5`, `true`, `null` — is silently nulled and **the request is SERVED**. No refusal, and `id_raw` is never emitted. A conforming integer echoes correctly (`{"id":7}` → `id:7`). Not op-specific: on `mounts`, `{"id":5}` echoes while `{"id":"5"}` answers `id:null` — same frame, same op, only the lexeme differs | **DECLARED, not waived, and §3.1 STANDS UNAMENDED.** The law is the law; the artifact does not yet serve it. **The corruption-law interaction, stated plainly:** a client sending non-conforming ids receives served work AND frames that §3.1's null-id corruption law instructs it to treat as channel corruption — fail all outstanding, respawn. Two subsystems tell such a client opposite things. **Why this declares rather than blocks the release:** only NON-CONFORMING senders can reach it; no promised surface serves a wrong result to a CONFORMING caller; and the blast radius is a misbehaving client receiving a confusing-but-defensive signal, never silent corruption of a correct one. **CONFORMANCE IS OWED.** Serving `id_raw` and refusing non-conforming lexemes per §3.1 is the standing **v1.x direction** — this row records a gap on the way to it, and is not the end state |
| 10 | **The fixture's S1/S2 receipt bytes were printed nowhere.** §6.3's receipt lines measured 260 B / 262 B against the 222 B / 224 B the fixture's own spans and file sizes required: 38 B per line unprinted (dogfood 2026-08-09, s10). §0.3's promise that "every file this document hashes is printed in this section" did not hold for S1/S2 | **CLOSED 2026-08-09 by rebaseline — this row is now the RECORD of a closed debt, not a declaration of an open one.** The two causes were deviations from this document's own law and were fixed in the TEMPLATE, not waived in the text: `root_before=` where §6.1's standing noun is `fingerprint_before=` (**7 B**), and the `Goals>Q3` pretty join where §2.1 form is mandatory and §6.3 says in its own words not to re-teach it (**31 B**). **This was a FIXTURE act, which is why it took a rebaseline and not an edit.** Receipts are ordinary markdown inside the hash domain (§6.1), so moving receipt bytes moved the receipt node rev, which advanced the workspace fingerprint. R1 and R2 were RECOMPUTED by the engine over the new fixture bytes, never re-typed: R1 `b3:10769ae1…` → `b3:7f3b4437…`, R2 `b3:83b4ba59…` → `b3:6e866e13…`, receipts `file_rev@S1` `2731acfa…` → `51ad6428…`, `file_rev@S2` `9167b12b…` → `6cb0e939…`, the `r-000042` leaf `[26,248]`/`639a2dca…` → `[26,286]`/`60bbee70…`, the `r-000043` leaf `[249,473]`/`c912d457…` → `[287,549]`/`5c6ca7ec…`, and the receipts file 26 → 249 → 474 B → 26 → 287 → 550 B. Every one is gated by recomputation in `crates/testsuite/tests/pf_frozen_sweep.rs`, which derives them from the committed S0 bytes rather than transcribing them — so a wrong value fails a test instead of shipping. **§0.3's promise now holds without exception**: §6.3 prints the S1/S2 receipt bytes, and they ARE the fixture's. **What did NOT close, and is not owed:** the shipped CLI lane writes **254 B** node (**255 B** line) because a CLI invocation is not a wire request and mints no request id — `id=` is 6 B of §9's absent-inputs law working, ruled not a defect, and the same template writes the fixture's 260 B byte-for-byte when a request carries one (gated: `crates/receipt/tests/frozen_receipts.rs :: e3_receipt_line_byte_exact`). Live S1/S2 therefore still leave the published R1/R2 timeline on the CLI lane, by lane and not by shortfall. **The escaping this form now requires is §6.7 rule 2**, landed ahead of this rebaseline for a reason stated there: the forging hazard is created by emitting the JSON form, so the escape had to exist before the emission, never alongside it |
| 11 | **§12.3's worked table taught its arithmetic through a forbidden surface.** The published S2-anchored v0/v1 pair closes only if the domain config's own bytes stay out of the domain — true of the legacy non-md `mdfs_config.yaml`, false of the standing `meridian/domain.md`, which self-hashes by design (`crates/fs/src/domain.rs`). §12.3's values therefore contradicted §0.3's own "participates when present" note and were unreachable from the surface §12.1 mandates (dogfood 2026-08-09, s10) | **FIXED** — §12.3 recomputes over the standing `meridian/domain.md` with engine-measured values, §0.3 prints that file's v0 and v1 bytes, and the superseded pair is printed at §12.3 rather than scrubbed. §12.1 stands unamended: the legacy filename remains do-not-create, do-not-teach. **The S0 file set did not move** — `meridian/domain.md` is ABSENT at S0, R0 unchanged, and printing a file's bytes never makes it a member (proved on a fresh fixture: absent → R0, v0 present → `b3:23421037…`, removed → R0 returns). Ruled 2026-08-09, advisor scope |
| 12 | **CLI-lane commits advance the fingerprint and mint no Delta.** §7.1 laws one Delta per batch per fingerprint advance and §10.1's `changes_seq` is that counter. Measured at the v1 cut (dogfood 2026-08-09, s9): an `mrd put` commit moves the fingerprint the same daemon serves immediately, while `changes_seq` reads 0 before AND after | **DECLARED, not waived; §7.1 and §10.1 STAND UNAMENDED.** A consumer using `changes_seq` as a change monotone misses every CLI-lane write, silently — the answer is in an honest tense but the counter under it never moved. The fingerprint is the only monotone covering both lanes today, so cross-lane catchup is diff-by-root (§4.7), the same answer §7.1 already gives for cross-epoch catchup. Minting the delta on the CLI lane is owed |

| 13 | **The shipped v1.0.0 artifact cannot state the `-dirty` half of §A.3's identity token.** §A.3, amended 2026-08-09, laws `hello.identity.build` as `sha \| sha-dirty \| unknown`, where a bare sha asserts the build came from a WHOLE commit. `93184797` (= v1.0.0) bakes `git rev-parse HEAD` with no cleanliness probe, so it publishes a bare sha for a dirty-worktree build — an assertion it never measured | **DECLARED, not waived; §A.3 STANDS as written.** The released binary was itself built clean, so the shipped artifact tells no lie about ITSELF; the gap is that it cannot tell one about any other build made from that code. **Why this declares rather than blocks:** the running engine moves only by deliberate act (`docs/release.md` §5.1), the tree's `mrd` conforms as of this amendment, and the reader-visible consequence is a missing refusal, never a wrong served result. The row closes when the engine is next cut — it is a snapshot of a pinned artifact, not a standing intent |

Row 8 is **not reused**: the paragraph below refers to the dissolved row by that number, so retiring it keeps the record unambiguous.

The former row 8 (walk-plane charset/parity "collision") is **dissolved by the one-way-floor ruling:** parity is a one-way compatibility floor, not law, so a `_`-bearing anchor refusing loudly (§2.4, §4.5) is conforming behavior — there is no deviation to declare and no veto pending. Nothing else in this document knowingly deviates from ruled law; a deviation found without a row here is a contract bug (the assumption-audit law, §15).




---

## § A. Standing additions (compact)

These are **current law**, not optional history. Detail that only implements code may lag; the shapes below are what agents and hosts must learn.

### A.1 Fingerprint-or-force (every wire door)

Content-mutating writes on the **wire door** (the daemon socket — the only door, §3.3) require fingerprint match **or** `force`. Guard fields stay **schema-optional** (a guardless frame still **decodes**). A content-mutating write with neither fingerprint nor `force` is refused **after decode** as `guard_required` (recovery: `fix`) — semantic refusal, not a frame rejection. `force` is any client's refuse→rewrite path; MCP is not a separate trust plane. In-process paths (`mrd` without the wire door) are out of this ruling's reach by **scope**, not trust.

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

### A.3 Composed `read`, `check_write`, `mounts`, `plan_edits`, `pin`, `create`, `hello.identity`

| Surface | Role |
|---|---|
| `read` | Addressing + content + render + frontmatter props at one snapshot; section selectors use §2.1 segments (or anchor / dewey). Not a joined string address. |
| `check_write` | Standalone write pre-flight: the splice verdict computed without writing. Read-only. |
| `mounts` | Mount-table discovery: the live root registry, machine-scoped. Read-only (§ A.5). |
| `splice.plan_edits` | Plan-level batch shapes; addresses are **segment arrays**. |
| `splice.pin` | Pin rides the write choke-point; selector is segments/anchor. |
| `create` | File birth through the guarded door; full body bytes. |
| `hello.identity` | Optional `{build: sha \| sha-dirty \| unknown}` for deploy identity. The `-dirty` marker rides the sha TOKEN (git-describe convention), so this stays one field: `sha` = built from a whole commit, `sha-dirty` = built from a worktree diverging from that commit, `unknown` = no attributable identity was readable. A caller matching a declared sha matches the WHOLE token, never a substring — a decorated sha is a different build and must refuse (`docs/release.md` §5.1). *(2026-08-09: the marker is new; the field, its optionality, and its v3-only rule are unchanged.)* |

**Composed-`read` selector resolution (2026-08-06, dogfood F4–F6):**

- A section selector matching **more than one** node refuses `ambiguous_ref` naming each candidate's machine address (its `n`-carrying segment array) — §2.1's "the strict plane never silently picks" applies to strict reads exactly as to `cat` and `splice`. Never a silent first match, never `ref_not_found`.
- When **all** selectors fail, the refusal names **every** failed selector with its own reason (no match / ambiguous), symmetric with the partial-read `notice`, which names them the same way.
- Refusal **remedies speak the operation, not one host's tool name**: the recovery clause names the toc read in each surface's own dialect (MCP `mode:"toc"`, CLI `--section`-less read) and never prescribes a binary the caller may not have. *(Ruled 2026-08-06, dogfood F5: dual-dialect IS this spec, not a partial fix — a remedy leads with the caller's surface (MCP `mode:"toc"` first) and MAY carry a labeled CLI alternative in the same sentence.)*

**Door symmetry over duplicate headings (2026-08-06, fix-write-dup-symmetry):**

- An `n`-less address that matches more than one node refuses `ambiguous_ref`-class at **every** door — read and write alike (`splice.plan_edits`, and any host lowering onto it). No door may pick an occurrence the caller did not name: the write-door refusal names each candidate's machine address (its `n`-carrying segment array) and teaches `n`, the same evidence the read door gives. Two doors, one answer — a selector one door refuses as ambiguous, no other door resolves.
- The published loop is untouched: addresses the read face publishes carry `n` exactly where the document is ambiguous, so read → verbatim address → write always lands.
- **The PROSE is symmetric too (2026-08-09, dogfood s4).** The machine bodies already matched while the sentences did not: the read door taught *"pin one occurrence by its machine address, or its dewey ordinal from the toc"*, and the write door taught *"address the duplicate by block id or node index"* — which never names `n`, and whose "block id" prescribes minting an id on a heading the caller may not own. The write door speaks the read door's remedy: **pin one occurrence by its `n`-carrying machine address, or by its dewey ordinal from the toc.** Renaming a duplicate heading stays a legitimate, secondary fix and is named as one — it edits the document, where the `n` address does not.
- ⛔ **Neither ambiguity refusal ends in a wikilink.** `[[selector-grammar]]` inside a machine-facing message is a vault-local address the caller cannot dereference — it names a page without saying how to reach it, and it survives into logs and agent transcripts as literal brackets. Both ambiguity remedies are self-contained (an `n` address, a dewey ordinal, a distinct block id), so the citation bought nothing it was paying for. *Scoped to the ambiguity pair: the `see [[address-grammar]]` tail on the `crates/addr` refusals is the same class and is NOT swept here — recorded as a finding rather than changed under a card that did not measure it.*

**Door symmetry over duplicate block ids (2026-08-08, dogfood-p1-read-ambiguous-ref):**

- An anchor selector (`^id`) whose id appears on more than one block refuses `ambiguous_ref` at **every** strict-plane door — the composed read's `sections[]`, `cat`, `pin`, and `splice` alike (§2.1's duplicate-anchor row). Never a silent first match: a read that picks one occurrence hands the caller a `sec_rev` the write door then refuses, so read-then-write on a duplicated anchor is unserviceable — the exact death mode §2.1 closes for writes.
- Duplicate ids share one spelling, and the anchor grammar carries no occurrence index (`n` disambiguates hpath segments; `{"anchor":id}` has no `n` slot), so **no machine address exists per candidate**: the refusal's `candidates` stays `[]` and the message names how many blocks carry the id. The map stays honest evidence: `toc`'s `anchors[]` publishes every occurrence with its span, duplicates included.
- The remedy **speaks the anchor grammar, never the heading one**: give each duplicate block a distinct id (a block id addresses exactly one block in its file), or address the enclosing section by heading path. "Rename one heading" is the heading-duplicate remedy and never appears on an anchor refusal.

**Teaching row — the anchor host-kind gate is a READ-face law, and the write door does not carry it (2026-08-09):** `unaddressable_host` and the set `anchors[]` publishes are both scoped to the block kinds this read face addresses. The write door has no host-kind gate. So a paragraph-hosted `^id` that the map never publishes, and that the read door refuses `unaddressable_host`, is still a legal `{"anchor":id}` splice target and arms a rev transition normally. Read this in one direction only: the map remains honest about its OWN door — every address it publishes, it serves — but it is not an index of the write plane, and absence from `anchors[]` is not evidence that a write will refuse.

**`check_write` — the standalone pre-flight (recorded 2026-08-07: the deployed host consumes this op on every guarded put):**

- Request: `{"op":"check_write","path":…,"target":…,"actor":…,"now":…,"edits":[…]}`, strict-decoded, v3-only at dispatch (`crates/wire-serve/src/decode.rs:194-227`); advertised in v3 `caps` (`crates/wire-serve/src/rev.rs:103`). Each edit is `{op, at, find?, body?, rev?, all?}`; `at` is the §2.1 segment array (`{h, n?}`), the same shape the committer takes — the single-segment forms carry a block `^id` or a frontmatter key (`crates/wire/src/lib.rs:842-861`). `path` addresses the file under the workspace root; `target` is the raw host path that labels refusal strings.
- Reply body: `{"refuse":…?, "repairs":[…], "forced":[…]}` (`crates/wire/src/lib.rs:990-999`). `refuse` absent = the write may proceed. `refuse` is `{class, code, message, remedy?}`; `class` picks the host's render template — `rebuild` (the candidate could not be built) vs `verdict` (the severity ladder refused). `repairs` are `{key, value}` autofill property sets the host folds into the same atomic write; `forced` echoes overridden warn rule-ids. `repairs`/`forced` always serialize, so the body is never shapeless.
- Read-only over the warm engine (`crates/registry/src/server.rs:1173-1188`); a path with no file under the workspace root is `file_not_found`. A real file outside the hash domain is served from disk on the same snapshot (§12.1 addressability) — corpus residency is not the admission test. Mandatoriness stays host policy (§5.3): the engine computes the verdict, the host decides to refuse on it. `splice` re-runs the same verdict inside its own flock (`crates/wire-serve/src/write.rs:307-310`), closing the check→apply TOCTOU gap — the standalone op is the host's pre-flight and error-rendering surface; the splice-internal run is the law.

**Law A-1 at the create door — `create.rev` (docs-first, 2026-08-07, August
team-e multi-root contract, L3 end state):**

- The `create` plan-edit shape becomes
  `{"create":{"parent_hpath":[…],"title":…,"body":…,"rev":…?}}`. `rev` is the
  **parent section's** node-grain token — the `node_rev` the caller read for
  the section its birth appends under. A section create lowers to a
  parent-append (the parent's bytes change; the parent's rev is the honest
  grain), and `rev` threads to the lowered edit's `if_node_rev`, compared per
  §5.1 — re-derived at execution from the pre-batch state, one rev derivation,
  no second comparison rule. The child-absence guard stays beside it,
  unchanged (an already-born subject refuses `cas_mismatch`).
- **Schema-optional, like every guard field (§ A.1):** a rev-less `create`
  frame still decodes, forever. The demand below is a semantic refusal after
  decode, never a frame rejection.
- **The demand (the engine-side create-door law):** on a wire-origin splice, a
  `create` whose `parent_hpath` carries any `n`-bearing segment demands `rev`
  or `force`; absent both it refuses `guard_required` (fix class), teaching
  the slot (`create.rev`) and the toc read that mints the token. Why the
  occurrence class is not negotiable: `{h, n}` binds by *position among
  identical texts* — between the caller's read and its create, a same-titled
  sibling insert re-binds `n`, and the child-absence CAS cannot see it,
  because it checks a fact about the tree as the engine resolves it *now*. A
  guard that mints its own token proves nothing (Law A-1); the caller's parent
  rev is the only fact that ties the create to the tree the caller actually
  read.
- `force` bypasses this demand exactly as it bypasses every fingerprint-plane
  demand (§ A.1): loud, with the bypassed plane named against the parent
  subject. No new force semantics.
- Advertised as dotted cap `splice.create_rev` (v3 projection; the frozen v2
  caps stay byte-identical). Occurrence-free parents: an offered `rev` is
  honored (CAS) wherever it is present; whether it is *demanded* beyond the
  occurrence class is the §5.3 ratchet — a named future-amendment candidate,
  not this law. Do not widen the demand here.

**Frontmatter-properties plane on the composed `read` (docs-first,
2026-08-07, the mcp-face §3.3 wire demand — engine leg of the props
deferral):**

- The composed-read reply gains a `props` plane: the document's top-level
  frontmatter key facts, served at the SAME engine snapshot as every other
  fact in the body — one row per key, document order, first occurrence wins
  (the model's flat parse is the keys authority; the v2 toc `keys` echo and
  this plane serve the same order). The demand this answers is the host
  face's `props:` line: a face that builds it from a second read breaks
  one-snapshot coherence, and a face that parses bytes itself breaks
  daemon-zero-markdown-semantics — so the facts land engine-side, mirroring
  how `words` landed.
- Row shape `{key, value, span, prop_rev}`:
  - `key` — the top-level key, the same string the §2.1 `fm_key` form
    addresses; the read→set loop closes off this row.
  - `value` — the key line's value **decoded through the § A.6 scalar law**:
    the colon remainder, whitespace-trimmed, then unquoted when it is a
    well-formed quoted scalar. A block value (indented continuation lines)
    serves the key line's own remainder — empty when that line carries none.
    **Amended 2026-08-07 (§ A.6): this bullet formerly read "quotes kept",
    which made the plane serve source bytes where every reader expects a
    value; the superseded wording and why it failed are recorded at § A.6.**
  - `prop_rev` — the key's CAS token: blake3 over the full key grain span
    bytes (the key line plus its indented continuation lines), 16 hex —
    the SAME token `cat` on the `fm_key` node serves and `if_node_rev`
    compares (§5.1: one rev per node, re-derived at execution, no second
    derivation).
  - `span` — that grain span; intra-file byte offsets, root-independent.
- Emission law (the `anchors` precedent): always emitted — empty means
  "this document has no top-level frontmatter keys", never "ask again with
  a flag". Decoding stays tolerant of older recorded frames; serialization
  is unconditional.
- Document-grain, both modes, never `frag`-scoped: frontmatter belongs to
  the document, not to any subtree — a `frag` cannot contain it and does
  not filter it.
- v3-only by construction: the composed read is v3-only at dispatch, so the
  plane never appears on a v2 session and the frozen v2 caps and bytes stay
  byte-identical. No new cap: a response-side additive field under the
  tolerant-client law (§3.2) — the `words`/`anchors` precedent.
- Delta grain unchanged: this plane is the map tense of the frontmatter
  plane (§7.2 — one projection, three tenses); §7.4's ruled node-grain and
  its named future-only `keys` amendment path are untouched.

**Per-selector unresolved facts on the composed `read` (docs-first,
2026-08-08, engine leg of the 07-05 miss-facts card):**

- The composed-read reply gains an `unresolved` plane: one row per section
  selector that resolved to no served section, in request order. This is the
  machine tense of the facts the partial-read `notice` and the all-fail
  refusal already phrase in prose — the A.3 symmetry law, now three tenses of
  ONE fact set — so a consumer acts on each failure individually instead of
  parsing one human sentence. The prose `notice` and `truncated` stay
  unchanged beside it (the props precedent: the plane lands beside the prose
  it structures, and both derive from the same resolution pass, so they
  cannot disagree).
- Row shape `{sel, reason, candidates, count?, host?, nearest}`:
  - `sel` — the failed selector echoed in its own request grammar
    (`{"hpath":…}` / `{"n":…}` / `{"anchor":…}`): the caller correlates by
    shape or position, never by re-parsing a display string.
  - `reason` — closed vocabulary, one per row:
    `no_match` (nothing carries the address) ·
    `ambiguous` (a heading or dewey selector matched more than one node) ·
    `duplicate_anchor` (more than one block carries the `^id`) ·
    `unaddressable_host` (the id exists on the page, but its host block kind
    is outside the face's anchor plane — the P2-c truth-telling row, distinct
    from `no_match` because the honest remedy differs).
  - `candidates` — `ambiguous` only: each candidate's machine address as the
    §2.1 `n`-carrying segment array (actual arrays, never encoded strings),
    in the order the refusal names them. Always serialized; `[]` on every
    other reason — including `duplicate_anchor`, where no per-candidate
    machine address exists (the 2026-08-08 door-symmetry law above).
  - `count` — `duplicate_anchor` only: how many blocks carry the id.
  - `host` — `unaddressable_host` only: the true host kind (`paragraph`,
    `task`, `heading`, `frontmatter`, …) — the same open string the toc
    anchor row echoes, never a fallback.
  - `nearest` — anchor-`no_match` only: the nearest live ids as
    `{anchor, kind}` rows. **The candidate pool spans every `^id` on the
    page, non-addressable hosts included** (season-1b addendum: a typo one
    character short of a paragraph-hosted id refused with no candidate,
    because the pool held face-addressable ids only — excluding exactly the
    ids that would explain the miss). `kind` is the host kind, so a render
    teaches the host-kind gate on a non-addressable candidate instead of
    implying absence. Empty when the page carries no `^id` at all. Always
    serialized.
- The prose teaching draws from the same widened pool: the miss clause's
  nearest list names a non-addressable candidate with its host kind and the
  servable way in, and the no-anchors clause claims a bare page only when
  the page truly carries no `^id` of any host kind.
- Emission law (the `anchors`/`props` precedent): always emitted — empty
  means "every selector resolved" (a toc read trivially so), never "ask
  again with a flag". Decoding stays tolerant of older recorded frames;
  serialization is unconditional.
- v3-only by construction: the composed read is v3-only at dispatch, so the
  plane never appears on a v2 session and the frozen v2 caps and bytes stay
  byte-identical. No new cap: a response-side additive field under the
  tolerant-client law (§3.2) — the `words`/`anchors`/`props` precedent.

CLI inventory (descriptive): `status.md`. Cross-root agent address grammar: `address-grammar.md`. Config parse: `meridian-md-schema.md`.

### A.4 What this document does not teach as core

- Joined hpath strings as machine addresses  
- `mdfs_config.yaml` as the domain config (use `meridian/domain.md`)  
- SQL / `view_path` / DuckDB as agent path  
- Dual wire constitutions ("v2 vs v3" for agents)

### A.5 `mounts` — mount-table discovery (the live root registry)

*Docs-first (2026-08-07, August team-e multi-root contract, W20): this section
lands before its code. Strict decode, dispatch, caps push, and tests are the
implementation card's.*

The one new engine surface multi-root addressing adds. Read-only; machine-
scoped; v3-only at dispatch, advertised at op grain as cap `mounts` (the
`create` precedent — no dotted `mounts.<field>` at birth). A v2 session
answers `unknown_op`; the frozen v2 caps stay byte-identical.

Request — no parameters, and **no workspace binding required**: the mount
table is what `~/MERIDIAN.md` binds for the whole machine, not a property of
any workspace, and the caller discovery exists for is exactly the agent that
does not know a root yet. A workspace-less connection (a bare `hello`) may
call it.

```json
{"id":7,"op":"mounts"}
{"id":7,"ok":true,"body":{
 "config_rev":"9f27a2814b517681",
 "mounts":[
  {"name":"field-notes","kind":"vault","state":"bound",
   "workspace":"/Users/Shared/repos/field-notes"},
  {"name":"sessions","kind":"vault","state":"bound",
   "workspace":"/Users/Shared/projects/field-notes-sessions"},
  {"name":"assets","kind":"git-folder","state":"grey(path-unseeable)"}]}}
```

Row shape `{name, kind, state, workspace?}`:

| Field | Law |
|---|---|
| `name` | the canonical `MountName` — the bindable layer's spelling, lowercase `[a-z0-9-]` (`address-grammar.md` § 4.3) |
| `kind` | `vault` \| `git-folder` — the `MountKind` words verbatim |
| `state` | the `MountState` reason word verbatim, ONE spelling across the human line, `--json`, and this wire: `bound` · `grey(path-unseeable)` · `grey(undeclared)` · `grey(declaration-unreadable)` · `grey(claim-unverifiable)` · `red(content-drifted)`. Every word but `bound` refuses: a client gates on `state == "bound"` and treats an unrecognized word as not-bound — the tolerant-client law applied to an open-for-amendment word set |
| `workspace` | the canonical bound path, post-canonicalization — the same handle `hello` returns as `workspace`. Present exactly when the binding canonicalized; absent at least on `grey(path-unseeable)` |

**Freshness — the config-hash rebind law.** Every `mounts` call fingerprints
`~/MERIDIAN.md` (blake3 over the file bytes) before answering: unchanged hash
serves the derived table; changed hash re-derives first. No mtime, no TTL, no
hello-time snapshot — a mount added mid-session is named on the next call, and
a vanished root degrades to its grey row. `config_rev` is that token on the
response: 16 lowercase hex, the `file_rev` family (`blake3(whole file
bytes)[:16]`, §1 rev sub-laws), opaque and equality-only — the face caches
against it and never parses it.

**Changed-invalid refuses.** When `~/MERIDIAN.md` HAS changed and the
re-derive fails loud — duplicate names/paths/vault names, nested mounts, the
closed-schema refusals (`address-grammar.md` § 3) — the op refuses; it never
serves the previous table as if current:

```json
{"id":8,"ok":false,"error":{"code":"mount_table_invalid","recovery":"env",
 "path":"~/MERIDIAN.md",
 "message":"two mounts bind the canonical path /Users/Shared/repos/field-notes (duplicate-mount-path)"}}
```

`env` class: the binding file is an environment fact the caller must change.
The refusal names the offending entry (Law A-3c: scope + offending member).
Per-root grey states are NOT this refusal — an absent, undeclared, or drifted
root is a served row carrying its state word; only a table-level parse/bind
failure refuses the op. And a table that FAILS to re-derive while unchanged
does not exist by construction: the hash gate re-derives only on change.

**The staleness triple does not apply.** `as_of`/`live`/`changes_seq` exist
for view-shaped answers computed at a fingerprint that may trail the live one
(§10.1). `mounts` re-derives per call by construction, so the answer is always
current-tense — and the binding file lives outside every workspace's hash
domain (`~/` is no workspace), so no workspace fingerprint could describe it.
Do not bolt the triple on.

**Why it is not a bare `read`:** an optional `read.ref` would turn a missing
required argument — today a loud error — into a success returning content the
caller never asked for. `read`'s arguments stay required; discovery is its own
op.

### A.6 The frontmatter scalar law — decode on read, encode on write

*Docs-first (2026-08-07, dogfood season 1 findings 1 and 2). One law, two
directions: what the engine PUBLISHES as a property value is the decoded
string, and what the engine WRITES for a property value is a YAML scalar that
decodes back to exactly the caller's string. Read and write are inverses, and
nothing between them is quote-tolerant.*

**The defect this closes.** Every property value plane on this wire is a plane
of STRINGS — `props[].value`, the `fm_key` `cat` remainder, and the
`set_property` value are all `string`, never a YAML node. Before this law the
engine served the value's SOURCE BYTES on the read side and wrote the caller's
string as SOURCE BYTES on the write side, so quoting was in neither
direction's contract and the two ends disagreed with the corpus:

- **Read, fail-INERT.** `owner: "3f9a1c07"` served `"3f9a1c07"` — 10 bytes with
  the quotes — so a comparison against `3f9a1c07` was false, no rule armed, and
  the face rendered the legitimate "no effects armed". A silent false.
- **Write, fail-CLOSED.** `set_property owner=[[b1892b5a]]` emitted
  `owner: [[b1892b5a]]`, which is a list-of-list, so the I4 substrate law
  refused the write and blamed the caller's value for a nesting the EMITTER
  manufactured.
- Net: the engine could read the fleet-canonical `owner: "[[b1892b5a]]"` and
  could not write it. Superseded wording, § A.3 `value` bullet, verbatim:
  *"the key line's value as stored: the colon remainder, whitespace-trimmed,
  quotes kept … The engine re-serializes nothing (no YAML library — honest
  limit, stated not worked around)."* The limit was honest about the library
  and wrong about the plane: unquoting a scalar is not a YAML library, and
  serving source bytes where the schema says `string` is not an honest limit.

**A.6.1 Decode (every read seam).** A value is unquoted when — and only when —
it is a **well-formed** quoted scalar: `'…'` with interior `'` only as `''`, or
`"…"` with no unescaped interior `"`. Single-quoted resolves `''`→`'` and
nothing else; double-quoted resolves `\\ \" \n \t \r` and leaves any other
escape verbatim. Everything else is served verbatim after the whitespace trim:
plain scalars, flow collections (`[a, b]`), and **malformed quoting, which no
reader may guess at**. A quoted scalar is a STRING in every schema — the
decode is the quoting layer only, never type inference.

The law binds the VALUE seams — every seam that publishes a frontmatter value
to a consumer, or compares one against a caller-supplied string. One owner
implements it (`model::scalar`) so the def checker and the read seams cannot
drift into two dialects. The enumerated set, audited 2026-08-08:

| Seam | Plane |
|---|---|
| composed read `props[].value` (§ A.3) | published value |
| a script's `fm` dict (`fm_key` value) | published value |
| the run plane's frontmatter binding values | published value |
| `preset`'s `^properties` rule check and its `type`/`defines`/`root`/`births` reads | compared value |
| `realise`'s `FieldEquals` — BOTH halves: the page's declared `realise.expected` and the observed field | compared value |
| the view projection's `frontmatter.value` column — and the `card` pivot and B2 tag parse riding it | published value |

**Why the last two rows joined (2026-08-08).** They read a value and compare it
against a caller-supplied string, which is exactly the shape § A.6's read-half
defect took: a fleet-canonical `status: "done"` compared raw against `done` is
false, no rule fires, and the face renders a legitimate-looking "no violation".
A silent false in a reconciliation loop is the same defect as a silent false in
a script condition, so the same law governs it. Both halves of a comparison
must decode, or the decode moves the mismatch instead of closing it.

**Why the view row joined (2026-08-08, the d5654f18 non-scope follow-up).**
This paragraph formerly named "the `view` index rows" in the stays-raw list
below; superseded wording, verbatim: *"`lock` (guard tokens), the `view` index
rows, and `policy::change`'s `diff_fields` all answer questions ABOUT THE
STORED BYTES."* The reasoning was right for `lock` and `diff_fields` and wrong
for this column: the view's `frontmatter.value` consumers — the `card` board
pivot over `type`/`status`/`owner`/`session`, `mrd sql` operator and agent
queries, the B2 tag parse — all ask VALUE questions, and a board predicate
`owner = '3f9a1c07'` compared against raw `"3f9a1c07"` is the read-half
silent false wearing a WHERE clause. The stored-bytes questions the view DOES
answer live in its locator and rev columns (`span_start`/`span_end`,
`node_rev`, `file_rev`), which § A.6.2 governs and which stay raw-computed.
The `value` column was the only column serving bytes where its own schema
comment says value.

**What stays raw, and why it is not an omission** (§ A.6.2's reasoning, the
same stance `cat` takes): `lock` (guard tokens) and `policy::change`'s
`diff_fields` answer questions ABOUT THE STORED BYTES.
A quoting-only edit IS a change to the stored form, and a differ that decoded
would report no change where the file's bytes moved.

**Named residual, not silently left:** `policy::change`'s `DocFacts.frontmatter`
— the `(key, value)` pairs the effect kernel's `on_change(event)` receives — is
a published value plane by this section's own test and still serves stored
bytes. It is out of this amendment's scope (it lands with the change-kernel's
own contract work), recorded here so the next reader finds it named rather than
missed.

**A.6.2 The stored form stays raw where hashing is the point.** `prop_rev`,
`span`, the props fingerprint (`props1`) and every node rev are computed over
SOURCE BYTES and are untouched by this law. This is not an inconsistency, it is
the reason the law is safe: a guard token must distinguish `owner: ""` from
`owner:` — the R4 three-state law (absent ≠ null ≠ empty string) lives in the
stored form, where the distinction exists. Decoding at the hash grain would
collapse two states into one and weaken the guard. The published VALUE plane
and the GUARD plane answer different questions, and only the first one is a
string.

**A.6.3 Encode (every value-plane write door).** The emitted line is
`{key}: {encoded}`, and the encoding is the inverse of A.6.1: **emit the plain form when the plain form decodes back to
exactly the caller's string; otherwise emit a double-quoted scalar** (`\` and
`"` escaped). The quoted form is the fleet-canonical one — the spelling
`ccc-cli task claim` writes — so a value this engine writes and a value the
fleet writes are the same bytes. Concretely, a value is quoted when it:

- is empty, or is a null spelling (`~`, `null`, `Null`, `NULL`) — the plane has
  no null, so emitting one would forge a type the caller cannot express;
- starts with `'` or `"` — the plain form would be decoded back as quoting;
- would parse as a **map or a nested collection** (`{…}`, `[[…]]`, an
  unterminated `[…]`) — the I4 nesting was the emitter's, never the caller's;
- carries `: ` unquoted, starts with `#`, or carries ` #` — a mapping or a
  comment in value position.

Unchanged, deliberately: a **typed scalar** (`true`, `7`, `2026-08-07`) and a
**one-level flow list** (`[a, b]`) still emit verbatim. Those spellings are the
only way this string plane can author a non-string value, and no reported
defect touches them. A newline in a value is still REFUSED, never sanitized: a
single-line frontmatter value cannot carry one, and an escaped-scalar workaround
leaks.

**A.6.3a The write doors this encoder owns (2026-08-08; the birth door and the
two caller-facing `fm_key` value scopes added 2026-08-09).** FIVE doors write a
frontmatter VALUE, and all five encode:

| Door | Path |
|---|---|
| `set_property` (and the `check_write` candidate sharing its owner) | the splice plan lowering |
| `put{at:"upsert"}` on an `fm_key` target | the native wire write door |
| the preset BIRTH door — a `^template` placeholder standing in a frontmatter value position | `preset::new_record` / `unfold` (run-plane Law 3.6) |
| `put{at:"end"}` on an `fm_key` target | the native wire write door, composed at the door |
| `match` on an `fm_key` target | the native wire write door, composed at the door |

The upsert door is a value-plane door: its `text` is a caller's flat STRING,
never a YAML node, so the same encoder governs it. Without this, the wire's own
door could not write the value its own read seam decodes — `[[b1892b5a]]` would
land as a nested flow sequence and the I4 substrate law would refuse it,
blaming the caller for nesting the emitter manufactured. **All five doors
refuse a multi-line value** — the encoder's `MultiLineValue` refusal, uniform at
every value-plane write door. A newline is refused, never sanitized.

**Why the birth door joined (2026-08-09, dogfood pass 1 f03).** It was the
door that had never been named here, and the omission was measurable: `mrd new
--actor $'zt\nstatus: closed'` against a template carrying `owner: {{actor}}`
interpolated the caller's SOURCE BYTES, so the born record carried `status:`
TWICE. § A.3's props plane serves one row per key, first occurrence wins, so
disk said `closed` while every read door served `open` — and no governed edit
could reach the shadow line (`fm_key` addresses the first occurrence; a `match`
needle over the served value finds 0 occurrences). The birth door wrote bytes
only a non-meridian editor could remove.

Two laws bind together here and name one answer. Run-plane Law 3.4 stamps
`actor`/`now` **exactly as given**, so sanitizing the caller's identity is
forbidden — a door that strips a newline falsifies the provenance it records.
§ A.6.3 refuses a multi-line value at every value-plane door, because a
single-line scalar cannot carry a newline and an escaped-scalar workaround
leaks. Together: **encode what is representable, refuse what is not, alter
nothing.** A multi-line `--actor` therefore REFUSES THE BIRTH (`bad_request` /
`fix`, the uniform sentence plus the placeholder that carried the newline), and
a representable value — `zt: closed`, `[[b1892b5a]]` — is born quoted and
decodes back to exactly the caller's string.

**Byte-level consequence, stated rather than discovered later:** a born value
that trips a § A.6.3 quote trigger now lands in the canonical quoted spelling
(`owner: "[[b1892b5a]]"` where the pin wrote `owner: [[b1892b5a]]`). The
DECODED value is unchanged, the plain form is still emitted whenever it decodes
back to the caller's string, and the encoder is the shared one — a birth-door
dialect of its own is exactly the drift this section exists to prevent.

**The rule the table's silence used to leave (2026-08-09, ruling
`0021-fmkey-value-grain-ruling`).** The table enumerated its doors one by one and said nothing
about the OTHER scopes reaching an `fm_key`, so `at:"end"` and `match` wrote
raw: `owner: seedhand: x` at exit 0, YAML that no external parser accepts, and
`hand #c` committing with the comment silently dropped. v1.0.0 behaviour at four
engines. **The line that decides every future scope, so the next one falls under
a rule instead of a silence:**

> **A CALLER-FACING value scope on an `fm_key` target is VALUE-grain — the
> engine owes the encode. An ENGINE-INTERNAL lowering slot is LINE-grain — it
> carries a pre-composed line and stays raw.**

`at:"end"` and `match` are caller-facing: their input is a fragment of a VALUE,
so the door composes stored + fragment, encodes the WHOLE result, and writes it
as a span replace. `at:"all"` and `at:"content"` are the lowering's own slots —
A.6.3a′ lowers `set_property` through `at:"all"` carrying an already-encoded
line, so encoding or refusing there would break `set_property`. One target, one
grain: the `fm_key` address is the value plane, and a caller addressing a key is
thinking in values.

**The composition is AT the door, never below it.** The encoder takes a WHOLE
value while these doors supply a FRAGMENT, so "route the door through the
encoder" is not an available shape — encoding the fragment yields
`owner: seed"hand: x"`, broken in a new way. The door composes and lowers to a
`put{at:"all"}` span replace; the kernel stays raw-grain per the sentence below.
**The receipt renders the CALLER's edit** (`put:end`, `match`) — the lowering is
engine-internal mechanism, exactly as `set_property`'s lowering renders today
(A.6.3a′ precedent). Armed facts state true before/after revs regardless.

**Uniform means the WORDS too (2026-08-09, dogfood s7).** One law refused in two
dialects is two laws to the callers who meet it: the `set_property` door named
the offending key and taught the executable escape (*"frontmatter values are
single-line in v1; put multi-line content in a body section"*), while the upsert
door said only *"put at:upsert value must be single-line (no newline)"* — no key,
no remedy. Recovery quality became a function of which door the caller happened
to enter. Both doors now carry the same sentence: the key by name, the v1
single-line rule, and the body-section escape.

**A.6.3a′ One armed fact per key — the `set_property` CREATE arm is the upsert
door (2026-08-09, dogfood s11-40/s11-50).** The plan lowering emits ONE edit per
key, each targeting its OWN `fm_key`: an existing key as `put{at:"all"}` over
its line, an **absent key as `put{at:"upsert"}`**, which is the only shape that
addresses a key the document does not carry yet. It is therefore the same door
row above, reached by lowering rather than by a caller, and it encodes there —
`set_property`'s own multi-line refusal still fires first, in the door's words.

Why the grain is law and not style: armed facts carry *op, target identities and
rev transitions* (§6.1) and a node entry names the deepest node containing each
changed byte range (§7.1), so a fact must name the key that moved. The former
lowering folded every create onto the **last existing key** with `put{at:"end"}`
— a batch setting `owner` and `status` over frontmatter holding `title` armed
and receipted `title put:end <rev>-><same rev>`: an identity the batch never
wrote, an op nobody asked for, a transition claiming nothing changed while two
keys landed, and a count two short of the intents. Facts are the normative
receipt content (§6.4), so the collapse made every props write unauditable and a
§11 lint asserting receipts against intents would false-negative on all of them.

Consequence carried deliberately: a created key lands at the upsert door's
insertion point (first-key position) rather than after the last key. Key
ORDER inside the block is not a law of this contract — the auditable identity
of the write is.

**Teaching row — the create arm's `node_rev_before` is `blake3("")[:16]`, and
that token is not a claim that an empty key existed.** No node stood at the
address, so the door arms the empty-input hash (`af1349b9f5f9a1a6`) as the
before-token. A.6.5 keeps ABSENT and EMPTY apart as distinct ratified states;
armed facts do NOT — a consumer reading facts alone cannot tell born-from-
nothing from born-from-empty, and must read the op (`put{at:"upsert"}` is the
create arm) rather than the token. The same arm births a MISSING frontmatter
block outright when the document carries none.

**The kernel below the doors stays raw-grain.** `model::plan_fm_upsert`
composes the value verbatim, because the run plane's `md.set_field` writes
WHOLE-VALUE grains through it and their spelling must land as sent. The encode
belongs at the door, where the input is known to be a flat string, and nowhere
below it.

**A.6.3b The splice consumer reads the ENCODED value.** One `set_property`
lowering splices the VALUE SPAN of an existing key rather than composing a
whole line: the def-plane `rebuild` path, and the `check_write` candidate that
shares its owner. There the separator guard — the one that inserts the space in
`{key}: {value}` over a stored bare `key:` line — must test the ENCODED bytes,
not the caller's string.

*(Located precisely, 2026-08-08: the WIRE's own `set_property` lowering
composes the full `{key}: {value}` line and never reaches this guard, so a
wire-door test cannot cover it and a wire-door matrix that passes says nothing
about it. Coverage belongs at the `rebuild` door — measured, and
mutation-proven there.)* The two differ exactly where this law bites:
the empty string encodes to `""`, so a guard on the caller's value sees "empty,
no separator needed" and emits `note:""` — which no external YAML parser reads
as a property. One malformed line voids the whole frontmatter block for
yaml.v3, PyYAML, Obsidian and `ccc-cli` alike, so the failure is not local to
the key that was set.

For the same reason a CREATE has ONE line shape, `{key}: {encoded}\n`. The
former empty-value special case emitted a bare `{key}:\n` — a YAML null, the
type A.6.3 says this plane cannot express, forged by the engine out of a
caller's empty string. The encoder never returns empty bytes, so the uniform
shape needs no special case to be correct.

**A.6.3c Spelling preservation on a semantic no-op (2026-08-08).** An UPDATE
whose stored spelling already decodes to the caller's string keeps the stored
bytes: when `decode(stored)` (§ A.6.1) equals the caller's value, and the
stored spelling classifies as neither `Nested` nor `Null`, the door keeps the
stored value bytes verbatim instead of re-encoding. The read-modify-write of
an untouched value is therefore byte-stable — `owner: "3f9a1c07"` reads as
`3f9a1c07` and writes back as `owner: "3f9a1c07"` — and nothing computed over
SOURCE BYTES moves: `prop_rev`, `span`, the `props1` fingerprint, and any pin
held over the key survive the no-op. The two writers § A.6.3 names stop
oscillating: the fleet's quoted spelling and the engine's plain emit are each
fixed points under the other's write-back. ONE owner implements the predicate,
beside the encoder, and every § A.6.3a door consults it on update — the
value-span splice keeps the span bytes; the line-composing doors keep the
value spelling inside the one `{key}: {spelling}` line shape.

Excluded, deliberately — each is a standing law outranking byte quiet:

- **A stored NULL spelling** (bare `key:`, `~`, `null`): a text-equal
  write-back still re-encodes to the quoted string. Preserving would leave the
  one type this plane cannot express (§ A.6.3) standing under an `ok` string
  write; R4 demands the write of a string LAND a string, distinguishable from
  the null it replaces.
- **A stored NESTED spelling**: preserving would leave the I4 class in place
  under an `ok` write. The no-op write repairs it to the quoted canonical
  form instead.
- **A multi-line caller value**: REFUSED (D11) before preservation is
  consulted, so a stored escape spelling that decodes to the same text cannot
  smuggle a newline past the uniform refusal.

Preservation is of the VALUE spelling, never the line geometry: a nonstandard
stored geometry (a doubled separator space) normalizes once at a line-composing
door and is byte-stable thereafter.

**A.6.4 What conformance means here.** Round-trip is the test, per direction and
composed: a fleet-canonical quoted value reads back without its quote bytes, and
a `set_property` of an `[[id]]`-shaped value lands quoted and reads back as the
caller's string. A quote-tolerant comparison ANYWHERE — in a host, a caller, or
a second engine seam — is a defect against this section, not a compatibility
measure.

**A write-back that CHANGES the value may RE-SPELL; a semantic no-op may not**
*(amended 2026-08-08 by § A.6.3c. This paragraph formerly opened "A write-back
may RE-SPELL, and that is not a byte no-op" and closed "Making the round trip
byte-stable is a separate change to the encoder's canonical form, carded on
its own; this section does not claim it" — that card landed as § A.6.3c)*.
Decode and encode are inverses on the VALUE, not on the bytes: a write that
lands a DIFFERENT value emits the encoder's spelling, and anything computed
over SOURCE BYTES moves with it — `prop_rev`, `span`, the `props1`
fingerprint, and any pin held over the key (§ A.6.2's planes are exactly the
ones affected). A write-back of the value a read served is the no-op
§ A.6.3c pins byte-stable, under its three named exclusions. A caller
comparing across a read-modify-write still compares values, never tokens: the
exclusions re-spell, and a value CHANGE never promises byte geometry.

**Round-trip alone is not the test.** A conformance test asserts the STORED
LINE SHAPE, byte for byte, and only then the round trip. The engine's own
decode is tolerant by design, so a value-only assertion passes over bytes that
no external parser accepts: `note:""` round-trips through this engine and voids
the frontmatter block for everyone else. A test that asserts the value and not
the line is the escape hatch the A.6.3b defect hid behind. The bytes are the
contract; the round trip only proves the engine agrees with itself.

**A.6.5 R4 binds the DEF plane too — the empty string is empty**
*(**RATIFIED** by ZT, 2026-08-08, relayed via `2c47b75e`)*. A.6.3 makes every value-plane write door emit `key: ""`
for an empty value. The def plane reads the TYPED frontmatter value, where that
lands as a string rather than the YAML null, so every emptiness predicate
written against the null alone silently reads a released card as still set.
Measured before the repair: `set_property(owner, "")` on a card whose def marks
`owner` required returned `ok`, landed `owner: ""`, and PASSED conformance — the
bare-null spelling refused the identical write. `closed_at: ""` satisfied the
terminal biconditional, so a card reached a terminal status carrying no close
time and the close-stamp autofill minted no repair.

**The ruling: the predicates learn the second spelling.** A key is empty when
it is absent, when it is the null, or when it is the empty string. This aligns
the code with its own refusal text, which already reads *"missing or empty"*,
and it keeps R4's three states — absent ≠ empty ≠ set — readable at the def
grain where they are judged.

**Rejected: emitting a bare null instead.** It re-opens exactly what A.6.3
closes — forging the one type this string plane cannot express — and reinstates
the A.6.3b splice geometry. The value plane and the def plane must agree on
what empty means; they may not disagree about which SPELLING of it is real.

Deliberately not empty: whitespace, `0`, `false`, an empty list. Those are
values a caller authored. This is a predicate about absence, never truthiness.

---

## § B. Process

1. Edit this file (or the relevant SPEC under `docs/`) **before** code.  
2. Do not reintroduce versioned contract files or amendment piles.  
3. Optional history only: `worker-log.md` (deletable).  
4. **UNVERIFIED** when evidence is missing.
