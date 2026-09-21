---
type: contract
id: release
status: standing
description: What a meridian-rs release promises, and the stamp + tag mechanics that make the promise cuttable.
owns: [what a release promises, stamp and tag mechanics, what a tag publishes]
---

# Release definition

> Standing law: `README.md` (process), `wire-contract.md` (design), `laws.md` (architecture). Docs-first: every promise row cites the standing doc that rules it.

This file says what a meridian-rs release promises a caller, and how the release
point is stamped and tagged. It makes no law of its own: the wire design, the
crate architecture and the process are ruled by the docs named above.

## §1 A release is a two-key point

A release is a named point on this repo's history at which every surface in §2 and §3 holds **both** keys:

| Key | Held when |
|---|---|
| **doc key** | the surface is law in a standing `docs/` file — the cited section |
| **code key** | the built artifact serves it — the op dispatches, the verb runs |

A surface that holds the doc key alone is **designed, not promised** (§4.2). A
behavior that holds the code key alone is **not law and not promised**: a caller
who relies on it carries that risk, and shipping it never makes it a promise.

"Doc correct > code correct" (`README.md`) rules which document wins when design
and code disagree. Design leads; the promise is the intersection of the two
keys.

## §2 The wire promise

The promise surface is `caps` in the `hello` response (`wire-contract.md`
§3.2) — the complete set of what the built artifact serves. An op is in `caps`
or answers `unknown_op`, never both. Capability comes from `caps`, never the
`server` string — no version sniffing, ever. `caps` is a **set** with no
promised order. Each cap holds as its cited section specifies, at `hello` with
`contract:"v3"` (the contract rev that selects the standing vocabulary; §4.4).

### §2.1 Read and discovery

| Cap | Law | A caller may rely on |
|---|---|---|
| `toc` | §4.1 | the complete write kit per file: `hpath` + `node_rev` per section, anchors with revs, frontmatter keys, header `fingerprint` |
| `cat` | §4.2 | full-span bytes, heading-inclusive; rev is blake3 of those bytes |
| `extract` | §4.3 | full node objects, 11-variant kind enum, total node order; unknown `kinds` refuses |
| `read` | § A.3 | addressing + content + render + `props` + `anchors` + `unresolved`, one engine snapshot |
| `resolve` | §4.5 | best-effort app-compatible two-stage walk; location facts only |
| `resolve.content` | §4.5 | those facts plus the fragment bytes; still no rev |
| `links` | §4.6 | per-edge resolved/unresolved counts, per file or corpus-wide |
| `links.require_fingerprint` | §10.2 | opt-in `stale_view` refusal instead of an answer that does not say which view it came from |
| `mounts` | § A.5 | live root registry, re-derived per call against `~/MERIDIAN.md`'s hash |
| `hello.identity` | § A.3 | `{build: sha\|unknown}` — read, never invented |

### §2.2 Write

| Cap | Law | A caller may rely on |
|---|---|---|
| `splice` | §4.4 | only write op; batch-only; one response shape; atomic through one reparse |
| `splice.if_node_rev` | §5.1 | node-grain CAS, re-derived at execution from pre-batch state |
| `splice.if_fingerprint` | §5.1, §5.4 | bare: world-grain CAS (the v2 root premise, checked first). With `scope` under `scoped-guards`: one-premise sugar at that node |
| `scoped-guards` | §5.4–§5.7, §4.7 | scoped-premise family: `guards[]`, sugar `scope` on splice/script, mint arm `fingerprint {scope}` / `{scope_bytes}`; never pushed to a frozen v2 session |
| `splice.dry` | §4.4 | all but disk: same response shape, `fingerprint_after:null`, no receipt |
| `splice.receipt` | §6.1 | receipt entry committed in the same batch as the content edit |
| `splice.verdicts` | §11.1 | always present, in the §11.1 row shape (§4.2 on packs) |
| `splice.plan_edits` | § A.3 | plan-level batch shapes addressed by segment arrays |
| `splice.pin` | § A.3 | pin riding the write choke-point |
| `splice.create_rev` | § A.3 | parent-section `rev` slot at the create door; `guard_required` on an `n`-bearing `parent_hpath` |
| `create` | § A.3 | file birth through the guarded door |
| `check_write` | § A.3 | splice verdict alone, read-only, no write |

### §2.3 Integrity and change

