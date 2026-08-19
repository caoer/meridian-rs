---
type: contract
id: wire
status: standing
updated: 2026-08-18
description: Standing wire constitution. One document. Docs define law; code may lag.
owns: [the wire constitution — nouns, ops, guards, receipts, errors]
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
| `Path` | string, `/`-separated, workspace-relative, UTF-8 | never absolute; no `..`; the workspace root is ambient (`fs::WorkspaceRoot`); the agent-plane `[root:]path` spelling resolves at the DOOR — the wire carries the rel half only (§ A.12) |
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
| anchor | `{"anchor":"r-000042"}` | block id, exact match; the resolved node is the id's HOST BLOCK under the Obsidian attachment law (F-R4, `model::anchor_host_span`: a tail id keys its enclosing block — the whole paragraph run, callout or table, a list ITEM line, a heading line; an own-line id attaches to the nearest preceding block through blanks, or joins a directly-adjacent paragraph/list item; a document-start orphan and a frontmatter caret keep the marker's own line). Duplicate id in one file → the mint plane refuses `ambiguous_ref` (loud), while the walk plane follows the app (last wins, silent) — the silent-last-wins mint death mode is closed |
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

Correlation: one response per request, id echoed by value; in-flight uniqueness required. *(Amended 2026-08-12, §18 row 14: the `MAX_FRAME_BYTES = 256 MiB` corruption bound is STRUCK. It was a protobuf varint length-prefix bound standing from `crates/transport-proto`, a crate deleted 2026-08-03 (8a57a3da — Law 1.4 keeps the seam JSON-only). The seam is NDJSON — newline-delimited lines, no length prefix (`crates/transport` NdjsonCodec) — and serves no frame-size bound; none is invented here. Whether NDJSON wants a line-length cap is an open question, recorded at §18 row 14.)*

### §3.2 hello / caps (proto-1 retained)

```json
{"id":1,"op":"hello","proto":1,"client":"md-cli/0.3"}
{"id":1,"ok":true,"body":{"proto":1,"server":"meridian-daemon/1.0.0",
  "caps":["toc","cat","extract","resolve","resolve.content","links","links.require_fingerprint",
          "splice","splice.if_node_rev","splice.if_fingerprint","splice.dry","splice.receipt",
          "splice.verdicts","fingerprint","diff","sub"],
  "fingerprint":"b3:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9"}}
```

`caps` is the complete set — no version sniffing, ever. The example shows the sixteen-cap base set (the v2 spelling is byte-identical minus the fingerprint renames); a negotiated v3 session is pushed eighteen more on top — `read`, `check_write`, `splice.plan_edits`, `splice.pin`, `pin-cross-root`, `splice.pin.proof`, `splice.set`, `splice.create_rev`, `create`, `remove`, `mounts`, `mounts.primary`, `hello.identity`, `script`, `run`, `walk`, `sql`, `scoped-guards` (§A.3/§A.5/§A.7/§A.8/§A.10/§A.11/§5.4) — thirty-four caps in all. Field-only amendments ship as dotted `op.field` strings (`mounts.primary` is one: the mounts row's declared-primary designation, §A.5); `pin-cross-root` is a behavior cap on the existing `splice.pin` field (§A.3). `scoped-guards` is a behavior cap in the `pin-cross-root` pattern covering the whole scoped-premise family at once — the `guards[]` list (each entry's `scope`/`scope_bytes` premise pair), the singular `scope` field on `splice` (single and set form) and `script`, and the `fingerprint` op's mint arm with its own `scope`/`scope_bytes` pair (§4.7, §5.4–§5.7; `scope_bytes` is a top-level field on no door — §5.4's field matrix): one family, one flag. A frozen v2 session is never pushed it, and un-negotiated use of any guard-family field refuses `bad_request` loudly at this section's strict wall — never silence. `fingerprint` in the hello body is optional (the engine may not have walked yet); when present it is the first ambient fingerprint.

**Hello is config-grade (ruled 2026-08-16, roots-hello-starved).** A workspace `hello` pins storage, validates the domain CONFIG (an ambiguous or unreadable config still refuses `io_error{cause}` at the handshake), and binds the connection — it never walks the corpus, never builds the engine, and never queues behind corpus-scoped work (the resident fold is read without waiting; under lock contention the field is absent, the same honest answer as cold). `fingerprint` is therefore present exactly when a resident engine holds a fold that is readable this instant; a cold workspace answers without it, and the first corpus read *starts* the warm (next paragraph). Why ruled: `hello`'s former inline warm let one client's cold whole-corpus build hold every other client's `hello` — and `mounts` behind it, the op § A.5 defines precisely for the caller that knows no root yet — past the face's own deadline (dogfood 2026-08-16: `roots` timed out while one cold `links` scan ran). A discovery op answers at config cost.

**The cold build never blocks the read door for minutes (ruled 2026-08-16, post-promote-corpus-warm).** An op that serves from the warm engine — the read family, `sql`, the `script` entry pass — against a workspace with NO resident engine (the state every workspace is in right after a daemon restart) starts the drawer rebuild in the BACKGROUND (one rebuild per workspace, however many callers ask) and gives it a short bounded wait. A small drawer lands inside the wait and the read SERVES on first contact — first contact with an ordinary workspace never changes shape. A drawer still rebuilding when the wait expires refuses `corpus_warming` (§8, retry), and every further corpus read while that rebuild runs refuses the same in milliseconds; the first read after the rebuild lands serves from it. The wait's value is engine-internal and deliberately unpublished (host deadlines stay host knowledge, §8.1) — the guarantee is its ORDER: bounded well under any sane op deadline, never minutes. A rebuild that FAILS surfaces its cause as `io_error{cause}` (env) — to the read that kicked it when the failure lands inside the wait, else to the next corpus read — and warming never masks a broken corpus: a later read starts a fresh rebuild. A WARM workspace's currency pass is unchanged: it stays inline at every read (incremental — O(domain) in `stat`s, O(delta) in parses; `run-plane.md` § What an entry costs), so only the cold whole-corpus build leaves the read door. Why ruled: the inline cold build read as a hung product — after an install restart every corpus read blocked for minutes (a `toc` timed out at 20 s twice) while `hello`/`mounts` answered in milliseconds, so the green config plane said "up" over an unservable corpus (dogfood 2026-08-16). The refusal makes the drawer's state a wire fact instead of a timeout guess. In-process registries (the CLI's direct lane and test fixtures) keep the inline build: with no daemon and no deadline on the other end, blocking IS the honest answer there.

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

Eleven ops in this table (the § A.3 standing additions `read`, `create` and `remove`, and the § A.7 `script` op, land on top, not re-tabled here). The five-verb interface maps onto the original ten 1:1 (§4.8). Read ops are classified by the wire-op criterion: feeds-an-action → wire fact op; feeds-orientation → dashboard-only, NOT on this wire.

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
request object is refused in the seam's own words (`the edits on stdin are not
the §4.4 batch shape … stdin takes the BARE edits ARRAY, not the wire §4.4
request object: send the value of its "edits" field (id / op / path are argv's
here)`, exit 2 before any engine contact). The
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

**Teaching row — `at:"end"` on a LINE-grain anchor host always refuses; two families split which way it dies (family split measured 2026-08-09; grain qualifier added by F-R4, 2026-08-13).** A block-leaf span excludes its terminator, so the insertion point sits on it, and an end-append to an `{"anchor":id}` target whose host is a LINE (a list item, a heading line) lands one of two refusals — never a commit:

| the `text` you send | what the reparse measures | `family` |
|---|---|---|
| carries no newline (` tail`) | the appended bytes join the line, so the id is no longer line-final: the target stops resolving | `target_identity` |
| carries a newline (`\nX`) | the first newline terminates the host line, so the bytes land in a NEW line outside the node: the target still resolves and its rev cannot move | `transition_unrepresentable` |

Since F-R4 an anchor's host is the attached/enclosing BLOCK, not always one line, and the refusal pair is defined over the SPAN LAW, not the door: an end-append whose bytes stay INSIDE the host block — a paragraph run growing a continuation line, a table absorbing a row — changes the node's bytes, arms a true rev transition, and commits. The remedy on the refusing shapes is unchanged: an append to a line-grain anchor must re-supply the id line-final in its own `text`, and an append that means to add a LINE belongs to the enclosing section, not to the anchor. *(Correction rationale: this row previously read "the `target_identity` family is the WHOLE of `at:"end"` on an anchor". The refusal it required stands and is unweakened; its single-family attribution was wrong, and the newline half was measured COMMITTING at v1.0.0 `93184797` and at `b1fcc6e3` — silently, exit 0, with a null rev transition. The same escape was measured on `fm_key` targets, whose leaf span excludes its terminator by the same §4.4 law. The 2026-08-09 measurement "host block kind is NOT the discriminator" held while every host was line-grain; F-R4's block-grain hosts made GRAIN the discriminator — the span law itself is unchanged.)*

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

**The set form (dotted cap `splice.set`, v3-only; ruled 2026-08-14 — OQ1: any
v3 client holding the cap).** A splice request MAY carry
`files:[{path, edits|plan_edits}, …]` instead of `path` + `edits`/`plan_edits`
— strictly one form or the other (`bad_request` at decode when both or
neither appear; the same wall as `edits` vs `plan_edits` today). Two or more
entries; paths pairwise distinct; one request-level `if_fingerprint` (or
`guards[]` — scoped premises, §5.4, 2026-08-15),
`actor`, `now`, `receipt`, `dry`, `force`. Per-edit guards ride inside each
entry unchanged; no `pin` (the pin rides the single form, whose `path` is the
pinning page). The batch laws of this section apply per file (pre-batch
resolution, disjointness, one reparse per file, the `would_corrupt`
families); the guard law is §5.1 unchanged — a scope-less `if_fingerprint`
is checked first and, being world-grain, covers every entry (scoped
premises on the set form answer to the §5.5 Coverage Law at admission). **The commit is sealed
across the set**: every entry validates before any byte lands
(validate-all-then-apply), one fingerprint advance covers all files plus the
receipt, one receipt entry (one anchor, §6.6 checked once) names every file,
one Delta (§7.1) carries every file. A validation refusal anywhere answers
for the whole request with nothing landed — the refusal names the entry
(`files[i]` and its path) that measured it. Response: `armed` becomes an
ARRAY of per-file armed groups (`[{path, file_rev_after, edits:[…]}, …]`,
request order), one `fingerprint_before`/`fingerprint_after` pair, one
`seq`. Crash posture — including the case where the in-memory restore
ITSELF fails — is §6.5, the set paragraph; this document commands no
journal on any set path (ruled 2026-08-14, and folded through rather than
appended). The cap ships by the §3.2 evolution law (the
`splice.plan_edits`/`splice.pin` precedent); v2 sessions and cap-less v3
sessions are byte-identical to today.

**Two ceilings, named separately, because they bound different audiences
(amended 2026-08-15, adversarial review F1).** The wire set cap and the
script arm budget are not one number read twice:

| ceiling | binds | value |
|---|---|---|
| **wire set cap** | any v3 client holding `splice.set` — the widest audience OQ1 opened | **the corpus is the bound (ruled 2026-08-14): no engine-minted numeric cap.** A set names existing, pairwise-distinct corpus files, so the corpus's own file count is the ceiling |
| **script arm budget** | the in-process script evaluator only (§A.7) | `max_armed_edits` = 64 — arming past it faults the ATTEMPT, never a transport limit, and a wire client passes through no evaluator at all |

Both are stated because the face-honesty clause requires it: a limit that
can refuse must be discoverable before it refuses. Reading only the script
number under-builds the wire caller — the first real batch anyone ran was
103 files, so 64 fails on contact at that door. **Moving the wire bound is
ZT's alone (overrule window open);** the measurement below prices the ruled
bound and proposes no cap.

**The price, measured end-to-end rather than projected (amended 2026-08-15,
review F1/F2 against the live set form).** Hold ≈ per-attempt term(corpus)
+ N × per-member marginal, and the two terms behave differently:

- **The per-attempt term is O(corpus), not O(N):** ~57 ms at 200 docs,
  ~210 ms at 6696 docs (entry fold + post-commit re-fold + commit residue —
  the commit leg folds TWICE). This is what the set form amortizes: paid
  once per sealed set instead of once per file.
- **The per-member marginal is corpus-independent and linear with no knee
  to N=1024:** 10.2–11.0 ms/member at both corpus sizes (validate
  ≈ 0.2–0.4 ms; stage + fsync + rename ≈ 10 ms — fsync-dominated). The
  in-crate bench reads 12.3–13.1 ms/file on a synthetic tree; treat
  ~11–13 ms/file as the level band.
- **The resident daemon does NOT amortize the fold across calls:** a wire
  single-form commit (236 ms @6696 docs) equals the CLI's (232 ms). Every
  lane pays the per-attempt term per attempt, which is exactly why the set
  form pays: 16.6× at N=64 and 21.2× at N=1024 against N single commits,
  same door and corpus.
- **Wall = hold.** At every measured cell ≥1 s the exclusive-flock hold band
  is ~100% of the end-to-end wall; there is no unheld phase worth pricing.
  So the denial table for every other writer on that workspace is the commit
  column read directly — at 6696 docs: N=1 ≈ 0.24 s · N=64 ≈ 0.91 s ·
  N=256 ≈ 2.98 s · N=1024 ≈ 11.4 s, and a whole-wiki sealed set extrapolates
  to ~73 s. A competing writer is refused `workspace_busy` in ≤0.1 ms —
  immediately, with no engine retry and no queue, so waiting is entirely the
  caller's policy. Two framings of one total, both true: the per-file lane at
  N=1024 holds ~242 s across 1024 windows each offering an interleave; the
  set holds ~11.4 s in ONE window. Hosts ratchet stricter per §5.3; the wire
  stays permissive.
- **Levels are quiet-darwin claims** (M4 Max/APFS, load 5.3–5.9, KB-scale
  members); the shape claims — linearity, the corpus-independent marginal,
  wall = hold, immediate refusal — are the load-robust ones. Curve,
  instruments, weaknesses and refutation commands:
  `12-04-f2-mrd-integration` `results/splice-set-batch-bound-measure.md`
  (engine `fcd4b7a1`); fold half: `results/sqlwrite-fold-evidence.md`.

No knee exists anywhere on that curve, so a finite N minted here would name
a boundary the mechanism does not have. The bound stays where it was ruled.

**Sweep composition — what the set form does NOT seal (added 2026-08-15,
review F3).** Sealing is per ATTEMPT. The workloads that motivate
corpus-wide write-back are 5827 and 2635 files; a sweep that does not fit
one attempt's tolerable span is k sealed sets plus at most one refusal, and
**cross-set atomicity does not exist** — the world may move between sets.
Three facts a first caller otherwise rediscovers by refusal:

1. **List-building is the caller's plane.** The wire serves no
   corpus-enumeration op by design, and content predicates ("docs missing
   `created_at`") are not glob-expressible, so `files[]` arrives from an
   enumeration plane — §A.11 `sql`, or the caller's own. Patterns in
   `files[]` (§A.7, ruled 2026-08-14 OQ3) expand NAMES, never contents.
2. **The script lane faults at arm 65.** A glob matching 200 files does not
   chunk itself; the attempt dies inside the evaluator's budget, above.
3. **The sweep loop, written once so it is not re-derived:** sets in sorted
   order, one `if_fingerprint` per set chained from the previous commit's
   `fingerprint_after`, stop at the first refusal, re-enumerate and resume.
   An idempotent predicate converges because a re-run's expansion is empty —
   the script plane's own termination word (`no_effect`, §A.7). A torn
   sweep's archaeology is k sealed-set receipts plus one refusal, and it is
   readable because ONE receipt entry names every file of its set, which no
   consumer can confuse with N per-file entries.

**Visibility is a lane property, not a set property (added 2026-08-15,
review F4).** One `seq` and one Delta are minted where a daemon serves the
write. On in-process lanes there is no seq sink: the commit answers
`seq: 0` and mints no Delta, so a sealed set — however large — is invisible
to every watch-plane consumer. That is §18 row 12's declared debt read at
set grain, not a second defect; cross-lane catchup stays diff-by-root
(§4.7). A watched corpus writes sets through the daemon door.

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

**An unresolved edge may carry `unresolved_reason` (session decision 0034).** Four distinct facts used to collapse to the single word `unresolved` — the three §12.1 exclusion classes and a plain broken link — which left a deliberately unhashed file INDISTINGUISHABLE FROM A TYPO at every face. The map keys a subset of `unresolved` by the same linkpath and carries the §12.1 rule word that decided it: `non-md`, `dot-segment`, or `custom-ignore`.

```json
 "files":{"notes/plan.md":{
   "resolved":{"receipts/2026-07-18.md":1},
   "unresolved":{"roadmap":1,".private/secret":1},
   "unresolved_reason":{".private/secret":"dot-segment"}}}
```

Three properties are load-bearing, in the order they matter:

1. **A GENUINE TYPO CARRIES NO REASON.** A key appears only when a real file sits under the path the mint resolved — the literal spelling, the `.md` append rule, or the bare-name fallback below — and the domain excludes it. `[[.private/typo]]` — an excluded directory holding no such file — is a plain miss and stays bare. The reason is a claim about a FILE; without a file there is nothing to claim, and a reason attached to every miss inside an excluded directory would restore the collapse in a new costume.
2. **`resolved` and `unresolved` do not move.** The edge stays unresolved, `resolved` stays a bool, the human word is unchanged: **the app mirror above is preserved intact**. This map is read BESIDE the edge, never instead of it, and is omitted when empty.
3. **The word is minted once** (`fs::domain::LinkTargetProbe`) and the `sql link` projection's `exclusion` column asks through the same mint, so the two edge-map faces cannot name one rule differently.

**The bare-name fallback (ruling 2026-08-14).** A target with no `/` that misses the literal probe resolves by exact basename over the out-of-domain files — an excluded file is absent from the corpus index by construction, so the ambient basename search cannot answer for it, and without the fallback every `[[TAG-FILES.base]]`-style link read as a plain miss. Two guards are part of the ruling: the match is **case-exact** (`abc.BASE` never matches `abc.base` — a case-folding probe would stamp a genuine typo as deliberate), and an ambiguous basename takes the **deterministic tie-break** shortest path then lexicographic. A PATHED spelling never falls back: `git/GIT.base` written where only `sources/git/GIT.base` exists is genuine rot, and a suffix walk would stamp it as deliberate. Stated limit, not left to be discovered: a pathed spelling only a suffix walk could find (a subfolder attachment written with a partial path) stays bare and keeps reading as a plain miss.

**`path` present is a DOOR; `path` absent is an ENUMERATION, and they answer under different rules (§12.1).** Named, the op serves the page even when the hash domain excludes it — a real file outside the domain comes back with its edges resolved against the corpus it is not in, and only a path with no file under the root is `file_not_found`. Absent, the op speaks for the whole corpus and carries **`excluded`**: the workspace-relative markdown under the root that the hash domain does not hold, absent from `files` and named here rather than left to be inferred (§12.1 enumerator clause). The key is omitted when the list is empty, so a workspace whose domain is its whole md tree is unchanged on the wire.

### §4.7 fingerprint and diff — the integrity rung

```json
{"id":90,"op":"fingerprint"}
{"id":90,"ok":true,"body":{
 "fingerprint":"b3:6e866e13b5e65ef9961c050f8a621cf1980b00ee293be650deef5f4dbc6823f0","seq":2}}
