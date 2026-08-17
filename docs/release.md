---
type: contract
id: release
status: standing
updated: 2026-08-17
description: What a meridian-rs release promises, and the stamp + tag mechanics that make the promise cuttable.
owns: [what a release promises, stamp and tag mechanics, what a tag publishes]
---

# Release definition

> **Standing.** What a release PROMISES, and the mechanics that stamp it.
> **Docs-first:** this file promises nothing that another standing doc does not
> already make law — every promise row below cites where its law lives.
> Design law is `wire-contract.md`; architecture law is `laws.md`; process is
> `README.md`.

## §1 A release is a two-key point

A release is a named point on this repo's history at which every surface in §2
and §3 holds **both** keys:

| Key | Held when |
|---|---|
| **doc key** | the surface is law in a standing `docs/` file — the cited section |
| **code key** | the built artifact SERVES it — the op dispatches, the verb runs |

A surface holding only the doc key is **designed, not promised** (§4.2). A
behavior holding only the code key is **not law and not promised** — an
undocumented behavior a caller relies on is that caller's risk, and this file
never converts one into a promise by shipping it.

The two-key rule is what keeps `README.md`'s "doc correct > code correct" from
being misread at release time. That law rules **which document wins when design
and code disagree**. It never says a caller may rely on a design the artifact
does not serve. Design leads; the promise is the intersection.

## §2 The wire promise — the caps set IS the promise surface

The wire promise has a machine-readable spelling already: the `caps` array in
the `hello` response (`wire-contract.md` §3.2). `caps` is the complete set — an
op is in it or answers `unknown_op`, never both — and there is **no version
sniffing, ever**, so a caller reads capability from `caps` and never from the
`server` string.

`caps` is a **set**; its serialization order is not promised.

A release promises: for every cap below, the behavior the cited section
specifies, at `hello` with `contract:"v3"`.

### §2.1 Read and discovery

| Cap | Law | A caller may rely on |
|---|---|---|
| `toc` | §4.1 | the complete write kit per file — `hpath` + `node_rev` per section, anchors with revs, frontmatter keys, header `fingerprint` |
| `cat` | §4.2 | full-span bytes, heading-inclusive; the rev is blake3 of exactly those bytes |
| `extract` | §4.3 | full node objects, the 11-variant kind enum, total node order; an unknown `kinds` value refuses loud |
| `read` | § A.3 | addressing + content + render + `props` + `anchors` + `unresolved` at ONE engine snapshot |
| `resolve` | §4.5 | best-effort app-compatible two-stage walk; location facts only |
| `resolve.content` | §4.5 | the fragment bytes alongside those facts — still no rev |
| `links` | §4.6 | per-edge resolved/unresolved counts, one call per file or corpus-wide |
| `links.require_fingerprint` | §10.2 | the opt-in `stale_view` refusal instead of an answer in an unnamed tense |
| `mounts` | § A.5 | the live root registry, re-derived per call against `~/MERIDIAN.md`'s hash |
| `hello.identity` | § A.3 | `{build: sha\|unknown}` — read, never invented |

### §2.2 Write

| Cap | Law | A caller may rely on |
|---|---|---|
| `splice` | §4.4 | the ONLY write op; batch-only; one response shape; atomic through one reparse |
| `splice.if_node_rev` | §5.1 | node-grain CAS re-derived at execution from the pre-batch state |
| `splice.if_fingerprint` | §5.1, §5.4 | world-grain CAS when bare (the v2 root premise, checked FIRST); with `scope` under `scoped-guards` it is the one-premise sugar at that node |
| `scoped-guards` | §5.4–§5.7, §4.7 | the whole scoped-premise family: `guards[]`, sugar `scope` on splice/script, mint arm `fingerprint {scope}` / `{scope_bytes}`; a frozen v2 session is never pushed it |
| `splice.dry` | §4.4 | everything except disk — same response shape, `fingerprint_after:null`, no receipt |
| `splice.receipt` | §6.1 | the receipt entry committed in the SAME batch as the content edit |
| `splice.verdicts` | §11.1 | the field is always present with the §11.1 row shape (see §4.2 on packs) |
| `splice.plan_edits` | § A.3 | plan-level batch shapes addressed by segment arrays |
| `splice.pin` | § A.3 | the pin riding the write choke-point |
| `splice.create_rev` | § A.3 | the parent-section `rev` slot at the create door, and the `guard_required` demand on an `n`-bearing `parent_hpath` |
| `create` | § A.3 | file birth through the guarded door |
| `check_write` | § A.3 | the splice verdict standalone, read-only, computed without writing |

