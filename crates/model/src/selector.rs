//! U2.2 — the selector grammar (four classes), the edge-color law, and the
//! ambiguity teaching refusal.
//!
//! # The one selector grammar (d2 §2.2)
//! One grammar across core, verbs, and packs — four classes:
//!
//! | class | form | color disposition |
//! |---|---|---|
//! | [`Selector::Page`] | `page` | resolves to the document root |
//! | [`Selector::Heading`] | `page#Heading` (`/`-joined path) | resolves a section |
//! | [`Selector::Block`] | `page#^block-id` | resolves a block anchor |
//! | [`Selector::ImmutableRoot`] | `session-id#seq-N` | grey `immutable-root`, never resolved |
//!
//! The transcript class (`session-id#seq-N`) is an ADDITIVE parse-level class
//! (d2 §2.2, E6): the engine RECOGNIZES the form (source 1 — parse), stores it
//! as opaque data, and renders such hops **grey `immutable-root`** — honest,
//! because the engine has not verified them and the address class cannot drift
//! by construction. It is never a write target and is never privately re-parsed
//! per verb.
//!
//! # The edge-color law (d2 §2.3; decision #9)
//! Per edge, computed never stored: **green** (`live_rev == pinned_rev`),
//! **red** (resolves but drifted, or the address fails to resolve), **grey**
//! (the ledger cannot verify). Decision #9 splits the address-failure reds off
//! from the content-drift red so they are never conflated:
//!
//! - [`RedReason::Drifted`] — the address resolves, the content moved.
//! - [`RedReason::DanglingAnchor`] — a pinned `^block-id` whose target vanished.
//! - [`RedReason::SelectorUnresolved`] — a pinned heading/page that resolves to
//!   nothing.
//!
//! Both address-failure reds carry a **nearest-candidate hint** (d1 § selector
//! ambiguity F6b): the live toc's nearest names, a hint an author confirms by
//! re-pinning — never an auto-repair (the engine holds no rename history).

use crate::{Document, Node, NodeKind, NodeRev, Ref, ResolveError, resolve};

/// The four selector classes (d2 §2.2). Parsed from a ref string by
/// [`Selector::parse`]; the transcript class is recognized by its `#seq-N`
/// fragment form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selector {
    /// `page` — the whole document root (no fragment).
    Page,
    /// `page#Heading` — a heading-path section; the path is the `/`-joined
    /// heading chain (`Task/Objective` → `["Task", "Objective"]`).
    Heading(Vec<String>),
    /// `page#^block-id` — a block anchor, exact id.
    Block(String),
    /// `session-id#seq-N` — a transcript immutable-root ref. Recognized, stored
    /// opaque, rendered grey `immutable-root`; never a write target.
    ImmutableRoot { session: String, seq: u64 },
}

impl Selector {
    /// Classify a ref string into its selector class (source 1 — parse alone).
    ///
    /// The fragment after the FIRST `#` decides the class: `^…` → [`Block`],
    /// `seq-<digits>` → [`ImmutableRoot`] (the transcript form), anything else →
    /// [`Heading`] (`/`-split). No `#` → [`Page`]. Recognition is byte-level and
    /// never per-verb private (d2 §2.2 additivity).
    ///
    /// [`Block`]: Selector::Block
    /// [`ImmutableRoot`]: Selector::ImmutableRoot
    /// [`Heading`]: Selector::Heading
    /// [`Page`]: Selector::Page
    #[must_use]
    pub fn parse(r#ref: &str) -> Selector {
        let Some((left, frag)) = r#ref.split_once('#') else {
            return Selector::Page;
        };
        if let Some(id) = frag.strip_prefix('^') {
            return Selector::Block(id.to_string());
        }
        if let Some(seq) = frag
            .strip_prefix("seq-")
            .and_then(|n| n.parse::<u64>().ok())
        {
            return Selector::ImmutableRoot {
                session: left.to_string(),
                seq,
            };
        }
        Selector::Heading(frag.split('/').map(str::to_string).collect())
    }

    /// The transcript class renders grey and is never resolved (d2 §2.2).
    #[must_use]
    pub fn is_immutable_root(&self) -> bool {
        matches!(self, Selector::ImmutableRoot { .. })
    }
}

