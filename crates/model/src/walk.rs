//! The walk plane — Obsidian interop resolution (contract §4.5, decision 013).
//!
//! `resolve` alone accepts the app's loose ref algebra; this module reproduces
//! it **best-effort, app-compatible** (decision 013: a one-way compatibility
//! floor, not a two-way parity law). Two stages, per the frozen behavior spec:
//!
//! - **Stage 1** — `getFirstLinkpathDest(linkpath, from)` parity over the vault
//!   name/alias index in [`CorpusIndex`]: basename + frontmatter aliases,
//!   case-insensitive, source-relative shortest-unambiguous, unresolved
//!   first-class. Empty linkpath ⇒ the source file itself.
//! - **Stage 2** — the subpath walk: split on `#` only; a heading path walked
//!   case-insensitively, strictly-deeper-level, anywhere-after (a closing
//!   heading never stops the walk), generation-skipping, first-match-wins on
//!   duplicate headings (silent); or a `^id` block lookup, last-wins on
//!   duplicates (silent) — the app's map-overwrite, contract §2.1.
//!
//! The ruled grammar always wins: input outside it never reaches here (the mint
//! plane refuses `bad_request` first). A `_`-bearing block id, though, is not a
//! grammar error on this plane — it is simply an id the app never indexes, so
//! the walk misses (`ref_not_found`, stage 2), reproducing the app's silent
//! drop. Both dispositions conform (decision 013 consequence 3).
//!
//! **D-C2 (frozen §4.5):** the return type carries **location facts only** —
//! `dest` + `span`, never a `NodeRev`. The mint partition is a type-level fact:
//! a ref cannot arm a write because the type that carries it cannot express a
//! rev. The `no_rev_field` compile test pins this.

use crate::{ByteSpan, CorpusIndex, Document, Node, NodeKind};
use std::collections::BTreeMap;

/// A resolved walk location — **location facts only** (D-C2: no rev field, ever).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Location {
    /// The resolved vault path (corpus-relative).
    pub dest: String,
    /// The byte range within `dest`.
    pub span: ByteSpan,
}

/// Which stage a walk reached — observable in every failure transcript (§4.5:
/// `dest` rides every stage-2 outcome, success or failure).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// `getFirstLinkpathDest` — the linkpath resolved to no file (no `dest`).
    One,
    /// The subpath walk — the file was found, the subpath was not.
    Two,
}

impl Stage {
    /// The wire's 1-based stage number (`1`/`2`).
    #[must_use]
    pub fn number(self) -> u8 {
        match self {
            Stage::One => 1,
            Stage::Two => 2,
        }
    }
}

/// A walk that resolved nothing — the failing stage plus, at stage 2, the file
/// it got to (`ref_not_found{stage, dest?}`). Stage 1 carries no `dest`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Miss {
    /// The stage the walk failed at.
    pub stage: Stage,
    /// The file reached before failing — `Some` at stage 2, `None` at stage 1.
    pub dest: Option<String>,
}

/// Walk an Obsidian linktext (`path#sub`, `#sub`, `path#^id` — no brackets, the
/// interface strips `[[…|alias]]` sugar first) against the corpus, source file
/// `from`. Best-effort app-compatible (§4.5); output is location facts only.
///
/// # Errors
/// [`Miss`] with `stage: One` (no `dest`) when the linkpath resolves to no file;
/// `stage: Two` (with `dest`) when the file is found but the subpath is not.
pub fn walk(
    index: &CorpusIndex,
    docs: &BTreeMap<String, Document>,
    from: &str,
    link: &str,
) -> Result<Location, Miss> {
    let (path, subpath) = parse_linktext(link);

    // Stage 1: linkpath → file. Empty linkpath is the source file itself.
    let dest = if path.is_empty() {
        from.to_string()
    } else {
        index.resolve_linkpath(path, from).ok_or(Miss {
            stage: Stage::One,
            dest: None,
        })?
    };
    let Some(doc) = docs.get(&dest) else {
        return Err(Miss {
            stage: Stage::One,
            dest: None,
        });
    };

    // Stage 2: subpath walk over the resolved file.
    match stage2(doc, subpath) {
        Some(span) => Ok(Location { dest, span }),
        None => Err(Miss {
            stage: Stage::Two,
            dest: Some(dest),
        }),
    }
}

/// `parseLinktext` parity: split on the FIRST `#` into `(linkpath, subpath)`.
/// The returned subpath excludes that leading `#`. No `#` ⇒ empty subpath.
fn parse_linktext(link: &str) -> (&str, &str) {
    match link.find('#') {
        Some(i) => (&link[..i], &link[i + 1..]),
        None => (link, ""),
    }
}