| Cap | Law | A caller may rely on |
|---|---|---|
| `fingerprint` | §4.7 | workspace content hash plus `seq`; under `scoped-guards`, optional `scope` / `scope_bytes` mints that node's token and echoes the pair (`absent` is a value) |
| `diff` | §4.7, §7.3 | replay ≡ live: byte-identical Delta objects the live stream carried |
| `sub` | §4.7 | ack-then-push at the daemon door, one Notification frame per Delta batch |

### §2.4 Wire-wide promises

| Promise | Law |
|---|---|
| One wire door — the daemon's unix socket; NDJSON line dialogue | §3.1, §3.3 |
| The `id` **echo**: a JSON integer lexeme in `[0, 2^53)` returns unchanged. **A non-conforming lexeme is nulled and the request is still served** — no refusal, no `id_raw`, though §3.1 requires both: the law stands, and the artifact does not yet serve it | §3.1; `wire-contract.md` §18 row 9 |
| Strict server / tolerant client; unknown request fields are refused | §3.2 |
| `node_rev` is MUST on every `toc`/`cat`/`extract` node while `splice ∈ caps` | §3.2 |
| Every error carries `code` + `recovery` from the closed six-class enum | §8 |
| A content-mutating wire write demands fingerprint or `force` (`guard_required`) | § A.1 |
| The armed plane refuses on block-severity verdicts; never-armed stays advisory | § A.2, `armed-plane.md` |

## §3 The promise beyond the wire

These rows promise behavior that is not a wire op: how an address is spelled,
how bytes are hashed, and what the `mrd` CLI guarantees. Each row cites where
its law lives.

| Promise | Law |
|---|---|
| **Machine address is segments only** — `{"hpath":[{"h":"Goals"},{"h":"Q3"}]}`, optional `n`, `anchor` or `fm_key`; never a joined writeable form | `wire-contract.md` §2.1; `README.md` standing A |
| **One block-id charset**, `[A-Za-z0-9-]+`, on both planes | §2.4 |
| **One hash family** — BLAKE3-256 for rev, `file_rev`, leaf, interior, fingerprint | §1; `node-rev-merkle-spec.md` |
| **`node_rev` is 16 lowercase hex over the node's full span bytes**; `fingerprint` is `b3:` + 64 hex, never truncated | §1 |
| **The hash domain is md-only**, dot-segment default ignore, `meridian/domain.md` the only standing custom-ignore surface; a domain-rule change bumps the prefix | §12 |
| **Span law** — sections newline-inclusive to the next boundary, leaf blocks exclude the final terminator, a span splitting a multi-byte character refuses | §1 |
| **The frontmatter scalar law** — decode on every read seam, canonical encode at every value-plane write door; the def plane reads absent/null/empty-string as empty | § A.6; § A.6.5 |
| **The cross-root address grammar** and the mount states' closed vocabulary | `address-grammar.md`; `meridian-md-schema.md` |
| **The CLI exit triad** — 0 done, 1 engine refusal, 2 CLI refusal, across engine-backed verbs | `status.md` § Workspace CLI |
| **`mrd help` is the authoritative CLI surface** — flags, refusal legs, per-verb exit codes | `status.md` § Workspace CLI |

## §4 What a release does not promise

### §4.1 Named limits

A release promises these limits hold and are surfaced, not that they are absent.

| Not promised | Law |
|---|---|
| Resistance to an adversary — 16-hex revs are trusted-local, ≈2^32 birthday work | §13.1 |
| Any staleness lag bound, ever | §10.1, §13.2 |
| `seq` catchup across a daemon epoch — a restart resets it; cross-epoch catchup is diff-by-root | §7.1 |
| History beyond the 256-deep root ring — older ranges answer `fingerprint_unknown` | §7.3, §13.5 |
| Multi-file atomicity across a crash — content-without-receipt is possible, reported by lint | §6.5, §13.6 |
| A receipt carrying the root it produces — structurally impossible | §6.2, §13.7 |
| Two-way Obsidian parity — the floor is one-way; out-of-grammar input refuses | §0.1 GOAL 2, §4.5 |
| An answer to a request whose frame never arrived — transport loss is not a recovery class; re-read before retry, never `force` | §8.1 |

### §4.2 Doc key only

| Surface | Standing law | Why it is not promised |
|---|---|---|
| **Rule packs** — pack loading, budgets, fixtures-as-load-gate, Starlark predicates | §11.1–§11.4 | the daemon loads no pack, so `verdicts` serves `[]`; field and row shape are promised (§2.2), admitting a pack over the wire is not |
| Any future key-grain Delta (`keys:[…]`) | §7.4 | a future-only additive amendment path; no slot ships |

