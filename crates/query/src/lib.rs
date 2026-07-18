//! Rung-5 corpus reads stub: backlinks, board queries, span-exact rename
//! planning — borrows the model's index, applies nothing.
//!
//! # Charter
//! **Owns:** corpus-level reads. Backlinks (find-references over wikilinks),
//! board queries (session tree as database), and rename *planning* — the
//! corpus-wide, span-exact wikilink-rewrite plan (depth/anchor/alias-preserving,
//! meridian's `mv` relocated) that nothing in the stack has today.
//!
//! **Never does:** apply edits (a rename plan is a list of splices; application
//! goes through `model` validation + `fs` execution like every other write),
//! own the corpus index (borrowed from `model` — sibling of `policy`, no edge
//! between them).
//!
//! # Rungs
//! Rung 5 entirely. The crate exists from day one so additivity is shown as
//! code: it appears in the graph, nothing depends on it, and its arrival as a
//! real implementation reshuffles nothing (constraint 4).

use model::{CorpusIndex, Ref};

/// One inbound reference: which file, where, linking how.
#[derive(Debug, Clone, PartialEq)]
pub struct Backlink {
    pub path: String,
    pub span: model::ByteSpan,
}

/// Find-references: every wikilink/embed in the corpus resolving to `target`.
#[must_use]
pub fn backlinks(index: &CorpusIndex, target: &str) -> Vec<Backlink> {
    let _ = (index, target);
    todo!("rung 5: reverse lookup over the borrowed name index")
}

/// A planned corpus-wide rename: the spliceable edit set, span-exact. Applying
/// it is the caller's loop through model-validate + fs-execute.
#[derive(Debug, Clone, PartialEq)]
pub struct RenamePlan {
    pub edits: Vec<(String, model::SpliceRequest)>,
}

/// Plan a heading/file rename: every affected wikilink rewritten
/// depth/anchor/alias-preservingly, nothing applied.
#[must_use]
pub fn plan_rename(index: &CorpusIndex, from: &Ref, to: &str) -> RenamePlan {
    let _ = (index, from, to);
    todo!("rung 5: span-exact corpus rename planning")
}
