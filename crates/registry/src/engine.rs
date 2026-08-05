//! Resident per-workspace query engine (U1).
//!
//! Warm [`WorkspaceEngine`] per workspace: parsed corpus + content hash it was
//! built at. [`Registry::warm_or_build`](crate::Registry::warm_or_build)
//! rebuilds only when the hash changes; else reuses and parses nothing.
//!
//! Disposable projection of disk — no persistence. Eviction is idle-reap
//! ([`Registry::reap`](crate::Registry::reap)), not a separate policy (R4).
//! Reuse key is the corpus CONTENT hash (R5), not workspace-identity Merkle.

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

/// Whether `warm_or_build` reused or rebuilt. `Reused` proves zero parses
/// (`build_corpus` is rebuild-only). In-process evidence; not on the wire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WarmOutcome {
    /// Content hash unchanged; warm engine reused, nothing parsed.
    Reused,
    /// Hash changed or cold; rebuilt. `docs` = documents parsed.
    Built { docs: usize },
}