/// One edge's computed color (d2 §2.3) — three tones, reds and greys carrying
/// their reason so a caller renders "why", never a bare tone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Color {
    /// `live_rev == pinned_rev` — the pinned content is current.
    Green,
    /// The ledger cannot verify this edge; the reason names why.
    Grey(GreyReason),
    /// The pinned edge is wrong; the reason distinguishes content drift from an
    /// address that no longer resolves (decision #9), each never conflated.
    Red(RedReason),
}

/// Why an edge renders grey (the ledger cannot verify it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GreyReason {
    /// A `session-id#seq-N` transcript hop — recognized, not verified; the
    /// address class cannot drift by construction (d2 §2.2/§2.3).
    ImmutableRoot,
    /// `pinned_rev` is NULL — declared in the manifest, never pinned; the first
    /// pin turns it green (d2 §2.1/§2.3).
    DeclaredUnpinned,
    /// The pinned selector BECAME ambiguous (a duplicate appeared) — the ledger
    /// cannot say which target the pin meant, so it will not measure drift it
    /// cannot address (d1 § selector ambiguity, point 3: grey — "selector
    /// unresolvable" — red would claim drift nobody measured).
    Ambiguous,
}

/// Why an edge renders red — the three reasons kept distinct (decision #9).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RedReason {
    /// The address resolves, but `live_rev != pinned_rev` — measured content
    /// drift. A `red(drifted)` pin never refuses; `realise` converges it (U2.5).
    Drifted,
    /// A pinned `page#^block-id` whose target anchor vanished (delete/rename).
    /// Distinct from [`Drifted`] — drift is measured, a dangling anchor is not.
    /// Carries the live toc's nearest block-id candidates (never auto-repair).
    ///
    /// [`Drifted`]: RedReason::Drifted
    DanglingAnchor { candidates: Vec<String> },
    /// A pinned heading/page selector that resolves to NOTHING (rename/move
    /// beyond recognition). Carries the live toc's nearest heading candidates.
    SelectorUnresolved { candidates: Vec<String> },
}

/// Compute an edge's color from its pinned selector, its pinned rev, and the
/// live target document (d2 §2.3). `target` is `None` when the target PAGE
/// itself does not resolve (a moved/deleted page) — the whole address is
/// unresolved. `pinned_rev` is `None` for a declared-but-unpinned manifest item
/// (grey).
///
/// This is the pure color law: caller-agnostic, computed per run, never stored.
#[must_use]
pub fn classify_edge(
    selector: &Selector,
    pinned_rev: Option<&NodeRev>,
    target: Option<&Document>,
) -> Color {
    // The transcript class is grey before anything else — it is never resolved
    // (d2 §2.2), so it outranks even a NULL pinned_rev.
    if selector.is_immutable_root() {
        return Color::Grey(GreyReason::ImmutableRoot);
    }
    let Some(pinned) = pinned_rev else {
        return Color::Grey(GreyReason::DeclaredUnpinned);
    };
    // A vanished target PAGE: the whole selector is unresolved. A block address
    // dangles; a heading/page address is selector-unresolved. Candidates are
    // empty — no live doc to draw them from.
    let Some(doc) = target else {
        return match selector {
            Selector::Block(_) => Color::Red(RedReason::DanglingAnchor {
                candidates: Vec::new(),
            }),
            _ => Color::Red(RedReason::SelectorUnresolved {
                candidates: Vec::new(),
            }),
        };
    };
    // The whole-page selector resolves to the DOCUMENT ROOT (== `file_rev`); it
    // has no mint-plane `Ref` form, so its rev is read directly. A present page
    // never dangles — only drifts (a vanished page is the `target: None` case).
    if matches!(selector, Selector::Page) {
        return if &doc.root.node_rev == pinned {
            Color::Green
        } else {
            Color::Red(RedReason::Drifted)
        };
    }
    match resolve(doc, &selector_ref(selector)) {
        // Resolves — the only question left is content drift (source 2).
        Ok(t) if &t.node_rev == pinned => Color::Green,
        Ok(_) => Color::Red(RedReason::Drifted),
        // The address failed. Block → dangling-anchor; heading/page →
        // selector-unresolved. Each with the live toc's nearest candidates.
        Err(ResolveError::NotFound) => match selector {
            Selector::Block(id) => Color::Red(RedReason::DanglingAnchor {
                candidates: nearest(id, &live_anchors(doc)),
            }),
            _ => Color::Red(RedReason::SelectorUnresolved {
                candidates: nearest(&selector_display(selector), &live_headings(doc)),
            }),
        },
        // The pinned selector became ambiguous — grey, unknowable (d1 point 3).
        Err(ResolveError::Ambiguous(_)) => Color::Grey(GreyReason::Ambiguous),
    }
}