/// The stage-2 subpath walk. Empty subpath ⇒ the whole file; `^id` ⇒ block
/// lookup; otherwise a `#`-split heading path.
fn stage2(doc: &Document, subpath: &str) -> Option<ByteSpan> {
    if subpath.is_empty() {
        return Some(doc.root.span.clone());
    }
    if let Some(id) = subpath.strip_prefix('^') {
        return block_span(doc, id);
    }
    let segs: Vec<&str> = subpath.split('#').collect();
    walk_headings(&headings(doc), &segs)
}

/// Every heading as `(level, text, span)` in document order — the flat list the
/// walk scans (the walk is anywhere-after, so it is NOT scoped to the tree).
fn headings(doc: &Document) -> Vec<(u8, &str, ByteSpan)> {
    let mut out = Vec::new();
    collect_headings(&doc.root, &mut out);
    out.sort_by_key(|(_, _, span)| span.start);
    out
}

fn collect_headings<'a>(node: &'a Node, out: &mut Vec<(u8, &'a str, ByteSpan)>) {
    if let NodeKind::Section {
        heading_text,
        level,
    } = &node.kind
    {
        out.push((*level, heading_text.as_str(), node.span.clone()));
    }
    for c in &node.children {
        collect_headings(c, out);
    }
}

/// Match heading segments in order: each next segment is the first heading —
/// scanning forward from the last match, anywhere-after — that is
/// case-insensitively equal AND strictly deeper than the previous match's level
/// (so level 1→3 skips the intermediate; a same-or-lower level never matches).
/// First match on duplicate headings, silent.
fn walk_headings(heads: &[(u8, &str, ByteSpan)], segs: &[&str]) -> Option<ByteSpan> {
    let mut pos = 0;
    let mut prev_level = 0u8;
    let mut matched = None;
    for seg in segs {
        let want = seg.to_lowercase();
        let (i, level, span) =
            heads[pos..]
                .iter()
                .enumerate()
                .find_map(|(off, (level, text, span))| {
                    (*level > prev_level && text.to_lowercase() == want)
                        .then(|| (pos + off, *level, span.clone()))
                })?;
        prev_level = level;
        pos = i + 1;
        matched = Some(span);
    }
    matched
}

/// Block-id lookup over the tree's anchor nodes, case-insensitive, **last-wins**
/// on duplicates (silent — the app overwrites its block map, contract §2.1). A
/// `_`-bearing id never appears among anchor nodes (the lexer drops it, app-exact
/// charset §2.4), so the walk misses it exactly as the app does.
fn block_span(doc: &Document, id: &str) -> Option<ByteSpan> {
    let want = id.to_lowercase();
    let mut hits: Vec<ByteSpan> = Vec::new();
    collect_block(&doc.root, &want, &mut hits);
    hits.sort_by_key(|span| span.start);
    hits.into_iter().next_back()
}

