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

## M1 additive surface (2026-07-24) — the v3 session grows three capabilities

M1 (BIOS cut + data-model migration) adds three ADDITIVE surfaces to a v3
session. All three are v3-ONLY: a v2 session stays byte-for-byte the frozen
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
 "display_path":"$SESSION/notes/plan.md"} // header spelling; defaults to path
```

Response body (`mode` decides `toc` XOR `sections`):

```
{"path":…,"file_rev":…,"fingerprint":…,"words_total":N,
 "toc":[{"n":"1.2","depth":2,"title":…,"hpath":…,"words":N,"sec_rev":…}],
 "sections":[{"sel":…,"hpath":…,"sec_rev":…,"words":N,"content":…}],
 "truncated":true, "notice":"unresolved selectors (no rev minted): …",
 "rendered_text":"…"}
```

`file_rev` + `fingerprint` come from the SAME borrowed snapshot (the
atomicity witness). `rendered_text` is the token-efficient text projection,
byte-parity with the Go host face's `readText` (gated against the U0 captured
corpus). Unresolved selectors follow the PARTIAL-read rule (`truncated` +
`notice`, no rev minted); ALL selectors missing refuses `ref_not_found`.
Refusal `message` strings are the Go host face's VERBATIM texts, so a thin
proxy forwards `error.message` without re-minting.

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

### Error-code taxonomy additions (M1)

| Code | Recovery | Raised by |
|---|---|---|
| `write_conflict` | `refresh` (re-read) | the splice choke-point: pre-rename verify detects a concurrent external change |
| `workspace_busy` | `retry` | the cross-process write flock: another cooperating writer holds `.meridian/write.lock` |

`render_failed` is the render plane's TYPED internal failure
(`{node_kind, node_ref, reason}`, recovery: none — a bug, not a retry); on
the wire it surfaces under `internal` with the `render_failed` spelling in
`message`. No frame ever half-renders.

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
- Frozen and unchanged, still green: `crates/wire/tests/contract_v2.rs`,
  `crates/testsuite/tests/wire_vocab.rs`, `crates/sidecar/tests/dispatch_v2.rs`,
  `crates/transport-proto/tests/wire_agreement.rs`.
