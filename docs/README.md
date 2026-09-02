---
type: convention
id: index
status: standing
description: Process, standing corrections, inventory, and reading order for this directory.
owns: [process, standing corrections, inventory, reading order]
---

# meridian-rs `docs/`

## Docs define law; code follows

1. **Doc correct > code correct.** These files teach the accurate design. If code, goldens, or MCP schemas disagree, the **document wins**.
2. **Docs first.** Material change updates the correct doc **before** code.
3. **One standing wire contract:** `wire-contract.md` only (no v2/v3 stack). It is authored in this file — edit it directly, docs-first.
4. **Self-contained tree.** Standing docs cite **only other files in this directory** (plus in-repo `crates/` paths for implementers). No out-of-tree markdown as an authority.
5. **No dated markers.** State the current fact in positive form; history lives in git.

## Standing corrections (always on)

| | Law |
|---|---|
| **A — Address** | Machine address is **segments only**: `{"hpath":[{"h":"Goals"},{"h":"Q3"}]}` (optional `n`, or `anchor` / `fm_key`). Never joined `Goals>Q3` / `Goals/Q3` as writeable form. |
| **B — Receipts** | Armed **wire** facts are normative. Default md template must not publish a second path. |
| **C — View organ** | DuckDB / `view_path` / SQL boards are **not** agent core. |

| Topic | Law |
|---|---|
| Write | Only write op is `splice`. CLI append = `put{at:"end"}` |
| Mint | `toc` / `cat` / `read` mint; `resolve` is walk plane (**no rev**) |
| Content hash | Wire noun is **`fingerprint`** (`b3:…`) |
| Spans | No client spans in requests |
| Op reality | `check_write` is a consumed wire op (standalone splice verdict, read-only — wire-contract § A.3); `sub` is SERVED at the daemon door (wire-contract §4.7), not a reserved/future shape |

## Files in this directory

| File | Role |
|---|---|
| `README.md` | Process, standing corrections, inventory |
| `wire-contract.md` | Standing wire constitution |
| `laws.md` | Architecture laws + crate charters |
| `release.md` | What a release promises (two-key rule) + stamp/tag mechanics |
| `address-grammar.md` | Cross-root / mount / `addr::Addr` |
| `meridian-md-schema.md` | `MERIDIAN.md` config parse |
| `node-rev-merkle-spec.md` | `node_rev` + merkle encoding (+ `.assets/`) |
| `fingerprint-norm-spec.md` | Fingerprint CID-token + norm-v2 algorithm |
| `armed-plane.md` | Arming ladder + `gate()` seam |
| `base-projection.md` | `.base` (Obsidian Bases) projection into the sql face: membership, relations, `base_fold` witness |
| `body-projection.md` | Section body text in the sql face: exclusive-chunk law, `body` relation, content-addressed cache protocol |
| `run-plane.md` | Run plane + preset/session birth |
| `status.md` | CLI / build **descriptive** snapshot |
| `doc-system.md` | How this directory is organized and maintained |

## Reading order

1. This README  
2. `wire-contract.md`  
3. `laws.md` if editing crates  
4. `status.md` only for “what the binary exposes today”  
5. `release.md` only when cutting or consuming a release  
