//! Corpus reads: backlinks, the §4.6 edge map, span-exact rename planning —
//! borrows the model's index, applies nothing.
//!
//! # Charter
//! **Owns:** corpus-level reads. The outgoing edge map (contract §4.6 `links` — the app's
//! `resolvedLinks`/`unresolvedLinks` shape, per-edge counts), backlinks (find-references
//! over wikilinks/embeds), board queries, and rename *planning* — the corpus-wide,
//! span-exact wikilink-rewrite plan (depth/anchor/alias-preserving).
//!
//! **Never does:** apply edits (a rename plan is a list of splices; application goes
//! through `model` validation + `fs` execution like every other write), own the corpus
//! index (borrowed from `model` — sibling of `policy`, no edge between them), serialize
//! (no-serde law: `wire` twins these shapes; the serving host converts).
//!
//! Edge-map resolution is the walk plane's stage 1 only
//! (`CorpusIndex::resolve_linkpath` — `getFirstLinkpathDest` parity, contract §4.5): the
//! app's `resolvedLinks` counts a link toward its destination file; heading/block
//! fragments never split an edge.

use std::collections::BTreeMap;

use addr::MountSet;
use model::{
    ByteSpan, CorpusIndex, Document, Edit, EditKind, HpathSeg, Node, NodeKind, Ref, RefResolution,
    RootedCorpus, SpliceRequest,
};

/// One inbound reference: which file, where, linking how.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Backlink {
    pub path: String,
    pub span: ByteSpan,
}

/// One file's outgoing edges (contract §4.6): per-edge counts, dangling refs
/// first-class. `resolved` keys are destination corpus paths in the ambient
/// root; `unresolved` keys are the raw linkpaths as written (no vault file
/// exists to name). The model-side twin of `wire::FileLinks` (no-serde law).
///
/// The two cross-root channels are separate maps: a rooted destination is a
/// root and a path, and a joined `root:path` key in `resolved` would put a
/// string address in a machine surface — and collide, since a corpus may hold
/// a real ambient file whose path contains a colon.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FileLinks {
    pub resolved: BTreeMap<String, u64>,
    pub unresolved: BTreeMap<String, u64>,
    /// Edges that resolved inside a mounted root — keyed by root, then by the
    /// path inside that root. A consumer fetching bytes must read the root to
    /// know which corpus to fetch from; resolving the path against the ambient
    /// corpus fetches the wrong bytes.
    pub resolved_rooted: BTreeMap<addr::MountName, BTreeMap<String, u64>>,
    /// Edges the address owner refused, keyed by the linkpath as written.
    /// Distinct from `unresolved`: an ambient dangling link is ordinary
    /// authoring state; a refused cross-root edge means the author believed a
    /// mount relationship that does not hold — or wrote something that is not
    /// an address at all.
    pub refused: BTreeMap<String, RefusedEdge>,
}

/// One refused edge: the address owner's own answer, carried structurally, plus
/// how many times the page wrote it. Kept as [`model::RefResolution`] rather
/// than flattened to a word — rendering is the colour plane's job
/// (`view::walk`), and a second classification here is how one address comes to
/// have two answers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefusedEdge {
    pub resolution: RefResolution,
    pub count: u64,
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
    links_with(index, docs, &RootedCorpus::ambient(docs), None, path)
}

/// [`links`] against a root-keyed corpus and a mount table — the form that
/// asks the address owner ([`CorpusIndex::resolve_ref`]).
///
/// The ambient shorthand above is the same function with no mount authority,
/// so there is one precedence here rather than two that agree until one is
/// edited.
#[must_use]
pub fn links_rooted(
    index: &CorpusIndex,
    docs: &BTreeMap<String, Document>,
    corpus: &RootedCorpus<'_>,
    mounts: &MountSet,
    path: Option<&str>,
) -> BTreeMap<String, FileLinks> {
    links_with(index, docs, corpus, Some(mounts), path)
}

