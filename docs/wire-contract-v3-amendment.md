# Wire contract v3 amendment — `root` → `fingerprint`

Status: shipped on `cli-foundation` (worker, 2026-07-20). `docs/wire-contract-v2.md`
is FROZEN and unedited; this file is the sole normative text for the v3 rev.

## What v3 is

Contract v3 is contract v2 with ONE vocabulary change: every wire token that
spells the Merkle **content hash** — the thing v2 calls `root` — is renamed to
`fingerprint`. `root` was always the content hash, never a directory; the
workspace directory is a separate concept (`workspace`, never `root`). The
amendment removes that collision before an external community locks the name in.

v3 changes **spelling only**. Every value, every recovery class, every shape,
every law of contract v2 holds byte-for-byte under v3. A v3 frame is a v2 frame
with the fingerprint keys re-spelled.

## Why an amendment, and why now

Contract v2 §3.2 says capability discovery is complete and there is **no version
sniffing, ever**. This amendment does not break that rule. Sniffing is the
server GUESSING a client's dialect from frame shape. v3 negotiation is the
opposite: the client **DECLARES** its rev in the `hello` request. A declaration
is not a sniff — the server never guesses. The `caps` set stays the complete,
whole-set discovery surface within each rev.

The rename ships pre-community on purpose. A glossary bridge (`root` aliased to
`fingerprint`) could never be removed once external consumers depend on it, so
the clean rename must land before there are external consumers to bridge for.
(Decision 0001 round 6, user 2026-07-20; advisor-coordinated with 108da20a.)

## Negotiation mechanism (rev token = `v3`)

- The `hello` REQUEST gains an OPTIONAL field `contract`:
  - absent or `"v2"` ⇒ the session is **v2** — today's behavior, bit-for-bit.
  - `"v3"` ⇒ the session serves the **v3** vocabulary from the hello response
    onward.
- The negotiated rev is **per-process serve-session state**: one epoch, one rev.
  A daemon restart is a new epoch and re-negotiates from scratch.
- An unknown declared rev (e.g. `"v4"`) is refused **LOUD**: a `bad_request`
  (recovery `fix`) whose message names the unknown rev and the revs this sidecar
  speaks. Never a silent fallback to v2.
- The `hello` RESPONSE echoes the negotiated rev as `"contract":"v3"` **only
  under v3**. It is NOT echoed for a v2 session, because the frozen v2 golden
  pins the hello body exactly (`crates/sidecar/tests/dispatch_v2.rs`
  `gate3_hello_caps_equal_frozen_full_list`); adding a key there would break
  byte-identity. A v2 client therefore sees the unchanged v2 hello body.

## No dual-emit (hard rule)

Within one rev there is exactly one spelling. A v2 session emits `root` and
never `fingerprint`. A v3 session emits `fingerprint` and never `root`. There is
no frame that carries both, and no per-field opt-in. The rev is chosen once at
`hello` and governs every frame after it.

## v2-support-until-confirmed-cutover (hard sequencing constraint)

Live `sidecarv2` consumers (ccc-statusd) pin contract v2 via `hello` and keep
receiving `root`, untouched. v2 emission is on the frozen typed path and is not
modified by this change. `root`-rev support is removed **only after** advisor
108da20a confirms the ccc-statusd stage-2 thin-proxy live cutover — gated on
confirmation, not a date. Removal is explicitly OUT of scope for this task.

## Rename table — every affected message

The concept renames everywhere it is spelled. Vocabulary-neutral slots that hold
a fingerprint value but do NOT spell "root" (`expected`, `actual`, `required`,
`changed`) keep their names.

### Response / notification fields (v2 → v3)

| v2 key         | v3 key                | Carried by (message)                          |
|----------------|-----------------------|-----------------------------------------------|
| `root`         | `fingerprint`         | `hello` body, `toc` body, `root`-op response  |
| `root_before`  | `fingerprint_before`  | `splice` response, Delta (notification + `diff`) |
| `root_after`   | `fingerprint_after`   | `splice` response, Delta (notification + `diff`) |
| `as_of_root`   | `as_of_fingerprint`   | `links` body, `stale_view` error extras       |
| `live_root`    | `live_fingerprint`    | `links` body, `stale_view` error extras       |

### Request fields (v3 → v2 on the way in)

| v3 key               | v2 key          | Carried by (request) |
|----------------------|-----------------|----------------------|
| `if_fingerprint`     | `if_root`       | `splice`             |
| `from_fingerprint`   | `from_root`     | `diff`               |
| `to_fingerprint`     | `to_root`       | `diff`               |
| `require_fingerprint`| `require_root`  | `links`              |

### Op verb (v2 → v3)

| v2 op   | v3 op         | Note |
|---------|---------------|------|
| `root`  | `fingerprint` | The integrity read op returns the fingerprint; the verb surface speaks the concept. |

Every other op keeps its v2 name (`toc`, `cat`, `extract`, `resolve`, `splice`,
`diff`, `links`, `sub`, `hello`).

### Error codes (v2 → v3)

| v2 code          | v3 code                 | Recovery (unchanged) |
|------------------|-------------------------|----------------------|
| `root_mismatch`  | `fingerprint_mismatch`  | `resync`             |
| `root_unknown`   | `fingerprint_unknown`   | `resync`             |

### Hello `caps` strings (v2 → v3)