```

**The scoped mint arm (`scoped-guards` cap — §5.4; ruled base: D-04).** Under the cap, `fingerprint` takes an optional `scope` (a `Path`, §1) or `scope_bytes` (base64url over the raw path bytes, for names the UTF-8 `Path` noun cannot carry) — exactly one of the two; both absent is the root mint above, byte-identical to v2; both supplied refuses `bad_request` — a mint names ONE node (bounce-2 closure, 2026-08-15; teaching: §8.2, the mint-pair text — the broken-premise-pair text cannot fit this door, since a mint supplies no fingerprint to pair). The op mints the NAMED node's token: the workspace root, a folder, or a file leaf — `fingerprint {scope}` is the one mint home for every premise the §5.4 guard family accepts. A lawful path with no node answers the reserved non-hex value `absent` (§5.6); an unlawful path refuses `scope_unresolved` (§5.6, §8). The response echoes the request's scope pair beside the token, so a caller can never desync what it minted from where. Worked scoped-token *spellings* are the engine's: this document prints no hex an engine did not compute (the interior encoding moves under the width ruling, `node-rev-merkle-spec.md`); the served shape is `{fingerprint, seq, scope}` or `{fingerprint, seq, scope_bytes}`, and `fingerprint: "absent"` is a legal body.

`diff` is reserved AT the integrity rung with its shape standing now — the compound front door:

```json
{"id":95,"op":"diff",
 "from_fingerprint":"b3:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9",
 "to_fingerprint":"b3:6e866e13b5e65ef9961c050f8a621cf1980b00ee293be650deef5f4dbc6823f0"}
{"id":95,"ok":true,"body":{"batches":[ /* Delta seq 1, Delta seq 2 — §7, byte-identical
                                          to the live notification frames */ ]}}