### §2.3 Integrity and change

| Cap | Law | A caller may rely on |
|---|---|---|
| `fingerprint` | §4.7 | the workspace content hash plus `seq`; under `scoped-guards`, optional `scope` / `scope_bytes` mints that node's token and echoes the pair (`absent` is a value) |
| `diff` | §4.7, §7.3 | replay ≡ live — the byte-identical Delta objects the live stream carried |
| `sub` | §4.7 | ack-then-push at the daemon door, one Notification frame per Delta batch |

### §2.4 Wire-wide promises (not per-cap)

| Promise | Law |
|---|---|
| One wire door — the daemon's unix socket; NDJSON line dialogue | §3.1, §3.3 |
| The `id` **echo** for conforming ids — a JSON integer lexeme in `[0, 2^53)` comes back unchanged. **A non-conforming lexeme is nulled and the request is still SERVED**: no refusal, no `id_raw`. §3.1 requires the refusal and `id_raw`, the law STANDS, and the artifact does not yet serve it — declared, measured, at `wire-contract.md` §18 row 9 | §3.1; §18 row 9 |
| Strict server / tolerant client; unknown request fields reject loudly | §3.2 |
| `node_rev` is MUST on every `toc`/`cat`/`extract` node while `splice ∈ caps` | §3.2 |
| Every error carries `code` + `recovery` from the CLOSED six-class enum | §8 |
| A content-mutating wire write demands fingerprint or `force` (`guard_required`) | § A.1 |
| The armed plane refuses on block-severity verdicts; never-armed stays advisory | § A.2, `armed-plane.md` |

## §3 The promise beyond the wire

| Promise | Law |
|---|---|
| **Machine address is segments only** — `{"hpath":[{"h":"Goals"},{"h":"Q3"}]}`, optional `n`, or `anchor` / `fm_key`. Never a joined writeable form | `wire-contract.md` §2.1; `README.md` standing A |
| **One block-id charset**, `[A-Za-z0-9-]+`, on both planes | §2.4 |
| **One hash family** — BLAKE3-256 for rev, `file_rev`, leaf, interior, fingerprint | §1; `node-rev-merkle-spec.md` |
| **`node_rev` is 16 lowercase hex over the node's full span bytes**; `fingerprint` is `b3:` + 64 hex, never truncated | §1 |
| **The hash domain is md-only** with the dot-segment default ignore and `meridian/domain.md` as the only standing custom-ignore surface; a domain-rule change bumps the prefix | §12 |
| **Span law** — sections newline-inclusive to the next boundary, leaf blocks exclude the final terminator, a span splitting a multi-byte character refuses | §1 |
| **The frontmatter scalar law** — decode on every read seam, canonical encode at every value-plane write door, and the def plane reads absent/null/empty-string as empty | § A.6; § A.6.5 (**RATIFIED** by ZT, 2026-08-08, relayed via `2c47b75e`) |
| **The cross-root address grammar** and the mount states' closed vocabulary | `address-grammar.md`; `meridian-md-schema.md` |
| **The CLI exit triad** — 0 done, 1 the engine refusing, 2 the CLI's own refusal, across the engine-backed verbs | `status.md` § Workspace CLI |
| **`mrd help` is the authoritative CLI surface** — flags, refusal legs, per-verb exit codes | `status.md` § Workspace CLI |