/// Project a resolvable selector to its mint-plane [`Ref`] — the Heading and
/// Block classes only. Page compares the document root directly and the
/// transcript class is grey before resolution, so neither reaches here (both
/// guarded in [`classify_edge`]); an empty hpath is the inert fallback (it
/// resolves `NotFound`), never a live path.
fn selector_ref(selector: &Selector) -> Ref {
    match selector {
        Selector::Heading(hpath) => Ref::Hpath(
            hpath
                .iter()
                .map(|h| crate::HpathSeg {
                    h: h.clone(),
                    n: None,
                })
                .collect(),
        ),
        // A dangling anchor id resolves NotFound rather than a charset refusal —
        // the color plane never mints the CHARSET-GUARD refusal.
        Selector::Block(id) => Ref::Anchor(id.clone()),
        Selector::Page | Selector::ImmutableRoot { .. } => Ref::Hpath(Vec::new()),
    }
}

/// A human display of a selector's address (for the unresolved candidate rank
/// and messages). Matches d1's `Task/Objective` heading-path spelling.
#[must_use]
pub fn selector_display(selector: &Selector) -> String {
    match selector {
        Selector::Page => "(page)".to_string(),
        Selector::Heading(hpath) => hpath.join("/"),
        Selector::Block(id) => format!("^{id}"),
        Selector::ImmutableRoot { session, seq } => format!("{session}#seq-{seq}"),
    }
}

/// Every block-id anchor name in a document, document order — the live toc's
/// block candidates for a dangling-anchor hint.
#[must_use]
pub fn live_anchors(doc: &Document) -> Vec<String> {
    let mut out = Vec::new();
    collect_anchor_names(&doc.root, &mut out);
    out
}

fn collect_anchor_names(node: &Node, out: &mut Vec<String>) {
    if let NodeKind::Anchor { name } = &node.kind {
        out.push(name.clone());
    }
    for c in &node.children {
        collect_anchor_names(c, out);
    }
}

/// Every heading-path in a document (`/`-joined), document order — the live
/// toc's heading candidates for a selector-unresolved hint.
#[must_use]
pub fn live_headings(doc: &Document) -> Vec<String> {
    let mut out = Vec::new();
    collect_heading_paths(&doc.root, &mut out);
    out
}

fn collect_heading_paths(node: &Node, out: &mut Vec<String>) {
    if matches!(node.kind, NodeKind::Section { .. })
        && let Some(hpath) = &node.hpath
    {
        out.push(hpath.join("/"));
    }
    for c in &node.children {
        collect_heading_paths(c, out);
    }
}

/// Rank `candidates` by nearest to `want` (a hint, never auto-repair — d1 F6b):
/// a shared-bigram score, ties broken by document order. The renamed successor
/// of a vanished selector ranks first because it shares the most text.
#[must_use]
pub fn nearest(want: &str, candidates: &[String]) -> Vec<String> {
    let mut scored: Vec<(usize, usize, &String)> = candidates
        .iter()
        .enumerate()
        .map(|(i, c)| (bigram_overlap(want, c), i, c))
        .collect();
    // Higher overlap first; ties keep document order (the stable `i`).
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    scored.into_iter().map(|(_, _, c)| c.clone()).collect()
}

