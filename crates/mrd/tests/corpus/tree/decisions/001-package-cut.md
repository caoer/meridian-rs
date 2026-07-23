---
type: decision
status: decided
task: "[[crate-architecture-design]]"
owner: ZT
created_by: c3b85275
created: 2026-07-17
tags: [type/decision, topic/meridian-rs]
---
# How to cut the Rust codebase into packages → 10 packages (laws-as-crates)

**Decided + executed.** Jump to: [github.com/caoer/meridian-rs](https://github.com/caoer/meridian-rs) main @ `4736ff1` · [[meridian-rs-crate-architecture]] (design + execution ledger) · [[crate-architecture-review-synthesis]] (why reviewers were split).

You picked compiler-enforced walls. 11 packages live including the `wire-map` converter all 4 reviewers required; all tests green against the frozen wire contract.