fn collect_block(node: &Node, want: &str, hits: &mut Vec<ByteSpan>) {
    if matches!(&node.kind, NodeKind::Anchor { name } if name.to_lowercase() == *want) {
        hits.push(node.span.clone());
    }
    for c in &node.children {
        collect_block(c, want, hits);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{NodeRev, build};

    /// D-C2 (frozen §4.5): the walk return type has NO rev field — a type-level
    /// fact, not a discipline. The exhaustive destructure (no `..`) binds every
    /// field into an explicitly-typed tuple; adding a `node_rev: NodeRev` field
    /// to `Location` or `Miss` would fail to COMPILE here. `NodeRev` is imported
    /// purely to prove it is nameable yet absent from these types.
    #[test]
    fn no_rev_field_on_walk_return_type() {
        // Returning the fully-typed tuple forces both the exhaustive destructure
        // (no `..`) and each field's type — a `node_rev: NodeRev` field would
        // break the pattern or the return type.
        fn _location_is_location_facts_only(loc: Location) -> (String, ByteSpan) {
            let Location { dest, span } = loc;
            (dest, span)
        }
        fn _miss_is_stage_and_dest_only(miss: Miss) -> (Stage, Option<String>) {
            let Miss { stage, dest } = miss;
            (stage, dest)
        }
        // A NodeRev exists in this crate; it is simply not a field above.
        let _: fn(String) -> NodeRev = NodeRev;
    }

    fn corpus(files: &[(&str, &str)]) -> (CorpusIndex, BTreeMap<String, Document>) {
        let mut index = CorpusIndex::new();
        let mut docs = BTreeMap::new();
        for (path, raw) in files {
            let doc = build((*raw).to_string(), syntax::parse(raw));
            index.insert(path, &doc);
            docs.insert((*path).to_string(), doc);
        }
        (index, docs)
    }

    #[test]
    fn stage1_resolves_frontmatter_alias_case_insensitively() {
        // §4.5 stage 1 resolves frontmatter aliases (walkvault has none, so the
        // alias path is proven on its own fixture). `codename` → alias-target.md.
        let (index, docs) = corpus(&[
            ("notes/plan.md", "# Plan\n"),
            (
                "alias-target.md",
                "---\naliases: [codename, cover-name]\n---\n# Secret\n",
            ),
        ]);
        let got = walk(&index, &docs, "notes/plan.md", "CODENAME");
        assert_eq!(
            got,
            Ok(Location {
                dest: "alias-target.md".to_string(),
                span: 0..docs["alias-target.md"].raw.len(),
            })
        );
    }

    #[test]
    fn stage1_miss_has_no_dest() {
        let (index, docs) = corpus(&[("walk.md", "# H\n")]);
        assert_eq!(
            walk(&index, &docs, "walk.md", "nosuchfile#H"),
            Err(Miss {
                stage: Stage::One,
                dest: None
            })
        );
    }

    #[test]
    fn empty_subpath_is_whole_file() {
        let (index, docs) = corpus(&[("walk.md", "# H\n\nbody\n")]);
        assert_eq!(
            walk(&index, &docs, "walk.md", "walk#"),
            Ok(Location {
                dest: "walk.md".to_string(),
                span: 0..docs["walk.md"].raw.len(),
            })
        );
    }

    /// The §0.3/§6.3 S1 receipts state (249 B): the `# Receipts` heading + the E3
    /// receipt list item carrying `^r-000042` at end-of-block (Form A). Built the
    /// way the contract's verify.py builds it — heading bytes + the receipt line
    /// interpolating R0 — with a 249-byte transcription guard (no S1 fixture is on
    /// disk; wsfix carries only s0/s2).
    fn receipts_s1() -> String {
        const R0_HEX: &str = "74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9";
        let receipts_v0 = "# Receipts \u{2014} 2026-07-18\n"; // em dash = 3-byte UTF-8
        let receipt_42 = format!(
            "- splice notes/plan.md id=42 actor=agent:b0864fb2 now=2026-07-18T20:31:04Z \
             root_before=b3:{R0_HEX} edits=1 Goals>Q3 match 33d5b0e1b27cb48b->41f643f034e5681f ^r-000042\n"
        );
        let raw = format!("{receipts_v0}{receipt_42}");
        assert_eq!(raw.len(), 249, "S1 receipts fixture is 249 bytes (§0.3)");
        raw
    }

    /// GATE 2 (ANCHOR-GRAIN walk plane): block lookup for `^r-000042` over the S1
    /// receipts state returns the anchor's HOST BLOCK-LEAF span (`[26,248]` — the
    /// list-item line, terminator excluded, §1/§4.1/§6.3), NOT the 9-byte
    /// `[239,248]` inline marker. The walk plane shares the model `Anchor` node
    /// span with the mint plane, so the one build-side fix corrects both.
    ///
    /// APP-ORACLE grain note (Gate 4, DATA not contract law): the Obsidian app
    /// resolves a `#^id` block ref to the whole host block, never the bare marker;
    /// `block_span` mirrors that grain. The only `^id` app-oracle probe pinned in
    /// the pack is WX-6 (`^under-probe_x` → `ref_not_found`, grain-independent).
    #[test]
    fn block_span_serves_host_block_leaf_s1_receipts() {
        let raw = receipts_s1();
        let doc = build(raw.clone(), syntax::parse(&raw));
        let mut index = CorpusIndex::new();
        let mut docs = BTreeMap::new();
        index.insert("receipts/2026-07-18.md", &doc);
        docs.insert("receipts/2026-07-18.md".to_string(), doc);
        // empty linkpath ⇒ the source file itself; subpath `^r-000042` ⇒ block lookup.
        let loc = walk(&index, &docs, "receipts/2026-07-18.md", "#^r-000042")
            .expect("walk resolves the ^r-000042 block anchor");
        assert_eq!(loc.dest, "receipts/2026-07-18.md");
        assert_eq!(
            loc.span,
            26..248,
            "host block-leaf span, not the [239,248] marker"
        );
        assert_ne!(loc.span, 239..248, "marker-grain defect must not resurface");
    }
}