### §4.3 Deliberately outside the promise

| Surface | Where ruled |
|---|---|
| SQL / DuckDB / a view organ as agent core — `mrd sql` is an operator face over an ephemeral `:memory:` build | §10.3–§10.4; `README.md` standing C |
| A second wire door — the daemon's unix socket is the only one | §3.3 |
| Orientation surfaces as wire ops (dashboards, counts, trees) | §10.3, §16 |
| In-process `mrd` paths as a wire surface — a CLI is not a wire door | §3.3, § A.1 |
| Rust crate APIs — nothing published; no crate carries a semver promise (§5.1) | `Cargo.toml`; `laws.md` § Additivity governs shape, not API stability |

### §4.4 The v2 dialect

The engine negotiates a contract rev per session: `hello.contract:"v3"` selects the standing vocabulary, absent or `"v2"` the frozen v2 dialect (`crates/wire-serve/src/rev.rs`). A release promises the v2 dialect stays **byte-identical**: that is the frozen-caps law the v3 projection is built to preserve. It promises nothing about how long v2 is served. **v2 retirement is an open fork** this file does not rule; "served and frozen" promises neither permanence nor removal. `wire-contract.md` § A.4's "one constitution for agents" is a teaching law, not a claim that v2 is unserved.

### §4.5 Multi-root addressing

The `roots` surface advertises every bound root, while `read` serves one. That
disagreement is one defect family: the reserved-prefix face shadows the
registered-root lane. A release does **not** promise cross-root addressing
through it.

**The defect is the advertisement, not the refusal**: refusing a root the face
never bound is correct, while advertising five bound roots against one served
`read` misleads. The recovery line that refusal teaches is
**known-inexecutable** pending multi-root — a caller following it cannot
succeed today. Executable recovery on this path is a **v1.x direction**, not a
v1 promise.

### §4.6 Refusal codes

§2.4 promises the classified refusal, never the sentence that teaches a way
out. The absence of such a teaching sentence breaks no promise.

At the write door, `crates/wire-serve/src/write.rs` refuses through
`bad_request(...)`. That helper sets `ErrorCode::BadRequest` + `Recovery::Fix`
and no teaching clause (`crates/wire-serve/src/lib.rs`, test
`bad_request_carries_the_fix_class_and_message`).

`replace_section` is the taxonomy's **known unreached path**: classified in law,
never exercised by a landed case. An `append` at the same address and rev
landed, so the gap is coverage, not addressing.

### §4.7 Recorded at cut time

Each row is a limit recorded, not repaired; none narrows a promise row.

| Recorded | What it says |
|---|---|
| **The script wall** | At production corpus scale the 7 s wall binds **before** the 64-read ceiling, so the read count is not the operative limit. Reads with §7 |
| **Mount enumeration** | The surface enumerates **paths**, not mount names |
| **Non-vault root kind** | Unreachable in the measured deployment: both admitted roots are `kind: vault` |
| **Unmounted-root staging** | **Not exercised at cut time.** It rewrites the operator's live `~/MERIDIAN.md`, so only in a coordinated maintenance window, never on a registry in use |

## §5 Stamp mechanics

The stamp is how a reader checks a binary against the release point. §5.1 rules
the version string a build carries, §5.2 and §5.3 the tag that names the point,
§5.4 the bytes a tag publishes.

### §5.1 The version string

One version at `[workspace.package]` in the root `Cargo.toml`; every crate inherits it (`version.workspace = true`). Three surfaces show it:

| Surface | Today | Note |
|---|---|---|
| `mrd --version` | `mrd {CARGO_PKG_VERSION} (git {MRD_BUILD_SHA})` | sha read at compile time, never invented |
| `hello.identity.build` | build sha, or `unknown` | § A.3; sha only, not the version |
| `hello.server` | `meridian-daemon/{CARGO_PKG_VERSION}` | informational, never sniffed (§3.2) |

`hello.server` is derived:
`concat!("meridian-daemon/", env!("CARGO_PKG_VERSION"))`
(`crates/registry/src/server.rs`). So the string cannot drift from the stamp. A
client that parses it takes its own risk (§1).

**The release version and the contract rev are different axes; neither renames
the other.** Release 1 ships contract rev v3. Renaming the rev to a release
number would break every client that negotiates `contract:"v3"`. That is a wire
break, not a tidying, and it is refused.

#### The sha token