/// The one implementation behind [`links`] and [`links_rooted`], distinguished
/// by whether the caller holds mount authority.
///
/// `None` is not an empty table: an empty table means this machine binds
/// nothing, so a rooted address refuses grey `unmounted`; `None` means the
/// caller (the resident daemon) never consulted a table at all, so a rooted
/// spelling is reported `unresolved`, first-class, no refusal (see
/// `mrd::engine::answer_links`).
fn links_with(
    index: &CorpusIndex,
    docs: &BTreeMap<String, Document>,
    corpus: &RootedCorpus<'_>,
    mounts: Option<&MountSet>,
    path: Option<&str>,
) -> BTreeMap<String, FileLinks> {
    docs.iter()
        .filter(|(source, _)| path.is_none_or(|p| p == source.as_str()))
        .map(|(source, doc)| {
            (
                source.clone(),
                file_links(index, source, doc, corpus, mounts),
            )
        })
        .collect()
}

/// One document's outgoing edges, resolved against `index`/`corpus`.
///
/// Split out of [`links_with`] because `source` need not be a corpus MEMBER:
/// a path outside the hash domain is addressable by explicit path (§12.1), so
/// the door that is asked about one page loads it and resolves its edges
/// against the corpus it is not in. Membership decides what the enumeration
/// walks, never what a named path is entitled to.
#[must_use]
pub fn file_links(
    index: &CorpusIndex,
    source: &str,
    doc: &Document,
    corpus: &RootedCorpus<'_>,
    mounts: Option<&MountSet>,
) -> FileLinks {
    let mut entry = FileLinks::default();
    for (target, _span) in link_nodes(doc) {
        match resolve_edge(index, source, target, corpus, mounts) {
            RefResolution::Ambient(dest) => *entry.resolved.entry(dest).or_insert(0) += 1,
            RefResolution::Rooted { root, path } => {
                *entry
                    .resolved_rooted
                    .entry(root)
                    .or_default()
                    .entry(path)
                    .or_insert(0) += 1;
            }
            // An ambient miss stays first-class and non-refusing — the
            // ordinary authoring state.
            RefResolution::NotFound { root: None, .. } => {
                *entry.unresolved.entry(target.to_string()).or_insert(0) += 1;
            }
            // Everything else is the address owner refusing, and the
            // refusal is carried whole rather than collapsed to a word.
            resolution => {
                let slot = entry
                    .refused
                    .entry(target.to_string())
                    .or_insert(RefusedEdge {
                        resolution,
                        count: 0,
                    });
                slot.count += 1;
            }
        }
    }
    entry
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
    backlinks_with(index, docs, &RootedCorpus::ambient(docs), None, target)
}

/// [`backlinks`] against a root-keyed corpus and a mount table.
///
/// `target` is an ambient corpus path: this enumerates one corpus, so a
/// cross-vault link is visible from the source side only. Threading the mount
/// table here keeps a rooted edge from being counted as an ambient backlink to
/// the same basename.
#[must_use]
pub fn backlinks_rooted(
    index: &CorpusIndex,
    docs: &BTreeMap<String, Document>,
    corpus: &RootedCorpus<'_>,
    mounts: &MountSet,
    target: &str,
) -> Vec<Backlink> {
    backlinks_with(index, docs, corpus, Some(mounts), target)
}

