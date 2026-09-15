---
type: contract
id: release
status: standing
description: What a meridian-rs release promises, and the stamp + tag mechanics that make the promise cuttable.
owns: [what a release promises, stamp and tag mechanics, what a tag publishes]
---

# Release definition

> Standing law: `README.md` (process), `wire-contract.md` (design), `laws.md` (architecture). Docs-first: every promise row cites the standing doc that rules it.

## §1 A release is a two-key point

A named point on this repo's history at which every surface in §2 and §3 holds **both** keys:

| Key | Held when |
|---|---|
| **doc key** | the surface is law in a standing `docs/` file — the cited section |
| **code key** | the built artifact serves it — the op dispatches, the verb runs |

Doc key only is **designed, not promised** (§4.2). Code key only is **not law and not promised**: relying on it is the caller's risk; shipping one never makes it a promise. "Doc correct > code correct" (`README.md`) rules which document wins, not what a caller may rely on.

## §2 The wire promise

The promise surface is `caps` in the `hello` response (`wire-contract.md` §3.2): an op is in `caps` or answers `unknown_op`, never both. Capability comes from `caps`, never the `server` string — no version sniffing, ever. `caps` is a **set** with no promised order. Each cap holds as its cited section specifies, at `hello` with `contract:"v3"`.

### §2.1 Read and discovery

| Cap | Law | A caller may rely on |
|---|---|---|
| `toc` | §4.1 | write kit per file: `hpath` + `node_rev` per section, anchors with revs, frontmatter keys, header `fingerprint` |
| `cat` | §4.2 | full-span bytes, heading-inclusive; rev is blake3 of those bytes |
| `extract` | §4.3 | full node objects, 11-variant kind enum, total node order; unknown `kinds` refuses |
| `read` | § A.3 | addressing + content + render + `props` + `anchors` + `unresolved`, one engine snapshot |
| `resolve` | §4.5 | best-effort app-compatible two-stage walk; location facts only |
| `resolve.content` | §4.5 | those facts plus the fragment bytes; still no rev |
| `links` | §4.6 | per-edge resolved/unresolved counts, per file or corpus-wide |
| `links.require_fingerprint` | §10.2 | opt-in `stale_view` refusal instead of an untensed answer |
| `mounts` | § A.5 | live root registry, re-derived per call against `~/MERIDIAN.md`'s hash |
| `hello.identity` | § A.3 | `{build: sha\|unknown}` — read, never invented |

### §2.2 Write

| Cap | Law | A caller may rely on |
|---|---|---|
| `splice` | §4.4 | only write op; batch-only; one response shape; atomic through one reparse |
| `splice.if_node_rev` | §5.1 | node-grain CAS, re-derived at execution from pre-batch state |
| `splice.if_fingerprint` | §5.1, §5.4 | bare: world-grain CAS (v2 root premise, checked first); with `scope` under `scoped-guards`: one-premise sugar at that node |
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
| The `id` **echo**: a JSON integer lexeme in `[0, 2^53)` returns unchanged. **A non-conforming lexeme is nulled and the request is still served** — no refusal, no `id_raw`, though §3.1 requires both: law stands, unserved | §3.1; §18 row 9 |
| Strict server / tolerant client; unknown request fields are rejected | §3.2 |
| `node_rev` is MUST on every `toc`/`cat`/`extract` node while `splice ∈ caps` | §3.2 |
| Every error carries `code` + `recovery` from the closed six-class enum | §8 |
| A content-mutating wire write demands fingerprint or `force` (`guard_required`) | § A.1 |
| The armed plane refuses on block-severity verdicts; never-armed stays advisory | § A.2, `armed-plane.md` |

## §3 The promise beyond the wire

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
| **Rule packs** — pack loading, budgets, fixtures-as-load-gate, Starlark predicates | §11.1–§11.4 | no pack loads, so `verdicts` serves `[]`; field and row shape are promised (§2.2), admitting a pack over the wire is not |
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

The engine negotiates a contract rev per session: `hello.contract:"v3"` selects the standing vocabulary, absent or `"v2"` the frozen v2 dialect (`crates/wire-serve/src/rev.rs`). A release promises v2 stays **byte-identical** — the frozen-caps law the v3 projection preserves — and nothing about how long it is served. **v2 retirement is an open fork** this file does not rule; "served and frozen" promises neither permanence nor removal. `wire-contract.md` § A.4's "one constitution for agents" is a teaching law, not a claim that v2 is unserved.

### §4.5 Multi-root addressing

`roots` advertises every bound root while `read` serves one — the reserved-prefix face shadows the registered-root lane — and a release does **not** promise cross-root addressing. **The defect is the advertisement, not the refusal**: refusing a root the face never bound is correct, while advertising five bound roots against one served `read` misleads. The taught recovery line is **known-inexecutable** pending multi-root; executable recovery is a **v1.x direction**, not a v1 promise.

