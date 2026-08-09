---
type: contract
status: standing
updated: 2026-08-09
description: What a meridian-rs release promises, and the stamp + tag mechanics that make the promise cuttable.
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
| `splice.if_fingerprint` | §5.1 | world-grain CAS, checked FIRST, failing the whole batch |
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
| `fingerprint` | §4.7 | the workspace content hash plus `seq` |
| `diff` | §4.7, §7.3 | replay ≡ live — the byte-identical Delta objects the live stream carried |
| `sub` | §4.7 | ack-then-push at the daemon door, one Notification frame per Delta batch |

### §2.4 Wire-wide promises (not per-cap)

| Promise | Law |
|---|---|
| One wire door — the daemon's unix socket; NDJSON line dialogue | §3.1, §3.3 |
| Raw-lexeme `id` validation before typed decode; `id:null` + `id_raw` on a bad lexeme | §3.1 |
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

## §5 Stamp mechanics

### §5.1 The version string

The workspace stamps one version at `[workspace.package]` in the root
`Cargo.toml`; every crate inherits it (`version.workspace = true`). It reaches
two reader-visible surfaces:

| Surface | Today | Note |
|---|---|---|
| `mrd --version` | `mrd {CARGO_PKG_VERSION} (git {MRD_BUILD_SHA})` | the sha is read at compile time, never invented |
| `hello.identity.build` | the build sha, or `unknown` | § A.3; sha only — the version does not ride here |

**The release version and the contract rev are DIFFERENT AXES and neither
renames the other.** A release numbered 1 ships contract rev v3. Renaming the
contract rev to match a release number would break every client that negotiates
`contract:"v3"` — it is not a tidying, it is a wire break, and it is refused.

**Bound consequence, named not hidden:** the daemon's `hello` `server` string is
a hardcoded `meridian-daemon/0.1` (`crates/registry/src/server.rs`), independent
of the workspace version. §3.2 makes that string informational — no version
sniffing, ever — so no promise breaks when it disagrees with the release number.
It is a reader-facing drift, not a contract one. Deriving it from
`CARGO_PKG_VERSION` so it cannot drift again is the honest repair, and it is a
code change on a downstream-visible string: it belongs to whoever rules the
version stamp, never to a doc edit.

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

## §6 How the promise changes after a release

- **Additive** — a new cap, a new response field. Old callers ignore it under
  the tolerant-client law (§3.2). No promise breaks; the notes name it.
- **Amending** — a standing doc section changes. Docs-first: the section changes
  BEFORE the code, and the release that ships it names the section.
- **Removing** — a cap leaves the set. This BREAKS callers and is a ruling, not
  a refactor. It carries its own decision record, exactly as §3.3 and §10.4 do.

A surface that neither doc nor `caps` names is not a promise, and no release
note may create one.
