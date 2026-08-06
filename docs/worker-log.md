# Worker log (optional — deletable)

**Not design law.** Time-sensitive notes, external provenance, machine measurements, and former filenames. **Delete this file anytime** — every other file under `docs/` must still read correctly without it.

Design docs may not depend on paths listed here. Standing law lives only in:

`README.md`, `wire-contract.md`, `laws.md`, `address-grammar.md`, `meridian-md-schema.md`, `node-rev-merkle-spec.md`, `fingerprint-norm-spec.md`, `armed-plane.md`, `run-plane.md`, `status.md`.

---

## External / historical provenance (not in this tree)

These were cited when specs were written. They are **not** in `docs/` and must not reappear as citations in standing docs.

| Historical name | What it contributed (already folded into standing docs) |
|---|---|
| Wiki decision `2026-07-24-cross-root-addressing` | Cross-root grammar, `obsidian://` stored form, name ownership, grey rule, dependency order → `address-grammar.md`, `meridian-md-schema.md` |
| Wiki decision `2026-07-24-meridian-md-entry-point` | One entry point, fail-loud parse → `meridian-md-schema.md` |
| Wiki decision `2026-07-24-fingerprint-cid-representation` | Fingerprint CID-token → `fingerprint-norm-spec.md` |
| Wiki decisions claim-link / meridian-lock-block | Anchor promotion; lock carries full fingerprints → `fingerprint-norm-spec.md`, lock crate |
| Advisor labels S3-R4 / S3-R6 / S3-R7 / … | Bound discipline; grey→exit 1; canonicalize-at-bind; no equal-or-nested mounts → restated as plain law in `address-grammar.md` / `meridian-md-schema.md` |
| Session findings `finding-02` / `finding-03` (ref grammar doors; cross-root misresolve) | Motivated body-level guards and grey unmounted → law in `address-grammar.md` §5–§6 |
| ABS team-c winner schema / design-stance D-C1…D-C10 | Wire standing law → `wire-contract.md` |
| Session `05-19-meridian-socket-mcp-leg` | Dogfood: dual address, view_path debt → corrections in `README.md` / `wire-contract.md` |
| Plan unit labels (U10, U11, U12, U4.x, …) | Scheduling history; not required to understand law |

## Former filenames (collapsed)

| Former path under `docs/` | Now |
|---|---|
| `wire-contract-v2*.md`, `wire-contract-v3*.md`, per-amendment pile | `wire-contract.md` |
| `norm-v2-spec.md` | `fingerprint-norm-spec.md` (algorithm name “norm-v2” remains in body) |
| `fingerprint-or-force.md` | `wire-contract.md` § Fingerprint-or-force |
| `arming-from-zero.md`, `gate-byte-landing.md` | `armed-plane.md` |
| `preset-session-birth.md` | `run-plane.md` |
| `NORMATIVE.md`, `TOMBSTONES.md`, `comment-cleanup/**` | gone / this log |

## Machine / session measurements (historical)

### Cross-root misresolve (repro sketch)

On a toy workspace, a body link `[[sessions:…/notes.md#…]]` with `sessions` unbound could resolve to ambient `notes.md` (wrong success). Law now: unmounted root → **grey**, exit 1 — see `address-grammar.md` §5–§6. No need to re-run this to implement.

### Symlink topology (mount canonicalize)

A mount table must treat symlink path and real path (with or without trailing slash) as one tree — refuse binding the same tree under two names. Law: `address-grammar.md` §8; implementation uses canonicalize-at-bind.

### `mrd resolve` sense

Operator `mrd resolve` is workspace-identity resolution (drawer / root discovery), **not** address→document resolve. Address resolve is the wire/engine path in `address-grammar.md` / `wire-contract.md`.

### Perf / harness notes

`status.md` may list live counters; claims that cite `perfsuite` result files are descriptive of a build machine, not design law.

## Code debt reminders (docs already correct)

| Surface | Debt |
|---|---|
| Receipt pretty join / goldens | still may emit join-string targets |
| MCP `at` string | agent face lag |
| Dual wire vocabulary in code | legacy `root` keys vs standing `fingerprint` |
| `mdfs_config.yaml` | legacy sole domain-config read path in `fs::Domain::load` — docs teach `meridian/domain.md` only |
| `run` bracket Display strings | may still say `mdfs_config.yaml` in error text while bracketing both domain surfaces |

## 2026-08-06 cleanup complete (design tree)

**Gate:** automated scan of design docs (excluding this file) for non-intentional
false patterns = **0 hits**. Taxonomy test green. `armed-plane.md` restored
from HEAD (earlier U→implementation corruption fixed). `fingerprint-norm-spec`
intro un-broken.

**Intentional leftovers only:**
- `mdfs_config.yaml` named as **legacy sole read path** (do not create/teach)
- `status.md` notes live dual `root`/`if_root` **code debt**
- `norm-v2` as **algorithm** name (not wire version)
- Negative teaching: “no `guard` op”, “never joined Goals>…”

**Not claimed:** every tournament-era index paragraph in `wire-contract.md`
§15–§18 is stylistically lean — law is consistent, prose still long.

## 2026-08-06 careful rewrite pass (files touched)

| File | Summary |
|---|---|
| `wire-contract.md` | Rebuilt from HEAD core; §12 = `meridian/domain.md`; no amendment dump |
| `meridian-md-schema.md` | Drop mdfs naming; domain version → domain.md |
| `run-plane.md` | fingerprint vocabulary; domain config bracket; restore preset section |
| `node-rev-merkle-spec.md` | fingerprint not root op; no guard op; hash domain leaves |
| `fingerprint-norm-spec.md` | plane table + tags |
| `address-grammar.md` | wire::Root vs fingerprint design noun |
| `laws.md` | charter table repair; fingerprint vocab; view optional; preset→run-plane |
| `status.md` | § A refs; residual wording |
| `armed-plane.md` | pointer to wire-contract § A.2 |

---

*Append-only if useful. Prefer not growing this file — standing law goes in the design docs, not here.*