## §4 What a release does NOT promise

### §4.1 Named limits — promised to be TRUE, never promised away

These are the honest limits the contract already registers. A release promises
that they hold and are surfaced, not that they are absent.

| Not promised | Law |
|---|---|
| Resistance to an adversary — 16-hex revs are trusted-local, ≈2^32 birthday work | §13.1 |
| Any staleness lag bound, ever | §10.1, §13.2 |
| `seq` catchup across a daemon epoch — a restart resets it; cross-epoch catchup is diff-by-root | §7.1 |
| History beyond the 256-deep root ring — older ranges answer `fingerprint_unknown` | §7.3, §13.5 |
| Multi-file atomicity across a crash — content-without-receipt is possible, loud via lint | §6.5, §13.6 |
| A receipt carrying the root it produces — structurally impossible | §6.2, §13.7 |
| Two-way Obsidian parity — the compatibility floor is ONE-WAY; out-of-grammar input refuses loudly | §0.1 GOAL 2, §4.5 |
| An answer to a request whose frame never arrived — transport loss is not a recovery class; re-read before retry, never `force` | §8.1 |

### §4.2 Doc key only — designed, not promised

| Surface | Standing law | Why it is not promised |
|---|---|---|
| **Rule packs** — pack loading, budgets, fixtures-as-load-gate, Starlark predicates | §11.1–§11.4 | the daemon loads no pack, so `verdicts` serves `[]` by construction. The FIELD and its row shape are promised (§2.2); admitting a pack over the wire is not |
| Any future key-grain Delta (`keys:[…]`) | §7.4 | named as a future-only additive amendment path; no slot ships |

### §4.3 Deliberately outside the promise

| Surface | Where ruled |
|---|---|
| SQL / DuckDB / a view organ as agent core — `mrd sql` is an operator face over an ephemeral `:memory:` build | §10.3–§10.4 (**RULED — DROP**, ZT, 2026-08-06, session `06-05-meridian-mcp-leg-2`); `README.md` standing C |
| A second wire door — the stdio sidecar is deleted | §3.3 (**RULED — DROP**, ZT, 2026-08-06, session `06-00-adhoc`) |
| Orientation surfaces as wire ops (dashboards, counts, trees) | §10.3, §16 |
| In-process `mrd` paths as a wire surface — a CLI is not a wire door | §3.3, § A.1 |
| Rust crate APIs — nothing is published; no crate carries a semver promise (§5.1) | `Cargo.toml`; `laws.md` § Additivity governs shape, not API stability |

### §4.4 The v2 dialect — served, frozen, and NOT ruled by this file

The engine negotiates a contract rev per session: `hello.contract:"v3"` selects
the standing vocabulary, and an **absent or `"v2"` value selects the frozen v2
dialect** (`crates/wire-serve/src/rev.rs`). A release promises the v2 dialect
stays **byte-identical** — that is the frozen-caps law the v3 projection is
built to preserve — and promises nothing about how long it is served.

**v2 retirement is an open fork.** This file does not rule it, and no reader may
take "served and frozen" as a promise of permanence or as a schedule for
removal. `README.md` § A.4's "no dual wire constitutions for agents" is a
TEACHING law — agents learn one constitution — never a claim that the frozen
dialect is unserved.

### §4.5 Multi-root addressing is NOT promised

The `roots` surface advertises every bound root, while `read` serves one. That
disagreement is a single defect family — the stage-2 reserved-prefix face
shadows the registered-root lane — and this release does **not** promise
cross-root addressing through it.

The truthful shape of the statement matters: `multi-root` and `roots` appear in
**no promise row** of §2 or §3, so naming the non-promise here retreats from
nothing. It records what a reader must not infer from the advertisement.

