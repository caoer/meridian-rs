//! **Charter:** consolidated integration-test member — the frozen GT pack as
//! data, wire-level golden pairs, and every cross-crate test, in one test
//! binary (matklad, "Delete Cargo Integration Tests": consolidating cut
//! compile time 3x, artifacts 5x).
//!
//! `data/gt/` is the frozen parser-bench GT subset (provenance in
//! `data/gt/FROZEN`, frozen 2026-07-17): the 10 ground-truth expectation
//! files plus their 8 corpus + 2 adversarial source documents, at their
//! fixture-relpaths so each expectation's `file` field resolves in-tree.
//! Read-only by law: disputes go to lane 0, never silent edits. The full
//! 206-file corpus sweep + remaining adversarial pack stay in parser-bench
//! (2.5 MB + a 10 MB adversarial file don't belong in this repo; the sweep
//! is a bench, not a unit gate).
//!
//! Known vintage note: the GT pack predates two contract revisions — its
//! `hpath` is a `/`-joined string (the wire ships an array) and its anchor
//! charset lacks `_`. The loader keeps `hpath` untyped for that reason; the
//! full GT harness normalizes when the parse core lands.

use std::path::{Path, PathBuf};

use serde::Deserialize;

/// One expected node, typed against the `wire` crate where the vocabulary is
/// shared — deserializing `kind` into [`wire::NodeKind`] is itself a test
/// that the wire enum matches the frozen pack.
#[derive(Debug, Deserialize)]
pub struct GtNode {
    pub kind: wire::NodeKind,
    pub span: wire::Span,
    pub text_prefix_16b: String,
    /// `/`-joined string in the pack (pre-contract vintage; see module doc).
    #[serde(default)]
    pub hpath: Option<serde_json::Value>,
    #[serde(default)]
    pub unterminated: Option<bool>,
}

/// One ground-truth expectation file: a source document and its node
/// inventory.
#[derive(Debug, Deserialize)]
pub struct GtCase {
    /// Fixture-relpath, resolved against `data/gt/`.
    pub file: String,
    pub nodes: Vec<GtNode>,
}

/// `data/gt/` at build-tree location — tests run from the workspace, data
/// lives with this member.
pub fn gt_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("data/gt")
}

/// Load every `*.expected.json` in the pack, with its source document's raw
/// bytes. Panics on any malformed pack file: the pack is frozen input, not a
/// runtime surface.
pub fn load_gt_pack() -> Vec<(GtCase, String)> {
    let root = gt_root();
    let mut out = Vec::new();
    let mut entries: Vec<_> = std::fs::read_dir(root.join("ground-truth"))
        .expect("gt pack present")
        .map(|e| e.expect("readable dir entry").path())
        .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().ends_with(".expected.json")))
        .collect();
    entries.sort();
    for path in entries {
        let raw = std::fs::read_to_string(&path).expect("expectation file reads");
        let case: GtCase = serde_json::from_str(&raw)
            .unwrap_or_else(|e| panic!("{} parses as GtCase: {e}", path.display()));
        let src = std::fs::read_to_string(root.join(&case.file))
            .unwrap_or_else(|e| panic!("source {} loads: {e}", case.file));
        out.push((case, src));
    }
    out
}
