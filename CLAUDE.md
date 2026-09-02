# meridian-rs

The markdown engine: parse, rev, merkle chains, strict-writer splice, receipts,
attestation, and the run plane. Everything that computes over markdown lives
here; clients (an MCP server, an editor, a script) drive the wire and hold no
markdown semantics.

- **Docs-first (binding):** accurate design lives in `docs/`. **Doc correct >
  code correct.** A material change updates the correct doc **before** code.
- Where the specs live: `docs/README.md` (process, standing corrections,
  inventory), `docs/wire-contract.md` (the **only** wire constitution — no
  v2/v3 stack), `docs/laws.md` (crate charters), `docs/node-rev-merkle-spec.md`
  (rev/hash law), `docs/run-plane.md` (run plane), `docs/status.md` (mrd CLI,
  descriptive only). Standing docs cite only files in this repo.
- Address law: machine addresses are **segments only** (`hpath` arrays,
  `anchor`, `fm_key`), never a joined `Goals/Q3` string. The DuckDB `sql` face
  is operator convenience, not agent core.
- **CI gates:** Woodpecker (`.woodpecker/ci.yaml`) runs `cargo fmt --all
  --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test
  --workspace --locked`, `cargo deny check`, and the perfsuite smoke. Land by
  PR; a red lane blocks the publish step.