```

Replay ≡ live (§7.3). A fingerprint range outside the retained history → `fingerprint_unknown` → full resync (honest bound: §13.5). `sub` (rung 5) is **served** at the daemon door (`crates/registry/src/server.rs`), and its anchor carries cursor identity (B-01, docs-first 2026-08-15; ring `seq` is per-tree-instance — merkle spec §6.3 — so a number alone can never prove position):

- **Live subscribe:** `{"op":"sub"}` — no cursor → ack `{"root":…,"seq":N,"tree_instance":I}` — the baseline root, so the first push frame's `root_before` matches — then the connection converts to push and carries Notification frames, each one Delta batch, starting after the acked `seq`. The ack is where a client learns its resumption cursor: `{tree_instance, seq}`, `seq` advanced by each delivered frame.
- **Resumption:** `{"op":"sub","tree_instance":I,"from_seq":N}` — instance is evaluated BEFORE any sequence compare. A dead instance (a daemon restart, an idle reap) refuses `root_unknown` with the diff-by-root remedy, sequence never consulted — a previous-epoch number can never anchor when the new ring's counter reaches it again. A live-instance `from_seq` outside the retained ring refuses `root_unknown` exactly as before.
- **Upgrade-required:** `from_seq` without `tree_instance` — anchoring by number alone — refuses `bad_request` with the upgrade teaching; `tree_instance` without `from_seq` is half a cursor and refuses the same way. Neither anchors.

A refused `sub` leaves an ordinary request channel. The delta stream is not actor-scoped: identities, revs, and spans only.

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

`if_fingerprint` compares against the current workspace fingerprint and is checked FIRST — world-grain, cheapest, fails the whole batch (`fingerprint_mismatch{expected,actual}` → re-plan); then per-edit `if_node_rev` (node-grain, `cas_mismatch{expected,actual}` → refresh). Merkle-spec §7 semantics carried; Rust computes hashes; hosts only compare opaque tokens.

*(Amended 2026-08-15 — scoped guards, §5.4.)* `if_fingerprint` with no `scope` keeps exactly this meaning: the root premise, byte-identical to v2. Under the `scoped-guards` cap it is the one-premise sugar for `guards:[{scope?, fingerprint}]`, and the order generalizes without changing what any v2 caller observes: coverage at admission (§5.5) → every supplied premise (§5.4; a scoped refusal names its premise — `fingerprint_mismatch{expected,actual,scope}`) → per-edit `if_node_rev`. A premise refusal still fails the whole batch before any byte lands.

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

### §5.4 Scoped guards — the premise list (served 2026-08-16; ruled base: D-04, fingerprint-grain plan §4.4)

*The family is served behind the `scoped-guards` cap: decode, §5.5 coverage, the scoped fold, the §4.7 mint arm, and refusal `scope` (§5.7). Worked hex tokens are the engine's — the interior encoding changes under the width ruling (`node-rev-merkle-spec.md`) — so the shapes below still carry ellipsis tokens on purpose.*

**Any legal token in the tree is a legal guard (ruling D-04).** A write's world premise is no longer root-only: a premise names any addressable PATH node — the workspace root, a folder, a file leaf — or holds the reserved value `absent` (§5.6). The engine checks every premise the caller supplies. Interior sharding structure below a path node (the radix buckets of the hash law) is never addressable as a scope — premises name path nodes only.

**Shape: a list.** `guards:[{scope?, scope_bytes?, fingerprint}, …]` rides `splice` (single and set form) and `script` — because "I read B and I am writing A" is two premises, and their common ancestor over-covers: two literals under `a/` LCA to `a/`, so a neighbor creating `a/3.md` — a file the plan never bound — would refuse. Singular `if_fingerprint` (+ optional `scope`) stays as sugar for the one-premise case and for v2 continuity; wire `splice` is single-file, so the sugar is sufficient for most put calls. A premise with neither `scope` nor `scope_bytes` is the root premise — the v2 world guard as a list entry.

**`scope` is a JSON field beside the token, never a token encoding** — settled by this section's own geography law (§5.3): requiredness binds to host-side PATH scopes ("`results/**` requires a premise"), and hosts never parse tokens — a path buried inside an opaque token is invisible to the exact plane that applies the policy. The field keeps the token opaque and the geography composable. Pair-validation is atomic at the door: `scope` without `fingerprint` refuses `bad_request` with teaching, so the one-string form's advantage (hash and path cannot desync) is preserved by construction. An `@`-form (`<token>@<scope>`) remains available to FACES as display spelling only — it is never a wire spelling, and no wire surface parses or emits it.

**Raw-byte names are addressable.** `scope_bytes` (base64url over the raw path bytes) rides beside the UTF-8 `scope` convenience — exactly one of the two per premise; mint (§4.7) and guard serve both. This closes the declared non-UTF-8 gap: "integrity-covered but unaddressable" is no longer the posture (`node-rev-merkle-spec.md` §9, amended in step — the UTF-8 read-face serving limits stand there).

**The field matrix — one law at every strict wall (bounce-1 closure, 2026-08-15).** Top level, per door: `splice` (single and set form) and `script` take `if_fingerprint` (+ optional `scope`) as the one-premise sugar, and `guards[]` as the list. **`scope_bytes` is a top-level field on NO door**: a raw-byte premise rides a `guards[]` entry, and the raw-byte mint rides the `fingerprint` op (§4.7) — the § A.7 field wall (12 → 14: `guards`, `scope`) is this matrix applied, and no fifteenth field exists. Sugar and list supplied together are legal: the sugar desugars to one more entry in the premise list, and the engine checks every premise. Per premise (a `guards[]` entry): `{scope?, scope_bytes?, fingerprint}` — exactly one of `scope`/`scope_bytes`, or neither for the root premise; `fingerprint` is required and holds a token or `absent` (§5.6). Pair violations — `scope` or `scope_bytes` without its `fingerprint`, both spellings in one premise, sugar `scope` without `if_fingerprint` — refuse `bad_request` with teaching (§8.2); the mint door's own pair violation — both spellings on one `fingerprint` request — refuses `bad_request` at §4.7 with its own fitted text (§8.2, bounce-2 closure). Effects doors take NO guard-family field (`if_fingerprint`, `guards`, `scope`, `scope_bytes` alike): inapplicable on `run` and beside `effects` on `script` — `bad_request` at the strict wall (§ A.7/§ A.8; teaching: §8.2).

**Guard-path freshness.** At check time the engine refreshes the named premise's own extent: guard one file, pay one file; guard a folder, pay the folder; guard the world, pay the world. Refusals narrow because the PREMISE narrows, never because the engine looked less hard.

**Negotiation.** The family is capability-advertised as `scoped-guards` (§3.2). A frozen v2 session is never pushed it, and un-negotiated use of any guard-family field refuses `bad_request` loudly at the §3.2 strict wall — never silence (teaching: §8.2).

### §5.5 The Coverage Law — legality is not sufficiency

*(Amended 2026-08-15, bounce-1 closure — the sufficiency quantifier re-drawn over the CALLER-AUTHORED write set; ruled: ZT, Arm B, `decisions/2026-08-15-coverage-quantifier-deviation.md`. A recorded deviation from the frozen plan: cite fingerprint-grain plan §4.4 WITH that deviation record.)*

Two different questions, two laws:

- **Legality (D-04):** every addressable node of the tree is a legal premise — root, any folder, any file leaf, and `absent`. The engine checks every premise the caller supplies.
- **Sufficiency (the Coverage Law):** legality does not satisfy requiredness. Let `W` be the caller-authored write set (every path the caller's own edits publish) and `G` the premise list — purely the caller's premises, everywhere this law speaks; no engine premise ever enters `G`. Requiredness holds iff **for every `w` in `W` there exists at least one `g` in `G` whose scope is ancestor-or-self of `w`** (an exact-section or absence premise covering `w` also suffices). A premise need not cover every target. A premise that covers no target is legal WIDENING — checked, strictest wins, never sufficient alone. Failure refuses `scope_does_not_cover` naming the UNCOVERED caller-authored target set; the engine never silently promotes the request to one common ancestor (LCA) or to root.
- **Placement:** coverage is enforced at transaction/set ADMISSION — the door seam where the complete `W` and the complete `G` exist, before any per-member validation or byte move.
- **Door scope** *(added 2026-08-15, bounce-2 closure — ruled execution class)*: the sufficiency demand binds exactly the doors under A.1's pure-write demand — the ops that land content by DECLARING their write set in the request (`splice` in every form, `create`, `remove`). The script door (§ A.7) is admitted by its own law, not this one: its commit premise is the engine-computed touch set — which always contains the armed writes — so a guardless pure script is admitted by construction, with an empty `G` and no uncovered set to name; caller premises there stay legal as WIDENING only (R3, `decisions/2026-08-15-plan-rulings-final.md`). Effects doors hold no premise at all (`decisions/2026-08-15-no-guard-on-effects.md`). Law source: fingerprint-grain plan §4.6, cited WITH `decisions/2026-08-15-coverage-quantifier-deviation.md` (standing citation law).
- **Engine-generated writes, outside `W`** (the receipt rider, which crosses scopes inside one commit): the engine's own commit act covers them — the engine verifies them against the live tree at commit; the caller's `G` must cover the caller-authored targets.

Consequences at the doors, stated so nothing is silently reinterpreted: a single-file `splice` whose every content edit carries `if_node_rev` is covered at those edits — an exact-section premise covers the mutation it guards, so A.1's demand is unchanged in effect; the set form's natural cover is each target file's own leaf token — one copyable token per file, membership-safe within the file; a root premise covers everything (today's `if_fingerprint`, unchanged); disjoint extra premises are legal as widening only.

### §5.6 `absent` — a value, not an error

A lawful path with no node — never created, emptied, pruned — mints the reserved non-hex token value `absent` (§4.7), and the chain law holds: **absence of the whole prefix is still `absent`** — `a/b/c` with `a/` itself missing mints `absent`, one value, not an error. Creation-guard plans stand on exactly this: an absence premise at the birth path refuses when anything now exists there. `absent` carries no algorithm/domain prefix because it names no fold — its comparison is node-existence at the named scope, not hash equality. A path is unlawful — `scope_unresolved`, recovery `fix` — only where it escapes the root or conflicts in kind with an EXISTING entry along its prefix. `scope_unresolved` is never the answer for lawful absence.

### §5.7 The error split — three errors, three facts

Never one word for two facts. Three states, three recoveries:

| fact | code | recovery |
|---|---|---|
| the premise MOVED | `fingerprint_mismatch{expected,actual,scope?}` | `resync` — re-read that scope, re-plan |
| the premise cannot be evaluated at that path | `scope_unresolved` | `fix` — fix the path |
| the reference is TOO OLD (the cursor family: `fingerprint_unknown`, a dead instance, `fingerprint_version_retired`) | §8 | `resync` — re-derive and resume |

The version vocabulary inside the cursor family stays split (§12.3): a token from a KNOWN RETIRED hash-law family refuses `fingerprint_version_retired` with re-mint teaching — never `fingerprint_mismatch`, which would lie (the premise did not move; the LAW moved). A token from an UNKNOWN FUTURE family refuses `fingerprint_version_unsupported` — distinct, because "your token is past my law" and "the law moved past your token" demand different acts. Register-law texts for the whole family: §8.2.

*(Amended 2026-08-16 — the malformed premise value; dogfood break #7.)* A fourth fact is the REQUEST, not the world: a supplied premise value that is neither the reserved `absent` (§5.6) nor a grammatical `Root`-family token (the merkle-spec §4.2 spelling — `b3` + bijective-base-26 suffix + `:` + 64 lowercase hex) refuses `bad_request` (recovery `fix`) at the premise rung, before any fold is compared. The teaching quotes the raw bytes debug-quoted, so damage the prose renders invisible — one leading space on an otherwise valid token, the measured case — shows as a byte. Comparing a damaged spelling instead would answer `fingerprint_mismatch` with an expected/live pair that can render character-identical and a re-read remedy that loops: the re-read returns the same value the caller already holds. `fingerprint_mismatch` therefore claims exactly one thing: a WELL-FORMED token was compared against the live fold and differed. This wall touches no version family (§12.3) — a grammatical token from a retired or future family is never malformed — and it is value grammar at the existing rung, never a permission plane.

*(Amended 2026-08-16 — the token premise at a node-less scope; dogfood break #6.)* A WELL-FORMED token premise whose scope holds no live node refuses `scope_does_not_cover` (recovery `fix`; the refusal carries `scope`, and `uncovered` stays §5.5's target-set extra — this mint home names no target set) — never `fingerprint_mismatch`, and never `scope_unresolved` (§5.6 bars it: lawful absence is not an unresolvable path). From `(token, absent)` the engine cannot tell "the node was removed since the mint" from "the caller paired a real token with a scope that never held one" — the measured break was the second — and the retired absent-actual teaching narrated the first as fact ("it was emptied or removed") with a `resync` remedy that re-reads a path that serves nothing, so the recovery could not terminate. What the engine KNOWS is coverage vocabulary: no node lives at the scope, so no token premise can hold there — a node-less scope's one lawful premise is `absent` (§5.6). The remedy is the mint, and it serves both worlds: `fingerprint{scope}` answers what the scope holds NOW (`absent` for lawful emptiness), and the caller re-pairs the premise or fixes the scope — one act, terminating whether the node was removed or never existed. A door that would narrate removal must actually know the history; no door today does, so a genuine post-mint removal draws this same refusal (register text: §8.2). The reverse seam is untouched: an `absent` premise against a live node stays `fingerprint_mismatch` (§5.6's creation-guard collision) — there both compared facts are live, and the re-read the teaching orders serves a node that exists.

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

The batch writes two files via tmp+fsync+rename each; a crash between renames can land content without receipt. Recovery is re-derive (cold rebuild → correct root, never wrong data) and the missing receipt is exactly what the lint finds — the failure is loud in the world model, not hidden in engine state. Stated as a limit (§13.6).

**A set commit (§4.4 set form) widens the sequence and keeps the posture — in-memory rollback, no journal (ruled 2026-08-14: "effect-less script should be only in memory state, simple is better").** The commit stages every file, verifies every pre-image, then renames member order with the receipt LAST. A rename FAILURE mid-sequence (process alive) restores every member already renamed from its held pre-image bytes — the same tmp+fsync+rename discipline run backwards — and the error names what failed and what restored. Two stated limits, both named rather than silent: a CRASH mid-rename-sequence can land a prefix of the set (each file still fully-old-or-fully-new — atomic renames never tear; cold rebuild yields the correct root of whatever landed); and the restore can ITSELF fail, in which case the error lists exactly which files hold the new bytes, so recovery is a statement, never a guess. Receipt-rename-LAST is load-bearing: in every reachable state a resolvable receipt anchor implies the whole set landed, so the §6.6 collision door remains the lost-answer probe for the entire set. The multi-file atomic commit this section previously deferred as a "rung-3 amendment candidate" is THIS mechanism — delivered by the set form, with the crash window stated instead of journaled away.

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

One Delta = one batch = one fingerprint advance. A §4.4 SET commit mints ONE
Delta whose `files[]` carries every content file plus the receipt —
cardinality is data, and a consumer that assumed ≤2 files was reading the
old single-content world. E3's delta, every value computed:

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

Laws: `seq` is a monotone per-workspace batch counter (the `changes_seq` of §10), **per-daemon-epoch** — a daemon restart resets it (memory is disposable and disk is markdown-only, §14, so no counter survives on disk to reload), which means `from_seq`/`changes_seq` catchup is valid only within one epoch and cross-epoch catchup is diff-by-root (§4.7), the root being the only restart-durable handle — so the `sub` resumption anchor carries `{tree_instance, from_seq}`, instance evaluated before sequence (B-01, §4.7); file `change ∈ {created, modified, deleted, renamed, unattested}` (renamed carries `from_path`; `unattested` is § A.9's re-scope honesty word — the file LEFT THE ATTESTED SET while its bytes remain on disk, v3-only, demoted to `deleted` for a frozen v2 session); node `change ∈ {added, edited, removed, anchored}` (`anchored` is the attestation-honesty word — the node moved SOLELY by gaining an anchor id, so it was attested to and not rewritten; it is a byte verdict, never an intent, and a write that changes content and mints an anchor in the same node stays `edited`; v3-only, demoted to `edited` for a frozen v2 session, the node's rev having genuinely moved); node entries name the **deepest section containing each changed byte range** — ancestor section revs change implicitly (rev = span hash) and are re-readable via `toc`, never duplicated into the delta. External changes (a human editing in Obsidian) produce deltas with `actor`/`now` **absent** — the engine never invents identity or time it wasn't given; `seq` is assigned at detection.

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
| `fix` | your request is wrong; change it | `bad_request`, `unknown_op`, `bad_path`, `no_match`, `not_unique`, `would_corrupt{family,lost?,cause?,target?}`, `ambiguous_ref{candidates}`, `remove_refused{referrers}` (§ A.3 remove door — inbound references exist; unlink the named referrers, then resend), `scope_does_not_cover{uncovered}` (§5.5 — coverage failed; the extra names the uncovered target set. §5.7's amended arm mints it too — a token premise at a node-less scope — carrying `scope` alone, no target set), `scope_unresolved` (§5.6 — the path cannot hold a token) |
| `env` | the world outside the workspace is wrong | `file_not_found`, `io_error{cause}`, `invalid_utf8{path,message}`, `daemon_only`, `mount_table_invalid{path,message}` |
| `refresh` | your picture of a node is stale; re-read one thing | `cas_mismatch{expected,actual}`, `ref_not_found{stage,dest?}` |
| `retry` | transient; same request may succeed | `lock_timeout`, `stale_view{required,as_of_fingerprint,live_fingerprint}`, `corpus_warming` (§3.2 — the drawer is rebuilding after a cold start; reads serve once it lands) |
| `resync` | your picture of the world is stale; re-plan | `fingerprint_mismatch{expected,actual,scope?}`, `fingerprint_unknown`, `fingerprint_version_retired` (§5.7, §12.3 — the token's hash-law family is retired; re-mint at the same scope), `fingerprint_version_unsupported` (§5.7 — the token's family is unknown and newer than the serving law) |
| `respawn` | the channel itself is broken | `bad_frame`, `unsupported_proto`, `internal` |

W4 dispositions: v1's `not_found` is **retired** — `file_not_found` (env: the file is gone) is distinct from `ref_not_found` (refresh: the name dangles), and `io_error` carries its cause. `ref_not_found.stage` makes the two-stage decomposition observable in every failure (1 = vault-namespace miss, no `dest`; 2 = subpath miss, `dest` present — §4.5). `budget_exceeded` is deliberately NOT here: it is a typed *finding* inside `verdicts` (§11), never a wire error. **`daemon_only`** (env class) is **RETIRED** (hosts ruling, §3.3, 2026-08-06): it named the one deployment gap — a corpus-class rules pack, one whose WHEN needs the resident corpus name index (e.g. `link_resolves`, §11.2), loaded against a sidecar-mode engine with no resident index (the `BudgetClass::Corpus` law, §11.3). With the sidecar host deleted, every wire door is daemon-backed and the resident index is always reachable, so the code is unmintable; the `BudgetClass::Corpus` law stands, now gating nothing at the wire. Null-id frames: §3.1. Three declared deltas from the ruled class table (previously undeclared, now fixed): `fingerprint_mismatch` rebound refresh→`resync` (a failed world guard invalidates the plan, not one node's picture — §5.1's split), `unsupported_proto` rebound fix→`respawn` (a protocol mismatch is a channel property; no request edit repairs it), and `bad_id` dropped (folded into `bad_request` + `id:null`/`id_raw`, §3.1 — one malformed-envelope code, not two). All three are behavior-preserving relabelings, now declared. Deviation-from-v1 rows: `not_found` retirement (this table), unknown-`kinds` rejection (§4.3) — each with its rationale at the cited section; the consolidated ledger is §18.

*(Amended 2026-08-15 — the scoped-guard family, §5.4–§5.7.)* Four codes join the closed enum, each statically bound to exactly one class as the table now shows: `scope_does_not_cover{uncovered}` (fix), `scope_unresolved` (fix), `fingerprint_version_retired` (resync), `fingerprint_version_unsupported` (resync). The closed-enum law is unchanged — a client that doesn't recognize a code still dispatches on `recovery` alone. `fingerprint_mismatch` regains `scope` (optional; absent = the root premise, the exact v2 shape): §18 row 2's return clause FIRED — the scoped world guard arrived by amendment, and `scope` returns with it, as that row promised. The version split is law, not labeling: a retired-family token must NEVER answer `fingerprint_mismatch` — the premise did not move, the LAW moved (§5.7, §12.3).

### §8.1 The no-answer case — transport loss is not a class (RULED 2026-08-08)

The six classes ride **error frames** — cases where the daemon answered. A request whose answer never arrives — the client's own op deadline expired, or the connection died with the op in flight — is a **transport loss**, not a wire error: no frame arrived, so no `recovery` class exists, and for a `splice` **persistence is UNKNOWN**. The batch may have committed before the loss (observed 2026-08-08: a splice committed while the client's 10 s op deadline lapsed under a parallel build hold; the daemon answered a health probe in milliseconds — a slow op is not a hung daemon).

Two consequences, both **client law** — the wire cannot rule on frames it never served, so this subsection binds clients and hosts, not the engine:

- **The op deadline is a hang detector, never a safety mechanism.** Its value is host-chosen and MAY be op-class-aware (ccc-statusd's D4 bounds: 10 s per op; 60 s for `hello`, sized when `hello` still paid a cold whole-corpus build — since §3.2's config-grade law it no longer can, and the cold build no longer rides inside ANY op's deadline: a cold workspace's first corpus read starts it in the background, absorbs at most a short bounded wait, and refuses `corpus_warming` past it (§3.2); warm-cost classes are host knowledge, the engine publishes neither). It does not scale with load — the engine publishes no load surface, and orientation is not a wire op (§10.3) — and no finite value closes the ambiguity window: any deadline can expire after the commit landed. Correctness comes from the retry discipline below, never from the number.
- **Re-read before retry.** After a lost `splice` answer the client's picture of the world is gone. Before any re-send, re-read the target and check whether the lost write **already landed** — checking content, not just tokens. The ordinary conflict path (`cas_mismatch` → refresh → re-apply with the fresh rev) is WRONG here: it re-applies a write that may already be in the file, and the ordinary teachings mislead — a post-loss `no_match` reads as "provably your typo" (§5.2) when the truth is "your first send landed and consumed the anchor".

A **blind re-send without `force` cannot double-apply** — the wire-origin guard demand (A.1, A.3) refuses every arm: a guarded edit's token re-derives against post-commit bytes (`cas_mismatch`, §5.1), a birth's subject now exists (`cas_mismatch`, absence guard), an unguarded content edit never reaches the write (`guard_required`). The refusal it draws is still ambiguous between "my lost write landed" and "a foreign write landed" — which is why the read comes first — but nothing applies twice while the client finds out. This is §5.2's adoption carrot extended: the guard demand is also what makes loss recovery safe. **`force` strips the node-grain tokens (A.1) and reopens the double-apply; a post-loss re-send MUST NOT carry `force`.**

Reads are idempotent: after a lost answer, re-send freely.

### §8.2 Register-law refusal texts — the scoped-guard family (docs-first, 2026-08-15)

Refusal teaching speaks the register law: **reason first, fitted remedy, never session rules.** The texts below are carried from the fingerprint-grain merged plan's Appendix C (k3's F-12 redrafted form) byte-for-byte; the additions are `fingerprint_version_unsupported` and the three `bad_request` guard-family texts (bounce-1 closure, 2026-08-15), each drafted HERE in the same register because Appendix C carried no text for them — recorded, not slipped in. Bounce-2 closure (2026-08-15), same drafted-here provenance: the mint-pair text joins, and the broken-premise-pair remedy is re-worded — its old teaching ("one scope spelling PLUS its token — exactly one spelling") contradicted the LEGAL bare-root premise `{"fingerprint": …}` with no scope spelling (§5.4). Break #6 closure (2026-08-16), recorded, not slipped in: the Appendix-C absent-actual `fingerprint_mismatch` entry is RETIRED and its slot re-minted under `scope_does_not_cover` — the retired text stated as fact a deletion ("it was emptied or removed") the engine cannot know, and it contradicted `scope_unresolved`'s own entry two rows below: a lawful path with no node mints "absent", a legal premise, not a deletion (§5.6, §5.7's amended arm).

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
scope_does_not_cover — token premise at a node-less scope (re-minted
2026-08-16, dogfood break #6; supersedes the RETIRED absent-actual
fingerprint_mismatch entry that stood here, whose text stated as fact a
deletion — "it was emptied or removed" — the engine cannot know, and
whose resync remedy re-read a path that serves nothing; §5.7's amended
arm):
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
fingerprint_version_unsupported (drafted here — no Appendix C source):
  "this token was minted under a hash law this engine does not know —
   the token is newer than the law being served. Re-mint at the same
   scope (fingerprint{scope: "<scope>"}) to proceed under the serving
   law; to keep the newer tokens, upgrade the engine, not the token."
bad_request — guard family, un-negotiated (drafted here — no Appendix C
source):
  "this session did not negotiate scoped-guards, so <field> cannot ride
   this request. Reconnect and negotiate the scoped-guards cap in hello,
   or drop the field — the v2 forms (root if_fingerprint, if_node_rev)
   are fully served without it."
bad_request — guard family, broken premise pair (drafted here — no
Appendix C source; remedy re-worded bounce-2 to admit the root form):
  "<detail — one of: <spelling> carries no fingerprint; both scope and
   scope_bytes in one premise; scope without if_fingerprint>. A premise
   is a token plus at most ONE scope spelling — the token is required,
   the spelling is not: a bare {fingerprint} is the legal root premise.
   To scope a premise, mint the pair together
   (fingerprint{scope: "<scope>"}) and send both; to guard the world,
   send the token alone."
bad_request — mint pair, both spellings on one fingerprint request
(drafted here — no Appendix C source; bounce-2 closure):
  "this mint names its node twice — scope and scope_bytes in one
   fingerprint request. A mint names ONE node: keep the one spelling
   that names your path (scope for UTF-8 names, scope_bytes for raw
   bytes) and re-send; both absent mints the root."
bad_request — guard family, malformed premise value (drafted here — no
Appendix C source; dogfood break #7, 2026-08-16):
  "<the premise at <scope> | the world premise> holds <raw bytes,
   debug-quoted>, which is not a premise token — a premise holds an
   engine-minted b3…:<64-hex> token or the reserved "absent", and the
   quoted spelling shows every byte, whitespace included. The world was
   NOT compared: fix the spelling, not the plan. Paste the token exactly
   as the engine served it, or re-mint it (fingerprint{scope: "<scope>"},
   §4.7) and send that."
refused trace, recovery fix — script door, malformed entry pin (drafted
here — no Appendix C source; dogfood break #7, script door, 2026-08-16;
engine-minted, so it rides the trace's fault triple with no §8 code —
§ A.7 response law):
  "the script entry pin holds <raw bytes, debug-quoted>, which is not an
   entry fingerprint — the pin holds an engine-minted b3…:<64-hex> token,
   and the quoted spelling shows every byte, whitespace included. The
   world was NOT compared: fix the spelling, not the plan. Paste the
   entry fingerprint exactly as the engine served it, or re-mint it
   (fingerprint{}, §4.7) and send that. The reserved "absent" is premise
   vocabulary (§5.6, guards[]), never an entry pin — a script evaluates
   against the world that exists."
bad_request — guard family, effects door (drafted here — no Appendix C
source):
  "<field> was supplied on an unguarded door: run and script-with-effects
   hold no premise — a guard here would promise what execution cannot
   keep (no-guard ruling). Drop the guard fields; to guard content
   writes, use splice, or script without effects (its commit premise is
   the engine-computed touch set)."
```

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
3. **Custom ignore** — optional rules on the **standing declaration page** `meridian/domain.md` (frontmatter carries `version` and `ignore` list; body may explain). Pattern semantics are gitignore-style (block list, last match wins, `!` re-includes) — and gitignore-style includes the **trailing-slash law**: a pattern ending in `/` names a DIRECTORY, so it excludes the files beneath a matching directory segment and never a bare FILE whose own basename fits the pattern body (`scratch*/` excludes `results/scratch-r4/venv.md`, not `tasks/scratch-cleanup.md`; `git check-ignore` rules the same pair). Stated because a build compiled the trailing slash into a zero-or-more suffix that also matched the file's own segment: a card file named `scratch-….md` silently left the hash domain, so the sql projection census undercounted a live board by one while every named-path door served the file (dogfood 2026-08-15, card record-reproject-on-put).

**Hash domain ⊂ addressable domain — one answer at every door.** A path outside the hash domain (an ignored `.md`, a dot-segment path) is still `toc`/`cat`/`read`/`extract`/`check_write`/`splice` by explicit path, at the READ door and the WRITE door alike: the read door serves its spans and mints its `file_rev` exactly as for a domain member, the write door commits to it, and its bytes simply do not move the fingerprint (`fingerprint_before == fingerprint_after` across such a write). The domain filter gates HASHING, not load — corpus residency is never a read admission test, so a door that refuses an out-of-domain path is a door defect, not domain law.

Two consequences, stated because a v1.0.0 build inverted exactly this (dogfood 2026-08-09, s10 — the warm read door refused what the write door committed): a guarded write's CAS token (`node_rev_before` / `file_rev`) for an out-of-domain page is mintable at the read door like any other, never only by the write door itself; and `file_not_found` means exactly one thing at every door — **no such file under the workspace root** — so its teaching must not offer domain exclusion as a second reading of the miss.

**The rule binds a DOOR FAMILY, and the family is every door the caller NAMES A PATH AT** — not the read/write pair the paragraph above happens to enumerate. `links <PATH>`, `walk <PAGE>` and `repair <PAGE>` take a path from the caller exactly as `cat` does, so each serves an out-of-domain path or is a door defect by the sentence above. Stated because a build served two of them and refused three, which reads as one law with an exception and is instead one law with three defects (dogfood 2026-08-09, f06 — the four-door reading of this paragraph was itself a subset of a nine-door family). The same predicate bounds the rooted-ref lane (2026-08-18): every door the caller names a PAGE at resolves the agent-plane `[root:]path` spelling at the door — § A.12, `address-grammar.md` § 4.6.

**Enumerators are the other half, and they are NOT bound to admit — they are bound to SAY** (ruled 2026-08-09, session decision 0017). A whole-corpus enumeration — `retire`'s sweep, the `sql` projection, `check`, bare `links` — stamps its answer `as_of` a fingerprint that an out-of-domain file's bytes cannot move, so carrying such a row under that stamp would publish a claim the stamp does not cover. An enumerator therefore MAY exclude what its attestation cannot reach, and **never silently: the exclusion is named in the output, and an enumeration that certifies ABSENCE either refuses or names what it did not see.** The engine already holds this shape for its neighbouring exclusion class — the unserved-member voice ("the file serves no spans/nodes … this scan does not see inside it") and `retire`'s refusal to certify over a partial corpus — and this rule is that reasoning carried to the domain-excluded case. The two halves compose: **a door that is asked about one path ADMITS; an enumeration that speaks for the whole corpus NAMES what it left out.**

**The VERDICT plane is the third half, and it is bound to say WHAT IT DID NOT LOOK AT** (ruled 2026-08-09, session decision 0034). The colour plane — `walk`, `check`, `status` — states a verdict about a pin's TARGET, a path the caller never named. Its corpus is the hash domain, so an out-of-domain target is absent from it for a reason that has nothing to do with the target: **the engine did not look.** A red there asserts evidence the engine does not hold — and because the taught response to a red is to fix the target or drop the pin, a false red drives a caller to destroy an attestation the engine itself minted (`pin` returns rc=0 on an out-of-domain path and writes its anchor to disk; §12.1's first paragraph is why). **So an out-of-domain target THAT EXISTS ON DISK renders `grey(outside-hash-domain)`, never red** — R-3, grey outranks red — and the reason word is what distinguishes *policy: seen but not hashed* from the greys that mean *blindness: could not look*. In-domain targets are untouched by this rule: a pin whose target the domain holds still colours green, or red on real drift or a real miss.

**The qualifier is load-bearing: "never red" binds a path that EXISTS but cannot be hashed** (ruled 2026-08-09, session decision 0049). **Absence outranks domain membership, because the order of questions is the order of facts: does the named path exist on disk, and only then, can the domain assess it.** Existence is a fact about the DISK; the domain filter is a fact about what the FINGERPRINT covers, and it is never a fact about what the disk holds. So the verdict plane answers the existence question by READING THE NAMED PATH — the same domain-independent read every named-path door owes (session decision 0045) — and **a named path that is absent from disk stays `red(file-not-found)` whether or not the domain would have excluded it.** This is the paragraph above about `file_not_found` meaning exactly one thing at every door, carried onto the verdict plane: a grey *"not in the hash domain"* over a file that is not there is a false sentence — the file is not anywhere, let alone outside the domain — and it fails in the certifying direction, because grey reads as intended exclusion and stops a reader looking. **The two states get two verdicts: out-of-domain and PRESENT is grey `outside-hash-domain`; out-of-domain and ABSENT is red `file-not-found`.**

**The ordering binds BOTH planes: an absent page is `file-not-found` WHEREVER it is absent** (ruled 2026-08-09, session decision 0054, extending 0049 at the same seam rather than widening it). The paragraph above scopes its two verdicts to a target the domain EXCLUDES, because that is the plane the finding arrived on. The existence question is not scoped that way: **it is asked of the disk, and the disk does not know the domain.** So an IN-DOMAIN target that is absent from disk is `red(file-not-found)` too, on the same two grounds the exclusion class gets it on:

1. **The verdict word's own claim is false in this case.** `selector-unresolved` is documented as *the page resolved and the selector failed*, and `dangling-anchor` as *the page's anchor vanished* — both assert a resolution. With the page gone there is no page to resolve, and the emptiness of the rendered detail is the structural proof: the candidate list a resolution red carries is drawn from the live doc, and there is no live doc to draw it from, so the caller gets the LEAST informative form of the red exactly where the cause is largest. **A verdict may not assert a resolution that did not occur.** `file-not-found`'s documented meaning — *root reached, path genuinely absent* — fits this case as written.
2. **Two worlds share one sentence, and the collapse defeats the caller.** Page-present-heading-moved (candidates listed) and page-gone (empty detail) arrive under one word, and their recoveries DIFFER — fix the selector versus restore or re-pin the file. A caller told the heading moved hunts a heading in a file that is not there.

That an in-domain target's absence from the corpus map is real evidence — the engine truly did look — is true and does not save the word: **looking and finding nothing is what `file-not-found` MEANS.** So the verdict plane asks its questions in one order on both planes: **does the named path exist on disk → absent is `file-not-found`; present → can the domain assess it → out-of-domain is grey `outside-hash-domain`; in-domain → the address and fingerprint questions as before.** Because the existence question runs ahead of every address question, it displaces the resolution reds for EVERY selector class on an absent page, the block class included.

Scope, stated because the states are told apart by different mechanisms: the existence question is answerable only for the AMBIENT root today (a mounted root's corpus is built by its own workspace's filter and no face carries those filters across, and a disk read there is not the corpus builder's to answer), so the existence read runs where the domain arm runs. **A miss inside a MOUNTED root is measured by resolution as before and is unchanged by these rules** — it already renders `file-not-found`, naming its root. A face that supplies no disk gets `cannot say` and keeps its pre-0049 verdict rather than a guess, exactly as a face that supplies no domain does.

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

**Hash-law retirement rides the same ladder (docs-first 2026-08-15; ruled: `decisions/2026-08-15-width-sharding-now.md`, GO per `decisions/2026-08-15-plan-rulings-final.md` R1).** The one-time interior-encoding cutover (fixed-256 radix child maps — `node-rev-merkle-spec.md`) changes the hash LAW, so the prefix advances exactly as this section's ladder already guarantees: old tokens never silently compare equal. What this section adds is the refusal law at the boundary, three facts never flattened (§5.7, §8.2):

- A held token from a KNOWN RETIRED family refuses `fingerprint_version_retired` with re-mint teaching — never `fingerprint_mismatch`, which would lie: the premise did not move, the LAW moved.
- A token from an UNKNOWN FUTURE family refuses `fingerprint_version_unsupported` — distinct, taught apart.
- Only a current-family unequal digest is the normal scoped mismatch.

**When retirement begins is the cutover's no-return boundary** (stated as law
in `node-rev-merkle-spec.md` §4.2.5, bounce-1 closure; amended 2026-08-16):
before the boundary the old law is still serving and nothing refuses
`fingerprint_version_retired`; the boundary is crossed by the durable B-04
cutover record alone — the downgrade-fence tombstone NEVER activates (ZT
standing law 2026-08-15: no old-binary users, the fence landed dormant;
`node-rev-merkle-spec.md` §4.2.5 amendment carries the verbatim law) — and
the non-serving shadow build is not implemented (`B_cutover` answered — pay
once).

**No dual-hash serving window exists**: the engine never serves two hash laws at once — the honest price is one typed, taught re-plan event per workspace at cutover, not permanent double maintenance. `sub` re-baselines at the cutover with a labeled epoch boundary, never a silent chain break.

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
| `crates/transport-proto` | **DELETED** (8a57a3da, 2026-08-03) — Law 1.4 keeps the seam JSON-only; the varint framing, the wire-agreement drift pin, and the `MAX_FRAME_BYTES` bound died with the crate (§18 row 14) | — |
| `crates/model` | `build`, richer NodeKind, no-serde law, **sealed `ValidatedSplice` capability discipline** (an unvalidated write cannot reach disk by construction), `merkle_root` seam | `SpliceRequest{span, if_node_rev, text}` reshaped to match-based (`target + match/put` — §4.4); `resolve` stays but serves the strict plane only; hash algorithm decided = blake3 (the "rung-2 wire amendment" the model doc reserved — §1) |
| `crates/fs` | `load`/`walk`/`apply_splice` seams, tmp+fsync+rename, no-storage law ("the moment memory can't be thrown away, the architecture has been violated") | `apply_splice` takes the batch (content + receipt append, one commit — §6.1); walk gains the §12 domain filter |
| `crates/wire-map` | `prefix_16b` (implemented, contract examples as passing tests), the one model+wire projection seam | `project` implements the superset-by-embedding predicates (§15) |
| `crates/policy` | `Violation`/`Severity`/`RulesetPin`/`CompiledRuleset`/`CompileError` — already this schema's §11 shapes; `policy::gate` stays deferred off the splice path (actor is a wire input, not an engine gate — §9) | `evaluate` output rides splice responses as `verdicts` |
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
| 2 | The repo's reserved `fingerprint_mismatch` shape (`crates/wire` §6.5 reserved-codes note) carries extra fields `expected/actual/scope/changed`; this contract ships `{expected,actual}` — the `scope` and `changed` drops are declared here | **WAIVED, declared** — the only world-grain guard is `if_fingerprint` (§5.1); no scoped-fingerprint construct exists for `scope` to describe. If a scoped world guard ever arrives by amendment, `scope` returns with it. *(Return clause FIRED 2026-08-15: the scoped world guard arrived — §5.4 — and `scope` returns with it, `fingerprint_mismatch{expected,actual,scope?}` (§5.7, §8). `changed` stays struck; its 2026-08-10 strike below is untouched.)* *(Amended 2026-08-10: `changed` STRUCK. It was promised here and MINTED BY NOTHING — the only assignment in the whole workspace was a hand-written conformance fixture, while both real producers set `expected`/`actual` only. Measured against a real daemon: the set is NOT derivable by the caller — an intruding write to a file the caller never read and never armed moves the workspace-wide root and the caller's premise cannot name it — but the doors cannot hold it either: the daemon's root-history ring is out of scope at both doors, RAM-only per epoch, and bounded at 256 with eviction, so it would be UNAVAILABLE EXACTLY WHEN THE CALLER IS MOST STALE. A field a door cannot always honestly fill is manufactured, not aligned; `resync` already instructs the full re-read that is the set's only honest recovery. Zero wire change — no producer ever emitted it.)* |
| 3 | The frontmatter node's span `[0,20]` is terminator-inclusive, against the v1 §5.2 / merkle-spec §2 leaf-block law (exclude the final terminator) — previously undeclared | **WAIVED, declared** — the frontmatter node is a fence-to-fence container, span-lawed with the section (newline-inclusive) family, not the leaf-block family; the `fm_key` leaf inside it (`[4,15]`, §4.4) excludes its terminator, consistent with the leaf law. All hashes stand |
| 4 | Two silent rebinds vs the ruled failure-class table plus one dropped code | **FIXED, declared** — §8 now declares all three deltas with rationale: `fingerprint_mismatch`→`resync`, `unsupported_proto`→`respawn`, `bad_id` folded into `bad_request` + `id:null`/`id_raw`. Behavior-preserving |
| 5 | The base packet's A6 self-claim "replay ≡ live stated and tested" — nothing executable tests it today | **FIXED, restated honestly** — executed: the fixture recomputation behind every worked value (§17 tool attest). NOT executed: any replay ≡ live test, any conformance-pack run — both are impl-rung deliverables in the impl-plan (rung-4 test; GT regeneration). The word "tested" is retracted |
| 6 | v1 `not_found` retired | Declared at §8 with rationale (split into `file_not_found` env / `ref_not_found` refresh) |
| 7 | v1's frozen "unknown `kinds` match nothing" reversed to loud `bad_request` | Declared at §4.3 with rationale (the strict-server evolution law applied to values) |
| 9 | **The v1 artifact deviates from the raw-lexeme id law (§3.1).** §3.1 fixes valid ids as JSON integer lexemes in `[0, 2^53)` and requires a non-conforming lexeme to be refused with `id:null` plus the offending lexeme verbatim in `id_raw`. Measured at the v1 cut against the built release binary over a live socket: every non-conforming lexeme — `"1"`, `-1`, `1.5`, `true`, `null` — is silently nulled and **the request is SERVED**. No refusal, and `id_raw` is never emitted. A conforming integer echoes correctly (`{"id":7}` → `id:7`). Not op-specific: on `mounts`, `{"id":5}` echoes while `{"id":"5"}` answers `id:null` — same frame, same op, only the lexeme differs | **CLOSED 2026-08-12 — this row is now the RECORD of a closed debt, not a declaration of an open one.** The conformance this row owed is SERVED: the daemon door scans the raw `id` lexeme at frame classification, BEFORE decode/dispatch (`transport::scan_id` at the `crates/registry` door — the one wire door, §3.3), and a non-conforming lexeme is refused `bad_request` with `id:null` plus the verbatim lexeme in `id_raw` — never served, never reclassified as a notification (`2^64` pinned at the door). The error's `recovery` is `fix` (§8's static binding for `bad_request`; the respawn consequence stays client-side law keyed off the `id:null` frame header, not this field). §3.1 STANDS UNAMENDED — the artifact now serves it. Gated by `crates/registry/tests/v3_key_set_pins.rs`: `contract_3_1_a_non_integer_id_is_refused_with_id_raw` (un-ignored — its R3a premise, "no host serves it", is dead), the flipped served-behavior pin `a_non_integer_id_is_refused_with_id_raw_at_the_daemon_door`, and `an_out_of_range_id_is_refused_never_reclassified_as_notification` |
| 10 | **The fixture's S1/S2 receipt bytes were printed nowhere.** §6.3's receipt lines measured 260 B / 262 B against the 222 B / 224 B the fixture's own spans and file sizes required: 38 B per line unprinted (dogfood 2026-08-09, s10). §0.3's promise that "every file this document hashes is printed in this section" did not hold for S1/S2 | **CLOSED 2026-08-09 by rebaseline — this row is now the RECORD of a closed debt, not a declaration of an open one.** The two causes were deviations from this document's own law and were fixed in the TEMPLATE, not waived in the text: `root_before=` where §6.1's standing noun is `fingerprint_before=` (**7 B**), and the `Goals>Q3` pretty join where §2.1 form is mandatory and §6.3 says in its own words not to re-teach it (**31 B**). **This was a FIXTURE act, which is why it took a rebaseline and not an edit.** Receipts are ordinary markdown inside the hash domain (§6.1), so moving receipt bytes moved the receipt node rev, which advanced the workspace fingerprint. R1 and R2 were RECOMPUTED by the engine over the new fixture bytes, never re-typed: R1 `b3:10769ae1…` → `b3:7f3b4437…`, R2 `b3:83b4ba59…` → `b3:6e866e13…`, receipts `file_rev@S1` `2731acfa…` → `51ad6428…`, `file_rev@S2` `9167b12b…` → `6cb0e939…`, the `r-000042` leaf `[26,248]`/`639a2dca…` → `[26,286]`/`60bbee70…`, the `r-000043` leaf `[249,473]`/`c912d457…` → `[287,549]`/`5c6ca7ec…`, and the receipts file 26 → 249 → 474 B → 26 → 287 → 550 B. Every one is gated by recomputation in `crates/testsuite/tests/pf_frozen_sweep.rs`, which derives them from the committed S0 bytes rather than transcribing them — so a wrong value fails a test instead of shipping. **§0.3's promise now holds without exception**: §6.3 prints the S1/S2 receipt bytes, and they ARE the fixture's. **What did NOT close, and is not owed:** the shipped CLI lane writes **254 B** node (**255 B** line) because a CLI invocation is not a wire request and mints no request id — `id=` is 6 B of §9's absent-inputs law working, ruled not a defect, and the same template writes the fixture's 260 B byte-for-byte when a request carries one (gated: `crates/receipt/tests/frozen_receipts.rs :: e3_receipt_line_byte_exact`). Live S1/S2 therefore still leave the published R1/R2 timeline on the CLI lane, by lane and not by shortfall. **The escaping this form now requires is §6.7 rule 2**, landed ahead of this rebaseline for a reason stated there: the forging hazard is created by emitting the JSON form, so the escape had to exist before the emission, never alongside it |
| 11 | **§12.3's worked table taught its arithmetic through a forbidden surface.** The published S2-anchored v0/v1 pair closes only if the domain config's own bytes stay out of the domain — true of the legacy non-md `mdfs_config.yaml`, false of the standing `meridian/domain.md`, which self-hashes by design (`crates/fs/src/domain.rs`). §12.3's values therefore contradicted §0.3's own "participates when present" note and were unreachable from the surface §12.1 mandates (dogfood 2026-08-09, s10) | **FIXED** — §12.3 recomputes over the standing `meridian/domain.md` with engine-measured values, §0.3 prints that file's v0 and v1 bytes, and the superseded pair is printed at §12.3 rather than scrubbed. §12.1 stands unamended: the legacy filename remains do-not-create, do-not-teach. **The S0 file set did not move** — `meridian/domain.md` is ABSENT at S0, R0 unchanged, and printing a file's bytes never makes it a member (proved on a fresh fixture: absent → R0, v0 present → `b3:23421037…`, removed → R0 returns). Ruled 2026-08-09, advisor scope |
| 12 | **CLI-lane commits advance the fingerprint and mint no Delta.** §7.1 laws one Delta per batch per fingerprint advance and §10.1's `changes_seq` is that counter. Measured at the v1 cut (dogfood 2026-08-09, s9): an `mrd put` commit moves the fingerprint the same daemon serves immediately, while `changes_seq` reads 0 before AND after | **DECLARED, not waived; §7.1 and §10.1 STAND UNAMENDED.** A consumer using `changes_seq` as a change monotone misses every CLI-lane write, silently — the answer is in an honest tense but the counter under it never moved. The fingerprint is the only monotone covering both lanes today, so cross-lane catchup is diff-by-root (§4.7), the same answer §7.1 already gives for cross-epoch catchup. Minting the delta on the CLI lane is owed |

| 13 | **The shipped v1.0.0 artifact cannot state the `-dirty` half of §A.3's identity token.** §A.3, amended 2026-08-09, laws `hello.identity.build` as `sha \| sha-dirty \| unknown`, where a bare sha asserts the build came from a WHOLE commit. `93184797` (= v1.0.0) bakes `git rev-parse HEAD` with no cleanliness probe, so it publishes a bare sha for a dirty-worktree build — an assertion it never measured | **DISPOSITION RESCINDED 2026-08-12 (0025); §A.3 STANDS as written.** The prior disposition declared rather than blocked on the stated premise that *"the reader-visible consequence is a missing refusal, never a wrong served result."* **That premise is measured false** (receipt `839fdb38`): a foreign resident daemon adopted a brand-new workspace and answered `read` with wording that does not exist in the caller's tree — a wrong served result, no error, following the documented contract. A declared divergence is P2 only while its stated reasoning holds; a disposition whose factual premise is refuted by measurement is void, and the item re-grades on what was measured. The re-grade is served by the socket law (§A.3): the tree's client now compares `hello.identity.build` at connect and refuses across builds — the missing refusal exists, so the wrong-served-result class is closed at the serve door. The `-dirty` half itself remains a snapshot of the pinned v1.0.0 artifact and still closes when the engine is next cut |
| 14 | **§3.1's `MAX_FRAME_BYTES = 256 MiB` "stands from `crates/transport-proto`", and §14 tabled that crate as standing — both ghosts.** The bound was a protobuf varint length-prefix bound; the crate was deleted 2026-08-03 (8a57a3da — Law 1.4 keeps the seam JSON-only), and no `MAX_FRAME_BYTES` exists anywhere in the tree: the NDJSON seam (`crates/transport` NdjsonCodec) reads newline-delimited lines with no length prefix and no size cap. A reader implementing the wire found a bound nothing serves, anchored to a crate that does not exist. The deletion commit measured these citations and deferred their rewrite as "Amendment-3: Leader's call" | **FIXED 2026-08-12 — AMENDED, nothing enforced (ZT ruling, 2026-08-12).** §3.1's "stands from" claim is struck and §14's crate row is marked DELETED (the sidecar-row pattern): a corruption bound nothing serves is not law, and this contract invents none — the ruling chose amendment over enforcement. **Open question, deliberately NOT answered here:** whether the NDJSON seam wants a NEW corruption bound (a line-length cap) is a fresh design decision, not a ratification. A cap declared by this row would be a law with no implementation behind it — the same defect this row removes. If a cap is ruled later, it amends §3.1 forward and lands in `crates/transport` with its tests |

Row 8 is **not reused**: the paragraph below refers to the dissolved row by that number, so retiring it keeps the record unambiguous.

The former row 8 (walk-plane charset/parity "collision") is **dissolved by the one-way-floor ruling:** parity is a one-way compatibility floor, not law, so a `_`-bearing anchor refusing loudly (§2.4, §4.5) is conforming behavior — there is no deviation to declare and no veto pending. Nothing else in this document knowingly deviates from ruled law; a deviation found without a row here is a contract bug (the assumption-audit law, §15).




---

## § A. Standing additions (compact)

These are **current law**, not optional history. Detail that only implements code may lag; the shapes below are what agents and hosts must learn.

### A.1 Fingerprint-or-force (every pure write door)

Content-mutating writes on the **wire door** (the daemon socket — the only door, §3.3) require fingerprint match **or** `force`. Guard fields stay **schema-optional** (a guardless frame still **decodes**). A content-mutating write with neither fingerprint nor `force` is refused **after decode** as `guard_required` (recovery: `fix`) — semantic refusal, not a frame rejection. `force` is any client's refuse→rewrite path; MCP is not a separate trust plane. In-process paths (`mrd` without the wire door) are out of this ruling's reach by **scope**, not trust.

*(Amended 2026-08-15 — coverage, §5.5.)* The demand's satisfying set is the §5.4 premise vocabulary: any legal tree token, judged by the Coverage Law at admission. `guard_required` keeps its exact meaning — a content-mutating write carrying NO premise at all and no `force`; a write carrying premises that fail coverage refuses `scope_does_not_cover{uncovered}` instead (§5.5, §8.2). Where the demand was already satisfiable it still is, unchanged in effect: per-edit `if_node_rev` covers its own edit, and `if_fingerprint` covers everything.

*(Amended 2026-08-15, bounce-1 closure — requiredness limited to the PURE write doors; ruled: `decisions/2026-08-15-no-guard-on-effects.md`.)* The demand above binds every op that lands content by DECLARING its write set in the request — `splice` in every form with its composed fields (§ A.3/§ A.5), `create`, `remove`. It does NOT bind the effects lane: `run` (§ A.8) and `script` carrying `effects` (§ A.7) are unguarded by ruling — no CAS premise, no fingerprint requiredness, no synthesized touch-set guard — because on execution whose consequences mrd cannot bound, a guard promises what it cannot keep. A guard field supplied on those doors refuses `bad_request` at the §3.2 strict wall (inapplicable to the op — § A.8; teaching: §8.2), never `guard_required`; `guard_required` is a pure-write-door refusal only. `script` WITHOUT `effects` stays inside the demand and satisfies it by construction: its commit premise is the engine-computed touch set (§ A.7), caller premises legal as widening. The section title is amended with this paragraph ("every wire door" → "every pure write door"); the one-door transport law (§3.3) is untouched — the limit is op scope, not transport scope.

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

#### A.2.1 Middleware on the write door (2026-08-17, mw-engine)

*(Amends A.2 — the armed plane's third kind. Full doctrine: `armed-plane.md`
Part A2. This section is the wire shape only.)*

- **`rules/middleware`** pages evaluate ON the write door — after CAS and
  batch validation, before bytes land — in `id`-ascending order, mode
  `off|block`. They may `refuse`, transform THIS file, transform OTHER files,
  and birth files; every disk emit joins the caller's write in **one sealed
  set** (validate-all-then-apply: everything lands or nothing does, one root
  advance, one Delta carrying every member). They may also emit `send`
  **intents**, which are never applied by the engine.
- **Request field `fields`** (splice single form + create): an optional
  `{string: string}` object, opaque to the engine — no key is interpreted, no
  key is required. It is delivered to middleware verbatim as `ctx.fields`.
  Hosts put caller context here (ccc-statusd: `created`, session, agent id);
  `actor`/`now` remain the §9 envelope inputs, not `fields` keys. Absent
  `fields` decodes as the empty map. The set form (`splice.set`) does not
  carry it (no middleware evaluates there in V1 — a `fields` key on that form
  refuses at the strict wall like any unknown field).
- **Response field `armed.intents`** (splice; `intents` top-level on the
  birth response, which has no `armed` group): on every non-dry successful
  write through a door that evaluates middleware, an ARRAY — possibly empty,
  never absent. V1 items are exactly:

  ```json
  { "kind": "send", "to": ["<seat-or-channel>", …], "body": "<text>", "rule_id": "<middleware id>" }
  ```

  `kind` is closed (V1: `send` only). The engine never marks an intent
  delivered — realization is the host's (ccc-statusd), and a host must not
  return its caller a bare success while an intent's realization result is
  missing. An intent failure after commit names itself on the host's response;
  the disk set STAYS (send is not this write).
- **Response field `armed.set`** (splice, single form; 2026-08-18, card
  p2-face-honesty): on every non-dry successful write through a door that
  evaluates middleware, an ARRAY — possibly empty, never absent — naming
  every OTHER file the sealed set committed. The caller's own path never
  appears (its facts are the response's existing `armed` group). V1 rows
  are exactly:

  ```json
  { "path": "<root-relative>", "change": "modified" | "created",
    "file_rev_after": "<16hex>", "rules": ["<middleware id>", …] }
  ```

  `path`, `change` and `file_rev_after` are lifted from the commit's own
  Delta row for that file — the response repeats a committed fact, it never
  re-derives one (`change` speaks the §7.1 vocabulary; V1 members are
  member edits (`modified`) and births (`created`)). `rules` names the
  middleware id(s) whose emits compiled the member, first-touch order.
  Receipt appends and pin promotions are commit machinery, not middleware
  members, and never appear. The field exists so a host can put the sealed
  write's real cross-file effects on its agent-facing receipt: without it a
  dependent file flipped in the same commit is visible only to a later
  read, and the write's own face under-reports what it did.
- **The put-path HOOK feed retired with this section.** Write responses carry
  no reaction envelopes (`armed.effects` serializes empty on this path);
  `rules/hook` + `proto.send` still ride the external-change detector only.
  A middleware refusal refuses `convention_fault` naming the rule id and its
  passing scenario, exactly as a check refusal does; middleware armed-law
  faults (red / unloadable / unevaluable rows) fail closed under the same
  A.2 codes.

### A.3 Composed `read`, `check_write`, `mounts`, `plan_edits`, `pin`, `create`, `hello.identity`

| Surface | Role |
|---|---|
| `read` | Addressing + content + render + frontmatter props at one snapshot; section selectors use §2.1 segments (or anchor / dewey). Not a joined string address. |
| `check_write` | Standalone write pre-flight: the splice verdict computed without writing. Read-only. |
| `mounts` | Mount-table discovery: the live root registry, machine-scoped. Read-only (§ A.5). |
| `splice.plan_edits` | Plan-level batch shapes; addresses are **segment arrays** — a heading path, or a `^id` block ref as the array's single segment (W-2, 2026-08-12: the plan lane resolves the read face's OWN anchor plane, so a toc-listed anchor is writeable by its id and a host-excluded or absent id misses on both doors alike; F-R4, 2026-08-13: that plane carries every body host Obsidian addresses — paragraph, list item, task, callout, table, fence, heading — leaving the frontmatter caret the one host-excluded miss; `match` edits inside the block-leaf bytes, `replace_section` replaces the block's content and preserves the `^id` marker by construction — the section arm's heading-preservation mirror — and `append` keeps refusing toward the containing section). |
| `splice.pin` | Pin rides the write choke-point; selector is segments/anchor. |
| `pin-cross-root` | `splice.pin.target` admits the ruled `name:rel` rooted spelling (address-grammar A-4/P5): the target is loaded, gated, promoted and blob-written in the NAMED mounted root under that root's own `LOCK_NB` write flock; the lock row's `object` carries `name:rel` minus `.md` verbatim; the proof compare (`splice.pin.proof`) runs against the TARGET root's live bytes under that same flock. A face keeps its own cross-root refusal until this cap is present, so an old engine refuses with its taught message instead of `pin_target_missing` on a spelling it cannot parse. |
| `splice.pin.proof` | Pin proof rides the request (the proof law, below): `splice.pin` carries `fingerprint` (required for a session actor) and optional `sec_rev`, both from the caller's own sections read; the composed read serves a `fingerprint` per section row; a session-actor pin without proof refuses `pin_proof_required`. No server-side read state exists behind this cap. |
| `create` | File birth through the guarded door; full body bytes. |
| `remove` | File death through the guarded door; refuses `remove_refused` while anything in the corpus still references the record (the remove-door law, below). |
| `hello.identity` | Optional `{build: sha \| sha-dirty \| unknown}` for deploy identity. The `-dirty` marker rides the sha TOKEN (git-describe convention), so this stays one field: `sha` = built from a whole commit, `sha-dirty` = built from a worktree diverging from that commit, `unknown` = no attributable identity was readable. A caller matching a declared sha matches the WHOLE token, never a substring — a decorated sha is a different build and must refuse (`docs/release.md` §5.1). *(2026-08-09: the marker is new; the field, its optionality, and its v3-only rule are unchanged.)* |

**The socket law (0025, 2026-08-12): a local client refuses a cross-build daemon.** The socket is keyed on the cache root alone — one cache root ⇒ one socket ⇒ one resident daemon, whatever binary bound it first — so an upgrade-in-place leaves a stale build serving every caller until someone restarts it. Measured (receipt `839fdb38`, session `09-11-mentor-8h-perfection-loop`): a caller following this contract got an answer computed by an engine it did not build — wording that does not exist in the caller's tree — with no error and no way to tell.

- **Scope: LOCAL clients only.** A client dialing the derived socket on its **own cache root** MUST compare `hello.identity.build` against its own baked build identity, whole token (§A.3 above), at connect. Equal → serve. A different token, or **no identity published** (a build predating the identity token) → refuse. `hello.identity` stays **optional on the wire**: a remote peer, or a caller on a foreign cache root, is not bound by this law and must not over-apply it.
- **Cost: zero additional round trips.** The comparison is an in-memory equality on the hello frame the client's single dial already receives and parses. The one-dial discipline (connect-is-the-liveness-proof, measured 40.2→20.1 ms) is unchanged.
- **The refusal is client-minted, one voice.** It is not a §8 engine frame — the engine never sees the refused call — and no wire error code exists for it. It speaks on stderr (CLI exit 2, an environment refusal minted before the operation is sent) in the fleet's skew grammar: both build identities, the `SKEW` verdict, the reason, and fitted suggestions. Skew never degrades to the in-process engine: a silent degrade would serve a correct answer while hiding the stale resident from every other caller forever.
- **The remedy speaks the teaching register (ZT ruling 2026-08-14): reason first, then suggestions by applicability — never one demanded command.** The teaching explains WHY the refusal exists (the cache-root keying above, and that a resident survives an upgrade until something restarts it), then offers each fix under the condition that makes it the right one: *when you own the resident* — restart it (kill the pid in the named pidfile; the next call auto-starts the current build); *when an install or deploy pipeline manages the daemon* — rerun its install step (that step owns the restart duty, per "Mechanism only" below); *when neither is yours* — report the skew to the daemon's operator, quoting both builds. No single imperative applies to every caller — a caller on a foreign cache root must not kill a resident it does not own — so a bare command is the wrong shape. The engine teaches users, not only the dev team: conditions are applicability ("when you own…"), never authority ("only the owner may…"), and whichever fitted suggestion applies is the caller's to run, report-after.
- **Mechanism only.** The engine carries no version ordering, no replace/supersede machinery, and no restart duty: whoever installs a binary restarts the daemon — that duty lives in the install pipeline (`justfile` `install`), never in Rust. Known limit, stated: two `unknown` tokens compare equal and assert nothing — the discipline that keeps this vacuous case rare is the sha stamp itself (`build.rs`).

**Pin proof rides the request (2026-08-16, card `pin-receipt-request-proof`; ZT's semantic verbatim: "in put, the pin supplies `node_rev` or fingerprint — the agent proves it read the content by carrying the token from its own read. In script, no verification at all").** The engine holds NO server-side record of who read what — no read-receipt ledger, no journal, nothing minted by a read, nothing to persist, nothing a restart can lose. A read is identity-free and side-effect-free as a type-level fact: the composed `read` op carries no `actor` field. What replaces the dead ledger is a proof the request itself carries:

- **The read serves the token.** A sections-mode composed read serves, on every resolved section row, the section's own `fp1.…` fingerprint (the content-identity CID-token over that section's span, anchor-lines excluded — the same token a pin of that section mints as `pin.fingerprint`) beside its `sec_rev`. The toc serves neither content nor fingerprints — a map does not prove a read.
- **The pin spends it.** `splice.pin` takes two request fields: `fingerprint` — the proof half, the `fp1.…` token from the caller's own read of the pinned section — and `sec_rev` — the write-conflict half, optional, the section CAS token the same read served. The engine recomputes the live fingerprint over the resolved target span under the write flock `splice` already holds (the TARGET root's flock for a cross-root pin) and compares. Equal → the caller demonstrably held these exact bytes, and they are current — the pin proceeds. The promotion marker is invisible to the compare (anchor removals), so a re-pin after promotion needs no refreshed token.
- **Requiredness follows the door.** A splice carrying a real session `actor` (daemon/MCP) must carry `fingerprint`; absent refuses `pin_proof_required` (fix class: read the exact selector in a sections read, carry the served `fingerprint`, pin again). The bare CLI door (`actor` absent) stays local-operator-trusted and may pin proofless — but a proof it DOES supply is still verified: trust excuses absence, never a wrong token.
- **The mismatch split speaks two causes apart.** A failed compare with a supplied `sec_rev` that differs from the live section's is `write_conflict{expected: caller's, actual: live}` (refresh class — the world moved since the read; re-read, pin again). A failed compare with `sec_rev` matching, or absent, is `pin_proof_required` naming both possibilities honestly (the content moved since the read, or the token is not from a read of this section) with the same one-round-trip remedy. Bad input is never spoken as a moved world (§ A.7 precedent).
- **Script verifies nothing.** The script lane carries no pin and no proof vocabulary — a script's commit premise is the engine-computed touch set (§ A.7), and no read-verification exists anywhere in that lane, by ruling.
- **What died with the ledger (2026-08-16):** the `read_mint_required` refusal, the per-actor per-workspace receipt store and its restart/reap evaporation class, the cross-root foreign-ledger consult, and the D16 receipt refresh after anchor promotion. Proof is content-bound, not identity-bound: identity decides only whether proof is REQUIRED, never whose read satisfies it — carrying another session's token is carrying the content, exactly as being handed the bytes is.

**Composed-`read` selector resolution (2026-08-06, dogfood F4–F6):**

- A section selector matching **more than one** node refuses `ambiguous_ref` naming each candidate's machine address (its `n`-carrying segment array) — §2.1's "the strict plane never silently picks" applies to strict reads exactly as to `cat` and `splice`. Never a silent first match, never `ref_not_found`.
- When **all** selectors fail, the refusal names **every** failed selector with its own reason (no match / ambiguous), symmetric with the partial-read `notice`, which names them the same way.
- Refusal **remedies speak the operation, not one host's tool name**: the recovery clause names the toc read in each surface's own dialect (MCP: a read with `sections[]` omitted, CLI: `--section`-less read) and never prescribes a binary the caller may not have. *(Ruled 2026-08-06, dogfood F5: dual-dialect IS this spec, not a partial fix — a remedy leads with the caller's surface (the MCP spelling first) and MAY carry a labeled CLI alternative in the same sentence.)* *(2026-08-12: the MCP spelling was `mode:"toc"` until the `mode` parameter left the MCP read face (ZT ruling, executed daemon-side at ccc-statusd 3b68e37a); the MCP toc read is now a read with `sections[]` omitted.)*

**The `toc` scope (F-R3, ZT 2026-08-13; executed 2026-08-14):**

- The composed read's whole-call subtree scope is the `toc` field: **ONE tagged
  §2.1 selector**, not a segment array. It replaces `frag` — the retired field
  name refuses at the strict decode like any unknown field, and no `#fragment`
  concept survives anywhere on the wire (every position one meaning: `path` =
  which file, `sections` = which content, `toc` = which subtree map).
- Resolution precedes serving, through the same `selector_matches` the
  sections plane uses: a heading path or a **dewey ordinal** resolves to one
  row and the scope is that row's subtree-inclusive span (rows and the
  `anchors` plane are bounded by byte containment — the segment-prefix scope
  is retired, since it silently merged same-named siblings' subtrees).
- The **anchor arm refuses** `bad_request`: a block has no subtree, so no map
  exists under it; the refusal teaches the `sections` lane that serves a
  block's content.
- A **bare duplicate refuses** `ambiguous_ref` naming each candidate's
  `n`-carrying machine address with the published AMBIGUITY remedy — §2.1's
  never-silently-picks, now holding at the scope door exactly as at the
  section and write doors. A dewey miss refuses `ref_not_found` in the dewey
  lane's own voice (ordinals are positional toc facts; the remedy is the bare
  read that lists them).
- `toc` beside `sections` refuses `bad_request` **"pass one"** — the map and
  the content are two questions, and one call answers one.

**The read plane's own budget (2026-08-15, card `read-budget-refusal-missing`; dogfood r9 § F1):**

- A `sections[]` read carries **two bounds of its own**, both enforced in `wire-serve`'s `composed_read` so the wire door and the CLI door answer identically: at most **20 000 words served per call** (`READ_MAX_WORDS`) and at most **64 distinct selectors per call** (`READ_MAX_SELECTORS`). Over either, the read refuses `bad_request` — **refused, never truncated**, nothing read and no rev minted — and the refusal names the measured number, the ceiling, and its fitted `→` recovery.
- **The unit is WORDS, not bytes, and that is the discoverability half of the face-honesty law (clause 2).** The face already publishes `words_total` and a `words` on every toc row and every served section, so a caller reads what a section costs *before* asking for it and never learns the ceiling by tripping it. A byte bound would be invisible until refused. The number is a product knob, sized to fire *before* an MCP host clips the result (hosts clip near ~25k tokens ≈ 18–19k words); it is one named constant so a re-tune is a one-line diff.
- **The section map is never word-bounded.** It is the recovery the size refusal points at, and a refusal must point at a door that answers (clause 3) — so a toc read of any document, at any size, always serves.
- **Repeated identical selectors are collapsed, and the collapse is stated** (clause 1): a repeat resolves to the same node, so its row, its bytes and its `sec_rev` are identical, and serving it N times is waste, not N answers. Identity is the selector's serialized spelling — two *different* spellings that land on one node stay two rows, because the row carries the caller's own `sel` back. The 64 ceiling is applied to the collapsed set, so a caller who repeats themselves is never refused for a fan-out they did not ask for.
- The defect this closes, measured: one section of 223 137 words was served whole and the **host** clipped it — the caller got a host truncation with no engine banner, no marker, no `→` line, and the answer lost; the same call with 65 identical selectors served 65 byte-identical copies. The run plane already refused cleanly at the same count, so the deploy-13 "budget refusals on all three planes" claim held for runs alone.

**Door symmetry over duplicate headings (2026-08-06, fix-write-dup-symmetry):**

- An `n`-less address that matches more than one node refuses `ambiguous_ref`-class at **every** door — read and write alike (`splice.plan_edits`, and any host lowering onto it). No door may pick an occurrence the caller did not name: the write-door refusal names each candidate's machine address (its `n`-carrying segment array) and teaches `n`, the same evidence the read door gives. Two doors, one answer — a selector one door refuses as ambiguous, no other door resolves.
- The published loop is untouched: addresses the read face publishes carry `n` exactly where the document is ambiguous, so read → verbatim address → write always lands.
- **The PROSE is symmetric too (2026-08-09, dogfood s4).** The machine bodies already matched while the sentences did not: the read door taught *"pin one occurrence by its machine address, or its dewey ordinal from the toc"*, and the write door taught *"address the duplicate by block id or node index"* — which never names `n`, and whose "block id" prescribes minting an id on a heading the caller may not own. The write door speaks the read door's remedy: **pin one occurrence by its `n`-carrying machine address, or by its dewey ordinal from the toc.** Renaming a duplicate heading stays a legitimate, secondary fix and is named as one — it edits the document, where the `n` address does not.
- ⛔ **Neither ambiguity refusal ends in a wikilink.** `[[selector-grammar]]` inside a machine-facing message is a vault-local address the caller cannot dereference — it names a page without saying how to reach it, and it survives into logs and agent transcripts as literal brackets. Both ambiguity remedies are self-contained (an `n` address, a dewey ordinal, a distinct block id), so the citation bought nothing it was paying for. *Scoped to the ambiguity pair: the `see [[address-grammar]]` tail on the `crates/addr` refusals is the same class and is NOT swept here — recorded as a finding rather than changed under a card that did not measure it.*

**Door symmetry over duplicate block ids (2026-08-08, dogfood-p1-read-ambiguous-ref):**

- An anchor selector (`^id`) whose id appears on more than one block refuses `ambiguous_ref` at **every** strict-plane door — the composed read's `sections[]`, `cat`, `pin`, and `splice` alike (§2.1's duplicate-anchor row). Never a silent first match: a read that picks one occurrence hands the caller a `sec_rev` the write door then refuses, so read-then-write on a duplicated anchor is unserviceable — the exact death mode §2.1 closes for writes.
- Duplicate ids share one spelling, and the anchor grammar carries no occurrence index (`n` disambiguates hpath segments; `{"anchor":id}` has no `n` slot), so **no machine address exists per candidate**: the refusal's `candidates` stays `[]` and the message names how many blocks carry the id. The map stays honest evidence: `toc`'s `anchors[]` publishes every occurrence with its span, duplicates included.
- The remedy **speaks the anchor grammar, never the heading one**: give each duplicate block a distinct id (a block id addresses exactly one block in its file), or address the enclosing section by heading path. "Rename one heading" is the heading-duplicate remedy and never appears on an anchor refusal.

