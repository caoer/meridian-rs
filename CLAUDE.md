# meridian-rs

**The only markdown engine (end-state ruling, 2026-07-22, ratified).**
meridian-rs owns the markdown law: parse, rev, merkle chains, strict-writer
splice, receipts, attestation, and the run plane. Everything that computes
over markdown lands HERE, never in meridian-go.

- **ccc-statusd** is this engine's customer — a thin MCP/orchestration face
  that calls over the wire and holds zero markdown semantics.
- **meridian-go (`md`)** is a bridge: it dies **leg-by-leg as each leg's
  REDESIGNED CONTRACT is implemented.** Its five correct-context verbs (attest
  / chain / status / walk / realise) and its run plane migrate here. **The
  remaining legs owe NO byte parity with meridian-go — they get redesigned
  contracts, not ports, and each leg's gate is its own contract's design
  tests.**
- ⚠️ **AMENDED 2026-07-26 — the previous "dies at wire parity" framing and the
  "22-07 replay harness (byte-identical receipts)" gate were NEVER RATIFIED
  CONTENT** (page elaboration under a `ratified` stamp) **and the gate was
  additionally unsatisfiable** — its atomic unit is a process invocation, while
  the remaining legs are in-process Go library calls that spawn none. Superseded
  text preserved verbatim at the source, not here: this file is always-loaded
  context, so the record lives once.
- Full ruling + cutover sequence, and the superseded wording: llm-wiki
  `decisions/2026-07-22-meridian-go-end-state.md` **§ Amendment (2026-07-26)**.
- **Addendum (2026-08-02, DX-01 ruling):** ZT ruled — typed, session
  `94485806`, verbatim: *"my yes pls only ratify meridian-go would die after
  the feature implemnted"* — that his "yes pls" ratified his own typed line
  only, never the assistant turn it answered. The amendment's outcome stands;
  its mechanism reads precisely as **scope-of-assent**, not page elaboration
  alone.

Docs: `docs/README.md` (process + standing corrections), `docs/wire-contract.md`
(the **only** wire constitution — no v2/v3 stack), `docs/laws.md` (crate
charters), `docs/node-rev-merkle-spec.md` (rev/hash law), `docs/status.md`
(mrd CLI, descriptive only).

- **Docs-first (binding):** accurate design lives in `docs/`. **Doc correct >
  code correct.** Material changes update the correct doc **before** code. Do
  not re-derive dual addresses or teach DuckDB/`view_path` as agent core.