| v2 cap                 | v3 cap                        |
|------------------------|-------------------------------|
| `root`                 | `fingerprint`                 |
| `splice.if_root`       | `splice.if_fingerprint`       |
| `links.require_root`   | `links.require_fingerprint`   |

`splice.if_node_rev` and every other cap are unchanged (they do not spell the
concept).

## Scope decision: field is the floor; op + error codes rename for coherence

Decision 0001 round 6 names the "wire field `root`" rename. This amendment goes
one step further and renames the op verb and the two error codes as well, so v3
carries **zero** `root` tokens for the fingerprint concept. Rationale: the
amendment exists to give a future community a clean vocabulary; a v3 that still
spoke `{"op":"root"}` or `root_mismatch` beside `fingerprint` fields would defeat
that purpose, and a fully coherent v3 is SIMPLER for the ccc-statusd stage-2
proxy to map (one rule: `root` → `fingerprint` everywhere) than a partial rename.
All of it lives in the reversible v3 projection layer (not yet consumed), so the
line is easy to move if the advisor wants to narrow it.

## Input acceptance (non-normative leniency)

A v3 session re-keys the fingerprint vocabulary to the v2 vocabulary before the
strict decoder runs, then decodes on the frozen v2 path. The rename is a no-op
for keys a frame does not carry, so a v3 client that happens to send a v2
spelling (`if_root`, or `{"op":"root"}`) is still accepted. This leniency never
violates no-dual-**emit** (emission is strict fingerprint-only under v3) and
keeps the projection minimal. Strict rejection of mixed-vocabulary INPUT is not
promised.

## M1 additive surface (2026-07-24) — the v3 session grows five capabilities

M1 (BIOS cut + data-model migration) adds five ADDITIVE surfaces to a v3
session. All are v3-ONLY: a v2 session stays byte-for-byte the frozen
contract (`crates/wire/tests/contract_v2.rs` pins it), and both hosts — the
per-workspace sidecar and the resident registry daemon — serve them through
the one shared `wire-serve` implementation.

### 1. The composed `read` op (decision D6)

Addressing + content + render at ONE engine snapshot — one round trip
replacing the `extract`→`cat`→render split whose non-atomicity would create a
`fingerprint_mismatch` retry race. Advertised as the v3-only cap `read`
(appended by the hello projection; the frozen v2 `caps` lists never carry it,
and a v2 session's `read` answers `unknown_op` — §3.2 discovery honesty).

Request (all beyond `path` optional):

```
{"id":7,"op":"read","path":"notes/plan.md",
 "mode":"toc"|"sections",          // default "toc"
 "frag":"Goals",                   // scope to one section subtree
 "sections":["Goals/Q3","^b1","2"],// selectors: sanitized hpath | dewey | ^anchor
 "display_path":"$SESSION/notes/plan.md", // header spelling; defaults to path
 "actor":"agent:b0864fb2"}         // §9 read provenance (D-Actor/B): the
                                   // DAEMON-derived actor, never
                                   // MCP-caller-settable; carried now so
                                   // stage-2 read-mint receipts are additive
                                   // (no receipt is minted in M1)
```

Response body (`mode` decides `toc` XOR `sections`):

```
{"path":…,"file_rev":…,"fingerprint":…,"words_total":N,
 "toc":[{"n":"1.2","depth":2,"title":…,"hpath":…,"words":N,"sec_rev":…}],
 "sections":[{"sel":…,"hpath":…,"sec_rev":…,"words":N,"content":…}],
 "truncated":true, "notice":"unresolved selectors (no rev minted): …",
 "rendered_text":"…"}
```

> **This M1 sketch is superseded by § Stage-2 additive surface below.** The read
> body now also carries an always-emitted `anchors[]` array and per-`toc`-row
> `span` / `content_span`, and the `actor` slot below — UNREAD in M1 — now mints
> a read receipt. The current shape is § Stage-2 item 6.

`file_rev` + `fingerprint` come from the SAME borrowed snapshot (the
atomicity witness). `rendered_text` is the token-efficient text projection,
byte-parity with the Go host face's `readText` (gated against the U0 captured
corpus). Unresolved selectors follow the PARTIAL-read rule (`truncated` +
`notice`, no rev minted); ALL selectors missing refuses `ref_not_found`.
Refusal `message` strings are the Go host face's VERBATIM texts, so a thin
proxy forwards `error.message` without re-minting.