**The defect is the advertisement, not the refusal.** A refusal on a root the
face never bound is correct behavior; advertising five bound roots while `read`
serves one is what misleads. Consequence, stated rather than left to be
discovered: the taught recovery line for that refusal is **known-inexecutable**
pending multi-root — a caller following it cannot succeed today.

Executable recovery on this path is a stated **v1.x direction**, not a v1
promise.

### §4.6 Refusal CODES are promised; teaching lines are best-effort

§2.4 promises that every error carries `code` + `recovery` from the closed
six-class enum. This release promises exactly that and no more: the classified
refusal, not the sentence that teaches a way out.

The distinction was tested at the write door and the promise held.
`crates/wire-serve/src/write.rs` refuses through `bad_request(...)`, and
`bad_request` (`crates/wire-serve/src/lib.rs`) sets `ErrorCode::BadRequest` with
`Recovery::Fix` — asserted by its own unit test
`bad_request_carries_the_fix_class_and_message`. Both promised fields are
present. What is absent on that leg is a **teaching clause**, and no promise row
in §2 or §3 requires one. Teaching lines are best-effort, and their absence is
not a broken promise.

Two facts recorded beside it, neither narrowing a promise:

- `replace_section` is the refusal taxonomy's **known unreached path** in this
  tree — classified in law, not exercised by a landed case.
- An `append` at the same address with the same rev landed correctly, so the
  address layer underneath is sound. The gap is in the taxonomy's coverage, not
  in addressing.

### §4.7 Recorded at cut time — limits named, promises unchanged

These were found at the v1 cut and are written down rather than repaired. None
narrows a promise row; recording them is what keeps §1's two-key rule honest.

| Recorded | What it says |
|---|---|
| **G10** | At production corpus scale the 7 s script wall binds **before** the 64-read ceiling — the read-count limit is not the operative one. Reads with §7 |
| **D4** | The surface enumerates **paths**, not mount names |
| **D6** | Structurally unreachable in this deployment shape — both admitted roots are `kind: vault`, so the other branch has no way to arise here |
| **D5** | **SKIPPED with reason.** Staging an unmounted root touches `~/MERIDIAN.md` — ZT's own floor — while he is dogfooding on it. Ruled post-cut, in an advisor-coordinated window outside his active hours |

## §5 Stamp mechanics

### §5.1 The version string

The workspace stamps one version at `[workspace.package]` in the root
`Cargo.toml`; every crate inherits it (`version.workspace = true`). It reaches
two reader-visible surfaces:

| Surface | Today | Note |
|---|---|---|
| `mrd --version` | `mrd {CARGO_PKG_VERSION} (git {MRD_BUILD_SHA})` | the sha is read at compile time, never invented |
| `hello.identity.build` | the build sha, or `unknown` | § A.3; sha only — the version does not ride here |

#### The sha token states the TREE, not only the commit *(2026-08-09)*

`MRD_BUILD_SHA` is a sha with an optional marker, not a bare sha. It reads
`<sha>` where the build's worktree matched HEAD, `<sha>-dirty` where tracked
content diverged from it, and `unknown` where no attributable identity could be
read at all. The marker rides the sha token — git-describe's convention — so
`hello.identity.build` carries it with no schema change and no third identity
field.

Two baked facts distinguish three build states, and the third is DERIVED:

| Build state | `--version` says | What tells the reader |
|---|---|---|
| built clean at `X` | `(git X)` | a bare sha |
| built from a dirty tree based at `X` | `(git X-dirty)` | the marker |
| built at `X`, tree has since moved to `Y` | `(git X)` while HEAD is `Y` | the reader COMPARES — a binary cannot observe commits made after it |

**The stamp proves identity; identity plus comparison proves provenance.** A
binary can state what it was built from and whether that was a whole commit. It
can never state what the tree did afterwards, which is why a gate compares the
string against a declared sha rather than trusting it alone.

**A clean build's string is unchanged, byte for byte.** Only a dirty build says
anything new, so the marker adds a refusal where there was a silent pass and
moves nothing else.