/// The one implementation behind [`backlinks`] and [`backlinks_rooted`] — see
/// [`links_with`] for what `None` mount authority means and why it is not an
/// empty table.
fn backlinks_with(
    index: &CorpusIndex,
    docs: &BTreeMap<String, Document>,
    corpus: &RootedCorpus<'_>,
    mounts: Option<&MountSet>,
    target: &str,
) -> Vec<Backlink> {
    docs.iter()
        .flat_map(|(source, doc)| {
            link_nodes(doc)
                .into_iter()
                .filter(|(linkpath, _)| {
                    matches!(
                        resolve_edge(index, source, linkpath, corpus, mounts),
                        RefResolution::Ambient(dest) if dest == target
                    )
                })
                .map(|(_, span)| Backlink {
                    path: source.clone(),
                    span,
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

/// Edge resolution, split on one lexical question —
/// [`addr::head_carries_root_separator`]: could the address plane have
/// something to say about this spelling?
///
/// - No ⇒ the ambient plane, `resolve_linkpath` (§4.6 `getFirstLinkpathDest`
///   parity, stage 1 only). Deliberately not the pin plane's three rules:
///   routing ambient spellings through [`CorpusIndex::resolve_ref`] would
///   silently widen the contract this crate's header pins.
/// - Yes ⇒ the address owner, [`CorpusIndex::resolve_ref`], which peels the
///   root, looks it up in the mount table, and runs the three rules against
///   that root's own corpus.
///
/// The same predicate gates all three sites (`resolve_linkpath`'s guard, this
/// split, the CLI's in-process degrade); two predicates that merely agree
/// drift the moment either is edited. The guard is what stops a basename
/// fallback answering the ambient root's file for a rooted address.
///
/// An empty linkpath is the source itself (stage-1 parity, a self-reference).
fn resolve_edge(
    index: &CorpusIndex,
    source: &str,
    linkpath: &str,
    corpus: &RootedCorpus<'_>,
    mounts: Option<&MountSet>,
) -> RefResolution {
    if linkpath.is_empty() {
        return RefResolution::Ambient(source.to_string());
    }
    // No mount authority ⇒ no standing to classify a rooted spelling: it falls
    // through to the ambient plane, whose head-colon guard refuses it, and is
    // reported `unresolved`.
    let Some(mounts) = mounts else {
        return match index.resolve_linkpath(linkpath, source) {
            Some(dest) => RefResolution::Ambient(dest),
            None => RefResolution::NotFound {
                root: None,
                path: linkpath.to_owned(),
                selector: None,
            },
        };
    };
    if !addr::head_carries_root_separator(linkpath) {
        return match index.resolve_linkpath(linkpath, source) {
            Some(dest) => RefResolution::Ambient(dest),
            // The §4.6 edge map keys a dangling edge by the linkpath as
            // written, and a wikilink's fragment is carried beside its target
            // rather than inside it — so there is no selector to report here.
            None => RefResolution::NotFound {
                root: None,
                path: linkpath.to_owned(),
                selector: None,
            },
        };
    }
    index.resolve_ref(linkpath, source, corpus, mounts)
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
/// it is the caller's loop through model-validate + fs-execute. Each entry is one
/// `(path, batch)` splice; the list order is load-bearing (see [`plan_rename`]).
#[derive(Debug, Clone, PartialEq)]
pub struct RenamePlan {
    pub edits: Vec<(String, SpliceRequest)>,
}

/// Plan an in-file heading rename with corpus backlink fixup (meridian's `mv`
/// for a heading). `from_path` is the file holding the heading, `from` its
/// section ref (an `hpath`, or an `anchor` whose host section is renamed), `to`
/// the new heading text. Nothing is applied — the returned `edits` are
/// `(path, batch)` splices the caller runs through `model::validate_batch` +
/// `fs::apply_batch`, like every other write.
///
/// Two preserved properties:
/// - **Block ids survive.** The heading edit is a `match` over the heading line
///   only (`## Old` → `## New`); every `^block-id` in the section body (and any
///   `^id` trailing the heading line) is byte-untouched.
/// - **hpath pins redden, block pins keep their rev.** A dependent pinning
///   `from_path#Old` resolves to nothing after the rename →
///   `red(selector-unresolved)`; one pinning a body `^block-id` still resolves,
///   at an unchanged rev (`model::selector::resolve_selector`). Backlink fixup
///   rewrites `[[…]]` wikilinks, never lock pins.
///
/// Edit order is load-bearing: every backlink rewrite first, the heading rename
/// last — a self-referential link inside the renamed section is addressed by
/// the section's old hpath, which the heading rename would invalidate if it ran
/// first.
///
/// Stated limits (fail loud at apply, never silent corruption): renaming a
/// heading that has sub-sections refuses `would_corrupt` (the children's hpaths
/// would move); two byte-identical affected links in one section refuse
/// `not_unique`.
#[must_use]
pub fn plan_rename(
    index: &CorpusIndex,
    docs: &BTreeMap<String, Document>,
    from_path: &str,
    from: &Ref,
    to: &str,
) -> RenamePlan {
    let mut edits: Vec<(String, SpliceRequest)> = Vec::new();
    let Some(doc) = docs.get(from_path) else {
        return RenamePlan { edits };
    };
    // The section being renamed: the deepest `Section` containing the resolved
    // target byte (an hpath resolves to the section itself; an anchor to its host
    // block, whose enclosing section is the one renamed).
    let Ok(target) = model::resolve(doc, from) else {
        return RenamePlan { edits };
    };
    let Some(section) = enclosing_section(&doc.root, target.span.start) else {
        return RenamePlan { edits };
    };
    let (NodeKind::Section { heading_text, .. }, Some(section_hpath)) =
        (&section.kind, &section.hpath)
    else {
        return RenamePlan { edits };
    };
    let old_text = heading_text.clone();
    if to == old_text {
        return RenamePlan { edits };
    }
    let section_ref = hpath_ref(section_hpath);

    // Backlink fixup first (see order note): every corpus wikilink/embed whose
    // fragment is the renamed heading and resolves to `from_path`.
    for (src_path, src_doc) in docs {
        collect_backlink_edits(
            index, src_path, src_doc, from_path, &old_text, to, &mut edits,
        );
    }

    // Heading rename last: a `match` over the heading line (marker + text) so the
    // section body and any trailing `^id` on the line survive byte-for-byte.
    if let Some(edit) = heading_match_edit(doc, section, &old_text, to, section_ref) {
        edits.push((from_path.to_string(), single(edit)));
    }
    RenamePlan { edits }
}

/// The deepest `Section` node whose span contains `offset` (byte). A section's
/// heading byte resolves to the section itself; a body byte to the section it
/// lives under. `None` when `offset` sits above every heading (frontmatter or a
/// pre-heading preamble — un-addressable by the mint-plane hpath grammar).
fn enclosing_section(node: &Node, offset: usize) -> Option<&Node> {
    let mut found = if matches!(node.kind, NodeKind::Section { .. })
        && node.span.start <= offset
        && offset < node.span.end
    {
        Some(node)
    } else {
        None
    };
    for c in &node.children {
        if let Some(deeper) = enclosing_section(c, offset) {
            found = Some(deeper);
        }
    }
    found
}

/// The heading-line rename edit: a `match` that rewrites only the heading's
/// marker-plus-text run (`## Old` → `## New`) within the section's full span,
/// leaving the body (and any trailing `^id`) untouched. `None` when the heading
/// text is not found on the heading line (defensive — it always is for a live
/// `Section`).
fn heading_match_edit(
    doc: &Document,
    section: &Node,
    old_text: &str,
    to: &str,
    section_ref: Ref,
) -> Option<Edit> {
    let raw = &doc.raw;
    let span = &section.span;
    let line_end = raw[span.start..span.end]
        .find('\n')
        .map_or(span.end, |p| span.start + p);
    let heading_line = &raw[span.start..line_end];
    let pos = heading_line.find(old_text)?;
    let old_match = heading_line[..pos + old_text.len()].to_string();
    let new_match = format!("{}{}", &heading_line[..pos], to);
    Some(Edit {
        target: section_ref,
        edit: EditKind::Match {
            old: old_match,
            new: new_match,
        },
        if_node_rev: None,
    })
}

/// Every wikilink/embed in `doc` whose fragment is `old_text` and whose linkpath
/// resolves to `from_path` becomes one section-scoped `match` splice rewriting
/// `#old_text` → `#to` in the exact link text (embed `!`, `|alias`, and path all
/// preserved). Each rewrite is its OWN batch so two links in one section never
/// collide on a shared target span.
fn collect_backlink_edits(
    index: &CorpusIndex,
    src_path: &str,
    doc: &Document,
    from_path: &str,
    old_text: &str,
    to: &str,
    out: &mut Vec<(String, SpliceRequest)>,
) {
    fn walk(
        node: &Node,
        section_hpath: Option<&Vec<String>>,
        ctx: &BacklinkCtx<'_>,
        out: &mut Vec<(String, SpliceRequest)>,
    ) {
        let here = match (&node.kind, &node.hpath) {
            (NodeKind::Section { .. }, Some(hp)) => Some(hp),
            _ => section_hpath,
        };
        if let NodeKind::Wikilink {
            target, heading, ..
        }
        | NodeKind::Embed {
            target, heading, ..
        } = &node.kind
            && heading.as_deref() == Some(ctx.old_text)
            && resolves_to(ctx.index, ctx.src_path, target, ctx.from_path)
            && let Some(hpath) = here
        {
            let raw_link = &ctx.raw[node.span.clone()];
            let new_link =
                raw_link.replacen(&format!("#{}", ctx.old_text), &format!("#{}", ctx.to), 1);
            if new_link != raw_link {
                out.push((
                    ctx.src_path.to_string(),
                    single(Edit {
                        target: hpath_ref(hpath),
                        edit: EditKind::Match {
                            old: raw_link.to_string(),
                            new: new_link,
                        },
                        if_node_rev: None,
                    }),
                ));
            }
        }
        for c in &node.children {
            walk(c, here, ctx, out);
        }
    }
    let ctx = BacklinkCtx {
        index,
        src_path,
        from_path,
        old_text,
        to,
        raw: &doc.raw,
    };
    walk(&doc.root, None, &ctx, out);
}

/// The read-only context threaded through the backlink walk (keeps the recursive
/// arg list small — clippy's `too_many_arguments`).
struct BacklinkCtx<'a> {
    index: &'a CorpusIndex,
    src_path: &'a str,
    from_path: &'a str,
    old_text: &'a str,
    to: &'a str,
    raw: &'a str,
}

/// Does `linkpath` written in `source` resolve to `target_path`? Empty linkpath
/// (`[[#H]]`) is the source itself (stage-1 parity), matching [`resolve_edge`].
fn resolves_to(index: &CorpusIndex, source: &str, linkpath: &str, target_path: &str) -> bool {
    let dest = if linkpath.is_empty() {
        Some(source.to_string())
    } else {
        index.resolve_linkpath(linkpath, source)
    };
    dest.as_deref() == Some(target_path)
}

/// A single-edit batch (unguarded) — one `(path, batch)` entry of a [`RenamePlan`].
fn single(edit: Edit) -> SpliceRequest {
    SpliceRequest {
        if_root: None,
        edits: vec![edit],
        engine: None,
    }
}

/// A heading-path (`/`-joined text chain) as a mint-plane `hpath` ref.
fn hpath_ref(hpath: &[String]) -> Ref {
    Ref::Hpath(
        hpath
            .iter()
            .map(|h| HpathSeg {
                h: h.clone(),
                n: None,
            })
            .collect(),
    )
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

    /// `plan_rename` emits backlink fixups FIRST (a heading link `#Foo` → `#Bar`,
    /// alias-preserved) and the heading rename LAST (a `match` over the heading
    /// line only), never touching a `^block-id` link.
    #[test]
    fn plan_rename_emits_backlink_fixup_then_heading_edit() {
        let (index, docs) = corpus(&[
            ("page.md", "# Foo\n\nbody ^abc\n"),
            (
                "other.md",
                "# L\n\n[[page#Foo]] and [[page#Foo|note]] and [[page#^abc]]\n",
            ),
        ]);
        let plan = plan_rename(
            &index,
            &docs,
            "page.md",
            &hpath_ref(&["Foo".to_string()]),
            "Bar",
        );

        // Heading rename is LAST, over the heading LINE only.
        let (last_path, last_batch) = plan.edits.last().expect("a heading edit");
        assert_eq!(last_path, "page.md");
        assert!(
            matches!(&last_batch.edits[0].edit, EditKind::Match { old, new } if old == "# Foo" && new == "# Bar"),
            "heading edit rewrites the heading line: {:?}",
            last_batch.edits[0].edit
        );

        // Backlink fixups come first; the `^abc` block link is never rewritten.
        let backlink_news: Vec<&str> = plan.edits[..plan.edits.len() - 1]
            .iter()
            .map(|(p, b)| {
                assert_eq!(p, "other.md");
                match &b.edits[0].edit {
                    EditKind::Match { new, .. } => new.as_str(),
                    other @ EditKind::Put { .. } => panic!("backlink edit is a match: {other:?}"),
                }
            })
            .collect();
        assert!(
            backlink_news.contains(&"[[page#Bar]]"),
            "bare link rewritten"
        );
        assert!(
            backlink_news.contains(&"[[page#Bar|note]]"),
            "aliased link rewritten, alias preserved"
        );
        assert!(
            !backlink_news.iter().any(|n| n.contains("^abc")),
            "the ^block-id link is never a rename edit: {backlink_news:?}"
        );
    }
}