### §4.6 Refusal codes

§2.4 promises the classified refusal, never the sentence that teaches a way out; its absence breaks no promise. At the write door: `crates/wire-serve/src/write.rs` → `bad_request(...)` → `ErrorCode::BadRequest` + `Recovery::Fix` (`crates/wire-serve/src/lib.rs`, test `bad_request_carries_the_fix_class_and_message`), no teaching clause. `replace_section` is the taxonomy's **known unreached path**: classified, never exercised by a landed case; an `append` at the same address and rev landed, so the gap is coverage, not addressing.

### §4.7 Recorded at cut time

Found at the v1 cut, recorded not repaired; none narrows a promise row.

| Recorded | What it says |
|---|---|
| **The script wall** | At production corpus scale the 7 s wall binds **before** the 64-read ceiling, so the read count is not the operative limit. Reads with §7 |
| **Mount enumeration** | The surface enumerates **paths**, not mount names |
| **Non-vault root kind** | Unreachable in the measured deployment: both admitted roots are `kind: vault` |
| **Unmounted-root staging** | **Not exercised at cut time.** It rewrites the operator's live `~/MERIDIAN.md`, so only in a coordinated maintenance window, never on a registry in use |

## §5 Stamp mechanics

### §5.1 The version string

One version at `[workspace.package]` in the root `Cargo.toml`; every crate inherits it (`version.workspace = true`). Three surfaces show it:

| Surface | Today | Note |
|---|---|---|
| `mrd --version` | `mrd {CARGO_PKG_VERSION} (git {MRD_BUILD_SHA})` | sha read at compile time, never invented |
| `hello.identity.build` | build sha, or `unknown` | § A.3; sha only, not the version |
| `hello.server` | `meridian-daemon/{CARGO_PKG_VERSION}` | informational, never sniffed (§3.2) |

`hello.server` is derived — `concat!("meridian-daemon/", env!("CARGO_PKG_VERSION"))` (`crates/registry/src/server.rs`) — so it cannot drift; a client that parses it takes its own risk (§1).

**The release version and the contract rev are different axes; neither renames the other.** Release 1 ships contract rev v3; renaming the rev to a release number breaks every `contract:"v3"` client — a wire break, refused.

#### The sha token