**The two content planes (op-owner ruling, 2026-07-24):** `sections[].content`
is the RAW face — the verbatim bytes its `sec_rev` was minted over, so every
row is self-verifying and a `put` built from it round-trips byte-identically.
Block elision (decision #8: `meridian-*` fences hidden on the render face)
applies to `rendered_text` ONLY. The one composed exchange therefore carries
both planes: raw content to edit against, elided text to display.

### 2. In-band timing: `meta.duration_us`

Every DISPATCHED response frame on a v3 session carries a top-level sibling
`meta: {"duration_us": N}` — integer µs of engine work, measured around the
dispatch call at each host (sidecar `arms::dispatch`; daemon `dispatch_read`).
Success and refusal frames alike; frame-layer verdicts, the serve-layer
`sub`, and the daemon's intercepted `hello` carry none. A safe additive slot
under the tolerant-client law (unknown response fields are ignored).

### 3. `extract` heading enrichment

Under v3, `extract` heading nodes additionally carry the host-face
addressing facts, computed engine-side with Go-exact semantics: `n` (dewey
ordinal), `hpath_text` (the sanitized joined address), `words`
(`strings.Fields` count over the subtree-inclusive content span). Absent on
every non-heading node and on every v2 frame.

### 4. The `check_write` op (M1 U8c)

The engine-side I4 def-conformance verdict (`meridiandefs.CheckWrite`):
`{path, target, actor, now, edits[]}` where `edits` is the put-plan
vocabulary `{op, at, find?, body?, rev?, all?}`. Pure verdict — no flock, no
CAS, no disk mutation; the host owns authz and sequencing. Refusals come back
in the body (`refuse: {class: "rebuild"|"verdict", code, message, remedy}`),
never as a wire error frame. v3-only at DISPATCH (a v2 session answers
`unknown_op`); advertised by the hello projection.

### 5. `splice.plan_edits` (M1 U8b)

`splice` gains ONE optional top-level field, `plan_edits` — the plan-level
batch (the Go daemon's deleted `buildSpliceEdit`/`buildPropertyEdits`
emulation, moved behind the wire), mutually exclusive with `edits` (both →
`bad_request`; neither → the frozen `missing `edits` on `splice``). v3-only
AT DECODE: the session rev threads into the strict decoder
(`decode(obj, rev)`), and under v2 the field hits the FROZEN unknown-field
wall byte-for-byte. Items are externally tagged, exactly one of:

```jsonc
{"append":          {"hpath": s, "body": s}}
{"match":           {"hpath": s, "old": s, "new": s, "all": bool?, "rev": s?}}
{"replace_section": {"hpath": s, "body": s, "rev": s?}}
{"create":          {"parent_hpath": s, "title": s, "body": s}}
{"set_property":    {"key": s, "value": s}}
```

Addresses are the HOST-face sanitized hpath forms. The engine lowers each
shape to native edits at the splice intake (`wire-serve::plan`,
emulation-byte-faithful: append newline discipline, match-all
read-modify-write, create = parent-append, set_property = the property-group
dance with conditional quoting); armed facts align 1:1 with the LOWERED
edits. `rev` values thread to `if_node_rev` (the v2 CAS domain). Cap:
`splice.plan_edits`, projection-advertised.

Target-class refusals (unresolvable section, block-anchor append, top-level
create, no-frontmatter property) are `bad_request` whose `message` is the
Go-face teaching WITHOUT the host's `put: ` verb prefix. NOTE the deliberate
asymmetry (op-owner ruling 2026-07-24): the composed `read` op mints its
`read: ` prefixes ENGINE-side, while `plan_edits` refusals are prefix-less
and the put host renders the verb — each face is internally consistent and
golden-arbitrated.

### Error-code taxonomy additions (M1)

| Code | Recovery | Raised by |
|---|---|---|
| `write_conflict` | `refresh` (re-read) | the splice choke-point: pre-rename verify detects a concurrent external change |
| `workspace_busy` | `retry` | the cross-process write flock: another cooperating writer holds `.meridian/write.lock` |

`render_failed` is the render plane's TYPED internal failure
(`{node_kind, node_ref, reason}`, recovery: none — a bug, not a retry); on
the wire it surfaces under `internal` with the `render_failed` spelling in
`message`. No frame ever half-renders.

## Stage-2 additive surface (2026-07-25) — the attestation core loop

Stage 2 adds the attestation CORE LOOP: the pin verb, the read-is-the-mint
receipt, the meridian-lock drift color, and the `@fp` claim-link view grammar.
Everything below is v3-ONLY and additive. A v2 session stays byte-for-byte the
frozen contract — `crates/wire/tests/contract_v2.rs` pins it, and every new
request field rides `Option` + `skip_serializing_if` so an absent value
serializes away.

### 6. The composed `read` grows authz facts and its own anchor plane (S1, s1c)

The current response body, superseding the §1 sketch:

```jsonc
{"path":…,"file_rev":…,"fingerprint":…,"words_total":N,
 // mode toc only — the HEADING plane, whole
 "toc":[{"n":"1.2","depth":2,"title":…,"hpath":…,"words":N,"sec_rev":…,
         "span":[S,E],"content_span":[S,E]}],
 // ALWAYS emitted, in BOTH modes — the `^id` BLOCK-ANCHOR plane
 "anchors":[{"anchor":"b1","span":[S,E]}],
 // mode sections only
 "sections":[{"sel":…,"hpath":…,"sec_rev":…,"words":N,"content":…}],
 "truncated":true, "notice":"unresolved selectors (no rev minted): …",
 "rendered_text":"…"}
```

**Two row classes, two arrays. The discriminator is the ARRAY, never a field.**
`toc[]` carries headings only and `depth >= 1` always; `anchors[]` carries block
anchors, with `anchor` spelling the id WITHOUT its `^` marker. S1 first shipped
both classes interleaved in `toc[]`, discriminated by anchor-present
(equivalently `depth == 0`); that panicked ccc-statusd's `readText`, which
indents by `strings.Repeat("  ", depth-1)`, with "negative Repeat count". s1c
split the planes on the rule that *a field a caller may forget to check is not a
guard; a shape it cannot receive is* (`crates/wire-map/src/facts.rs:186-205`).

**The anchor plane is a property of the RESPONSE, not the mode.** `anchors` is a
`Vec` with `#[serde(default)]` and no `skip_serializing_if`
(`crates/wire/src/lib.rs:899-907`), so it is emitted unconditionally — toc mode
and sections mode alike. An empty `anchors[]` means "this scope holds no
addressable block anchor", and it means only that. A mode-conditional array
would make `[]` mean two things, and a caller cannot tell "no anchors here" from
"you asked in the wrong mode". `toc` and `sections` stay `Option` and vanish
when the other mode is in force.

The two byte facts, both half-open `[start,end)` offsets into the RAW on-disk
bytes of the file this response names:

| field | covers |
|---|---|
| `toc[].span` | the heading line INCLUSIVE and the whole subtree INCLUSIVE — so heading spans NEST, and one anchor answers every ancestor section |
| `toc[].content_span` | heading-EXCLUDED, subtree-inclusive. Typed `Option` (a content-less heading may omit it); present on every shipped row, possibly as a zero-width span at EOF |
| `anchors[].span` | the block-leaf span, marker included |

Together they answer governing-section derivation by BYTE CONTAINMENT: keep
every heading row whose `span` contains an anchor's start byte. That is the fact
that retired ccc-statusd's `sanitizeHeadingHost` markdown mirror — the host now
holds zero markdown semantics on the put authz plane (exit criterion 1).

Three consequences a consumer must not guess at:

- `sec_rev` is minted over the FULL `span` (heading INCLUDED), while
  `sections[].content` is `raw[content_span]` (heading EXCLUDED). For a heading
  section the content and the rev cover different byte ranges.
- Only heading and list-item nodes become rows. A `^id` hosted by a task,
  callout, fence, or table is addressable on NEITHER plane
  (`crates/wire-map/src/facts.rs:8-17`).
- `frag` scopes the two planes by different rules, deliberately: headings by
  hpath prefix, anchors by the same byte containment the host applies. A scoped
  read never leaks an anchor from outside the requested subtree.

**D12:** spans are intra-file byte offsets, root-independent by construction. A
later `root:` prefix changes which file is named, never this arithmetic.

### 7. `read.actor` mints a read receipt — read IS the mint (S6)

The `actor` slot §1 carried UNREAD now mints a read receipt at the composed-read
seam. The receipt is a minimal mechanical fact (D9) — `{actor, path, selector,
sec_rev}` — with no verdict envelope and no `predicate_type`; that
representation unification is stage-3.

- **No wire shape changes.** The receipt never leaves the engine. It is daemon
  session memory, held beside the workspace engines rather than inside one, so
  the pin's own write cannot evaporate the receipt that authorized it
  (`crates/registry/src/registry.rs:91-100`).
- **Sections mode only.** A `toc`-mode read serves the section map, not content,
  so it mints nothing — it must never gate an attestation over bytes nobody saw.
  The rev bound is the raw face's `sec_rev`, never anything derived from the
  elided `rendered_text` (`crates/wire-serve/src/read.rs:235-254`).
- **`actor` absent or blank mints nothing** (D16). The bare CLI sends no actor
  and is local-operator-trusted, exactly as `mrd put` skips the host's authz.
  The per-request sidecar holds no session and so mints nothing either; the
  resident daemon is the one host that mints.
- **The key is the CANONICAL selector**, `toc`-row `hpath` — not the caller's
  spelling. A read addressed by dewey ordinal `1.1` mints under `Goals/Q3`; an
  anchor read mints under `^b1`, caret INCLUDED. Note the deliberate asymmetry
  with `anchors[].anchor`, which carries no caret.
- **Lookup is EXACT on all three key parts, and fails CLOSED.** Reading
  `Notes/Plan` does not cover `Notes/Plan/Q3`, nor a `^anchor` inside the served
  subtree (`crates/receipt/src/read_mint.rs:121-134`). Widening to span
  containment is a permissive authz answer that would need its own ratified
  decision.
- **Lifecycle:** no TTL. The ledger is dropped when the daemon exits or the
  workspace is idle-reaped, and nothing is ever persisted. A per-actor cap of
  1024 distinct `(path, selector)` pairs is the memory backstop; a re-read
  replaces its receipt in place rather than adding one.
- **The grain is per-session, and that is a named relaxation.** A per-turn grain
  would be tighter. Per-session is what the engine can honestly express, because
  the engine has no turn concept; the rev-recheck under the pin's own flock
  (item 8) mitigates CONTENT staleness, and does not mitigate the temporal
  question of whether the content is still in the actor's context.

A receipt answers "was it read", never "is it current". A caller gating a write
re-checks the rev against disk inside its own flock — a receipt is not a lease.

### 8. `splice.pin` — the pin rides the write choke-point (S7)

`splice` gains ONE optional top-level field, `pin`. There is no `Op::Pin` and no
second flocked `lock_write` call (D7): the flock is non-reentrant per open file
description, so composing two flocked calls would self-refuse `workspace_busy`.
The pin instead rides the existing `commit_batch` primitive, which already
writes two files under one flock.

Request:

```jsonc
{"id":9,"op":"splice","path":"notes/plan.md",   // the PINNING page
 "pin":{"target":"guide.md",                    // the page holding the content
        "selector":"Guide/Leader's-Guideline",  // sanitized hpath, or "^id"
        "vibe":true}}                           // optional; absent ⇒ read-only oid
```

A pin is itself a write, so a pin-only splice is a COMPLETE batch — `edits` may
be absent without raising the frozen ``missing `edits` on `splice``` refusal.

Response — `ResponseBody::Splice` gains a sibling `pin` object, absent unless
the request carried one:

```jsonc
{"pin":{"target":"guide.md",
        "selector":"Guide/Leader's-Guideline",     // the CANONICAL selector
        "declared_ref":"guide.md#Guide/Leader's Guideline",  // the lock ref
        "fingerprint":"fp1.span2.b3.<64 hex>",
        "blob":"<40 hex>",                          // OMITTED when git could not answer
        "anchor":"leaders-guideline",
        "promoted":true}}
```

**There is deliberately NO `pin.actor` field (D13).** A pin's mint identity IS
its gate identity IS the splice's own daemon-derived `actor`. The sibling
`check_write` op carries a caller-settable actor; a pin must not, or a caller
forges a pin as somebody else. `pin.actor` is refused at decode with ``unknown
request field `actor` on `pin` ``.

**Two spellings of one address, and they are not interchangeable.** `selector`
is the SANITIZED host-face hpath — what callers pass, and what the read receipt
is keyed on. `declared_ref` is what lands in the lock, and its fragment is the
RAW `/`-joined heading chain, because `model::selector::Selector::parse` is the
verify plane's front door and matches heading text byte-exactly. Writing the
sanitized spelling into the lock would mint a ref that resolves to nothing for
any heading containing a space — a pin that reads `red(dangling)` the moment it
lands (`crates/wire-serve/src/write.rs:1117-1155`).

**The fingerprint is minted over exactly the span the ref RESOLVES to** — the
full node span, heading-inclusive and subtree-inclusive — because that is the
span the verify plane recomputes. The promoted `^slug` is deliberately NOT the
ref: an anchor node's model span is its HOST LINE, so an `^id` ref over a
promoted heading would narrow a section pin to its heading text and read green
on every body edit.

Ordered under the ONE flock the splice already holds:

1. **read-mint gate** (D16) — the receipt lookup, then a rev-recheck of that
   receipt against the bytes on disk right now. An absent actor bypasses both.
2. **slug decision** (D15) — an id already in the promotion slot is REUSED
   verbatim, which is what makes a re-pin idempotent.
3. **anchor promotion** — the ONE write ordered before the commit. It is
   rev-NEUTRAL, because norm-v2 removes the marker and its leading space, so the
   target's fingerprint cannot move and no other page pinning that target
   reddens. That rev-neutrality is what makes promotion into a possibly-unowned
   target honest (D14).
4. **fingerprint + blob oid** — over the RE-RESOLVED span, since a landed
   promotion widened the node by one line. `--vibe` adds `git hash-object -w`;
   without it the oid is computed read-only. When git cannot answer, `blob` is
   ABSENT — never a fabricated sha (D5).
5. **`commit_batch(content + lock)`** — the lock block rides the batch as the
   one engine-minted span edit, so content and lock land in ONE `apply_batch`:
   one flock, one rename.

**`splice.pin` IS advertised in the hello `caps` list — under v3 only** (S3 U1,
advisor R23). The v3 caps projection appends `read`, `check_write`,
`splice.plan_edits`, and `splice.pin`. A v2 session's caps never carry it, and
that is not an omission: a v2 session REFUSES the `pin` field at the strict
decoder's field wall, so advertising it there would be a false advertisement.

**The extension rule, stated once so the next field does not re-run this
exchange:** *a v3-era `splice` field is advertised as `splice.<field>` by the v3
projection; the enumeration test is the enforcement.* That test
(`crates/wire-serve/src/rev.rs`, `v3_splice_amendments_are_all_advertised`)
derives its expected set from the decoder's own arrays —
`SPLICE_V3_FIELDS \ SPLICE_V2_FIELDS` in `crates/wire-serve/src/decode.rs` — so a
splice field added under v3 without a matching `caps.push` fails the suite before
it can ship.

**One stated exception, and the arithmetic that closes it.** `force` is honoured
and advertised by neither list. It is **v2-era**, so advertising it would require
changing the FROZEN v2 `CAPS` constant, which whole-frame v2 byte-identity
forbids (stage-2 criterion 7). It is named here rather than silently left:

| Class | Fields | Disposition |
|---|---|---|
| Envelope/common (v2-era) | `path`, `actor`, `now`, `edits` | Covered at op grain by the bare `splice` cap |
| v2-era, dotted-advertised | `receipt`, `if_root`→`if_fingerprint`, `dry` | In both lists |
| v2-era, **not advertised** | `force` | The stated exception — the frozen v2 constant is why |
| v3-era amendments | `plan_edits`, `pin` | Both advertised by the v3 projection; both covered by the enumeration test |

4 + 3 + 1 + 2 = **10 = |`SPLICE_V3_FIELDS`|.** The enumeration closes.

**D12:** `pin.target` is carried VERBATIM into the lock's `ref` and `objects:`
key. Nothing on this path parses it, so a later `root:` prefix rides through
untouched.

#### The lock ARTIFACT is guarded, not just the pin verb (advisor R25)

The read-mint gate above guards the `splice.pin` DOOR. The `meridian-lock` block
it protects is ordinary page text, so every put shape can compose those bytes
without the `pin` field ever being set. **A write whose candidate document
changes the page's lock bytes is REFUSED (`bad_request`) unless those bytes are
exactly the block this call minted.** The comparison is byte-identity over the
raw block, so a change from one unparseable block to another is still a change.

This binds every door to those bytes, not one verb: native `edits`, a lowered
`plan_edits` batch, `create` (a birth carrying a lock block), the pin's own
anchor promotion (which must be lock-neutral), and the run plane's
`fs::apply_batch` — which bypasses the write choke-point entirely and therefore
mounts the same guard (`run::executor`, refusing before any byte lands). No
actor is consulted: the CLI is local-operator-trusted for the pin VERB (D16), and
is refused here like anyone else, because "the engine is the lock's sole writer"
is a statement about the artifact.

Two behaviors follow and are stated rather than discovered:

- **A pin that lands unchanged bytes is fine** — a re-pin of unchanged content is
  byte-idempotent, so the guard never fires on it.
- **A whole-section rewrite that would DELETE the lock refuses.** The block is
  birthed at EOF, so it lives inside the page's last section: `put at:content`,
  `put at:all` and `replace_section` on that section would erase the attestation,
  and erasing a RED pin to leave a page reading clean is exactly what the guard
  exists to prevent. Append (`put at:end`) into the same section is unaffected;
  deliberate removal is a hand edit, the same repair path a corrupt lock already
  documents (#8 §3).

### 9. Error-code taxonomy additions (stage 2)

| Code | Recovery | When it fires | What the caller does |
|---|---|---|---|
| `read_mint_required` | `fix` | A pin on the agent path whose actor holds no read receipt for that exact `(path, selector)` — or a host with no receipt ledger at all (the per-request sidecar) | Read the selector first, in mode `sections`, with that exact spelling; then pin. Against a sidecar, pin through the resident daemon or the local CLI instead |
| `pin_target_missing` | `fix` | The pin target page does not exist, its selector addresses no section, or the selector stopped resolving after anchor promotion | Re-read the target with mode `toc` to list its section paths, then pin an address that exists. The drift surface renders the same condition as `red(dangling)`, never silent green |
| `write_conflict` | `refresh` | Two sites: the pin's rev-recheck finds the receipt covers one rev and the section now carries another; and the splice choke-point's pre-rename verify detects a concurrent external change | Re-read the one node (re-reading also re-mints the receipt), then retry. `expected` and `actual` carry the two revs |
| `workspace_busy` | `retry` | Another cooperating writer holds `.meridian/write.lock`. The flock is non-reentrant, so this also fires if a caller composes two flocked writes | Retry the same request; it is transient. Never compose two flocked calls — a pin rides ONE |

`write_conflict` and `workspace_busy` were minted in M1 (§ Error-code taxonomy
additions above); stage 2 adds the pin firing conditions to both. Recovery
classes are unchanged and remain statically bound in `crates/wire/src/lib.rs`.

### 10. The `@fp` claim-link decoration (S10) — agent-plane, never in a claim-link position on disk

A claim link is decorated with its drift color on the way OUT and stripped on
the way IN.

**The claim, stated exactly as wide as the proof: NO `@fp` TOKEN SURVIVES IN A
CLAIM-LINK POSITION ON DISK.** That is the block-ref slot of a wikilink or
embed, and only that. It is deliberately NOT the wider claim "no `@fp` token is
ever in stored bytes", and NOT "no `@` anywhere" — both are false, and § The
bound on that claim below says why. Within the claim's scope the guarantee is
total: the engine never mints a fingerprint claim an author did not write, and
no put path can land one in a claim-link position.

The grammar is SHAPED, not opaque (D10): `@<tone-word>.<8 lowercase hex>`, where
`tone` is up to 12 lowercase ASCII letters and the digest is exactly 8 lowercase
hex characters (`crates/syntax/src/lib.rs:505-524`). A fully-opaque
"everything after `@`" rule would be unparseable against a heading fragment and
would eat authored text.

- **It rides the BLOCK-REF slot alone** — `[[target#^id@green.b3af12cd|label]]`.
  A heading fragment is a different slot, so `[[Page#Q@Home]]` is never even
  examined. The D10 ambiguity is closed structurally, not by luck of spelling.
- **Decoration lands in `rendered_text` ALONE.** `sections[].content` stays the
  raw face a put is built from — a read-decorated view feeding a write is the
  data-loss class.
- **It is capability-gated, not a flag.** There is no opt-in switch. The
  resident daemon decorates because it holds a corpus; the per-request sidecar
  and the bare CLI pass the one spelling of "nothing to decorate", because a
  host with one document cannot color a pin whose target is another page. An
  undecorated link claims nothing; a wrongly-colored one lies.
- **The strip runs at DOCUMENT grain, and what it cannot remove it refuses**
  (advisor R25). It is not a walk of named payload fields — that list missed
  `plan_edits.create.title`, and it judged each payload OUT of the document it
  lands in. The write path builds the candidate document once, identifies tokens
  in it through the ONE dialect parse, removes them from the payload that carries
  them, and then re-checks: a token this write would still INTRODUCE refuses
  `bad_request` rather than landing (`crates/wire-serve/src/write.rs`,
  `strip_fp_candidate`). Two consequences worth stating: a token composed out of
  bytes the batch did not supply (an edit that merely closes a link around them)
  refuses, and a token already on disk is left exactly as found — removing bytes
  this write does not own would move a fingerprint it does not own. `create`
  strips its whole body, which is the same grain by construction, and
  `check_write` builds and strips the same candidate, so the pre-flight judges the
  bytes the committer lands.
- **Addresses are peeled at their own owner, not by the document strip.** An
  address is compared, never stored: `model::Ref::anchor`'s two guard sites (the
  wire decoder and the single wire→model bridge —
  `crates/wire-serve/src/decode.rs`, `crates/wire-serve/src/read.rs`), the
  `match.old` NEEDLE at the one funnel every native and lowered edit passes
  through, the plan lowering's own needle search, and `check_write.at`. A needle
  copied from the decorated render face must find its undecorated bytes, and both
  entry points must resolve the same spelling.
- **The decorated address is addressable, not display-only.** An agent that read
  `[[guide#^goal@green.b3af12cd]]` may address `^goal@green.b3af12cd` and reach
  exactly the node `^goal` names.
- **The tone word ALWAYS rides, green included.** A token is never abbreviated
  to its digest for the green case. The consistency argument is the point: if
  the marker were present only when a pin is non-green, then an absent marker
  would mean either "green" or "nobody computed this", and a reader could not
  tell which. That is exactly the two-meanings-for-one-shape ambiguity s1c
  removed from the anchor plane, and the same discipline applies here — the
  decorator calls the minter unconditionally on the tone the color model
  returns (`crates/wire-serve/src/read.rs:376-378`).
- **An `@` the shape does NOT recognize refuses.** The block-id charset
  (`[A-Za-z0-9-]`, §2.4) has no `@`, so an unshaped tail survives to validation
  and raises `bad_request`: ``block id outside the one charset [A-Za-z0-9-]
  (§2.4): `<id>` ``. In a claim-link position there is no third outcome:
  shaped-and-stripped, or unshaped-and-refused.

#### The bound on that claim — a fenced code sample is not a claim link

**A token-shaped string inside a code fence is NOT stripped, and stored bytes
therefore CAN contain one.** A document whose body carries

````
```
[[guide#^goal@green.b3af12cd]]
```
````

round-trips through a put with those bytes intact
(`crates/syntax/src/lib.rs:1284-1287` pins exactly this case).

This is correct behavior, not a leak to apologize for. A token-shaped string
inside a fence is a **code sample** — documentation of the grammar, a test
fixture, this very amendment. Stripping it would be corruption: the engine would
silently edit an author's illustration of a claim link into something that is no
longer the thing being illustrated. The strip identifies tokens through the ONE
dialect parse, so it sees only real block-ref slots, and a fenced sample is not
one. The engine declines to touch it for the same reason it declines to touch
`[[Page#Q@Home]]` — neither is a claim-link position.

What survives inside a fence is inert. It is not a claim link, so nothing
decorates it, nothing resolves it, and no verdict is minted from it. The
narrower claim above holds precisely because the wider one was never the target.

**Three further positions are outside the claim for the same reason, and are now
tested as explicit exclusions**: frontmatter, HTML comments, and indented code.
The one dialect parse mints no wikilink node in any of them, so a token-shaped
string there is not a claim-link position — and the DECORATE side reads that same
tree, so the engine can never mint a token into a position the strip does not
reach. That symmetry is what makes the exclusion safe rather than a hole; it is
asserted directly, tree-shape and all, in
`crates/wire-serve/tests/s2fix_fp_document_grain.rs`.

#### The one true ambiguity, stated as a limit

**An author cannot write a literal `@green.deadbeef` immediately after a block
ref inside a wikilink.** Authored `[[guide#^goal@green.deadbeef]]` is
shape-recognized on the way in and stored as `[[guide#^goal]]` — the author's
literal text is lost, with no refusal.

This is the price of a shaped grammar and it is bounded to one position. It does
not spread, because the STORED plane stays unambiguous by construction: the
block-id charset is `[A-Za-z0-9-]` and contains no `@`, so no legitimate stored
block id can hold one, and there is no authored construct in that slot the strip
could be confusing for a token. Outside that slot — heading fragments, link
labels, prose, code fences — an author writes any `@` text freely. An author who
genuinely needs to show the decorated spelling puts it in a fence, which is what
this document does.

**D12:** the link target is opaque to the decorator. A later `root:` prefix
rides inside it untouched — the slot is reserved by not being parsed.

### 11. Drift color is NOT a wire field

The meridian-lock drift color (green / red / grey, each carrying its reason) is
computed per query run and never rides a frame. `model::selector::GreyReason`
has ZERO presence in `crates/wire`. Colors reach a consumer only as CLI human or
`--json` text (`mrd walk`, `mrd status`) and as an in-memory read-face column
that is never persisted. The color law is
`docs/wire-contract-v2-colors-amendment.md`; the shipped `mrd status` surface is
`docs/status.md`.

One consequence a reader will otherwise get backwards: **a green `lock` axis
does NOT imply the tree is current.** See `docs/status.md` § The composed status
line.

### Named residuals carried by this surface

Accepted, documented, not prevented. A doc that hides a residual is worse than
no doc.

- **G1 — pending-anchor durability is `gc.pruneExpire`.** A `--vibe` blob is
  written eagerly but reachable from no ref, so git may prune it (default two
  weeks). Committing the file is the only durable anchor. A pruned blob honestly
  re-classifies as `never-anchored` rather than reading as anchored. The
  `mrd status` vibe-debt gauge measures the size of this window.
- **G2 — the flock serializes COOPERATING writers only.** An out-of-band write
  is DETECTED by the drift color, never prevented. The git pre-commit hook fence
  is stage-3.
- **G3 — the two-inode commit is not all-or-nothing.** A pin writes the promoted
  anchor before the batch. If the lock write then fails, what remains is a
  rev-neutral, slug-derived anchor — a benign orphan that a re-pin reuses and
  heals (D15). It is never silent corruption.
- **G4 — refs are intra-root only** (D12). Cross-root addressing is stage-3;
  stage 2 only keeps the seam open.
- **G5 — anchor promotion into an unowned target churns that file's
  `node_rev`/CAS**, a griefing surface. It is accepted for the core loop because
  the promotion is rev-neutral. The hook fence and the authz tightening are
  stage-3.

## Implementation shape

The frozen `wire` types serialize byte-for-byte as contract v2 and are NOT
touched (the only additive change is the optional `Op::Hello.contract` input
field, which carries the declaration; absent ⇒ serialized away; the M1
additive fields ride `Option` + skip-serializing, so v2 frames never grow a
key). v3 is a pure projection at the envelope layer — lifted from the
sidecar into `crates/wire-serve/src/rev.rs` so BOTH hosts project through
one implementation:

- Outgoing v2-shaped frames are re-keyed `root` → `fingerprint` (`project_response`,
  `project_delta_frame`). The projection touches only the known fingerprint
  slots and NEVER descends into the arbitrary-key maps (`links.files`,
  `resolved`, `unresolved`, Delta `files`), where a corpus path or raw linkpath
  could legitimately be the string `"root"`.
- Incoming v3 requests are re-keyed `fingerprint` → `root` before the strict
  decoder (`rename_request`), at the flattened request top level only.

The v2 emission path is unchanged (`wire::Response` serialized directly), so the
byte-identical guarantee is structural, proven by the untouched frozen goldens.

## Tests

- `crates/sidecar/tests/contract_v3.rs` — the negotiation gate: a v2 session
  emits `root`/never `fingerprint` (bytes matched); a v3 session emits
  `fingerprint`/never `root` in every message class (hello, toc, the renamed op,
  splice before/after, links triple, the two error codes); unknown rev → typed
  error; explicit `"v2"` ≡ absent. M1 adds: `meta.duration_us` on every v3
  dispatch frame + never on v2; extract enrichment on v3 headings + zero
  addressing keys on v2.
- `crates/wire-serve/src/rev.rs` unit tests — the projection in isolation, incl.
  the map-key collision guard (a `[[root]]` linkpath and a file named `root`
  survive) and the v3 caps `read` advertisement.
- `crates/registry/tests/wire_vocab_rev.rs` — the same gates on the daemon
  socket (the second host).
- `crates/testsuite/tests/u4a2_composed_read.rs` — the composed `read` op
  through the LIVE serve loop against the U0 captured goldens: rendered text
  and refusal messages byte-equal the Go host face; v2 `read` → `unknown_op`
  with frozen caps.
- Stage-2: `crates/wire-serve/tests/s7_pin.rs` (the pin under one flock — gate,
  rev-recheck, promotion idempotence, vibe blob, the refusal set),
  `crates/registry/tests/read_mint.rs` (the ledger survives the write that
  rebuilds the warm engine, and mints nothing to disk),
  `crates/wire-serve/tests/s10_fp_decorate.rs` (decorate → strip round-trip
  leaving no token in a claim-link position; heading-`@` intact; an unshaped
  `@fp` refuses and writes nothing) and
  `crates/syntax/src/lib.rs`'s `strip_fp_removes_block_ref_tokens_only` (the
  bound: heading fragments, labels, prose, and FENCED CODE all survive),
  `crates/view/tests/board_pin_verdict_gates.rs` (the board's verdict equals the
  walk's, value for value, over all six outcomes).
- Stage-2 fix loop (R25): `crates/wire-serve/tests/s2fix_artifact_guard.rs` — the
  lock ARTIFACT guard driven door by door (an un-read actor's forged pin through
  ordinary `edits`, the lowered `plan_edits` batch, a birth carrying a lock, a
  hand rewrite of a minted fingerprint, a whole-section rewrite that would delete
  the block), each asserting the WRITE refuses and the file is byte-unchanged,
  with the minted pin and its idempotent re-pin as controls;
  `crates/run/tests/executor.rs` for the run-plane door;
  `crates/wire-serve/tests/s2fix_fp_document_grain.rs` — the strip at document
  grain (`create.title`, every payload shape without a field list, a token
  composed out of retained bytes refusing, a token landing inside a fence
  surviving, the frontmatter / comment / indented-code exclusions with their
  decorate-side control, and `check_write` agreeing with the committer).
- Frozen and unchanged, still green: `crates/wire/tests/contract_v2.rs`,
  `crates/testsuite/tests/wire_vocab.rs`, `crates/sidecar/tests/dispatch_v2.rs`,
  `crates/transport-proto/tests/wire_agreement.rs`.
