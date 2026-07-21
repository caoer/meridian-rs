//! The resident per-workspace query engine (decision 0002 spine root, U1).
//!
//! The daemon holds a warm [`WorkspaceEngine`] per workspace — the parsed
//! corpus (`model::CorpusIndex` + document map) plus the corpus content hash it
//! was built at. [`Registry::warm_or_build`](crate::Registry::warm_or_build)
//! rebuilds a workspace's engine ONLY when its corpus content hash changes; an
//! unchanged hash reuses the warm state and parses nothing.
//!
//! # Doctrine (decision 0002)
//! Resident state is a disposable projection of disk: no persistence, no
//! recovery machinery. A cold daemon holds no engines; the first
//! `warm_or_build` for a workspace rebuilds from disk. Eviction is NOT
//! engineered here — the daemon's existing idle-reap (5-day `last_use` horizon,
//! [`Registry::reap`](crate::Registry::reap)) drops a warm engine when it drops
//! the workspace's registration (risk R4: generous residency, no memory budget).
//!
//! # The reuse key (risk R5)
//! The fingerprint is the corpus CONTENT hash — `fs::domain_snapshot`'s folded
//! `model::MerkleRoot`, the world-grain the commit guards and `diff` already
//! compute fresh from disk. It is NOT the (unimplemented) workspace-identity
//! Merkle; U1 never builds that.

use std::collections::BTreeMap;

/// The warm per-workspace engine state: the parsed corpus that `query` reads
/// (U2), plus the corpus content hash it was built at (the reuse key).
///
/// Model state — derived, disposable (decision 0002; `model` law 2). Never
/// persisted: a restart rebuilds it from disk on the first `warm_or_build`.
#[derive(Debug)]
pub struct WorkspaceEngine {
    /// The vault name/alias index the `query` read ops borrow (U2).
    pub index: model::CorpusIndex,
    /// The parsed documents, keyed by workspace-relative path.
    pub docs: BTreeMap<String, model::Document>,
    /// The corpus content hash the index + docs were built at — the reuse key.
    pub at_fingerprint: model::MerkleRoot,
}

/// Whether `warm_or_build` reused the warm engine or rebuilt it.
///
/// This is the parse-count evidence the U1 gate asserts on: `Built { docs }`
/// reports the number of documents parsed (one `syntax::parse` per doc), and
/// `Reused` PROVES zero parses ran — the parse-heavy `fs::build_corpus` is only
/// reached on the rebuild branch, so a `Reused` result cannot have parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WarmOutcome {
    /// The corpus content hash was unchanged; the warm engine was reused and
    /// nothing was parsed.
    Reused,
    /// The corpus content hash changed (or the workspace was cold); the engine
    /// was rebuilt. `docs` is the number of documents parsed.
    Built { docs: usize },
}
