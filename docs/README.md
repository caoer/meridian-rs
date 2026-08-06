# meridian-rs `docs/`

## Docs define law; code follows

1. **Doc correct > code correct.** These files teach the accurate design. If code, goldens, or MCP schemas disagree, the **document wins**.
2. **Docs first.** Material change updates the correct doc **before** code.
3. **One standing wire contract:** `wire-contract.md` only (no v2/v3 stack).
4. **Self-contained tree.** Standing docs cite **only other files in this directory** (plus in-repo `crates/` paths for implementers). No wiki decisions, session result files, or out-of-tree markdown as authorities.
5. **Optional log:** `worker-log.md` holds time-sensitive provenance. **Safe to delete** — design docs must read correctly without it.

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

## Files in this directory

| File | Role |
|---|---|
| `README.md` | Process, standing corrections, inventory |
| `wire-contract.md` | Standing wire constitution |
| `laws.md` | Architecture laws + crate charters |
| `address-grammar.md` | Cross-root / mount / `addr::Addr` |
| `meridian-md-schema.md` | `MERIDIAN.md` config parse |
| `node-rev-merkle-spec.md` | `node_rev` + merkle encoding (+ `.assets/`) |
| `fingerprint-norm-spec.md` | Fingerprint CID-token + norm-v2 algorithm |
| `armed-plane.md` | Arming ladder + `gate()` seam |
| `run-plane.md` | Run plane + preset/session birth |
| `status.md` | CLI / build **descriptive** snapshot |
| `worker-log.md` | **Optional** history / provenance — deletable |

## Reading order

1. This README  
2. `wire-contract.md`  
3. `laws.md` if editing crates  
4. Task SPECs as needed  
5. `status.md` only for “what the binary exposes today”  

Do **not** start from `worker-log.md`.
