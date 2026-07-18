# wire

**Charter:** the only Go-visible surface — wire-contract-v1 vocabulary
(`path`/`span`/`node_rev`/`root`), node schema, op/req/resp/error types, `Message`
envelope, and the `Codec` framing seam, all in one crate. Never: business logic,
I/O, parsing markdown, knowing what an op means.

**Graduation claim:** JSON-RPC/LSP framing lands as a second `Codec` impl in THIS
crate, consumers untouched. Rung map + contract pin: `src/lib.rs`.

**Three laws:** (1) Rust is the world model, Go is the actor — this crate is the one
place they speak. (2) Disk = markdown, the only durable truth; nothing here persists.
(3) One-way arrows: files → Rust → Go; events are a latency optimization, never a
correctness dependency.