**Mechanism and its cost, both stated because a flag without its mechanism is
not a design.** The answer is a function of the working tree, so the build
script re-runs on EVERY build — it names a sentinel path that never exists,
which is cargo's way of saying always. A stamp that outlives its tree is the
defect this closes, and a watch list keyed on HEAD cannot see an uncommitted
edit. Measured 2026-08-09 at `440245b3`, five runs each: `git rev-parse HEAD`
20 ms, `git status --porcelain --untracked-files=no --no-optional-locks` 20 ms —
per cargo invocation, plus one relink of `mrd` at each clean↔dirty transition,
where the binary's identity genuinely did change. `--no-optional-locks` keeps
the probe from writing the index, so a build can never block a concurrent git in
the same tree. Untracked files are excluded deliberately: the question is
whether this build is attributable to HEAD, and an untracked file that reaches
the compiler does so through a tracked `mod` line.

A probe that cannot be read publishes `unknown`, never clean. **An unverifiable
clean claim is never published** — that is the whole rule in one sentence.
`MRD_BUILD_SHA` supplied in the environment rides verbatim with no probe: the
supplier owns the claim, and supplying it to make a gate agree invents the
answer the pin exists to give.

**The release version and the contract rev are DIFFERENT AXES and neither
renames the other.** A release numbered 1 ships contract rev v3. Renaming the
contract rev to match a release number would break every client that negotiates
`contract:"v3"` — it is not a tidying, it is a wire break, and it is refused.

**The `server` string is DERIVED, and that closed a drift class** *(done at the
v1 stamp, 2026-08-09)*. It used to be a hardcoded `meridian-daemon/0.1`
(`crates/registry/src/server.rs`) independent of the workspace version, so a
release numbered 1 would have announced `0.1` to its own customer. §3.2 makes
the string informational — no version sniffing, ever — so nothing broke while it
disagreed, and nothing breaks now that it agrees: it was a reader-facing drift,
never a contract one.

It is now `concat!("meridian-daemon/", env!("CARGO_PKG_VERSION"))`, so the
string cannot drift from the stamp again. The repair rode WITH the version bump
because it is a code change on a downstream-visible string — it belonged to
whoever ruled the stamp, never to a doc edit alone. It was safe to make in the
same change because the value's only readers were checked first: in
`ccc-statusd`, this engine's customer, `meridian-daemon/0.1` appears **only in
test fixtures** (the `registryclient` `client_test` / `lifecycle_test` /
`pool_test` fake responses) — no production code parses the field's value.

A third reader-visible surface therefore joins the two above:

| Surface | Today |
|---|---|
| `hello.server` | `meridian-daemon/{CARGO_PKG_VERSION}` — informational, never sniffed (§3.2) |

### §5.2 The tag

| Element | Shape |
|---|---|
| Name | `v<MAJOR>.<MINOR>.<PATCH>` on the whole workspace |
| Kind | annotated (`git tag -a`), never lightweight — the message is the release notes carrier (§5.3) |
| Points at | the commit whose tree satisfies both keys of §1 |

Existing tags are component- or snapshot-scoped (`sidecar-v0.1.0`,
`stage2-pin`, a dated backup) and set no workspace-release precedent. The bare
`v` prefix is the one this file establishes, so a workspace release is
distinguishable from a component tag at a glance.

### §5.3 Release notes

The **annotated tag message** carries the release notes. `docs/` stays the
standing law and grows no per-release pile: `README.md` § 5 keeps history
optional and deletable, and `wire-contract.md` § B forbids reintroducing
versioned contract files or amendment piles. A CHANGELOG file would be exactly
that pile under another name.

Notes state what the release promises that the previous one did not, in the
vocabulary of §2 and §3 — caps added, law amended — and cite sections, never
prose claims.

### §5.4 What the tag publishes

