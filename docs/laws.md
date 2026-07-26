# The three laws

`meridian-rs` is split into crates so that its core invariants are dependency
edges, not conventions. Each law is enforced by what a crate is allowed to
depend on — breaking it is a compile error, not a review comment.

## Law 1 — the wire cannot leak inward

`model`'s public types carry no `serde` derives. The in-memory world model is
deliberately non-serializable, so no wire shape can reach into the tree and no
serialization concern can shape the model's types. Anything that must cross the
process boundary is converted explicitly at the projection seam (Law 3), never
by deriving `Serialize` on a model type.

## Law 2 — nothing Go-facing exists beyond `wire`

`wire` is serde-only and does zero I/O. It is the single crate that defines the
vocabulary the host sees: paths, spans, node revisions, roots, and the
op/request/response/error types. If a type is not in `wire`, it is not on the
wire. The frozen contract in `wire-contract-v2.md` is exactly this crate's
surface.

## Law 3 — the bridge has two named organs; everyone else is a consumer

(As re-attested 2026-07-24. The original wording — "exactly two components
depend on both `wire` and `model`" — stopped being true when the typed edge
was extracted; ten crates now carry both deps. The law's INTENT is unchanged:
the bridge stays auditable because bridge *behavior* lives in exactly two
named places.)

- **`wire-map` is the projection seam.** The model tree flattens into wire
  shapes as a tested library function — projection behavior lives here and
  nowhere else. M1 added the host-face read facts (`facts`: dewey ordinals,
  sanitized hpath addresses, Go-exact word counts) to this seam.
- **`wire-serve` is the serve choke-point.** The strict-decode pass, the read
  arms (incl. the composed `read`), the `splice → commit` write choke-point,
  and the v3 vocabulary projection are ONE implementation here. "One served
  implementation, two hosts": the per-workspace `sidecar` and the resident
  `registry` daemon both dispatch through these arms. (Aspirational note: the
  hosts share the LEAVES, not the dispatch shell — sidecar drives
  `arms::dispatch` per request over a fresh corpus; the daemon drives
  `dispatch_read` over a resident warm engine. The shells stay host-owned.)

Everything else that names both `wire` and `model` is a host, a client, or a
test member consuming those two organs — never a second place where bridge
behavior may live: `sidecar` and `registry` (the two hosts, wiring-only),
`mrd` (a local client), `render` (consumes projection facts to produce the
text face), `check`/`preset`/`realise` (engine planes over wire vocabulary),
`testsuite` (observes). Growth pressure inside a host or client is the signal
a capability is missing from one of the two organs.

A corollary edge: `syntax` is the only crate that touches the pulldown-cmark
fork, so fork churn is a one-crate event.

## Additivity

New capability arrives as new leaf crates or new match arms, never as a
reshuffle of what already ships. New ops are new `Op` variants and new dispatch
arms, discovered by the host through the `hello` handshake's capability list;
`policy` and `query` are additive consumers of the model's index. Nothing that
has shipped is ever split.

## Crate charters

Each crate's `lib.rs` states its charter, what it owns, what it never does, and
which laws it carries. In one line each:

