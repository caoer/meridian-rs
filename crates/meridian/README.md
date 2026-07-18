# meridian

**Charter:** the markdown world model as one lib — dialect syntax, governed node
tree, and disk truth as internal modules (`syntax` / `model` / `fs`); every op the
sidecar answers is a call into this curated surface. Never: protocol types, framing,
body formatting (ccc-mdformat's, permanently), executing agent work, persistence.

Module charters + rung map: `src/lib.rs` and each module's doc comment.

**Three laws:** (1) Rust is the world model, Go is the actor. (2) Disk = markdown,
the only durable truth; this lib's memory is disposable — no snapshot files, no
second database. (3) One-way arrows: files → Rust → Go; recovery is re-derive from
the layer below.
