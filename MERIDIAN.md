---
description: the meridian engine — corpus/wire/run/script planes and the mrd binary; docs/ carries the standing design law (wire-contract, run-plane, laws)
type: meridian-root
version: 1
name: meridian-rs
---

# meridian-rs

This root's **self-declaration**: the canonical name this tree answers to in a
cross-root address (`meridian-rs:some/page.md`).

The name belongs to the root, not to any one machine's config. A machine's
`~/MERIDIAN.md` **binds** a name to a path — it does not baptize the root
(`decisions/2026-07-24-cross-root-addressing.md` §1a, *"MERIDIAN.md binds, it
doesn't baptize"*). A mount table that binds this tree under any other name
fails loud rather than picking a winner, so a link written into shared content
resolves to the same tree on every machine.

Read by meridian-rs `crates/config/src/mount.rs`. Additive: nothing in this
repo reads it, and removing it only returns this root to `grey(undeclared)`.
