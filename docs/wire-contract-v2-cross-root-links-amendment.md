# Wire contract v2 amendment — cross-root edges on §4.6 `links` (U21)

Status: normative amendment to `docs/wire-contract-v2.md` §4.6 on the read
plane. `docs/wire-contract-v2.md` is FROZEN and unedited; this file is the sole
normative text for cross-root edges on the edge map. It rides the existing rev —
both keys are additive and omitted when empty, so a single-root corpus
serializes byte-for-byte what it served before U21 — per the colors / refusal /
v3 amendment precedent, a separate normative doc, never an edit of the frozen
v2. Law: U21 Q3, Q5, and the sixth-question ruling (option C) with its three
gates.

## The two additive keys

```json
{"files":{"claim.md":{
  "resolved":{"local.md":1},
  "unresolved":{"roadmap":1},
  "resolved_rooted":{"sessions":{"notes.md":1}},
  "refused":{"sessions:absent.md":{
    "color":"red","reason":"file-not-found",
    "detail":"root 'sessions' holds no 'absent.md'",
    "message":"red(file-not-found): the address 'sessions:absent.md' names root 'sessions', which this machine binds and reads — …",
    "count":1}}}}}
```

**`resolved_rooted` is two levels, never one joined `root:path` key.** A rooted
destination is two facts. Folding them into one string would put a joined
address in a machine surface — ZT decision 14 / R1.6 — and would collide with
any ambient path that legitimately contains a colon. A consumer fetching bytes
MUST read the root: resolving the inner path against the ambient corpus is the
wrong-bytes success FINDING 03 named, one layer up.

**`refused` is not `unresolved`, and the split is a fact rather than a
formatting choice.** An ambient dangling link is an ordinary authoring state in
a working vault: first-class, non-refusing. A refused edge means the author
believed a mount relationship that does not hold, or wrote something outside the
address grammar. `color` and `reason` are the colour plane's own vocabulary
(`unmounted`, `path-unseeable`, `file-not-found`, `bad-ref`) — never a second
spelling of it, which is the cross-crate re-spelling S3-R6 forbids. `message` is
the full teaching refusal, verbatim.

Consumer exit-code guidance (U21 Q3(a)): a refused edge is a finding; an
`unresolved` one is not. `mrd links` rides exit 1 on the first and exit 0 on the
second.

## The asymmetry — STATED, NOT SMOOTHED

**For this one op the resident daemon is knowingly LESS CAPABLE than the
in-process engine.** Its warm state is one workspace corpus keyed by one
canonical path (`registry::warm_or_build`), and it holds **no mount authority at
all**. It therefore answers every rooted spelling `unresolved`, exactly as it
did before U21, and **never emits `resolved_rooted` or `refused`**.

This is deliberate, and three things follow that a reader must not have to infer:

1. **The daemon is not handed an empty mount table as a stand-in.** An empty
   table is a machine that BINDS NOTHING — a fact about the world, and the
   address plane rightly refuses a rooted address against it with
   `grey(unmounted)`. The daemon merely did not look. Turning *"I did not look"*
   into *"this machine binds nothing"* mints a claim it has no standing to make,
   and on any multi-root machine a false one.
2. **A client must not read a daemon-served `links` answer as authoritative
   about cross-root edges.** `mrd links` handles this by refusing to let the
   daemon answer at all when the page may carry a cross-root position: it
   degrades to the in-process engine on the lexical gate
   `addr::head_carries_root_separator`, so one address returns one answer
   whichever path served it. That gate is LEXICAL rather than parse-gated on
   purpose — a REFUSAL is also a changed answer, and a parse-gated gate would
   serve `Sessions:notes.md` warm as `unresolved` at exit 0 while in-process
   refuses it. One address, two answers, decided by which path served it, is the
   C-4 defect the address grammar exists to prevent.
3. **The successor is named, not deferred by forgetting.** End-to-end daemon
   root-awareness means N mounted corpora per workspace with per-root
   fingerprint invalidation, residency and reap — a designed subsystem. It is
   recorded as row C-1 in `docs/laws.md`.

## Why an amendment, not a new negotiated rev

Both keys are additive and `skip_serializing_if` empty. A v2 session's bytes are
therefore unchanged: the daemon serves v2 sessions and never populates either
key, and the in-process path that does populate them negotiates `contract:v3`.
No frame shape changes and no `hello` negotiation is forced, so this rides v2
exactly as the colors amendment does.

**A field-blind value sweep cannot see an added key** (All-Hands #1), so the
frozen-v2 guarantee above is pinned by a KEY-SET assertion in
`crates/wire/tests/contract_v2.rs` rather than by the worked-value sweep, which
would pass either way.

## The cross-root destination columns are ambient-only in the drawer

The cross-root destination columns are AMBIENT-ONLY in the published drawer, by
construction. `view::publish` is called from one production site — the resident
daemon (`registry.rs:597`) — which holds no mount authority at all
(`MountSet`/`RootedCorpus`: zero occurrences in the crate). An empty `dest_root`
in `view.duckdb` therefore means "this plane never asked", never "no cross-root
destination". The rooted answer lives on the in-process path
(`build_memory_rooted`, `mrd sql`). This is `docs/laws.md` C-1 seen from the
drawer; it is not a second residue, and it lifts when C-1 named successor lands.