`MRD_BUILD_SHA` is a sha with an optional marker: `<sha>` where the worktree matched HEAD, `<sha>-dirty` where tracked content diverged, `unknown` where no identity could be read. The marker rides the sha token (git-describe's convention): no schema change, no third identity field.

| Build state | `--version` says | What tells the reader |
|---|---|---|
| built clean at `X` | `(git X)` | a bare sha |
| built from a dirty tree based at `X` | `(git X-dirty)` | the marker |
| built at `X`, tree has since moved to `Y` | `(git X)` while HEAD is `Y` | reader compares; a binary cannot see later commits |

**The stamp proves identity; identity plus comparison proves provenance**, so a gate compares the string against a declared sha. A clean build's string is byte-unchanged; only a dirty build's changes.

A sentinel path that never exists makes cargo re-run the build script every build: `git rev-parse HEAD` and `git status --porcelain --untracked-files=no --no-optional-locks`, ~20 ms each per cargo invocation, plus one relink of `mrd` per clean↔dirty transition. `--no-optional-locks` stops the probe writing the index, so it never blocks a concurrent git. Untracked files are excluded: one reaches the compiler only through a tracked `mod` line.

**An unverifiable clean claim is never published**: an unreadable probe publishes `unknown`. `MRD_BUILD_SHA` from the environment rides verbatim, unprobed; the supplier owns the claim.

### §5.2 The tag

| Element | Shape |
|---|---|
| Name | `v<MAJOR>.<MINOR>.<PATCH>` on the whole workspace |
| Kind | annotated (`git tag -a`), never lightweight; the message carries the release notes (§5.3) |
| Points at | the commit whose tree satisfies both keys of §1 |

Existing tags (`sidecar-v0.1.0`, `stage2-pin`, a dated backup) are component- or snapshot-scoped and set no workspace-release precedent; the bare `v` prefix is the one this file establishes.

### §5.3 Release notes

The **annotated tag message** carries the release notes. `docs/` grows no per-release pile: `wire-contract.md` § B forbids versioned contract files and amendment piles, and a CHANGELOG is one. Notes state what the release newly promises in §2/§3 vocabulary — caps added, law amended — citing sections, never prose claims.

### §5.4 What the tag publishes

A tag builds both served platforms and publishes each binary to Forgejo's **generic package registry**, keyed by the **commit** the tag points at (§5.2), never the tag name; there is **no `latest`**.

| Element | Shape |
|---|---|
| Base | `https://git.0xdao.app/api/packages/caoer115/generic/mrd/<COMMIT>` |
| Files | `mrd-linux-amd64`, `mrd-darwin-arm64`, and a `.sha256` beside each |
| The pin a consumer records | `(COMMIT, SHA256)` |
| Re-publish of the same commit | HTTP **409**; first published bytes stay authoritative |
| Precondition | `ci` succeeded for that tag pipeline — all six verdict lanes |

| Artifact | Agent | Backend |
|---|---|---|
| `mrd-linux-amd64` | Linux runner, `tag-linux-amd64.yaml` selects by `labels` | docker, the `Dockerfile.ci` image |
| `mrd-darwin-arm64` | `platform: darwin/arm64` agent (`tag-darwin-arm64.yaml` `labels`) — a mac artifact needs a mac | local: steps run on the host's own toolchain, `image:` names a **shell** |

- **No publish before the verdict.** `ci.yaml` runs on `refs/tags/v*`; both tag workflows declare `depends_on: [ci]`, never `optional: true`, which would wave through a `ci` its own `when` filtered out. A red suite leaves both **skipped**, `publish` never-run: no release.
- **Slow by construction.** Tag lanes start ~18 minutes in, so every workflow clones with a non-rotating PAT (`clone:` block, woodpecker secret `forgejo_clone_token`), not the server's parse-time OAuth netrc, which expires and kills a late clone with `exit 128`. A failed clone fails closed: no release.
- **A lane refuses a tree it cannot attest.** Before any upload it checks the §5.1 stamp against the commit built, with the engine's own dirty probe, flag for flag (`--untracked-files=no`, `--no-optional-locks`; `crates/mrd/build_git.rs`). A failed probe or diverged tracked content exits 1, never `-dirty`. A lane supplying `MRD_BUILD_SHA` owns the claim: a bare `git status --porcelain` counts the lane's scratch directory and stamps `-dirty` on a tree matching HEAD.
- **The tag names the point; the commit keys the bytes.** A consumer resolves the movable tag ref once (`git rev-list -n 1 <tag>`) and records the pair. On a `tag` event `CI_COMMIT_SHA` is the **annotated tag object**, not the commit, so each lane peels with `git rev-parse <sha>^{commit}` before it keys, stamps or checks — a no-op on a commit sha, so all events agree. Same pin as the main-push publish: a tag adds a platform, not a pin vocabulary.
- **Append-only is the property, not an accident of the store.** A rebuild of the same commit can differ byte-wise; the 409 keeps that from invalidating a recorded digest. Holding every input (`git archive` of `4640044e0`, same CI image, runner, sccache shard, `MRD_BUILD_SHA`, target-dir **path**), one rebuild reproduced `mrd-linux-amd64` byte for byte (sha256 `0098356f0f9ac63af221128459595138f1600eb7aabf4f15a7e04f4306129ce0`, 54151480 bytes) — one measurement only. No input is pinned: a different target-dir path alone came out 64 bytes smaller (`OUT_DIR` strings reach the binary through build scripts); image tag, toolchain and `RUSTFLAGS` move independently.
- **Each lane asks the registry first** and builds only when this commit's artifact is absent, so a tag on a commit main already published re-prints the pin.
- **No Forgejo release object is created**, deliberately: attachments are mutable, the registry is not, so a release page would be a second home for the pin. Notes live in the tag message (§5.3); bytes only in the registry.

## §6 How the promise changes after a release

- **Additive** — a new cap or response field; old callers ignore it
  (tolerant-client law, §3.2). No promise breaks; the notes name it.
- **Amending** — a standing doc section changes before the code (docs-first);
  the shipping release names the section.
- **Removing** — a cap leaves the set, which breaks callers: a ruling, not a
  refactor, with its own decision record as §3.3 and §10.4 do.

A surface neither doc nor `caps` names is not a promise; no release note may
create one.

## §7 What the 7 s script wall bounds

The script entry's wall-clock budget is **7 s**: `WALL_CLOCK`
(`crates/mrd/src/script/cmd.rs`) in the CLI, `effects::DEFAULT_WALL_CLOCK` in
the daemon, two literals kept equal by hand. It binds at three layers inside
the engine, named in the constants' doc comments and in
`run-plane.md` § Where the budgets bind. The attempt is one § A.7 `script`
frame.

- **read** (daemon) — before each program read, against the pinned entry
  world (`registry::script_op`).
- **connect** (`mrd`) — on the socket (`SocketDoor::connect`); bounds the one
  `script` round trip.
- **commit** (daemon) — before the commit is issued (`registry::script_op`).

Startup and teardown sit outside all three. A host's bound on the `mrd` child
process is a fourth layer in the host, not ruled here.

It is an **engine** budget, never the operator's process wall: measure by engine ms;
process wall implies no door headroom either way. A write-bearing run can
cross 7 s of process wall and still commit correctly.

§4.7 script-wall record: at production corpus scale this wall binds before the
64-read ceiling, so the operative limit is time, not read count.