`MRD_BUILD_SHA` is a sha with an optional marker: `<sha>` where the worktree matched HEAD, `<sha>-dirty` where tracked content diverged, `unknown` where no identity could be read. The marker rides the sha token (git-describe's convention), so `hello.identity.build` carries it: no schema change, no third identity field.

| Build state | `--version` says | What tells the reader |
|---|---|---|
| built clean at `X` | `(git X)` | a bare sha |
| built from a dirty tree based at `X` | `(git X-dirty)` | the marker |
| built at `X`, tree has since moved to `Y` | `(git X)` while HEAD is `Y` | reader compares; a binary cannot see later commits |

**The stamp proves identity; identity plus comparison proves provenance**, so a gate compares the string against a declared sha. A clean build's string is byte-unchanged; only a dirty build's changes.

The answer is a function of the working tree, so the build script must re-run on every build: it names a sentinel path that never exists, which is cargo's way of saying always. Each build probes with `git rev-parse HEAD` and `git status --porcelain --untracked-files=no --no-optional-locks`, ~20 ms each per cargo invocation, plus one relink of `mrd` per clean↔dirty transition. `--no-optional-locks` stops the probe writing the index, so it never blocks a concurrent git in the same tree. Untracked files are excluded: an untracked file reaches the compiler only through a tracked `mod` line.

**An unverifiable clean claim is never published**: an unreadable probe publishes `unknown`. `MRD_BUILD_SHA` from the environment rides verbatim, unprobed; the supplier owns the claim.

### §5.2 The tag

| Element | Shape |
|---|---|
| Name | `v<MAJOR>.<MINOR>.<PATCH>` on the whole workspace |
| Kind | annotated (`git tag -a`), never lightweight; the message carries the release notes (§5.3) |
| Points at | the commit whose tree satisfies both keys of §1 |

Existing tags (`sidecar-v0.1.0`, `stage2-pin`, a dated backup) are component- or
snapshot-scoped, and set no workspace-release precedent. The bare `v` prefix is
the one this file establishes, so a workspace release is distinguishable from a
component tag at a glance.

### §5.3 Release notes

The **annotated tag message** carries the release notes. `docs/` grows no per-release pile: `wire-contract.md` § B forbids versioned contract files and amendment piles, and a CHANGELOG is one. Notes state what the release newly promises in §2/§3 vocabulary — caps added, law amended — citing sections, never prose claims.

### §5.4 What the tag publishes

A tag builds the engine for every served platform and publishes each binary to
Forgejo's **generic package registry**. Each binary is keyed by the **commit**
the tag points at (§5.2), never by the tag name, and there is **no `latest`**.

| Element | Shape |
|---|---|
| Base | `https://git.0xdao.app/api/packages/caoer115/generic/mrd/<COMMIT>` |
| Files | `mrd-linux-amd64`, `mrd-darwin-arm64`, `mrd-linux-arm64`, and a `.sha256` beside each |
| The pin a consumer records | `(COMMIT, SHA256)` |
| Re-publish of the same commit | HTTP **409**; first published bytes stay authoritative |
| Precondition | `ci` succeeded for that tag pipeline — all six verdict lanes |

| Artifact | Agent | Backend |
|---|---|---|
| `mrd-linux-amd64` | Linux runner, `tag-linux-amd64.yaml` selects by `labels` | docker, the `Dockerfile.ci` image |
| `mrd-darwin-arm64` | `platform: darwin/arm64` agent (`tag-darwin-arm64.yaml` `labels`) — a mac artifact needs a mac | local: steps run on the host's own toolchain, `image:` names a **shell** |
| `mrd-linux-arm64` | the same Linux runner, `tag-linux-arm64.yaml` | docker, the `Dockerfile.ci` image plus an in-step `g++-aarch64-linux-gnu` cross toolchain and the `aarch64-unknown-linux-gnu` rustup target |

A linux/arm64 artifact needs no arm64 host: the vendored DuckDB builds through
the `cc` crate, which takes the `CC_aarch64_unknown_linux_gnu` family of
variables, so the amd64 runner cross-compiles it and asks the result its
`--version` under `qemu-aarch64-static`.

- **No publish before the verdict.** `ci.yaml` runs on `refs/tags/v*`, and every
  tag workflow declares `depends_on: [ci]`, so none starts until every `ci`
  lane has succeeded. The dependency is never `optional: true`: optional means
  "enforced only if `ci` is part of the pipeline", which would let a `ci` that
  its own `when` filtered out wave a release through ungated. A red suite
  leaves every tag workflow **skipped** and `publish` never-run: no release.
- **Slow by construction.** Tag lanes start after the whole suite, ~18 minutes
  in. Every workflow therefore clones with a non-rotating PAT (the `clone:`
  block, woodpecker secret `forgejo_clone_token`), not the server's parse-time
  OAuth netrc. That netrc is stamped once at parse and expires on its own clock,
  so it kills a late clone with `exit 128`. A failed clone fails closed: no
  release.
- **A lane refuses a tree it cannot attest.** Before any upload, the lane checks
  the §5.1 stamp against the commit it built. It uses the engine's own dirty
  probe, flag for flag (`--untracked-files=no`, `--no-optional-locks`;
  `crates/mrd/build_git.rs`). A failed probe, or tracked content that diverges,
  exits 1 rather than stamping `-dirty` and publishing. A looser probe would
  stamp `-dirty` on a tree that matches HEAD: a bare `git status --porcelain`
  counts the lane's own scratch directory.
- **The tag names the point; the commit keys the bytes.** A tag is a movable
  ref, so a consumer resolves it to its commit once (`git rev-list -n 1 <tag>`)
  and records the `(COMMIT, SHA256)` pair above. On a `tag` event
  `CI_COMMIT_SHA` is the **annotated tag object**, not the commit. Each lane
  therefore peels with `git rev-parse <sha>^{commit}` before it keys, stamps or
  checks anything — a no-op on a commit sha, so push, manual and tag events
  all agree. This is the same pin the main-push publish hands out: a tag adds a
  platform, not a second pin vocabulary.
- **Append-only is the property, not an accident of the store.** A rebuild of
  the same commit can differ byte-wise, and the 409 is what keeps a digest a
  consumer already recorded valid. Holding every input, one rebuild did
  reproduce `mrd-linux-amd64` byte for byte: a `git archive` of `4640044e0`, the
  same CI image and runner, the same sccache shard, the same `MRD_BUILD_SHA`,
  the same target-dir **path** (sha256
  `0098356f0f9ac63af221128459595138f1600eb7aabf4f15a7e04f4306129ce0`, 54151480
  bytes). That is one measurement only. No input is pinned: a different
  target-dir path alone came out 64 bytes smaller, because `OUT_DIR` strings
  reach the binary through build scripts. Image tag, toolchain and `RUSTFLAGS`
  move independently.
- **Each lane asks the registry first** and builds only when this commit's artifact is absent, so a tag on a commit main already published re-prints the pin.
- **No Forgejo release object is created**, deliberately: attachments are mutable, the registry is not. Notes live in the tag message (§5.3); bytes only in the registry.

## §6 How the promise changes after a release

- **Additive** — a new cap or response field; old callers ignore it
  (tolerant-client law, `wire-contract.md` §3.2). No promise breaks; the
  notes name it.
- **Amending** — a standing doc section changes before the code (docs-first);
  the shipping release names the section.
- **Removing** — a cap leaves the set, which breaks callers: a ruling, not a
  refactor, with its own decision record as `wire-contract.md` §3.3 and
  §10.4 do.

A surface neither doc nor `caps` names is not a promise; no release note may
create one.

## §7 What the 7 s script wall bounds

The script entry runs a task block from a page. Its wall-clock budget is
**7 s**, spelled once on each side of the socket: `WALL_CLOCK`
(`crates/mrd/src/script/cmd.rs`) in the CLI and `effects::DEFAULT_WALL_CLOCK` in
the daemon, two independent literals kept equal by hand.

The budget binds at three layers inside the engine, named in the constants' doc
comments and in `run-plane.md` § Where the budgets bind. The whole attempt is
one `wire-contract.md` § A.7 `script` frame, so two of the three layers sit in
the daemon.

- **read** (daemon) — before each program read, against the pinned entry
  world (`registry::script_op`).
- **connect** (`mrd`) — on the socket (`SocketDoor::connect`); bounds the one
  `script` round trip.
- **commit** (daemon) — before the commit is issued (`registry::script_op`).

Startup and teardown sit outside all three. A host's bound on the `mrd` child
process is a fourth layer in the host, not ruled here.

It is an **engine** budget, never the operator's process wall. Measure by engine
ms: no door headroom may be inferred from process wall, in either direction. A
write-bearing run can cross 7 s of process wall and still commit correctly,
because the process wall is not what the budget bounds.
