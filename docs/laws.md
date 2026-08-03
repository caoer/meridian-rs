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
| `addr` | The agent-plane address: `[root:]path[#selector][@fp]` parsed into a fallible type carrying an optional canonical root name, plus the resolution-facing bound-name projection (`MountSet`) every plane resolves through. A `std`-only leaf UPSTREAM of `syntax` — it is where an address becomes a value, so nothing downstream re-splits a string. `Addr::parse` is the sole constructor and the path field carries no root prefix by construction; the colon law is root-wins with **no fallback to the literal reading**, because a fallback turns a typo'd root into a wrong success. Parse is not resolve: whether a named root is BOUND is the resolver's question and its answer is grey, never a parse error |
| `syntax` | Markdown bytes → dialect node list with byte-exact spans; sole owner of the pulldown-cmark fork |
| `model` | The governed node tree, resolve, CAS-splice validation, Merkle roots — non-serializable by design (Law 1); the frozen Go-text heading predicate (`gotext`), the single address law its two dependents share; and the content-identity plane: the `fp1.…` CID-token, `verify_content`'s four-arm verdict (carrying the whole `version.codec.hashfn` triple on `Unverifiable`, so a render names WHICH member is unknown), and the ONE reason-carrying `Color` model every drift surface computes through |
| `fs` | Disk read/walk/watch into the model; atomic tmp+fsync+rename splice execution |
| `wire` | The serde-only wire vocabulary — the whole Go-visible surface (Law 2) |
| `wire-map` | The named model→wire projection seam, tested as a library function (Law 3) |
| `git` | The git plumbing organ: shell-out content-addressing (blob object ids, the eager `-w` write) and object reachability against a `Repo` handle. git owns content-addressing — this crate asks git and reports what git said, and NEVER computes or guesses an oid. A `std`-only leaf: no production dependency, so `git`-invocation churn is a one-crate event |
| `receipt` | The receipt family, at three planes: the persisted `^receipt` line renderer committed in the same batch as its edit (the shipped default template — facts normative, template replaceable); the append-only journal and its chain-continuity forgery detector; the origin-freshness anchor axis plus the three-state blob classification (`anchored` / `pending-anchor` / `never-anchored`); and the ephemeral in-memory read-mint ledger. Dependencies are `wire` only, by gate — so it CLASSIFIES facts it never gathers (the `git` crate does the I/O), and stage-3 unifies the ledger's representation with the persisted projection |
| `transport` | Untyped NDJSON envelope + codec seam; framing without meaning |
| `policy` | Ruleset compile + assertion evaluation under budgets; edit-time verdicts; the blocking `gate()` at the armed change plane (`policy::authorize`) — see § Amendment |
| `query` | Corpus reads over the model's borrowed index; applies nothing |
| `wire-serve` | The shared typed edge (Law 3 choke-point): strict decode, read arms incl. the composed `read`, the `splice → commit` write choke-point, the v3 projection — one implementation, two hosts. **And the agent/stored SEAM for an address (U12):** `put` translates a cross-root `root:` address into its `obsidian://` stored form and `read` translates it back, at the CANDIDATE DOCUMENT and never at a verb (D9) — the transform rewrites the payload that introduced the address (a retained one is not this write's to move), and one door-facing artifact guard closes every byte-landing door in the crate. The positional law is `docs/address-grammar.md` §9: the wikilink target and the markdown link URL translate; lock `ref:`/`objects:` keep the canonical `root:` form by ratified law, and frontmatter is not an address position. It reads `config` for the mount table — LAZILY, only when a candidate can carry a cross-root position, because the stored plane is spelled in Obsidian VAULT names and the mount table is that axis's single authority |
| `sidecar` | The per-workspace NDJSON host binary — wiring only; dispatches through `wire-serve` (Law 3) |
| `render` | The compiled-in render plane: `Renderer` trait + node-grain walker producing the `readText` text projection, with the block-elision and claim-link decoration hooks. Decorations arrive as DATA — a `decorations` map on the render header, keyed by the link addresses the body literally contains — so the caller resolves and this crate never grows a `render → lock → fingerprint` edge; an empty map is byte-identity with the undecorated render |
| `lock` | The `meridian-lock` fenced-block format: canonical writer/reader, engine sole-writer; owns the reserved `meridian-*` block-language namespace predicate |
| `effects` | The effect kernel: pure Starlark evaluation — rules in, effect descriptors out; zero I/O, advisory-only |
| `run` | The mrd-local run plane: plan/execute under the workspace run lock. Owns `Authority`, the two-variant type the executor's choke point validates against — capabilities are real for starlark and do not exist for bash, structurally, see § Amendment |
| `realise` | The realise engine: observe → check → apply per claim, on the run plane |
| `view` | The DuckDB view organ: a write-only leaf projecting the warm corpus into a disposable, fingerprint-stamped file. Also the lock-aware read face — it reads `meridian-lock` `pins:` as a third pin form beside the legacy `^inputs` forms, and renders each pin's color through `model::selector`'s ONE computer, so the walk listing and the SQL board cannot answer the same question two ways. It renders the color; it never computes a second one |
| `check` | The check engine: the pure READ verb of the reconciliation loop |
| `preset` | Presets + session birth: def-pinned convention floor; `unfold`/`new` materialize through the guarded create |
| `transcript` | Transcript cross-check: a corroborating (never authenticating) detector over the journal's actor claims |
| `config` | The `MERIDIAN.md` plane: the one entry point parsed as CONTENT — the two-rung bootstrap chain (`MERIDIAN_CONFIG`, then `$HOME/MERIDIAN.md`), the four resolution states with absent and zero-mount reaching ONE mount table, the strictest parse in the system (a closed `&'static str` reason set, 1-based FILE lines, a teaching refusal that states nothing loaded), and the config's own rev and fingerprint by the shipped laws — `blake3(bytes)[:16]`, no new rev noun minted. No partial mount table is a property of the TYPE: `Config` has private fields and `parse` is its only constructor. Downstream of `model`. **And the mount table (`mount.rs`), which is where a declared entry BECOMES a bound root:** canonicalize at bind, then the `workspace::deny_reason` ceiling **reused whole and never re-implemented** — so a config cannot bypass the ceiling through a file that is itself ordinary editable content — then the three-way map's uniqueness invariants (name ↔ Obsidian vault name ↔ path, refusing equal-or-nested paths so one tree cannot be bound twice under two names), then each root's own self-declaration: **the root declares, `MERIDIAN.md` binds**, so a declared-vs-bound mismatch fails the whole parse and an absent declaration renders grey. Per-root state is S3-R6's closed vocabulary — one `bound`, four `grey(...)`, one `red(...)`, every non-bound state refusing on exit 1 with its own reason word. Mount-as-claim lives here too: a mount may pin the root it declares, verified through `model::fingerprint::verify_content` — no new codec, no new hash law. `MountTable`'s field is private and `bind` is its only constructor, so no partial mount table can exist to be observed. **And the bridge period (`bridge.rs`), which is the "checked against" half of the env-var inversion:** `CCC_LLM_WIKI_PATH` and `CCC_LLM_WIKI_REPOS_ROOT` become mount entries, and until they demote to overrides each is checked against the bound table **through `MountTable::by_path`** — the canonicalize-at-bind law reused whole, never a second comparison — so the symlinked, trailing-slash and real spellings of one tree are one lookup. When they disagree **the FILE WINS** (`Bridged::mount` is `Some` only on agreement, so a diverging variable names no root) and the divergence is **reported once per process, per variable, and never on an exit code** — fail-loud here would brick the CLI on every machine exporting the variable, and a bridge whose mismatch is fatal is not a bridge. An empty table is `unchecked`, not divergent: every unmigrated machine exports both variables, and a check that fires on all of them is deleted before it ever guards anything. **And that projection is now here (U12):** `MountTable::projection` yields the `addr::MountSet` the planes that resolve and translate consume — which names this machine BINDS, the **vault name** each bound vault root carries (the stored plane is spelled in vault names), and which declared names are unreachable here WITH the path to check, so a refusal for a declared-but-unreadable root never prescribes a declaration that already exists (S3-R50). It is not `mrd walk`'s projection and the difference is a FACT, not a second spelling: walk also marks a root unreachable when its CORPUS will not build, which only a caller holding corpora can know |
| `workspace` | Workspace identity: the discovery ladder (env override → git root → cwd default), canonicalization, the deny ceiling — pure filesystem functions (a leaf, `std` + `cache` only). The ladder answers ONE question — *which root does this path belong to* — and every answer names the rung that answered: `Answer::root` is `None` on the cwd default, so a caller cannot inherit an unanchored cwd silently (marker-retirement ruling, 2026-07-26). The two EXPLICIT planes are deliberately NOT rungs here: the mount table (`config::MountTable`) cannot be one without a dependency cycle, since `config` depends on this crate for the ceiling; and a declared root arrives on the serve path as the hello `workspace` field, pinned exactly by `registry::Registry::pin_declared`, because a daemon has no meaningful cwd to walk. All three planes meet at exactly one point: `deny_reason`, reused whole, never re-implemented |
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

## Amendment — capabilities do not apply to bash

Law: ZT ruling, made verbally, re-litigated in code, and ruled again live
2026-08-01. **Gate: `crates/mrd/tests/law_no_caps_on_bash.rs`** — that file is
what makes this hold, and this section is what it enforces. A reader who
proposes "just a small cap check on bash" must answer both.

> **Capabilities do not apply to `bash` tasks. Not now, not later, not in a
> weaker form.**
>
> 1. A bash task carries **no `caps:` line**, no cap resolution, no cap source,
>    and no `deny-default`.
> 2. The engine **never prints a claim about what a bash task may do** — most of
>    all not `(read-only)`.
> 3. Bash is **unsandboxed by definition.** The only honest description on any
>    surface is: *unsandboxed shell, undeclared effects.*
> 4. Capabilities remain a real, enforceable contract for **starlark**, and only
>    starlark.

**The guarantee is impossible, not merely difficult.** A capability claim is a
promise about what a process CANNOT do, and for bash the engine holds no
mechanism that makes one:

| layer | what exists | why it does not bound the process |
|---|---|---|
| in-window writes | the `out-of-band delta` detector | **detects, never prevents** — `docs/run-plane.md` scopes it so, and the offending file persists |
| after the window | nothing | the detector's own wording is *"during exec window"*; a `nohup`, launchd plist, cron line or daemon writes with no observer |
| outside the corpus | nothing | cwd isolation and env scrubbing do not restrict network, credentials, SSH, or `rm -rf` — none of it is an "effect" |

A guard escapable by `nohup` is not a guard, so **no honest value exists for a
bash `caps:` field — including `none`, including `(read-only)`.** What the
resolution ladder bought on that path was complexity with no guarantee behind
it, plus misleading-by-adjacency: `caps:` is TRUE on the starlark row above and
importing its enforcement model onto the row below asserted, in the engine's own
voice, a conclusion the engine cannot support.

**Structural, not cosmetic.** `run::caps::Authority` has two variants —
`Capabilities(CapResolution)` and `Unsandboxed` — and it is what the executor's
choke point validates against. The bash dispatcher holds no capability field at
all (`dispatch_bash::BASH_AUTHORITY`), so it cannot name, narrow, or
half-enforce one; `resolve_authority` is the only language-aware entry and does
not even READ a bash task's `task.<name>.caps` declaration, because validating a
value that governs nothing teaches that it might. Deleting the printing while
leaving resolution running underneath fails the gate's second half by
construction.

**What this does NOT weaken.** Starlark keeps the whole contract: hermetic
evaluator, closed builtin surface, no `exec`/`os`/`subprocess`, every effect a
descriptor the applier gates. Both refusal shapes are asserted in the same gate
file, so the bash half cannot be bought by weakening the starlark half:
`md.*` without its cap → `capability denied`, exit 1; `proto.*` without its cap
→ `state: unexecuted-no-capability`, exit 0. The `check-*` / `verify-*` bash
fence refusal also survives — that is a NAME law, not a capability.

**Behaviour changed, not just wording.** Bash reaches the tree through the
effect-shim fd and those descriptors were gated at the choke point, so an
undeclared bash block used to exit 1 with `capability denied` and now applies.
That gate never bounded the block: a denied block writes with `sed -i` instead,
where the bracket at most detects the change and never rolls it back. It only
pushed the write off the attested path.