**Teaching row — the anchor host-kind gate is a READ-face law, and the write door does not carry it (2026-08-09; F-R4 2026-08-13 widened the plane):** `unaddressable_host` and the set `anchors[]` publishes are both scoped to the block kinds this read face addresses — since F-R4 that is every body host Obsidian's block references cover (paragraph, list item, task, callout, table, fence, heading; the anchor's host span is the attached block per the Obsidian attachment law in `model::anchor_host_span`), leaving the frontmatter caret the one unpublished host. The native write door has no host-kind gate: even a frontmatter-hosted `^id` that the read door refuses `unaddressable_host` is still a legal `{"anchor":id}` splice target on the strict plane and arms a rev transition normally (the PLAN lane, by contrast, resolves against the face plane — door symmetry, A.3). Read this in one direction only: the map remains honest about its OWN door — every address it publishes, it serves — but it is not an index of the native write plane, and absence from `anchors[]` is not evidence that a native write will refuse.

**`check_write` — the standalone pre-flight (recorded 2026-08-07: the deployed host consumes this op on every guarded put):**

- Request: `{"op":"check_write","path":…,"target":…,"actor":…,"now":…,"edits":[…]}`, strict-decoded, v3-only at dispatch (`crates/wire-serve/src/decode.rs:226-259`); advertised in v3 `caps` (`crates/wire-serve/src/rev.rs:103`). Each edit is `{op, at, find?, body?, rev?, all?}`; `at` is the §2.1 segment array (`{h, n?}`), the same shape the committer takes — the single-segment forms carry a block `^id` or a frontmatter key (`crates/wire/src/lib.rs:842-861`). `path` addresses the file under the workspace root; `target` is the raw host path that labels refusal strings.
- Reply body: `{"refuse":…?, "repairs":[…], "forced":[…]}` (`crates/wire/src/lib.rs:990-999`). `refuse` absent = the write may proceed. `refuse` is `{class, code, message, remedy?}`; `class` picks the host's render template — `rebuild` (the candidate could not be built) vs `verdict` (the severity ladder refused). `repairs` are `{key, value}` autofill property sets the host folds into the same atomic write; `forced` echoes overridden warn rule-ids. `repairs`/`forced` always serialize, so the body is never shapeless.
- Read-only over the warm engine (`crates/registry/src/server.rs:1232-1243`); a path with no file under the workspace root is `file_not_found`. A real file outside the hash domain is served from disk on the same snapshot (§12.1 addressability) — corpus residency is not the admission test. Mandatoriness stays host policy (§5.3): the engine computes the verdict, the host decides to refuse on it. `splice` re-runs the same verdict inside its own flock (`crates/wire-serve/src/write.rs:320-330`), closing the check→apply TOCTOU gap — the standalone op is the host's pre-flight and error-rendering surface; the splice-internal run is the law.

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
- **The armed fact names the BIRTH (2026-08-12, create-armed-fact fix;
  A.6.3a′ is the precedent and the law is plan.rs's own §6.1/§7.1 citation):**
  a `create` row's armed edit carries the **born section** — `target` = the
  address the read face publishes for it (`n` exactly where the document is
  ambiguous, so read-back lands), `node_rev_before` = the empty-input hash
  (`af1349b9f5f9a1a6`, A.6.3a′'s born-from-nothing token — not a claim that
  an empty section existed; the op says birth), `node_rev_after`/`span_after`
  = the born node's own facts from the one post-batch reparse. The
  parent-append stays the lowering's mechanism: the parent's rev guards the
  CAS (`create.rev` above), and the parent's own transition stays implicit —
  re-readable via `toc` like every ancestor rev (§7.1). The receipt renders
  the same facts with op `create` (§6.4). The born node is identified by the
  POSITION the sealed batch placed it, never by counting siblings — an
  earlier same-batch edit whose text opens a same-title heading shifts
  occurrences, and a count would misattribute the birth. A composition whose
  reparse leaves no section heading at that position (destroyed or absorbed
  by a neighbouring edit's bytes) refuses `would_corrupt{target_identity}` —
  the §4.4 family for armed facts that cannot be represented. The native
  `put:end` door is untouched: a native append addressed the parent, and its
  fact keeps naming the parent.

**The remove door — guarded file death (docs-first, 2026-08-15, card
`engine-delete-door`; shape ZT-ruled, four selections recorded verbatim on the
card):**

- **Why this op exists — the write model was incomplete.** The write model
  governs two of the three mutations a corpus member can undergo: birth
  (`create`) and edit (`splice`) are locked, rev-minting, delta-transported,
  attributable and refusable. Death had no op at all: a record could leave
  the corpus only OUTSIDE the model — a raw unlink with no flock, no rev
  transition, no `actor`, and no Delta until the watch feed reconstructed it
  as an anonymous external change. That asymmetry is a defect in the write
  model itself, and death is the one mutation the corpus cannot recover from
  its own bytes. `remove` completes the model — one more op in this
  standing-addition family (the `create` precedent, not a new grammar) — so
  every mutation moves through the same flock, mints the same grain of
  facts, and can refuse. One consequence, stated once: with death inside the
  model, an unlink-and-rebirth no longer masquerades as legitimate creation
  (a birth has no pre-image, so nothing else distinguishes the two).
- **Request:** `{"id":…,"op":"remove","path":…,"if_file_rev":…,"actor":…?,
  "now":…?,"if_fingerprint":…?,"dry":…?}` — strict-decoded, v3-only at
  dispatch, advertised as cap `remove` at op grain (no dotted `remove.<field>`;
  frozen v2 caps stay byte-identical). `now` is RFC 3339, validated never
  generated (§9).
- **`if_file_rev` is the mandatory guard — remove-what-you-read.** The
  record's own whole-file rev from the caller's read. Schema-optional like
  every guard field (§ A.1: a rev-less frame still decodes); absent it refuses
  `guard_required` (fix) after decode, teaching the slot and the read that
  mints the token. Stale it refuses `cas_mismatch{expected,actual}` (refresh).
  The world-grain `if_fingerprint` is honored when present
  (`fingerprint_mismatch`, resync) and never demanded: the referential check
  below is recomputed by the engine inside the critical section, so a caller's
  stale world picture cannot sneak a referenced record past the door — the
  world guard adds convergence cost, not safety (the scope-key law: node-grain
  wherever a node token exists).
- **The referential guard — refuse while referenced.** Under the same write
  flock that performs the unlink, after the door-entry observation
  (`root_before` / the world guard), the engine enumerates inbound
  references through the existing query instruments — never a new index,
  never a merkle fold. Wikilinks and embeds go through `query::backlinks`
  (link-plane resolution, walk stage 1). Ambient `meridian-lock` pins go
  through `query::lock_pin_referrers` (the walk plane's Down predicate at
  corpus grain). Those instruments read parsed documents: the door lists
  the hash domain (`fs::hash_domain`) and reads member bytes for
  `fs::build_corpus` only. It does not call `domain_snapshot` — the
  fingerprint and the referential parse are different reads (card
  `bug-remove-corpus-snapshot`; the leftover that kept the retired ~1.5 s
  two-read mechanism on this door). Any inbound edge refuses
  `remove_refused` (fix) — the unlink never runs. Self-edges are excluded (a
  record cannot hold itself alive); a dangling inbound spelling resolves to
  nothing and does not block.
- **The refusal NAMES the referrers.** `remove_refused` carries
  `referrers:[{path, kind, count}]` — every referring file, its edge kind
  (`wikilink` / `embed` / `pin`), and how many edges it holds, path-lex
  sorted. The message speaks the teaching register: why the removal refused
  (the named records still reference this one; removing it would strand them
  dangling), then the fitted remedy — unlink or retarget each named edge,
  then resend; a bare "refused" would push callers back to `rm`, which is the
  exact hole this door closes.
- **Check and unlink are ONE critical section.** The referential check, the
  `if_file_rev` CAS, and the unlink all run under the workspace write flock —
  the same flock every meridian writer serializes on — so no cooperating
  writer can land a link between the check and the unlink (checking outside
  the lock would be a TOCTOU hole: a link written in that window would be
  destroyed by a door that just certified nothing pointed at the file).
  Editors that never take the flock (a human's editor, raw `rm`) remain
  outside every write door's serialization — the same stated residual every
  door carries, not a new one.
- **No `force`, by ruling.** The op declares no `force` field; strict decode
  refuses a frame carrying one. Everywhere else `force` escapes a guard on a
  reversible write; deletion is the one irreversible op, so it is the one
  door with no escape hatch — the forced-birth precedent ("guarded door has
  no forced-birth escape") applied to death. Operability consequence, stated:
  a caller holding a stale rev re-reads and resends; a fresh rev plus a
  referentially-empty record always lands.
- **No tombstone, by ruling.** The file ceases; the terminal fact is the
  death Delta the response and the delta plane carry — `change:"deleted"`,
  `file_rev_before` present, `file_rev_after` absent (§7.1, unchanged) — plus
  the fingerprint advance. `sub` transports it live with `actor`/`now`
  attribution; `diff` replays it within the ring; past the ring the removed
  path is simply absent from the re-derived world (`fingerprint_unknown` →
  full resync). No on-disk tombstone: disk is markdown only, and git history
  is the archaeology. Hash law is untouched — a removed leaf leaves the tree
  under the existing merkle composition (`node-rev-merkle-spec.md` records
  the ruling).
- **Response:** `{"path","file_rev_before","fingerprint_before",
  "fingerprint_after","seq","dry"?,"verdicts"}` — the armed shape of a death:
  what died (its confirmed rev), the world transition, the Delta's seq.
  `dry:true` runs everything except disk — guards, referential check,
  verdicts — and carries `fingerprint_after:null`.
- **Refusal codes, complete:** `guard_required` (fix — no `if_file_rev`) ·
  `cas_mismatch{expected,actual}` (refresh — the record drifted from the read
  rev) · `remove_refused{referrers}` (fix — inbound references exist) ·
  `fingerprint_mismatch{expected,actual}` (resync — a supplied world guard is
  stale) · `file_not_found` (env — nothing to remove) · `bad_path` (fix —
  escapes the workspace). The armed-plane gate runs over the death's
  before-state (`ChangeOp::Remove`) exactly as at every door — the
  index-integrity floor (the armed INDEX and once-armed marker refuse
  removal) stands unchanged and predates this door.
- **Stated limits.** (1) Cross-root inbound pins are invisible: the guard
  enumerates the workspace's own corpus, and a pin in a DIFFERENT root
  pointing into this one lives in a corpus this workspace does not serve —
  a remove here can strand that pin red (the walk plane colors it broken on
  the pinning side). §13-register honesty, not silent. (2) The guard reads
  the attested corpus (§12 hash domain): references written in files outside
  the domain are not links in the corpus sense and do not block. (3) A reader
  enumerating the corpus concurrently with a remove can still observe a
  path vanish mid-read and refuses whole (no-partial-load) — reader behavior
  on a vanished path is its own engine question, priced on the run-plane
  lane, not changed here.

**The `replace_section` containment law (docs-first, 2026-08-12; ZT-ratified
spec `replace-section-containment`, session 12-04-f2-mrd-integration):**

- **The invariant:** after `replace_section(target)`, every byte outside the
  target's subtree is identical, and the target's subtree is exactly the
  payload. A payload that would restructure the document refuses whole —
  never demoted, never clamped: a level change changes meaning, so silent
  rewriting costs trust where refusal costs one teaching round-trip.
- **The gate:** a payload heading at or above the target's own level refuses
  `bad_request` (fix class) with the `payload_escapes_section` grammar — the
  refusal names the offending body line, the payload heading's level, the
  target's level, and the honest alternative: restructuring is a write to the
  PARENT, so target the parent section or use `create_section`. No
  `allow_escape` flag; the capability lives where the ownership is.
- **Judged on the PARSED payload, never line-regex:** the dialect parse's own
  heading law applies (ATX only, ≤3 indent). `#`-lines inside a fenced code
  block are code, never headings. A setext underline is not a dialect
  heading: a setext-shaped payload splices contained as body text — the
  engine/CommonMark divergence this leaves (Obsidian renders an h2) is
  recorded in the ratified spec (case 9); the engine-side define is pending
  and is NOT this law.
- **The one normalization:** a payload whose FIRST line echoes the target's
  own heading — same level, same title — is the caller repeating the address
  ("replace the whole section including its heading" is the dominant mental
  model). That line is stripped silently and the remainder splices under the
  rules above. An echo-only payload normalizes to an empty section. The echo
  law is first-line-only: the same heading at any later position refuses as
  a duplicate sibling. A same-titled DEEPER heading is ordinary content and
  is never normalized.
- **One refusal carries both facts:** the gate runs at plan lowering, before
  the §5.1 CAS comparison — structure is judged against the same flocked
  pre-image CAS reads. When the payload escapes AND the passed `rev` is
  stale, the one refusal states the containment teaching and the stale-rev
  fact (the current rev inline, resend token included) — the caller fixes
  the payload and resends with the current rev in one round trip, and a
  stale CAS can no longer mask the structural refusal into a two-error
  teaching loop.
- **Receipt honesty by construction:** with containment enforced, a
  `replace_section` armed fact / receipt line `wrote §target rev:a→b` can
  never describe bytes that landed outside the target's subtree.
- **Scope:** this law binds the plan door's `replace_section`. The native
  `edits` face stays byte-exact Edit-model (§4.4 unchanged, including the
  truthful-transition law for sibling-opening appends). The ratified spec
  expects the same containment for the plan door's `append` and
  `create_section` bodies (untested there today); until that lands, those
  ops rely on the §4.4 post-reparse families alone.

**Splice hygiene at the plan doors (docs-first, 2026-08-12, N-1 — the
ZT-ratified companion of the replace_section containment spec):**

- The plan-level body verbs — `append`, `replace_section`, `create` — compose
  their lowered bytes so every boundary the splice touches is canonical:
  **exactly one blank line at block and section boundaries.** One blank line
  between a section's heading line and its content, between adjacent blocks,
  and before a following heading; a file ends on a single terminator. One
  exception is itself a boundary rule: a payload whose first content line is
  a list item, appended to a section whose last block is a list, joins that
  list flush — a blank line there is a paragraph break splitting one list
  into two (CommonMark loose-list), which is the measured N-1 defect, not a
  boundary. The payload's interior bytes stay the caller's, verbatim;
  hygiene governs boundaries only, so the payload's own leading and trailing
  blank lines collapse into the canonical separators.
- Mechanism selection is derived, never declared: the lowering composes the
  canonical result and compares it against the section's current content
  bytes. A pure extension (the canonical result starts with the existing
  content) lowers to `put{at:"end"}` exactly as before; a boundary needing
  surgery (a separator to remove or collapse — e.g. the trailing blank line
  that must not sit inside a joined list) lowers to a content-span rewrite,
  `put{at:"content"}`, that preserves every non-boundary byte. The armed
  fact and the receipt keep naming the lowered shape — the mechanism is the
  fact; target and rev transition are identical either way.
- The native §4.4 ops are untouched: `at:"end"` stays raw byte concatenation
  and a native caller owns its separators. Hygiene is plan-door composition
  law only. Byte-faithfulness to the deleted Go host arms is superseded for
  these three doors by this law; everywhere else the lowering stays
  byte-faithful.
- History (measured 2026-08-12, mrd-mcp probe N-1, fixture preserved at the
  probe scratch page): an `append` after a trailing list minted a paragraph
  break because the insert point sat past the section's trailing separator;
  an `append` at a section boundary landed flush against the next heading;
  `replace_section` consumed both the blank line under its own heading and
  the separator before the next heading. Spec of record: session
  `12-04-f2-mrd-integration` `results/replace-section-containment-spec.md`
  § Splice hygiene companion.

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
- Document-grain, both modes, never `toc`-scoped: frontmatter belongs to
  the document, not to any subtree — a `toc` scope cannot contain it and
  does not filter it.
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
    `unaddressable_host` (the id exists on the page, but its host is outside
    the face's anchor plane — since F-R4 the frontmatter caret alone, every
    body host being addressable; the P2-c truth-telling row, distinct from
    `no_match` because the honest remedy differs).
  - `candidates` — `ambiguous` only: each candidate's machine address as the
    §2.1 `n`-carrying segment array (actual arrays, never encoded strings),
    in the order the refusal names them. Always serialized; `[]` on every
    other reason — including `duplicate_anchor`, where no per-candidate
    machine address exists (the 2026-08-08 door-symmetry law above).
  - `count` — `duplicate_anchor` only: how many blocks carry the id.
  - `host` — `unaddressable_host` only: the true host kind (`frontmatter`
    in practice since F-R4; the field stays the same open string the toc
    anchor row echoes, never a fallback).
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

**The counting law — one `words` number per rev, every face (2026-08-13,
session `12-04-f2-mrd-integration` card `two-faces-word-count`; dogfood
F-S4 and D-USER r2 F3):**

- A `words` value is always `strings.Fields` over the RAW bytes of the range
  it names, and is NEVER assembled by summing other rows.
  - `words_total` (both modes, and the script toc face's `words`) names the
    FILE: fields over the whole document, frontmatter included — the number
    `wc -w` prints. It was formerly the sum of the toc rows, which counted
    every descendant once per ancestor level and published ~2x on any nested
    document; a reader budgeting a read from the banner was sent the wrong
    way (MISSION.md banner 10,504 on a ~5,240-word file).
  - `toc[].words` names a SECTION SUBTREE, unchanged: fields over the
    heading-excluded, subtree-inclusive content span. Rows therefore do not
    sum to the banner, and that is the law, not a defect — each number
    answers its own question.
  - `sections[].words` is that same section-grain count, off the same raw
    content bytes `sections[].content` carries, on the structured plane AND
    in the rendered head. The projection may show less than the section holds
    (engine-block elision) or more (claim-link decoration); what is SHOWN
    never changes what is COUNTED, so the two faces cannot answer one
    question with two numbers. `bytes` alone declares the served length.
- One derivation each, in code: `wire_map::facts::words_total` and
  `wire_map::facts::section_words`. A face that computes its own count is the
  defect this law names.

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
  {"name":"field-notes","state":"bound",
   "workspace":"/Users/Shared/repos/field-notes"},
  {"name":"sessions","state":"bound",
   "workspace":"/Users/Shared/projects/field-notes-sessions","primary":true},
  {"name":"assets","state":"grey(path-unseeable)"}]}}
```

Row shape `{name, state, workspace?, primary?}` — the `kind` field is RETIRED
(kind-sweep, ZT 2026-08-13): the taxonomy left the config schema
(`meridian-md-schema.md` §5.1), nothing on the serve path ever branched on it,
and a row field nobody can act on is a field the wire does not carry. A client
that still decodes `kind` sees an absent key — inert by the tolerant-client
law.

| Field | Law |
|---|---|
| `name` | the canonical `MountName` — the bindable layer's spelling, lowercase `[a-z0-9-]` (`address-grammar.md` § 4.3) |
| `state` | the `MountState` reason word verbatim, ONE spelling across the human line, `--json`, and this wire: `bound` · `grey(path-unseeable)` · `grey(undeclared)` · `grey(declaration-unreadable)` · `grey(claim-unverifiable)` · `red(content-drifted)`. Every word but `bound` refuses: a client gates on `state == "bound"` and treats an unrecognized word as not-bound — the tolerant-client law applied to an open-for-amendment word set |
| `workspace` | the canonical bound path, post-canonicalization — the same handle `hello` returns as `workspace`. Present exactly when the binding canonicalized; absent at least on `grey(path-unseeable)` |
| `primary` | the declared-primary designation, verbatim from the binding file (`meridian-md-schema.md` §5.1a): literal `true` exactly on the designated row, ABSENT everywhere else — absence is the only "not primary" spelling, mirroring the config grammar. At most one row carries it (two designations refuse the whole table, `duplicate-primary-designation` inside `mount_table_invalid`). A binding ROLE for fleet hosts (the primary-root rule set: ccc-statusd `docs/mcp-face.md` §8.1); the engine reports it and never acts on it. Field-only amendment, cap `mounts.primary`; a client that has not read the cap sees an unread key — inert by the tolerant-client law |

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
| the view projection's `frontmatter.value` column — and the `record` pivot and B2 tag parse riding it | published value |

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
for this column: the view's `frontmatter.value` consumers — the `record` board
pivot over `type`/`status`/`owner`/`session` (named `card` until the s4
rename), `mrd sql` operator and agent
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

**A.6.3′ The KEY half of the composed line (2026-08-14, dogfood r3 f6).** The
emitted line is `{key}: {encoded}`, so an unvalidated KEY forges frontmatter
exactly as an unvalidated value does. **A property key is dotted segments of
`[A-Za-z0-9_-]+`** — the flat dotted spelling run-plane.md mandates for the
task grammar (`task.index.caps`) and the birth door already writes. A dot
SEPARATES and is never a segment byte, so `.`, a leading or trailing dot, and
`..` are refused; so is any byte outside the segment charset. The owner is
`policy::defs::yaml_safe_key`, whose `SafeKey` has no other constructor — a
call site that composes a key without discharging the `Result` does not
compile.

Both write doors (the rebuild committer and the wire splice face) speak ONE
refusal, minted at `policy::defs::invalid_property_key_refusal` — it was two
literals until 2026-08-14, which made a caller's recovery quality a function of
which door they entered. **Why this widened:** the patch face refused
`task.index` while the SAME page's birth landed `task.index.caps` and this
contract's run plane mandated the dotted spelling — three surfaces, two laws,
and every task-contract iteration paid for it with an out-of-band disk edit.
The forgery surface is unchanged: a dot carries no `: `, no newline, no `---`.

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

### A.7 `script` — in-process script submission (docs-first, 2026-08-12, phase-2 script-plane ruling)

*The run plane's script entry (`run-plane.md` § The script entry) becomes
submittable over the wire: one request carries the whole program, the daemon
evaluates it in-process, and the response body is the trace. Ruled 2026-08-12
(phase-2 script plane): in-process Starlark evaluation in the engine daemon
plus a wire script-submission verb, superseding subprocess-per-call as
architecture. This section is the op's contract. The entry's own semantics —
budgets, the trace shape, echo/quiet, the armed law, the entry-world read
law — stay normative in `run-plane.md` and are not restated here. Like § A.5,
this section lands before its code: strict decode, dispatch, caps push, and
tests are the implementation card's.*

**Request** — the entry's own inputs, as one frame on the bound workspace
(the §3.2 binding guard applies exactly as at every other op):

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

- `source` is required; every other field is optional, exactly as at the CLI
  entry. Strict decode at every grain (§3.2's wall).
- `args` is the inert dict (string keys, string values); `files[]` is paths
  only — the wire still serves no corpus-enumeration op, so enumeration stays
  the host's. `files[]` binds in CALL ORDER (order-bind ruling): `files[i]`
  is the i-th path the caller named, because the program indexes the list
  and a host-substituted order would land edits on the wrong document
  silently. The trace opens with one `{kind:"bound", index, path}` row per
  member, so the binding is visible, never inferred.
- **Patterns in `files[]` (ruled 2026-08-14, OQ3).** A member containing `*`
  is a pattern in the one scope glob grammar (`**` spans whole segments, `*`
  a non-`/` run within one, everything else literal); other members stay
  literal paths. The daemon expands patterns at ENTRY against the entry
  world's hash-domain membership — the same walk that pins the entry
  fingerprint, so expansion is deterministic within the attempt. Members
  keep their typed position: a pattern expands IN PLACE to its sorted
  matches (the host enumerates a set — that order is the host's), and a
  path already bound earlier is dropped, first occurrence wins — never a
  global sort. The trace opens with a
  `{kind:"expanded", pattern, matched:[…]}` row per pattern and replay
  replays the recording. Zero matches contributes zero paths — data, not a
  refusal (an idempotent sweep succeeds on an already-clean corpus; the
  typo'd pattern is visible in its row). Patterns never name out-of-domain
  paths; a literal still may (§12.1 unchanged). The wire still serves no
  corpus-enumeration op: expansion is the entry's own walk, not a new read
  surface. The CLI lane (`mrd script --files`) forwards a pattern-carrying
  attempt through THIS op — the engine expands, never a CLI-private glob —
  so one expansion semantics exists in the system.
- **Literals first (ruled 2026-08-15, card `script-files-glob-index-stability`).**
  A list where a PATTERN member stands before a LITERAL member is refused at
  ENTRY — dry and armed alike — with zero evaluation, nothing armed and the
  workspace unchanged (`files_member_order`, recovery `fix`). Expansion in
  place means every member after a pattern binds at an index that moves with
  the day's match count: measured, a zero-match pattern rebound the literal
  the caller addressed as `files[1]` to `files[0]` and armed mode applied the
  retargeted write (dogfood r8 § B3). Under the order law a literal's index is
  its own member ordinal, computable from the call alone. Order inside the
  pattern region stays the host's by declaration, so no call-order index
  exists there to protect; an over-long index on a zero-match day is an
  out-of-range FAULT — never a write to a document the caller did not name.
  Dry alone cannot hold this door: arm and commit are two calls (§ the
  execution-model seam), and the armed call is the one that lands. All-literal
  and all-pattern lists are unchanged.
- `actor`/`now` ride per §9 and thread to the commit splice verbatim; absent
  stays absent.
- `dry`, `if_fingerprint`, `expect_armed`, and `receipt` carry the CLI
  entry's own semantics unchanged: rehearsal; pre-eval fast-fail plus §5.1
  commit authority; the pre-splice armed-set gate; the §6 receipt address.
  The host's arm→commit two-call sequencing (`run-plane.md` § the
  execution-model seam) rides this op as two ordinary calls — arm = `dry`,
  commit = `if_fingerprint` + `expect_armed` — with zero new mechanism.
- There is no budgets field. The CLI entry exposes none either; the entry's
  limits are the daemon's own (`EvalLimits` + the wall clock). If a lane ever
  needs an override, it arrives as a dotted `script.<field>` cap through the
  §3.2 evolution law, not by loosening this shape now.

*(Amended 2026-08-16 — the malformed entry pin; dogfood break #7, script
door.)* The pre-eval fast-fail inherits §5.7's malformed arm: a supplied
`if_fingerprint` that is not a grammatical `Root`-family token (the
merkle-spec §4.2 spelling) refuses as a REFUSED trace (recovery `fix`,
engine-minted — the fault triple, no §8 code) **before any compare** — never
`conflict`, which claims a WELL-FORMED pin was compared against the entry
fingerprint and differed. The teaching quotes the raw bytes debug-quoted
(§8.2), so damage the prose renders invisible — one leading space on an
otherwise valid token, the measured case — shows as a byte; `guard_expected`
stays absent because nothing was compared. The reserved `absent` (§5.6) is
premise vocabulary — legal inside `guards[]`, never as the entry pin: a
script evaluates against the world that exists, so an entry pin spells
exactly an engine-minted token. Version families are untouched (§12.3): a
grammatical retired/future token is never malformed. Both lanes refuse
identically — the CLI entry's pre-eval check and this op's are one trace
contract — and the wall is value grammar at the existing rung, never a
permission plane.

**Response** — `ok:true` with the run-plane `ScriptTrace` as the body,
verbatim, whenever the entry RAN. A fault, a refusal, and a conflict are
TRACES — the trace's own closed vocabulary
(`committed | no_effect | conflict | fault | refused`) — never §8 error
frames. §8 frames answer only what never reached the entry:

| Frame | When |
|---|---|
| `bad_request` | strict-decode failure, no workspace bound |
| `unknown_op` | a v2 session — the op is v3-only at dispatch |
| `io_error` / env class | the entry pass itself failed before an entry fingerprint existed |
| `corpus_warming` / retry class | the workspace is cold: the entry pass would be the whole-corpus build — it rebuilds in the background and the entry never began (§3.2) |

That last row is the wire tense of the run plane's absence contract: an
`ok:false` frame from this op means the entry never began — nothing was
armed, no splice was issued, the workspace is unchanged. Once a trace
answers, every claim in it is the engine's own. §8.1 (transport loss) binds
this op's clients unchanged.

**Dispatch:** v3-only; op-grain cap `script` (the `create`/`mounts`
precedent — no dotted fields at birth). A v2 session answers `unknown_op`;
the frozen v2 caps stay byte-identical. §3.2's v3 push was ten caps at this
section's landing, eleven with § A.8 — twenty-seven in all.

**The entry world (this op's read law, normative detail in
`run-plane.md`).** The currency pass runs ONCE, at entry: the daemon proves
the corpus current — the same corpus-grain proof every read op runs, Law
A-3c's scope unchanged, moved in time — and pins the entry fingerprint. The
program's reads serve from that pinned entry state at memory speed: zero
wire trips, zero re-walks, no doc-grain narrowing. Read-your-own-writes: a
read of a target the program itself armed serves the ARMED content (the
entry bytes with the program's own armed edits applied, in arm order) and
that content's own rev — what you read is exactly what is hashed (§4.2), on
the overlay too. Foreign mid-program changes are INVISIBLE to reads **within
the hash domain — the surface the entry pin covers**: the program reads the
pinned entry generation for the whole attempt (that is what frozen view IS —
visibility from the pin, rewritten in place 2026-08-15, bounce-1 closure),
disk moves only at commit, and commit authority is the TOUCH SET — the
engine verifies entry-vs-live at exactly the nodes the program read and
armed (the commit-premise amendment below), plus any caller premises as
widening (§5.4). Foreign churn outside the touch set neither becomes
visible mid-run nor refuses the commit. Every read of a domain
member in one attempt is therefore consistent with exactly one fingerprint
BY CONSTRUCTION, which is what the wire-client lane's composed-read bracket
exists to approximate across trips. **Out-of-domain paths stay addressable
and stay LIVE (§12.1: hash domain ⊂ addressable domain, one answer at every
door):** a real file under the root that the domain does not hold serves
from a single-file disk load on this lane exactly as on the wire-client
lane, so a script moving lanes does not regress — and the stand-still
guarantee does not extend to it, exactly as the entry fingerprint never
covered its bytes.

**Not the banned snapshot.** The entry world is attempt-scoped: born at
entry, dropped when the attempt answers, never retained across attempts,
never shared across connections, no version history, no as-of parameter.
The daemon still holds no MVCC. What `run-plane.md` bans is daemon-held
state ACROSS attempts; this is one attempt reading the one picture its own
entry pass took, with the commit's touch-set verify (the commit-premise
amendment below) as the only write authority.

**Containment (the eval boundary).** The kernel runs inside the daemon
under the entry's own limits — fuel, memory cap, call depth, source bytes,
the read and armed-edit ceilings — and a daemon-enforced wall clock checked
at entry, at every read builtin, and pre-commit. `catch_unwind` seals the
eval boundary: a panic inside evaluation answers a `fault` trace and the
daemon serves its next frame. The daemon holds no workspace lock during
evaluation — the entry pass and the commit each take the serve path's own
locking — so a long evaluation never parks another connection's op.

**Zero delta everywhere else.** Every §4 op, every § A.3/§ A.5 addition,
every v2 byte: unchanged. The CLI subprocess entry (`mrd script`,
wire-client mode) stays functional and byte-compatible — its removal is a
separate ruling this section does not make. The 0025 socket law (§ A.3) is
untouched: this op rides the same one door behind the same connect-time
identity comparison, and the CLI lanes' skew refusals are unaffected.
Because this lane's commit is issued daemon-side, its landed change advances
the delta ring like any wire splice — §18 row 12's CLI-lane delta gap does
not extend to this op.

**The 08-06 QUEUED question, answered.** The queued decision
`wall-clock-a7-doc-grain-serve-QUEUED.md` (session `08-06-triple-impl-wave1`)
asked whether the corpus-scoped refusal at the non-script door should become
amendable contract, unlocking doc-grain O(1) serves. Disposition:
**SUPERSEDED** by this section plus the entry-world ruling. The flat-curve
mechanism enters as corpus-grain-at-entry — the pass runs once, the
program's reads serve O(1) from the pinned root — NOT as doc-grain per-read
narrowing; the corpus-grain proof is kept and moved to entry; the non-script
doors' refusal scope is untouched. It enters via the authorized amendment
route as its own change with its own gates, inheriting nothing from the
race — exactly the port plan that file names.

**Effects mode (added 2026-08-13, script-effects ruling — supersedes the
same-day armed-run design, under which no code shipped).** The op gains two
request fields (the field wall grows 9 → 11): `effects:[…]` and
`invocation`. Absent `effects` = the pure script above, byte-identical —
provably pure by default. Present = the LIVE PROGRAM model (`run-plane.md`
§ Effects mode is normative): `read()` serves the live disk at call time;
`put()` applies immediately through the wire splice door — write flock held,
structural validation intact, the guard's own `force` bypass — no rev, no
snapshot, no CAS; `run()` (admitted by naming it in the list) executes the
§ A.8 lane at call time and returns its row as a value — run-then-decide.
The ruled principle, verbatim: *"the rev is a leash for the agent stale
context, not a property of writes. A script reads at execution time — its
own read is the freshest possible; guarding a millisecond gap means nothing.
Effects cannot be refused (out-of-world), so the transaction promise is
unkeepable there — no half-promises, no chimera."* Accepted tradeoff ON
RECORD: two effect-scripts can last-writer-wins each other on one section,
same as two shell scripts; the flock keeps files structurally intact;
exclusivity belongs to the coordination layer (a `mutex()` builtin mirroring
fleet make-mutex is recorded DO-NOT-BUILD).

Combination walls, all `bad_request` at decode: `effects` beside `dry`,
`if_fingerprint`, or `expect_armed` (a live program cannot rehearse and
holds no premise and no armed set); `effects: []` (name an effect builtin or
omit the field); an unknown effect name (the closed set today is `run`,
`token_count` — this sentence is the effects registry, one home); `effects`
without `invocation` (§9 — run identity derives host-minted: per-call ids
are `<invocation>-r<K>`, K the 0-based `run()` call ordinal).

**The `token_count` effect (added 2026-08-13, token_count ruling leg B —
the batch ruling's script seat).** The op gains one more OPTIONAL request
field (the field wall grows 11 → 12): `token_count_endpoint`, a unix-socket
path. Declaring `effects:["token_count"]` admits the builtin
`token_count(text) -> int` — the text's real token cost, measured NOW.
ONE measurement law: the string argument is measured VERBATIM — the tool
face's `{text}` arm; the builtin resolves no refs and no sections, so the
tool face's stored-vs-served split cannot enter it (a program measures
exactly what its own `read()` served, or what it built). The
engine never counts tokens (no tokenizer, no credentials — the
architecture constraint of the parent ruling): the live host is an NDJSON
socket client dialing the endpoint per call with the consumer daemon's own
`token_count` verb frame, identityless, so the daemon's optional-session
default picks the measuring instrument and the parent ruling's tokenizer
provenance mechanism answers. A frame carrying the endpoint without the
effect, or an explicit empty endpoint, refuses `bad_request` at decode; the
effect declared with NO endpoint decodes — the builtin then faults
"unbound" at call time, which is every lane that has no harness (the
pure/entry-world host, the CLI wire client). The endpoint's own refusal
faults the program with its words carried whole. The dial deadline caps at
the REMAINING wall clock: the call never outlives the entry's budget. A
measurement is not an act — no trace entry; a top-level
`n = token_count(…)` rides the bindings echo like any computed name.

Response on this model: a trace whose `outcome` is the NEW word `effects`
when eval completed; `trace[]` records the program's acts in call order —
read entries as today, `wrote` entries (a live `put()`: path, group facts,
the splice's own after-facts), `ran` entries (the § A.8 row, verbatim). A
mid-program fault answers `fault` with every prior act already landed and
still recorded — a live program has no rollback, and the trace says how far
it got. The outcome vocabulary grows exactly this one word; the pure path's
five words and their meanings are untouched; v2 unchanged. Delta honesty:
live `put()`s ride the wire choke-point and advance the ring like any
splice; `run()`s mint per committed batch through the run plane's delta
sink, exactly as at § A.8 (run-delta ruling, 2026-08-14).

**The commit premise — AMENDED to the touch set (docs-first 2026-08-15,
fingerprint-grain plan §4.6; read visibility RULED frozen view,
`decisions/2026-08-15-pre-merge-rulings.md` ruling 2).** The engine's
secret whole-corpus entry pin dies: commit authority is no longer a
re-pinned entry ROOT. In its place **the premise is the touch set — the
engine computes it; the caller declares nothing:**

- The pure lane already records everything: `toc`/`cat` point reads, armed
  write targets, `files[]` literal and pattern expansions, sql reads. Point
  reads and armed writes contribute leaf premises; pattern and selector
  expansions contribute their set-premise folds; sql contributes the
  provenance regions it actually scanned (set-premise law:
  `node-rev-merkle-spec.md`). Commit verifies entry-vs-live at exactly
  those nodes — O(touch set), never O(corpus). Foreign churn outside the
  touch set stops causing retries at all. **Zero new caller fields are
  required on this door** — ZT's "EASIER, not stricter", mechanically.
- **An explicit caller premise stays legal as WIDENING** (strictest wins)
  and can never drop write coverage: the touch-set floor always contains
  the armed writes. `if_fingerprint` (+ optional `scope`) and `guards[]`
  ride this op with §5.4's meaning under the `scoped-guards` cap; the
  field wall grows 12 → 14 (`guards`, `scope`). `effects` excludes
  `guards`/`scope` exactly as it excludes `if_fingerprint` (the
  combination wall above) — a live program holds no premise.
- **Read visibility is UNCHANGED — FROZEN VIEW, kept** (pre-merge
  ruling 2): the entry-world read law above stands. A
  running script sees the world exactly as it was when it started; foreign
  mid-run changes stay invisible until the next run; the ratified A.7
  read-stability promise is KEPT and existing tests keep their meaning.
  (Bounce-1 closure, 2026-08-15: the entry-world paragraph's former
  root-CAS commit sentence — `if_fingerprint` = the entry fingerprint,
  refusing on anything foreign — was rewritten in place to this
  amendment's touch set; the READ law stands, only the commit clause
  moved, so the two passages now speak one law.)
- `expect_armed` is orthogonal and stays: it proves the host authorized
  THIS set; the touch-set verify proves the world did not move under the
  premise. One gates set identity, the other set freshness.
- `dry` is untouched byte-for-byte; the dry trace additionally prints the
  recorded premise set.
- Retry budget: unchanged host policy; it now spends only on genuine
  same-subtree contention.
- **Host requiredness (R3 — `decisions/2026-08-15-plan-rulings-final.md`):**
  the host-policy ratchet that forced callers to hand-copy a fingerprint
  token onto script doors (`require_if_fingerprint`) is RETIRED. The
  protection it bought — no silent under-guarding — is now by
  construction: the touch-set premise guards exactly what the script
  touched. A caller-passed token remains legal as a widening guard (D-04
  unchanged). Host-policy change; the engine mechanism is identical either
  way.

### A.8 `run` — page-task execution over the wire (docs-first, 2026-08-13, run-crossing ruling)

*The run plane's task entry (`mrd run`, `run-plane.md`) becomes invocable
over the wire. Ruled 2026-08-13 (ZT: KEY FEATURE — a list of targets, and
callable from inside `script()`); this op is also the transport of the ruled
production ARMING door (2026-08-12: "use mrd run to run it") — arming gets
NO surface of its own here: a rule page's activation task is a task like any
other, and the receipt is the arming record. The run plane's own semantics —
addressing, contracts, capability resolution, fence languages, the exit
triad's meaning, receipts — stay normative in `run-plane.md` and are not
restated. Like § A.7, this section lands before its code.*

**Request** — a LIST of targets on the bound workspace (the §3.2 binding
guard applies exactly as at every other op):

```json
{"id":9,"op":"run",
 "targets":[
  {"page":"rules/escalate.md","task":"arm","args":["--scope","team"],
   "env":{"HOME_WIKI":"/w"},"dry":false},
  {"page":"notes/plan.md"}],
 "actor":"agent:b0864fb2","now":"2026-08-13T16:02:11Z",
 "invocation":"run-1755100931421-4417-3"}
```

- `targets[]` is required, 1..=64 entries (the fan-out ceiling every face
  list carries). Per target: `page` required, workspace-relative;
  `task` optional (the run plane's own single-task default and
  several-tasks listing apply); `args[]` strings and `env{}` string→string
  optional, contract-validated by the plane; `dry` optional. Strict decode
  at every grain (§3.2's wall).
- `invocation` is required: the host-minted, path-safe identity base.
  Per-target invocation ids derive as `<invocation>-t<index>` (zero-based
  list position), so receipts correlate to the caller's journal. The engine
  mints no identity (§9) — a daemon cannot be the author of the id its
  receipts carry.
- `actor`/`now` ride per §9, optional, absent stays absent. The supplied
  actor threads into the run receipt's `actor` fact; the CLI entry, its own
  host, keeps minting its `run:<task>` self-label when no actor exists.
- `fields{}` string→string, optional (cap `run.fields`; birth cap,
  2026-08-18): § A.2.1 opaque passthrough delivered verbatim as middleware
  `ctx.fields` on every `md.create` birth this run commits — the stamped
  lane for born-identity `created`/`session`/`spawned-by`. The engine
  interprets NO key. A host attaches it only when hello advertises the cap;
  an older engine's strict wall refuses the unknown field.
- `ambient` string, optional (cap `run.ambient`; md-create-ambient-paths,
  shape (c), 2026-08-18): the caller's ambient directory,
  workspace-relative, path-law-validated at the strict wall (a confined DIR
  path — never a `root:` ref, never absolute). A BARE `md.create` path this
  run births resolves under it — the face path law ("a bare path stays
  ambient in your session directory") on the birth lane; a rooted
  `root:rel` birth target is explicit and unaffected, and must name the
  bound workspace (a foreign root refuses with a teaching — a run's births
  ride the bound workspace's ring, locks, and armed law). The capability
  grain and the receipt judge the RESOLVED target (`md.create:tasks` covers
  a `tasks/` partition wherever it lands). No `ambient` = the bare-door
  law, workspace-root-relative, unchanged. Hosts resolve it per call from
  the caller's own identity — never a page-hardcoded dir.
  engine-appended to the run receipt file under the per-target invocation
  anchor on BOTH doors — nothing exists for a caller to aim. No capability,
  timeout, or code field: authority resolves from the page + declaring-root
  conventions, the timeout from the declaring root's config, and only
  corpus-declared task blocks run — the wire carries names, never code.

**Execution.** Targets run SEQUENTIALLY in list order, each an independent
run-plane invocation: its own `run.lock` window, its own receipt, its own
row. **Each target answers for itself; no target's outcome halts, gates, or
colors another's.** There is no multi-run transaction and this op does not
invent one — a caller needing dependent sequencing composes runs inside
`script()`, where the transaction plane already owns the decide-then-act
shape.

**Response** — `ok:true` with per-target rows in request order, whenever the
op reached the plane. No aggregate boolean exists anywhere in the body:

```json
{"id":9,"ok":true,"body":{"targets":[
 {"page":"rules/escalate.md","invocation":"run-1755100931421-4417-3-t0",
  "receipt":"receipts/run.md §^r-run-1755100931421-4417-3-t0",
  "dry":false, "task":"arm", "task_rev":"…", "guarantee":"hermetic",
  "state":"applied", "applied":[{"kind":"md.patch","domain":"…"}],
  "unexecuted":[], "caps":{"effective":["md.patch:rules/*"],
  "source":"explicit","narrowed":[]}, "cap_reached":false,
  "out_of_band_delta":false},
 {"page":"notes/plan.md","invocation":"run-1755100931421-4417-3-t1",
  "refusal":{"class":"invocation","reason":"several tasks declared — name one",
   "declared_tasks":["check-links","fix-drift"]}}]}}
```

- A target the plane CARRIED to a report answers the run plane's own report
  object verbatim (`run-plane.md` § the report; `caps` absent on an
  unsandboxed row, `exec` facts carried as the plane states them, bash
  stdout as the report's own bounded record — this op streams nothing),
  plus addressing (`page`, `invocation`, `receipt`, `dry`). The echoed
  `page` — on rows and in the run receipt alike — is the target ref's
  resolved workspace-relative spelling (§2.1's grammar), never the
  request's bytes: a caller that spells one page two ways still reads one
  history back.
- A target the plane REFUSED pre-report answers a `refusal` row:
  `class:"invocation"` for the CLI's exit-2 family (addressing, contract
  violation, authoring faults — `declared_tasks[]` rides the several-tasks
  listing), `class:"run"` for the exit-1 family that refused before a
  report existed (workspace busy, timeout; the former foreign-edit and
  root-mismatch legs are RETIRED — the no-guard amendment below),
  `reason` verbatim from the plane's typed error.
- `dry` rows carry the plane's dry legs unchanged: a starlark dry answers
  the full effect set with `applied:false`; a bash dry answers the block
  source with `executed:false` and `effects:"undeclared"` — bash effects
  only exist by running it, and a dry that invented them would be fiction.
  A dry target rehearses EVERY pre-apply gate the live target enforces —
  addressing, contract (arity, env declarations), capability admission
  through the executor's own choke point — so a gate-refused rehearsal
  answers the refusal row the live call would answer, byte-identical
  (`runner::rehearse`, one seam both doors consume; dogfood r2 F2:
  dry-green predicts live-green, and a contract fault never reaches eval,
  so no interpreter traceback can stand in for the typed refusal).
- §8 `ok:false` frames answer only what never reached the plane:
  `bad_request` (strict-decode failure, empty or oversize `targets[]`, no
  workspace bound), `unknown_op` (v2 session). Once rows answer, every
  claim in them is the engine's own.

**Dispatch:** v3-only; op-grain cap `run` (the `create`/`mounts`/`script`
precedent — no dotted fields at birth). A v2 session answers `unknown_op`;
the frozen v2 caps stay byte-identical. §3.2's v3 push is eleven caps,
twenty-seven in all.

**Containment (what this door inherits, all of it the plane's own).** The
wire arm drives the same runner seam as the CLI: capability resolution
deny-by-default on starlark, the bash-fence convention refusals, the
declaring root's configured timeout with process-group kill past deadline,
the inherited task environment *(amended 2026-08-16, run-env ruling — ZT,
verbatim: "run must not strip the daemon's environment". The superseded law
read "the scrubbed task environment (`env_clear` + exactly the declared
contract pairs and the plane's own variables)" — that strip was the defect,
not this page: a task whose `^env` gate needs a daemon-held variable, e.g.
`CCC_LLM_WIKI_PATH`, could never pass through the run face. The step now
inherits the daemon's environment; declared contract pairs and the plane's
own variables overlay it — declared pairs shadow inherited values, and the
plane's own `MD_EFFECT_FD` / `MERIDIAN_PROJECT_ROOT` shadow everything)*.
One stated amendment to U16: the CLI's
"the step runs where `mrd` runs" was written for a local entry whose cwd is
the caller's context; a daemon has no meaningful cwd, so ON THIS OP the task
step's working directory IS the bound workspace root — deterministic, and
narrower than the CLI. A long-running target parks only its own connection
(§ A.7's containment posture); the plane's `run.lock` refusals answer as
`class:"run"` rows, never hangs.

**Delta honesty (amended 2026-08-14, run-delta ruling — the § A.8 half of
the row-12 debt is DISCHARGED).** Run applies on THIS op mint Deltas like
every other daemon-side write: the plane's executor commits through its own
seam (`fs::apply_batch`, unchanged), and at each committed batch the serve
arm's delta sink assembles one frame at the §7.3 single constructor and
advances the workspace ring **under the workspace WRITE flock, held as a
bracket around the commit and the mint** (amended 2026-08-14b: the run
plane's own `run.lock` does not exclude the detector — run applies and wire
splices do not otherwise serialize — so the bracket takes `write.lock`, the
detector's and the choke-point's serialization point). One committed batch =
one root advance = one Delta (§7.1), the content page and the receipt file
as two entries of ONE frame's `files`. Because the detector reconciles under
the same flock, a detect cycle can never observe a half-landed run commit or
an un-advanced ring — no misattribution window exists.
Identity on the frame is §9's: a supplied `actor` threads verbatim; absent,
the frame carries the plane's own `run:<task>` self-label, the same fact the
receipt's actor field attests — a governed run is never unattributable, so
`actor`-absent still means exactly "edited outside the face". `now` is the
caller's or absent, never invented. A mid-run fault mints frames for the
batches that committed and none for what refused (no rollback, ruling 2).
The CLI entry (`mrd run`) is a separate process with no ring in reach: its
commits stay under §18 row 12 exactly as CLI-lane `put` commits do — that
half of the debt stands, and this section does not silently discharge it.

**Zero delta everywhere else.** Every §4 op, § A.3/§ A.5/§ A.7, every v2
byte: unchanged. The CLI entry (`mrd run`) stays functional and
byte-compatible — same runner, same receipts, its own host-minted identity.
The 0025 socket law (§ A.3) is untouched: this op rides the same one door
behind the same connect-time identity comparison.

**No guard on this door — RULED
(`decisions/2026-08-15-no-guard-on-effects.md`).** `run` is NOT guarded: no
CAS premise, no fingerprint requiredness, no synthesized touch-set guard —
on execution whose consequences mrd cannot bound, a guard PROMISES what it
cannot keep and buys complexity and slowness for the false promise.
Consequences on this op, each stated:

- **A supplied guard field is rejected as inapplicable, never ceremonially
  checked.** `if_fingerprint`, `guards`, `scope` — and `scope_bytes`, which
  is a top-level field on NO door (§5.4's field matrix) — are not in this
  op's field set, so the §3.2 strict wall refuses them `bad_request` at
  decode (teaching: §8.2) — that refusal is this law working, not a gap to
  close.
- **`task_rev` is TARGETING, never CAS.** A task-selection pin chooses WHAT
  to execute — which task bytes the plane resolved; it is never a world
  premise, and no refusal on this door is a premise refusal.
- **The `class:"run"` family loses its premise legs** (the row list above):
  the plane's self-pinned corpus root (root mismatch) and the per-target
  pin-and-verify (foreign edit) RETIRE. A foreign advance re-derives and
  proceeds; a vanished unrelated record drops from view and never fails
  another target. What remains in `class:"run"` is execution refusals —
  workspace busy, timeout. Normative detail: `run-plane.md` § the no-guard
  amendment.
- **Guard-free never means fold-invisible.** Every landed run write rides
  the same write choke-point, advances the resident folds other writers'
  premises compare against, and mints Deltas exactly as the delta-honesty
  paragraph above states. That is tree maintenance, not a guard.

### A.9 Re-scope honesty on the delta plane (docs-first, 2026-08-14, dogfood r3 f9)

**The defect this ratifies away.** A domain re-scope (the §12 config changed — `meridian/domain.md` landed on a live root) flooded the feed as one batch of 1,010 `deleted` file rows. Every one was false at the file grain: the files remained on disk; they left the ATTESTED SET. A watcher acting on `deleted` (cleaning up references, say) would act on 1,010 falsehoods, and the enumeration itself was undeliverable — the transport cap that finally bit was a consumer-side token cap with no drain-cursor semantics. Three amendments, all additive:

**1. The `unattested` file-change word (v3-only).** A path in the previous attested set and absent from the current one, whose file still exists on disk (any filesystem object at that path — probed at classification, never inferred), mints `change:"unattested"`: `file_rev_before` present when the departed bytes still parse, `file_rev_after` absent (no attested post-state exists), no node entries (the content did not change — nothing node-grain happened). `deleted` now claims exactly what it says: the path is gone from disk. Consequences carried with it: a still-on-disk path can never be claimed by the `renamed` pairing (a rename asserts the origin left the disk), and a frozen v2 session receives such a row demoted to `deleted` — v2 keeps its birth vocabulary, the honesty split is v3's.

**2. The `rescope` batch summary.** When the effective domain configuration changed between the detector's baselines — compared as parsed scope rules (config identity + `Domain` semantics), so a prose-only edit to `meridian/domain.md` re-scopes nothing — the emitted frame carries a root-level sibling of `delta` (the `effects` precedent):

```json
{"delta":{…},"rescope":{"cause":"meridian/domain.md","unattested":1010,"attested":2}}
```

`cause` names the config file whose change re-scoped the set (on a config switch, the file now in effect; on a config removal, the departed one). Under a `rescope`, membership-only changes COLLAPSE into the counts and are not enumerated per file: `unattested` counts paths that left the set while remaining on disk, `attested` counts paths that entered it (whether an entering path is also new on disk is unknowable at the set grain during a re-scope — re-read, never guess). Rows that state disk-true or content facts still ride: the cause file's own row **first in the batch** (config-change-rides-first, now law rather than sort-order luck), genuine `deleted` rows (path gone from disk), `modified` rows in full node grain, `renamed` pairs. The consumer's disposition is `resync`'s (§8): the attested set was re-planned; re-derive what you watch, then continue from the cursor. Replay ≡ live holds — the ring stores the collapsed frame, `diff` replays it byte-identical (§7.3).

**3. The `overflow` marker — the feed bounds itself before any transport does.** A frame's file enumeration is bounded at assembly (`MAX_DELTA_FILES`, 128 — sized against the measured storm: ≈158 chars per rendered row, a ≈25k-token consumer cap ≈ 630 rows per drain, so a 128-row frame keeps several frames deliverable per drain). Rows past the bound — deterministic order: cause first, then path-sorted — are dropped and counted:

```json
{"delta":{…},"overflow":{"dropped":988}}
```

An `overflow` frame is an explicit honesty mark: the enumeration below the bound is a sample, the count is complete, and the recovery is re-read — never a transport layer silently truncating an answer the producer believed delivered. Both new fields are v3-additive (`rev::V2_RESERVED_FIELDS` rows at the notification root, stripped for v2 typed — the `effects` discipline), and tolerant consumers ignore unknown fields (§7.4's law); a consumer meeting an unknown `change` word treats it as "this file's membership or content moved — re-read".

The consumer-side half (a drain face bounding rows per answer with partial-cursor semantics) is the consumer's own contract to amend; this section governs what the engine emits.

### A.10 `walk` — pin-graph context assembly (2026-08-14, parity-map crossing orders)

The CLI walk plane crosses to the wire: up (default) = what a page draws
from, transitively; `down: true` = who pins it — the dependents listing and
dry-run blast radius. Read-only; computed per query, never stored; every
answer cites the doc revs it read (§2.4 honesty citation). Workspace-bound
(unlike § A.5 `mounts`); v3-only at dispatch, advertised at op grain as cap
`walk` (the `create` precedent — no dotted `walk.<field>` at birth). A v2
session answers `unknown_op`; the frozen v2 caps stay byte-identical.

Request `{path, down?, depth?}` — `path` is the workspace-relative page,
`down` toggles direction, `depth` bounds the hops (`1` = direct edges). The
walk COMPUTER is the one the CLI pin planes color through
(`view::walk::walk_rooted` over the shared mount-corpus assembly), so a row
here and the same row under `mrd walk --json` carry ONE spelling of
color/reason/detail.

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
| `page` | the walked page, echoed at page grain. Named `page`, not the walk plane's `root`: on this wire the body-level `root` key IS the fingerprint slot (the v3 projection renames it), and a page path is not a fingerprint |
| `depth_bound` | the bound in effect, echoed; absent = unbounded |
| `entries[]` | BFS order, ascending depth then discovery; `{depth, selector, rev?, color, reason?, detail?, teaching?}`. `selector` is the lock row's canonical address, `root:`-qualified when the claim crosses roots; a section-scoped claim spells `path §selector` — the live grammar, never the retired `path#selector` join (ruling 2026-08-14: one grammar everywhere, display values included; the `#` spelling survives only on the stored/lock plane). `color` is the tone (`green`/`red`/`grey`); `reason` the stable word, absent exactly on green; `teaching` present only for colors that teach — the field never invents advice |
| `revs_read[]` | `{path, doc_rev}` — the docs the listing rests on, path order. The listing is falsifiable against exactly these revs |
| `excluded[]` | §12.1 enumerator clause, DOWN walks only: the blast-radius census names the markdown the hash domain left out instead of publishing a partial population as whole. Omitted when empty, and always on up — up drops nothing, naming an excluded ancestor by its correct path at a red edge |

**Doors and refusals.** A NAMED page the hash domain excludes is served
(§12.1 door-family clause — membership gates enumerations, never what a
named path is entitled to); a missing root refuses `file_not_found`; an
unserved member refuses `invalid_utf8`; an in-snapshot cycle refuses
`walk_cycle` (env class — the workspace's own pin graph is broken, not one
request), naming the loop. An unreadable hash domain fails the door
(`io_error`): degrading to the default domain would claim every path is
hashed — the false-red fail-open decision 0034 ruled out.

**The staleness triple does not apply.** The walk serves from the warm
engine's projection at one borrow — current-tense by construction, and the
per-row `rev` plus `revs_read` already carry the falsifiability a caller
needs. Do not bolt the triple on.

### A.11 `sql` — corpus SQL over the resident projection cache (2026-08-14, lifecycle-B ruling)

One SQL statement over the workspace's fingerprint-pinned, append-only
`sql.duckdb` projection cache (`view::store`; session design
`results/sql-duckdb-append-cache-design.md`), served by the resident engine.
This KNOWINGLY supersedes §10.4's close for sql, and only for sql: the
daemon is the cache file's single owner and its one append actor, and the
wire carries **results, never a file path** — §10.4's "never as a wire op"
close was about publishing a view file's path; no path crosses here.
Workspace-bound; v3-only at dispatch, advertised at op grain as cap `sql`
(the `create` precedent). A v2 session answers `unknown_op`; the frozen v2
caps stay byte-identical.

Request `{query}` and nothing else (strict field wall): cwd and row bounds
are host concerns — result bounding belongs to faces (the MCP face's
`max_rows` + output-file law). One execution path, no profile split (the
NO-SANDBOX ruling, 2026-08-14): the query runs exactly as the CLI lane runs
it, spill-bounded and always rolled back, nothing locked or disabled.

Serve shape per call: warm engine snapshot → pre-query pin check + delta
append (O(changed files), the cache-as-manifest protocol) → always-rollback
query (`BEGIN → statement → collect → ROLLBACK`) → post-result currency
fold through the workspace leaf memo, so `state` post-dates the rows (§Q3
honest tense).

```json
{"id":13,"op":"sql","query":"SELECT path FROM doc ORDER BY path"}
{"id":13,"ok":true,"body":{
 "as_of_fingerprint":"b3b:…","live":"b3b:…","state":"FRESH_AT_SAMPLE",
 "columns":[{"name":"path","type":"VARCHAR"}],
 "rows":[["a.md"],["b.md"]],"row_count":2}}
```

| Field | Law |
|---|---|
| `as_of_fingerprint` | the projection pin the rows were computed at — the engine's warm corpus fold, verbatim |
| `live` | the post-result currency fingerprint; absent exactly on UNVERIFIED |
| `state` | `FRESH_AT_SAMPLE` \| `STALE` \| `UNVERIFIED` — UNVERIFIED iff `error` is set (a failed query certifies nothing) |
| `columns[]` | `{name, type}` as `DuckDB` reports them — the same pair the CLI `--json` frame carries |
| `rows[]` | row-major JSON cells; list cells are real arrays per row (the F1 fix), never column dumps. Booleans and numerics ride as JSON numbers/bools; every other scalar family — timestamp (tz-aware columns marked `+00`), date, time, interval, decimal, enum, struct, map — is a string speaking `DuckDB`'s own `::VARCHAR` text, never a `Debug` repr (r6 S1); a union cell is its member's cell |
| `error` | the caller's own SQL failing is a SUCCESS body with the engine's words verbatim (faces render their `SQL:` register from it) — plus the teaching arms below. Never a wire error: `ok:false` frames are the door's own faults (`io_error`, `bad_request`, `unknown_op`) |

**Teaching arms on a refusal (reason first, then a suggestion that fits).**
The engine's words lead; three arms may extend or trim them:

1. **View-DML** (ruling OQ1): the remedy names the `hist.*` lane.
2. **A retired face name** names the name that replaced it — `card` was
   renamed to `record`, and the rename shipped with NO compat alias, so this
   refusal is the whole migration path for a caller who learned the old face.
3. **A Did-you-mean fitted to a catalog internal is dropped.** `DuckDB`'s
   suggestion is pure edit distance over the whole catalog, so a face name can
   land on metadata by accident (`card` → `pg_attrdef`, `board_drift` →
   `duckdb_constraints`). A `pg_` / `duckdb_` / `sqlite_` fit is never the
   answer to a face question; near-miss face suggestions (`records` →
   `record`) are untouched.

**DML law (ruled).** The latest-layer names (`doc`, `section`, …) are VIEWS
over append-only history: DML against them refuses through `DuckDB`'s own
error plus the remedy naming the `hist.*` lane. DML against `hist.*`
executes, is visible to its own statement, and dies at ROLLBACK — the
"writes nothing durable" contract on a persistent file. Nothing else is
guarded: trust posture, no statement classifier, no auth.

**The occurrence index is SERVED as a column, never re-derived (ruled
2026-08-15 — ZT, verbatim: "Rule: add n").** The projection's `section`
relation publishes `n`: the section's own 1-based occurrence among the
same-parent, same-raw-text sibling sections — §2.1's occurrence index, the
one this document already laws — and it is **NULL exactly where the
published address omits it**, so `n IS NOT NULL` is the ambiguity predicate
and the column is the last segment of that row's `hpath`, never a second
spelling of it. It is SERVED from the same address owner the `hpath` column
and the read face's toc publish from (`model`'s occurrence law), never
recomputed in the projector: a second owner of one fact drifts silently,
because both answer a plausible small integer.

The defect it closes is a wrong-target WRITE that passes CAS. Without the
column a caller building corpus-wide section edits re-derives the occurrence
from `node_seq`, and the two count different things — `node_seq` is the
document-order ordinal over EVERY section of the file (the row's identity),
while `n` counts only siblings sharing one parent and one heading text. The
address that mistake produces resolves to a real, different section, whose
`node_rev` the caller then reads and guards against, so the write commits
against a live guard and lands on the wrong section silently. A refusal
would have been the good outcome; this class does not get one.

⚠️ **`n` addresses a SECTION inside one file. It never says which entry of a
`files[]` request a row belongs to** — that question is answered where it is
already owned, by `files[i]` under §A.7 and the §4.4 set form, and this
section neither restates nor extends those rules. The two indices meet on any
corpus-wide sweep and must not be read as one.

**The CLI ladder.** `mrd sql` asks the resident daemon FIRST (this op),
opens the drawer file directly when unheld, and answers from `:memory:`
last. ONE ladder for every caller (the NO-SANDBOX ruling, 2026-08-14,
which retired OQ5's profile distinction); only `--rebuild` goes direct,
because repair needs the file itself.

### A.12 Rooted refs resolve at every page-taking door (docs-first, 2026-08-18, rooted-refs-everywhere)

Every agent-facing door at which the caller names a page resolves the agent-plane `[root:]path`
spelling through the one rooted lane. The law, its ratification receipt, the authority ruling
(the page's tree governs — conventions, caps, and receipts follow the PAGE's workspace), the
door-family snapshot, the preset-lane exception, and the superseded D11 wording all live in
`address-grammar.md` § 4.6; the full record is llm-wiki
`decisions/2026-08-18-rooted-refs-everywhere.md`. **On THIS contract the amendment changes
nothing structural**, and that is the point worth stating here:

- **Resolution happens at the door; the wire carries the rel half only.** The §1 `Path` law and
  its head-colon confinement arm (`addr::confined`) stand unchanged — a raw `root:` head
  arriving on the wire is an address that missed its door and refuses `bad_path`.
- **The mechanism is the shipped workspace jail.** `hello` pins the declared workspace
  exact-or-refuse, with no ancestor walk (*"a declaration never widens to an enclosing
  registered workspace"*), and the connection stays attached to that workspace for reads and
  writes alike — a rooted door resolves the root first and dials the resolved workspace, so
  rooted writes need no wire change.
- **The one in-band exception predates the amendment and is unchanged:** `splice.pin.target`
  carries `name:rel` on the wire (pin-cross-root, § A.3).
- **`run` over the wire (§ A.8) follows the authority law:** a rooted invocation executes under
  the page's tree — its conventions, its caps ceiling, its workspace for receipts.
- **The `script` lane's one-declared-root rule is convergence, not invention:** the customer
  face (ccc-statusd MCP `script`) already states it verbatim — *"Every files[] entry resolves
  through one root; that root is the workspace; in-program paths are relative to it."* Face
  grammars still differ deliberately: the MCP face admits absolute and session-relative refs;
  the mrd §1 path law does not, and this amendment does not harmonize them.

---

## § B. Process

1. Edit this file (or the relevant SPEC under `docs/`) **before** code.  
2. Do not reintroduce versioned contract files or amendment piles.  
3. Optional history only: `worker-log.md` (deletable).  
4. **UNVERIFIED** when evidence is missing.
