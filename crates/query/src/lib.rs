//! Rung-5 corpus reads: backlinks, the §4.6 edge map, span-exact rename
//! planning — borrows the model's index, applies nothing.
//!
//! # Charter
//! **Owns:** corpus-level reads. The outgoing edge map (contract §4.6 `links`
//! — the app's `resolvedLinks`/`unresolvedLinks` shape, per-edge counts),
//! backlinks (find-references over wikilinks/embeds), board queries (session
//! tree as database), and rename *planning* — the corpus-wide, span-exact
//! wikilink-rewrite plan (depth/anchor/alias-preserving, meridian's `mv`
//! relocated) that nothing in the stack has today.
//!
//! **Never does:** apply edits (a rename plan is a list of splices; application
//! goes through `model` validation + `fs` execution like every other write),
//! own the corpus index (borrowed from `model` — sibling of `policy`, no edge
//! between them), serialize (no-serde law: `wire` twins these shapes; the
//! sidecar converts).
//!
//! # Rungs
//! Rung 5 entirely. Edge-map resolution is the walk plane's **stage 1 only**
//! (`CorpusIndex::resolve_linkpath` — `getFirstLinkpathDest` parity, contract
//! §4.5): the app's `resolvedLinks` counts a link toward its destination FILE;
//! heading/block fragments never split an edge.

use std::collections::BTreeMap;

use model::{ByteSpan, CorpusIndex, Document, Node, NodeKind, Ref};

/// One inbound reference: which file, where, linking how.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Backlink {
    pub path: String,
    pub span: ByteSpan,
}

/// One file's outgoing edges (contract §4.6): per-edge counts, dangling refs
/// first-class. `resolved` keys are destination corpus paths; `unresolved`
/// keys are the raw linkpaths as written (no vault file exists to name).
/// The model-side twin of `wire::FileLinks` (no-serde law).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FileLinks {
    pub resolved: BTreeMap<String, u64>,
    pub unresolved: BTreeMap<String, u64>,
}

/// The §4.6 edge map over the borrowed corpus: every file's outgoing
/// wikilink/embed edges, resolved via the walk plane's stage 1 (see the crate
/// header). `path` present → that file's entry alone (the worked §4.6
/// exchange); absent → the whole-corpus edge map, one entry per corpus file —
/// link-less files carry empty maps, never vanish (the map names the corpus).
///
/// An empty linkpath (`[[#H]]`) is the source file itself (stage-1 parity),
/// so self-references count as resolved edges to their own file.
#[must_use]
pub fn links(
    index: &CorpusIndex,
    docs: &BTreeMap<String, Document>,
    path: Option<&str>,
) -> BTreeMap<String, FileLinks> {
    docs.iter()
        .filter(|(source, _)| path.is_none_or(|p| p == source.as_str()))
        .map(|(source, doc)| {
            let mut entry = FileLinks::default();
            for (target, _span) in link_nodes(doc) {
                match resolve_edge(index, source, target) {
                    Some(dest) => *entry.resolved.entry(dest).or_insert(0) += 1,
                    None => *entry.unresolved.entry(target.to_string()).or_insert(0) += 1,
                }
            }
            (source.clone(), entry)
        })
        .collect()
}

