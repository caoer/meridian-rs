---
type: convention
id: index
status: standing
description: Process, standing corrections, inventory, and reading order for this directory.
owns: [process, standing corrections, inventory, reading order]
---

# meridian-rs `docs/`

`meridian-rs` is a Rust engine over a Markdown workspace. It ships one binary,
`mrd`, which reads and writes pages, records pins over them, arms a workspace,
and runs the resident daemon. A client — an MCP server, an editor plugin, a
script — drives the daemon's socket and holds no Markdown semantics. Reads and
writes address sections by structure, never by a client byte offset, and a
write names the revision it expects (`wire §5.1`). Whether a scope requires
that revision guard is host policy (`wire §5.3`); `force` is a client's path
past the guard (`wire § A.1`).

The files in this directory define the law; code follows them. This README
carries the process rules, the standing corrections, the inventory, and the
reading order; each law itself lives in the file named for it below. A citation
names one of those files and a section in it: `wire §5.1` is section 5.1 of
`wire-contract.md`.

## The model in one page

These are the terms the other files here use, including the ones used above.
Each entry ends with the document and section that spells its law.

- **workspace** — one directory tree of Markdown pages, declared by a
  `MERIDIAN.md` root file (`schema`).
- **page / section** — a page is one `.md` file. A section is a heading and
  everything under it, up to the next heading of the same or higher level
  (`wire §1`).
- **address** — where a read or write points (`wire §2.1`; across roots,
  `addr`). Machine addresses are **segments only**: `hpath` (a list of heading
  segments `{"h":"Goals"}`, with optional `n` to pick a repeat), `anchor` (a
  block id), or `fm_key` (a top-level frontmatter key). A joined string like
  `Goals/Q3` is never a machine address.
- **span** — a `[start, end)` byte range on the raw file bytes (`wire §1`).
  Clients never send spans.
- **node_rev** — the 16-hex revision of one node: `blake3(span bytes)[:16]`
  (`merkle §2`). It is a CAS token (compare-and-swap): a write that names a
  stale `node_rev` is refused (`wire §5.1`). `file_rev` is the same kind of
  token over a whole file.
- **fingerprint** — the workspace content hash: `b3:` + 64 hex, never
  truncated (`fp §2`). It is a Merkle root over the hash domain
  (`merkle §4`). The wire noun is `fingerprint`.
- **hash domain** — the set of files the fingerprint covers: Markdown only,
  minus dot-segment paths and the ignores declared in `meridian/domain.md`
  (`wire §12.1`).
- **CAS** — compare-and-swap. A write states the revision it expects; a
  mismatch is refused, never merged (`wire §5.1`).
- **splice** — the only write op (`wire §4.4`). It replaces a section, block,
  or frontmatter key at an address, guarded by revs.
- **receipt** — the recorded fact of what a write did (`wire §6`). It is
  returned on the wire and, when armed, written into the workspace.
- **armed / gate** (`armed`) — armed is the workspace state in which rules and
  receipts are enforced. The gate is the seam where a write is checked before
  it lands.
- **daemon / wire** — the daemon is the resident `mrd daemon` process; its
  unix socket is the one wire door (`wire §3.3`). The wire is the NDJSON
  protocol on that socket: one JSON object per line, requests correlated to
  responses by `id`, plus id-less notification frames for deltas
  (`wire §3.1`).
- **run plane** (`run`) — `mrd run` / `mrd script`: the engine executes a task
  block from a page and turns what it emits into governed effects.
- **pin** — a recorded claim that page A draws from section B at B's
  fingerprint (`docsys §6`). `mrd check` reports whether it still holds; a pin
  that no longer holds is **drift**.

## Rules of this directory

1. **Doc correct > code correct.** These files teach the accurate design. If
   code, goldens, or MCP schemas disagree, the document wins.
2. **Docs first.** A material change updates the correct doc before code.
3. **One wire contract.** `wire-contract.md` is the only wire constitution;
   there is no v2/v3 stack. Edit that file directly, docs first.