A tag builds the engine for both served platforms and publishes each binary to
Forgejo's **generic package registry**, keyed by the **commit** the tag points
at (§5.2) — never by the tag name, and there is **no `latest`**.

| Element | Shape |
|---|---|
| Base | `https://git.0xdao.app/api/packages/caoer115/generic/mrd/<COMMIT>` |
| Files | `mrd-linux-amd64`, `mrd-darwin-arm64`, and a `.sha256` beside each |
| The pin a consumer records | `(COMMIT, SHA256)` |
| Re-publish of the same commit | HTTP **409**; the FIRST published bytes stay authoritative |

**The tag NAMES the point; the commit KEYS the bytes.** A tag is a movable ref
and a name is not a hash, so nothing a consumer pins may derive from it: a
consumer resolves the tag to its commit once (`git rev-list -n 1 <tag>`) and
records the pair above. That is the same pin the main-push publish already
hands out, so a tag adds a platform — it does not add a second pin vocabulary.

**Append-only is the property, not an accident of the store.** Rust builds here
are not bit-reproducible, so a rebuild of the same commit differs byte-wise; the
409 is what keeps that rebuild from silently invalidating a digest a consumer
already recorded. The lanes exploit it directly: each **asks the registry first**
and builds only when the artifact for this commit is absent, so a tag on a
commit main already published is a fast no-op that re-prints the pin.

| Artifact | Agent | Backend |
|---|---|---|
| `mrd-linux-amd64` | `workstation-nyc-2` | docker, the `Dockerfile.ci` image |
| `mrd-darwin-arm64` | `zmax` | local — steps run on the host, against its own toolchain |

A mac artifact needs a mac: no container on any Linux agent can produce one,
which is why the darwin lane runs on a workstation agent with the local backend
and why its `image:` names a **shell**, not an image.

**No Forgejo release object is created, deliberately.** Release attachments are
mutable and the registry is not; a release page would be a second home for the
pin that can drift from the bytes. The tag's annotated message stays the release
notes carrier (§5.3), and the registry stays the only place bytes live.

Each lane refuses to publish a binary that cannot name the tree it came from —
the §5.1 stamp is checked against the commit being built before any upload.

## §6 How the promise changes after a release

- **Additive** — a new cap, a new response field. Old callers ignore it under
  the tolerant-client law (§3.2). No promise breaks; the notes name it.
- **Amending** — a standing doc section changes. Docs-first: the section changes
  BEFORE the code, and the release that ships it names the section.
- **Removing** — a cap leaves the set. This BREAKS callers and is a ruling, not
  a refactor. It carries its own decision record, exactly as §3.3 and §10.4 do.

A surface that neither doc nor `caps` names is not a promise, and no release
note may create one.

## §7 What the 7 s script wall bounds

The script entry carries one wall-clock budget — `WALL_CLOCK = 7 s`
(`crates/mrd/src/script/cmd.rs`). **It is an ENGINE budget, never the
operator's process wall**, and a reader who confuses the two will infer headroom
that does not exist and pressure that is not there.

The budget binds at **three layers inside the engine process**, each named in
the constant's own doc comment and in `run-plane.md` § Where the budgets bind:

| Layer | Where it binds |
|---|---|
| ask | before every round trip (`WireHost::ask`) |
| connect | on the socket itself (`SocketDoor::connect`) |
| run | before the commit is issued (`run`) |

**Startup and teardown sit outside all three.** The MCP host's bound on the
child process is a **fourth** layer, and it lives in the other repo — it is not
this budget and this file does not rule it.

Consequence for reading measurements: **the decisive cell in the fuse report is
ENGINE ms.** Two of three write-bearing runs crossed 7 s of PROCESS wall while
committing correctly, precisely because the process wall is not what the budget
bounds. No door headroom may be inferred from that cell in either direction.

**G10 rides here** (§4.7): at production corpus scale this wall binds before the
64-read ceiling, so a program's operative limit is time, not read count.