/// Find-references: every wikilink/embed in the corpus resolving to `target`
/// (a corpus path), in deterministic order — path-lexicographic, then span
/// order within a file.
#[must_use]
pub fn backlinks(
    index: &CorpusIndex,
    docs: &BTreeMap<String, Document>,
    target: &str,
) -> Vec<Backlink> {
    docs.iter()
        .flat_map(|(source, doc)| {
            link_nodes(doc)
                .into_iter()
                .filter(|(linkpath, _)| {
                    resolve_edge(index, source, linkpath).is_some_and(|dest| dest == target)
                })
                .map(|(_, span)| Backlink {
                    path: source.clone(),
                    span,
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

/// Stage-1 edge resolution: empty linkpath ⇒ the source itself; otherwise
/// `getFirstLinkpathDest` parity over the borrowed index.
fn resolve_edge(index: &CorpusIndex, source: &str, linkpath: &str) -> Option<String> {
    if linkpath.is_empty() {
        Some(source.to_string())
    } else {
        index.resolve_linkpath(linkpath, source)
    }
}

/// Every wikilink/embed in the document, span order. `NodeKind::Link`
/// (external markdown links) never edges — the app's `resolvedLinks` counts
/// vault links only.
fn link_nodes(doc: &Document) -> Vec<(&str, ByteSpan)> {
    let mut out = Vec::new();
    collect_links(&doc.root, &mut out);
    out.sort_by_key(|(_, span)| span.start);
    out
}

fn collect_links<'a>(node: &'a Node, out: &mut Vec<(&'a str, ByteSpan)>) {
    if let NodeKind::Wikilink { target, .. } | NodeKind::Embed { target, .. } = &node.kind {
        out.push((target.as_str(), node.span.clone()));
    }
    for c in &node.children {
        collect_links(c, out);
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn corpus(files: &[(&str, &str)]) -> (CorpusIndex, BTreeMap<String, Document>) {
        let mut index = CorpusIndex::new();
        let mut docs = BTreeMap::new();
        for (path, raw) in files {
            let doc = model::build((*raw).to_string(), syntax::parse(raw));
            index.insert(path, &doc);
            docs.insert((*path).to_string(), doc);
        }
        (index, docs)
    }

    fn counts(pairs: &[(&str, u64)]) -> BTreeMap<String, u64> {
        pairs.iter().map(|(k, v)| ((*k).to_string(), *v)).collect()
    }

    /// The §4.6 worked entry shape: one resolving link (basename, extension
    /// as written), one dangling — per-edge counts, dangling first-class.
    #[test]
    fn edge_map_splits_resolved_and_unresolved() {
        let (index, docs) = corpus(&[
            (
                "notes/plan.md",
                "# P\n\n- see [[2026-07-18]]\n- blocked on [[roadmap]]\n",
            ),
            ("receipts/2026-07-18.md", "# Receipts\n"),
        ]);
        let map = links(&index, &docs, Some("notes/plan.md"));
        assert_eq!(map.len(), 1, "path present → one entry");
        let entry = &map["notes/plan.md"];
        assert_eq!(entry.resolved, counts(&[("receipts/2026-07-18.md", 1)]));
        assert_eq!(entry.unresolved, counts(&[("roadmap", 1)]));
    }

    /// Whole-corpus (path absent): every file has an entry, link-less files
    /// carry empty maps; repeated edges count, embeds edge, external links do
    /// not, fragments never split an edge (stage-1 resolution).
    #[test]
    fn whole_corpus_edge_map_counts_per_edge() {
        let (index, docs) = corpus(&[
            (
                "a.md",
                "# A\n\n[[b]] and [[b#H|alias]] and ![[b]]\n[external](https://x.example)\n",
            ),
            ("b.md", "# H\n"),
        ]);
        let map = links(&index, &docs, None);
        assert_eq!(map.len(), 2, "the map names the corpus");
        assert_eq!(map["a.md"].resolved, counts(&[("b.md", 3)]));
        assert_eq!(map["a.md"].unresolved, counts(&[]));
        assert_eq!(map["b.md"], FileLinks::default());
    }

    /// Empty linkpath (`[[#H]]`) is the source itself — a resolved self-edge
    /// (stage-1 parity: `getFirstLinkpathDest("") ⇒ from`).
    #[test]
    fn empty_linkpath_is_a_self_edge() {
        let (index, docs) = corpus(&[("self.md", "# H\n\n[[#H]]\n")]);
        let map = links(&index, &docs, None);
        assert_eq!(map["self.md"].resolved, counts(&[("self.md", 1)]));
    }

    /// Backlinks is the reverse read of the same edges: deterministic
    /// (path, span) order, alias/fragment/embed forms all found, dangling
    /// refs never phantom-match.
    #[test]
    fn backlinks_reverse_lookup_is_span_exact() {
        let (index, docs) = corpus(&[
            ("a.md", "# A\n\nsee [[target]] twice: [[target#H|t]]\n"),
            ("b.md", "# B\n\n![[target]] and [[missing]]\n"),
            ("target.md", "# H\n"),
        ]);
        let got = backlinks(&index, &docs, "target.md");
        assert_eq!(got.len(), 3);
        assert!(got.iter().all(|b| {
            let text = &docs[&b.path].raw[b.span.clone()];
            text.contains("[[target") // span-exact: each hit IS a link node
        }));
        assert_eq!(
            (got[0].path.as_str(), got[1].path.as_str()),
            ("a.md", "a.md")
        );
        assert!(got[0].span.start < got[1].span.start, "span order in-file");
        assert_eq!(got[2].path, "b.md");
        assert_eq!(backlinks(&index, &docs, "missing.md"), vec![]);
    }
}