/// The count of character bigrams `a` and `b` share (a cheap, allocation-light
/// similarity — enough to surface a renamed heading above unrelated ones).
fn bigram_overlap(a: &str, b: &str) -> usize {
    let bigrams = |s: &str| -> Vec<[char; 2]> {
        let chars: Vec<char> = s.chars().collect();
        chars.windows(2).map(|w| [w[0], w[1]]).collect()
    };
    let a = bigrams(a);
    let mut b = bigrams(b);
    let mut shared = 0;
    for g in a {
        if let Some(pos) = b.iter().position(|x| *x == g) {
            b.swap_remove(pos);
            shared += 1;
        }
    }
    shared
}

// ---------------------------------------------------------------------------
// The ambiguity teaching refusal (d1 § selector ambiguity — carried VERBATIM)
// ---------------------------------------------------------------------------

/// The d1 teaching-refusal exemplar, carried VERBATIM as the provenance anchor
/// (design-1.md § "Selector ambiguity — the law", the ruling §5 engine-owned
/// teaching refusal). [`render_ambiguity`] reproduces this wording with the real
/// selector and candidates interpolated; this const pins the exemplar so a drift
/// in the wording is a visible test failure.
pub const D1_TEACHING_REFUSAL_EXEMPLAR: &str = "refused: selector 'Task/Objective' is ambiguous — 2 matches: n=1.2 (^a1b2c3), n=1.5 (^d4e5f6). Unambiguous writes to this file remain served. Fix: address the duplicate by block id or node index and rename one heading; see [[selector-grammar]].";

/// One ambiguous candidate: its 1-based node index (the occurrence `n=`, the
/// §2.1 addressable disambiguator) and its block id if it carries one (`^block`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AmbiguityCandidate {
    /// The occurrence index — the node-index address that disambiguates the
    /// duplicate (`n=`).
    pub node_index: u32,
    /// The candidate's block anchor id, when it has one (`^block`); `None` when
    /// the duplicate has no block id (only the node index can address it).
    pub block: Option<String>,
}

