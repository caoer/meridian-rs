# crates/wire-serve/src/lib.rs — comment-cleanup notes

## Module charter (from lib.rs:1)

Shared typed edge — strict decode + read-op arms — lifted out of the sidecar so the resident registry daemon and per-workspace sidecar answer the wire contract through ONE implementation (arch map A6: "lift, don't duplicate").

**Owns:** hand-rolled strict-decode pass (`decode` — wire §3.2 server law: unknown request fields and unknown enum values rejected loudly, because serde's `deny_unknown_fields` does not compose with `flatten`), read-op arms (`read` — `toc`/`cat`/`extract`/`links`/`resolve`) over BORROWED parsed state, and the WRITE choke-point (`write::splice` — `splice → commit` seam both hosts commit through, W1).

**Read arms never** own the corpus, read disk, parse, or hold a serve loop: each takes already-built `model` state (`&Document`, or `&CorpusIndex` + document map) and ambient root as data; the CALLER obtains that state — sidecar builds per request, daemon reuses warm engine. One projection, two state sources, no drift.

**Write choke-point is inherently stateful** — a commit IS disk I/O — so `write` necessarily reads/writes disk (`fs`), reparses after-state (`syntax`), renders receipts (`receipt`), evaluates verdicts (`policy`). Still ONE implementation both hosts share; delta ring stays with the caller. Disk-load + fold helpers both paths use (`load_doc`, `ambient_root`, `domain_snapshot`) live here so there is one fs→wire mapper, not two.

## Root / diff live with the caller (from lib.rs:29)

`root` and `diff` are cursor/history plumbing whose source differs by host (sidecar owns per-epoch ring; resident daemon has none until watcher lands), so they are NOT arms here — each host builds those two responses from its own `seq`/history. This crate holds only corpus reads whose logic is identical across hosts.

## load_doc / ambient_root / domain_snapshot (from lib.rs:71)

`fs::load` with §8 error split: `file_not_found` (env — file gone, path echoed), `invalid_utf8` (refused, never lossy-decoded), `io_error{cause}` otherwise. Shared by both hosts' single-file reads and write path (loads fresh disk before validating).

Ambient workspace root (v2 §4.1/§12): §12 hash domain's file bytes folded through `model::merkle_root` — the one blake3 home. Fresh disk fold shared by sidecar read arms and write path's `root_before`/`root_after` (daemon's warm engine carries same fold as `at_fingerprint`).

§12 hash-domain snapshot: every domain file's bytes + root folded over exactly those bytes — one read, one fold, so a consumer parses the same bytes its `as_of_root` describes and the answer cannot drift from its stamp. Shared `fs::domain_snapshot` re-homed into wire `Root` token and `io::Error` into wire `io_error` frame.

## Test: v3 pin D13 actor wall (from lib.rs:235)

S7 `splice.pin`: v3-only at decode, strict on its three keys, and — security shape — carries NO actor key (D13). A pin IS a write, so pin-only splice is a complete batch and frozen "missing `edits`" refusal must not fire. Pin identity is the splice's own daemon-derived `actor` and nothing else — cannot forge as another actor via `pin.actor`.

## Test: create decode rev-agnostic (from lib.rs:335)

Decode is rev-AGNOSTIC (like `read`/`check_write`): the v3 gate is at DISPATCH, so a v2 frame decodes here and is refused `unknown_op` by the host arm. Asserted so a future reader does not "fix" decode by adding a rev branch that would move the refusal to the wrong plane.

## Test: create field wall / path law (from lib.rs:349)

No-forced-birth wall + not-a-batch wall: guarded door carries no `--force` escape, so admitting the key would advertise a bypass that does not exist; birth is ONE path with ONE body, so whole splice batch vocabulary is foreign. Refused BY NAME. Path-law wall is the FIRST jail: birth cannot even be spelled outside the workspace; `bad_path` echoes offending spelling; engine's `path_confined` is the second wall behind it.

## Test: plan_edits mutual exclusion (from lib.rs:443)

Mutual exclusion + explicit empty-array law (C note 6) + strict per-shape wall.

## Review verdict: GO
- reviewer: codex-exec
- code-invariance: pass — PASS: comment-only diff
- info-preservation: pass — Shared-edge ownership, caller/state split, error mapping, snapshot consistency, and strict-decode security contracts survive in comments or these notes.
- style: pass — Remaining docs are concise and accurate; no narration, commented-out code, or unsafe blocks.
- detail: Notes preserve the daemon/sidecar boundary, no-stamp-drift snapshot invariant, D13 actor wall, rev-agnostic create decode, and create field/path walls.