4. **Self-contained tree.** A standing doc cites only other files in this
   directory, plus in-repo `crates/` paths for implementers. No out-of-tree
   Markdown is an authority.
5. **No dated markers.** State the current fact in positive form; history
   lives in git.

## Standing corrections (always on)

These corrections are always in force. The first table holds the three lettered
laws (address, receipts, view organ); the second holds one law per topic.

| | Law |
|---|---|
| **A — Address** | A machine address is **segments only**: `{"hpath":[{"h":"Goals"},{"h":"Q3"}]}` (optional `n`, or `anchor` / `fm_key`). A joined `Goals>Q3` / `Goals/Q3` is never a writeable form. |
| **B — Receipts** | Armed **wire** facts are normative. The default md template must not publish a second path. |
| **C — View organ** | DuckDB / `view_path` / SQL boards are **not** agent core. |

| Topic | Law |
|---|---|
| Write | The only write op is `splice`. CLI append = `put{at:"end"}`. |
| Mint | `toc` / `cat` / `read` mint revs; `resolve` is the walk plane (**no rev**). |
| Content hash | The wire noun is **`fingerprint`** (`b3:…`). |
| Spans | No client spans in requests. |
| Op reality | `check_write` is a consumed wire op: a standalone, read-only splice verdict (wire-contract § A.3). `sub` is served at the daemon door (wire-contract §4.7); it is not a reserved or future shape. |

## How to cite

A citation names its document: `<id> §N`, for example `wire §4.4` or
`merkle §5`. The ids are the `id` column of the table below; their registry is
`docsys §2`. A bare `§N` is deprecated for new writing; it reads as `wire §N`.
Qualify it or leave it alone, but never change which section it names. Inside
one file, a
citation to that same file may stay bare. The full grammar, anchors, and pins
are in `doc-system.md`.

## Files in this directory

| File | id | Role | Read it when … |
|---|---|---|---|
| `README.md` | `index` | Process, standing corrections, inventory, reading order | you open this directory for the first time |
| `doc-system.md` | `docsys` | How this corpus is structured, addressed, cited, and locked | you write, cite, or pin a doc |
| `wire-contract.md` | `wire` | The standing wire constitution: nouns, ops, guards, receipts, errors | you integrate a client against the wire |
| `laws.md` | `laws` | The three architecture laws, enforced as crate dependency edges, plus every crate's charter | you edit crates |
| `release.md` | `release` | What a release promises (the two-key rule), plus stamp and tag mechanics | you cut or consume a release |
| `address-grammar.md` | `addr` | Cross-root addressing, the mount table, and the `addr::Addr` type | you address across roots or mounts |
| `meridian-md-schema.md` | `schema` | `MERIDIAN.md` config parse | you write or parse a `MERIDIAN.md` |
| `node-rev-merkle-spec.md` | `merkle` | Hash law for `node_rev` and the merkle fingerprint, the resident tree, and `.assets/` | you implement or verify revs and fingerprints |
| `fingerprint-norm-spec.md` | `fp` | The fingerprint CID token and the norm-v2 algorithm | you compute or compare fingerprint tokens |
| `armed-plane.md` | `armed` | The arming ladder and the `gate()` seam | you arm a workspace or touch the write gate |
| `base-projection.md` | `base-projection` | `.base` (Obsidian Bases) projection into the sql face: membership, relations, the `base_fold` witness | you work on `.base` files in the sql face |
| `body-projection.md` | `body-projection` | Section body text in the sql face: the exclusive-chunk law, the `body` relation, the content-addressed cache protocol | you work on body text in the sql face |
| `run-plane.md` | `run` | The run plane (`mrd run`) and preset / session birth | you work on `run`, `realise`, or `preset` |
| `status.md` | `status` | CLI / build snapshot, **descriptive** only; also the home of R12, the armed-plane exit reading | you want "what the binary exposes today" |

## Reading order

1. This README.
2. `wire-contract.md`.
3. `laws.md` if you edit crates.
4. `status.md` only for "what the binary exposes today".
5. `release.md` only when cutting or consuming a release.