/// Render the d1 teaching refusal for an ambiguous selector, naming EACH
/// candidate by both its node index (`n=`) and its block id (`^block`) when it
/// has one. The wording is carried verbatim from [`D1_TEACHING_REFUSAL_EXEMPLAR`]
/// with the real selector and candidates spliced in — refuse-ambiguous-only:
/// "Unambiguous writes to this file remain served."
#[must_use]
pub fn render_ambiguity(selector: &str, candidates: &[AmbiguityCandidate]) -> String {
    let named = candidates
        .iter()
        .map(|c| match &c.block {
            Some(b) => format!("n={} (^{b})", c.node_index),
            None => format!("n={}", c.node_index),
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "refused: selector '{selector}' is ambiguous — {count} matches: {named}. \
         Unambiguous writes to this file remain served. Fix: address the duplicate \
         by block id or node index and rename one heading; see [[selector-grammar]].",
        count = candidates.len()
    )
}

/// The first block-id anchor NAME contained within a byte span (an ambiguous
/// section's `^block`, so the refusal can name each duplicate by its block id).
/// `None` when the span carries no anchor. Shared with the wire-serve write door,
/// which enriches the `ambiguous_ref` candidates.
#[must_use]
pub fn first_anchor_in_span(doc: &Document, span: &crate::ByteSpan) -> Option<String> {
    fn walk(node: &Node, span: &crate::ByteSpan, found: &mut Option<String>) {
        if found.is_some() {
            return;
        }
        if let NodeKind::Anchor { name } = &node.kind
            && node.span.start >= span.start
            && node.span.end <= span.end
        {
            *found = Some(name.clone());
            return;
        }
        for c in &node.children {
            walk(c, span, found);
        }
    }
    let mut found = None;
    walk(&doc.root, span, &mut found);
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build;

    fn doc(raw: &str) -> Document {
        build(raw.to_string(), syntax::parse(raw))
    }

    /// The current live rev of a resolvable selector (to pin green, or an old
    /// value to simulate drift).
    fn live_rev(d: &Document, sel: &Selector) -> NodeRev {
        resolve(d, &selector_ref(sel)).expect("resolves").node_rev
    }

    #[test]
    fn parse_classifies_four_selector_classes() {
        assert_eq!(Selector::parse("notes/plan.md"), Selector::Page);
        assert_eq!(
            Selector::parse("notes/plan.md#Task/Objective"),
            Selector::Heading(vec!["Task".into(), "Objective".into()])
        );
        assert_eq!(
            Selector::parse("notes/plan.md#^a1b2c3"),
            Selector::Block("a1b2c3".into())
        );
        // The transcript class is recognized by its `#seq-N` fragment form.
        assert_eq!(
            Selector::parse("22-01-meridian-attestation-module#seq-160"),
            Selector::ImmutableRoot {
                session: "22-01-meridian-attestation-module".into(),
                seq: 160,
            }
        );
    }

    #[test]
    fn immutable_root_renders_grey_never_resolved() {
        let sel = Selector::parse("22-01-session#seq-42");
        assert!(sel.is_immutable_root());
        // Grey even with no target doc and no pinned rev — never resolved (d2 §2.2).
        assert_eq!(
            classify_edge(&sel, None, None),
            Color::Grey(GreyReason::ImmutableRoot)
        );
    }

    #[test]
    fn declared_unpinned_is_grey() {
        let d = doc("# Task\n\n## Objective\n\nbody\n");
        let sel = Selector::Heading(vec!["Task".into(), "Objective".into()]);
        assert_eq!(
            classify_edge(&sel, None, Some(&d)),
            Color::Grey(GreyReason::DeclaredUnpinned)
        );
    }

    #[test]
    fn green_when_rev_matches() {
        let d = doc("# Task\n\n## Objective\n\nbody\n");
        let sel = Selector::Heading(vec!["Task".into(), "Objective".into()]);
        let pinned = live_rev(&d, &sel);
        assert_eq!(classify_edge(&sel, Some(&pinned), Some(&d)), Color::Green);
    }

    /// The whole-page selector greens against the document root rev and drifts
    /// when it moves — it has no `Ref` form, so it is compared directly.
    #[test]
    fn page_selector_greens_and_drifts_against_document_root() {
        let d = doc("# Task\n\n## Objective\n\nbody\n");
        let root_rev = d.root.node_rev.clone();
        assert_eq!(
            classify_edge(&Selector::Page, Some(&root_rev), Some(&d)),
            Color::Green
        );
        let stale = NodeRev("staaaaaaaaaaaaaa".into());
        assert_eq!(
            classify_edge(&Selector::Page, Some(&stale), Some(&d)),
            Color::Red(RedReason::Drifted)
        );
    }

    /// A dangling `^block-id` renders `red(dangling-anchor)` — DISTINCT from
    /// `red(drifted)` — and carries the live toc's nearest block-id candidates
    /// (decision #9; a hint, never auto-repair).
    #[test]
    fn dangling_anchor_distinct_from_drifted() {
        // live doc: ^kept exists, ^gone does not; a heading whose content moved.
        let d = doc("# Task\n\n## Objective\n\nbody ^kept\n\n## Notes\n\nmore\n");
        let stale = NodeRev("staaaaaaaaaaaaaa".into());

        // dangling anchor: pinned ^gone no longer resolves.
        let dangling = classify_edge(&Selector::Block("gone".into()), Some(&stale), Some(&d));
        let Color::Red(RedReason::DanglingAnchor { candidates }) = &dangling else {
            panic!("a vanished pinned anchor must render red(dangling-anchor): {dangling:?}");
        };
        assert!(
            candidates.contains(&"kept".to_string()),
            "the nearest-candidate hint lists the live toc's block ids: {candidates:?}"
        );

        // drift: the heading resolves, but its rev is stale.
        let drifted = classify_edge(
            &Selector::Heading(vec!["Task".into(), "Objective".into()]),
            Some(&stale),
            Some(&d),
        );
        assert_eq!(drifted, Color::Red(RedReason::Drifted));

        // The two reds are DISTINCT enum values — never conflated (decision #9).
        assert_ne!(dangling, drifted);
    }

    /// A pinned heading that resolves to nothing renders `red(selector-unresolved)`
    /// with the live toc's nearest heading candidates — distinct from drift.
    #[test]
    fn selector_unresolved_for_missing_heading() {
        let d = doc("# Task\n\n## Objective\n\nbody\n");
        let stale = NodeRev("staaaaaaaaaaaaaa".into());
        let sel = Selector::Heading(vec!["Task".into(), "Goalz".into()]);
        let c = classify_edge(&sel, Some(&stale), Some(&d));
        let Color::Red(RedReason::SelectorUnresolved { candidates }) = &c else {
            panic!("a missing heading selector must render red(selector-unresolved): {c:?}");
        };
        assert!(
            candidates.contains(&"Task/Objective".to_string()),
            "candidates list the live heading paths: {candidates:?}"
        );
        assert_ne!(c, Color::Red(RedReason::Drifted));
    }

    /// The nearest-candidate rank surfaces a renamed heading first (d1 F6b: the
    /// author confirms by re-pinning; the engine never auto-repairs).
    #[test]
    fn nearest_ranks_renamed_successor_first() {
        let cands = vec![
            "Introduction".to_string(),
            "Objectives".to_string(), // the rename of "Objective"
            "Appendix".to_string(),
        ];
        let ranked = nearest("Objective", &cands);
        assert_eq!(ranked[0], "Objectives", "the closest name ranks first");
    }

    /// The teaching refusal carries the d1 wording VERBATIM (the fixed teaching
    /// sentence), naming each candidate by node index AND block id. The
    /// exemplar const and every rendered refusal share the same teaching tail.
    #[test]
    fn render_ambiguity_carries_d1_teaching_verbatim() {
        const TEACH_TAIL: &str = ". Unambiguous writes to this file remain served. \
             Fix: address the duplicate by block id or node index and rename one \
             heading; see [[selector-grammar]].";
        // The pinned d1 exemplar carries the verbatim teaching tail.
        assert!(
            D1_TEACHING_REFUSAL_EXEMPLAR.ends_with(TEACH_TAIL),
            "the d1 exemplar const must carry the verbatim teaching tail"
        );
        let msg = render_ambiguity(
            "Task/Objective",
            &[
                AmbiguityCandidate {
                    node_index: 1,
                    block: Some("a1b2c3".into()),
                },
                AmbiguityCandidate {
                    node_index: 2,
                    block: Some("d4e5f6".into()),
                },
            ],
        );
        assert!(msg.starts_with("refused: selector 'Task/Objective' is ambiguous — 2 matches: "));
        assert!(
            msg.contains("n=1 (^a1b2c3)"),
            "names candidate 1 by n= + ^block"
        );
        assert!(
            msg.contains("n=2 (^d4e5f6)"),
            "names candidate 2 by n= + ^block"
        );
        assert!(
            msg.ends_with(TEACH_TAIL),
            "every rendered refusal carries the verbatim teaching tail: {msg}"
        );
    }

    /// A duplicate with no block id is named by node index alone (only the node
    /// index can address it — d1: "by block id OR node index").
    #[test]
    fn render_ambiguity_names_blockless_by_node_index() {
        let msg = render_ambiguity(
            "Task/Objective",
            &[
                AmbiguityCandidate {
                    node_index: 1,
                    block: None,
                },
                AmbiguityCandidate {
                    node_index: 2,
                    block: None,
                },
            ],
        );
        assert!(
            msg.contains("2 matches: n=1, n=2."),
            "blockless → node index only: {msg}"
        );
    }

    #[test]
    fn first_anchor_in_span_finds_the_block() {
        let d = doc("# Task\n\n## Objective\n\nbody ^a1b2c3\n");
        // The Objective section span contains the ^a1b2c3 anchor.
        let sel = Selector::Heading(vec!["Task".into(), "Objective".into()]);
        let span = resolve(&d, &selector_ref(&sel)).unwrap().span;
        assert_eq!(first_anchor_in_span(&d, &span), Some("a1b2c3".to_string()));
    }
}
