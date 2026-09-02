---
type: decision
status: decided
task: "[[crate-architecture-design]]"
owner: lead
created_by: a1b2c3d4
created: 2026-04-17
tags: [type/decision, topic/engine]
---
# How to cut the Rust codebase into packages → 10 packages (laws-as-crates)

**Decided + executed.** Jump to: [git.example.test/acme/engine](https://git.example.test/acme/engine) main @ `0a1b2c3` · [[engine-crate-architecture]] (design + execution ledger) · [[crate-architecture-review-synthesis]] (why reviewers were split).

You picked compiler-enforced walls. 11 packages live including the `wire-map` converter all 4 reviewers required; all tests green against the frozen wire contract.