| Crate | Charter |
|---|---|
| `syntax` | Markdown bytes → dialect node list with byte-exact spans; sole owner of the pulldown-cmark fork |
| `model` | The governed node tree, resolve, CAS-splice validation, Merkle roots — non-serializable by design (Law 1); the frozen Go-text heading predicate (`gotext`), the single address law its two dependents share; and the content-identity plane: the `fp1.…` CID-token, `verify_content`'s four-arm verdict (carrying the whole `version.codec.hashfn` triple on `Unverifiable`, so a render names WHICH member is unknown), and the ONE reason-carrying `Color` model every drift surface computes through |
| `fs` | Disk read/walk/watch into the model; atomic tmp+fsync+rename splice execution |
| `wire` | The serde-only wire vocabulary — the whole Go-visible surface (Law 2) |
| `wire-map` | The named model→wire projection seam, tested as a library function (Law 3) |
| `git` | The git plumbing organ: shell-out content-addressing (blob object ids, the eager `-w` write) and object reachability against a `Repo` handle. git owns content-addressing — this crate asks git and reports what git said, and NEVER computes or guesses an oid. A `std`-only leaf: no production dependency, so `git`-invocation churn is a one-crate event |
| `receipt` | The receipt family, at three planes: the persisted `^receipt` line renderer committed in the same batch as its edit (the shipped default template — facts normative, template replaceable); the append-only journal and its chain-continuity forgery detector; the origin-freshness anchor axis plus the three-state blob classification (`anchored` / `pending-anchor` / `never-anchored`); and the ephemeral in-memory read-mint ledger. Dependencies are `wire` only, by gate — so it CLASSIFIES facts it never gathers (the `git` crate does the I/O), and stage-3 unifies the ledger's representation with the persisted projection |
| `transport` | Untyped NDJSON envelope + codec seam; framing without meaning |
| `transport-proto` | Opt-in typed protobuf transport transcribing the wire contract |
| `policy` | Ruleset compile + assertion evaluation under budgets; edit-time verdicts; the blocking `gate()` at the armed change plane (`policy::authorize`) — see § Amendment |
| `query` | Corpus reads over the model's borrowed index; applies nothing |
| `wire-serve` | The shared typed edge (Law 3 choke-point): strict decode, read arms incl. the composed `read`, the `splice → commit` write choke-point, the v3 projection — one implementation, two hosts |
| `sidecar` | The per-workspace NDJSON host binary — wiring only; dispatches through `wire-serve` (Law 3) |
| `render` | The compiled-in render plane: `Renderer` trait + node-grain walker producing the `readText` text projection, with the block-elision and claim-link decoration hooks. Decorations arrive as DATA — a `decorations` map on the render header, keyed by the link addresses the body literally contains — so the caller resolves and this crate never grows a `render → lock → fingerprint` edge; an empty map is byte-identity with the undecorated render |
| `lock` | The `meridian-lock` fenced-block format: canonical writer/reader, engine sole-writer; owns the reserved `meridian-*` block-language namespace predicate |
| `effects` | The effect kernel: pure Starlark evaluation — rules in, effect descriptors out; zero I/O, advisory-only |
| `run` | The mrd-local run plane: plan/execute under the workspace run lock |
| `realise` | The realise engine: observe → check → apply per claim, on the run plane |
| `view` | The DuckDB view organ: a write-only leaf projecting the warm corpus into a disposable, fingerprint-stamped file. Also the lock-aware read face — it reads `meridian-lock` `pins:` as a third pin form beside the legacy `^inputs` forms, and renders each pin's color through `model::selector`'s ONE computer, so the walk listing and the SQL board cannot answer the same question two ways. It renders the color; it never computes a second one |
| `check` | The check engine: the pure READ verb of the reconciliation loop |
| `preset` | Presets + session birth: def-pinned convention floor; `unfold`/`new` materialize through the guarded create |
| `transcript` | Transcript cross-check: a corroborating (never authenticating) detector over the journal's actor claims |
| `config` | The `MERIDIAN.md` plane: the one entry point parsed as CONTENT — the two-rung bootstrap chain (`MERIDIAN_CONFIG`, then `$HOME/MERIDIAN.md`), the four resolution states with absent and zero-mount reaching ONE mount table, the strictest parse in the system (a closed `&'static str` reason set, 1-based FILE lines, a teaching refusal that states nothing loaded), and the config's own rev and fingerprint by the shipped laws — `blake3(bytes)[:16]`, no new rev noun minted. No partial mount table is a property of the TYPE: `Config` has private fields and `parse` is its only constructor. Downstream of `model`; it never binds. Binding is the next half and is not here yet — canonicalization at bind, the `workspace` deny ceiling, the equal-or-nested refusal, the declared-vs-bound check, and projecting the bound names into `addr::MountSet` so resolution stays `model`'s |
| `workspace` | Workspace identity: the discovery ladder, canonicalization, the deny ceiling — pure filesystem functions (a leaf, `std` + `cache` only) |
| `cache` | The hashed cache drawer: addressing, atomic sentinel registration, corrupt-is-a-miss probing, last-use GC |
| `registry` | The daemon-held workspace registry: unix-socket RPC server + client, first-writer-wins, atomic state, idle-reap |
| `mrd` | The workspace CLI — wires `workspace`/`cache`/`registry` into `init`/`unregister`/`resolve`/`cache`/`daemon`, and mounts the local run plane (`mrd run` via `crates/run`). A local CLIENT of the engine crates, never a resident organ and never on the serve path; its `run`→`model` edge stays a single reviewable dependency |
| `testsuite` | Integration tests + the frozen ground-truth pack as data |
| `perfsuite` | Perf harness and claims registry (out of default-members) |

## Amendment — the policy gate (armed change plane)

Law: ZT ruling #2; plan unit U4.2; wire-contract v2 refusal amendment
(`docs/wire-contract-v2-refusal-amendment.md`).

`crates/policy` originally owned advisory edit-time verdicts only — findings the
host could act on or ignore. This amendment extends the charter: `policy` now
also owns the **blocking gate** at the armed change plane.

- **The seam.** `gate(change, armed_set) → Ok(verdicts) | Refusal(violations)`
  fills `policy::authorize` and converts the advisory `evaluate_verdicts` seam
  to blocking — evaluated after CAS, before bytes land, in both writer paths.
  When a workspace is armed, a block-severity verdict or a door-law violation
  refuses the write; the refusal carries a `{code, recovery}` pair from the
  closed §8 taxonomy (the refusal-amendment table).
- **Trusted-path armed set.** `gate()` loads and verifies the attested INDEX
  from the workspace path inside the trusted write path; the caller-supplied
  ruleset parameter is removed from the gating decision. Absent INDEX on a
  never-armed workspace is a no-op bit-for-bit; a missing INDEX on an
  once-armed workspace fails CLOSED (`convention-fault`).
- **Additivity holds (Law § Additivity).** `policy` is still an additive
  consumer of the model's index; the gate is a new match arm at the write seam,
  not a reshuffle of what ships. `model`, `wire`, and the projection seam are
  untouched — the engine still never derives `Serialize` on a model type (Law 1),
  and the gate mints wire refusals only through `wire`'s error types (Law 2).

**ATTACK-034 scoping.** Refusal makes violations "unrepresentable through an
armed change plane" — never a stronger claim. The genesis epoch (pre-first-arming
writes) renders grey, never green. The gate governs only the armed change plane:
out-of-band mutation (an offline pre-push git rewrite, a root-preserving forged
journal row) is caught by the git witness plus the receipt-engine-only write
restriction, or it is a named residual — it is never rendered green by refusal.
