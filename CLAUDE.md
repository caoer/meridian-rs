# meridian-rs

**The only markdown engine (end-state ruling, 2026-07-22, ratified).**
meridian-rs owns the markdown law: parse, rev, merkle chains, strict-writer
splice, receipts, attestation, and the run plane. Everything that computes
over markdown lands HERE, never in meridian-go.

- **ccc-statusd** is this engine's customer — a thin MCP/orchestration face
  that calls over the wire and holds zero markdown semantics.
- **meridian-go (`md`)** is a bridge: it dies leg-by-leg as the wire contract
  reaches parity. Its five correct-context verbs (attest / chain / status /
  walk / realise) and its run plane migrate here; the parity gate per leg is
  the 22-07 replay harness (byte-identical receipts, verbatim refusals).
- Full ruling + cutover sequence: llm-wiki
  `decisions/2026-07-22-meridian-go-end-state.md`.

Docs: `docs/laws.md` (crate charters), `docs/wire-contract-v2.md` +
`docs/wire-contract-v3-amendment.md` (the client seam),
`docs/node-rev-merkle-spec.md` (rev/hash law), `docs/status.md` (mrd CLI).
