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

## Law 3 — only two places see both worlds

The Rust↔host bridge is auditable because exactly two components depend on both
`wire` and `model`: the `wire-map` crate (the named projection seam, where the
model tree is flattened into wire nodes as a tested library function) and the
`sidecar` binary (which wires transport to dispatch). Projection *behavior*
lives in `wire-map` and is tested there; the binary stays wiring-only. Nowhere
else may wire and model meet.

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
| `model` | The governed node tree, resolve, CAS-splice validation, Merkle roots — non-serializable by design (Law 1) |
| `fs` | Disk read/walk/watch into the model; atomic tmp+fsync+rename splice execution |
| `wire` | The serde-only wire vocabulary — the whole Go-visible surface (Law 2) |
| `wire-map` | The named model→wire projection seam, tested as a library function (Law 3) |
| `receipt` | Receipt-line rendering, committed in the same batch as its edit |
| `transport` | Untyped NDJSON envelope + codec seam; framing without meaning |
| `transport-proto` | Opt-in typed protobuf transport transcribing the wire contract |
| `policy` | Ruleset compile + assertion evaluation under budgets; edit-time verdicts |
| `query` | Corpus reads over the model's borrowed index; applies nothing |
| `sidecar` | The NDJSON binary — the one place wire and model meet (Law 3) |
| `workspace` | Workspace identity: the discovery ladder, canonicalization, the deny ceiling — pure filesystem functions (a leaf, `std` + `cache` only) |
| `cache` | The hashed cache drawer: addressing, atomic sentinel registration, corrupt-is-a-miss probing, last-use GC |
| `registry` | The daemon-held workspace registry: unix-socket RPC server + client, first-writer-wins, atomic state, idle-reap |
| `mrd` | The workspace CLI — wires `workspace`/`cache`/`registry` into `init`/`unregister`/`resolve`/`cache`/`daemon`; sees no `wire` or `model` |
| `testsuite` | Integration tests + the frozen ground-truth pack as data |
| `perfsuite` | Perf harness and claims registry (out of default-members) |
