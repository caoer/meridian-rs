# testsuite

**Charter:** consolidated integration-test member — one test binary (matklad
doctrine) holding the frozen GT pack as data (`data/gt/`, provenance in its `FROZEN`
header), wire-level golden pairs, and every cross-crate test. The pack is read-only
by law: disputes go to lane 0, never silent edits.

**Three laws:** (1) Rust is the world model, Go is the actor — fixtures test the
model's answers, never actor behavior. (2) Disk = markdown, the only durable truth —
the GT pack is disk truth for tests, frozen. (3) One-way arrows: files → Rust → Go.
