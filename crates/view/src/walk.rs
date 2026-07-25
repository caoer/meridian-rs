//! U2.3 — the walk plane: the context-assembly listing (d2 §2.4 / §3).
//!
//! The walk computes — **per query, never stored** — the reachability listing
//! over the declared pin graph: every edge [`crate::read_face::page_lock_items`]
//! parses, in all three forms it reads (the legacy `^inputs` form-1/form-2 and
//! the engine's own `meridian-lock` block). One traversal, two directions:
//!
//! - **[`Direction::Up`]** — ancestors: what the root draws from, transitively —
//!   d2 §2.4's context-assembly walk (the retired "pack" noun avoided).
//! - **[`Direction::Down`]** — descendants: who pins the root — the dependents
//!   renderer and dry-run blast radius (`--depth 1` = the direct dependents).
//!
//! Each reached edge is one [`WalkEntry`] `{selector, rev, color, depth}`, its
//! color computed by [`model::selector::classify_edge`] (U2.2). Every report
//! cites the doc revs it read ([`WalkReport::revs_read`]) — the honesty law
//! (§2.4: every answer cites the doc revs it read; a walk output is itself a
//! pinnable fact). In-snapshot cycles are errors ([`WalkError::Cycle`]).
//!
//! # Grain
//! U2.3 traverses at PAGE grain: the root is a page (a `#fragment` in the arg is
//! stripped), and a hop follows a page's whole `^inputs` set. Each entry still
//! carries the full SELECTOR of the reached edge (`page` or `page#sel`), so the
//! listing is selector-accurate even though the traversal key is the page.
//! Selector-grain traversal is deferred to the wire leg (U3.1).
//!
//! # Never stored
//! [`walk`] is a pure function of a SHARED-borrowed corpus (`&BTreeMap`) with no
//! writer, no `Connection`, and no filesystem handle in its signature — it
//! cannot persist anything by construction, and it is idempotent (no
//! memoization). The engine mints no receipt and writes no row for a walk
//! (§2.4 / §3: "computed per query, never stored; no verb memoizes a walk").

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use model::Document;
use model::selector::{Color, GreyReason, RedReason, Selector, classify_edge, classify_pin};

use crate::read_face::{LockItem, corpus_index, page_lock_items_in_corpus};

/// Which way the walk runs over the `^inputs` pin graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Ancestors — what the root draws from (the context-assembly walk).
    Up,
    /// Descendants — who pins the root (dependents / blast radius).
    Down,
}

impl Direction {
    /// The stable lowercase label (`up` / `down`) for output.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Direction::Up => "up",
            Direction::Down => "down",
        }
    }
}

/// One entry in the context-assembly listing (d2 §2.4 / §3): a reached edge,
/// depth-tagged, color-computed, never stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalkEntry {
    /// The canonical selector of the reached end — the TARGET drawn from for
    /// [`Direction::Up`], the DEPENDENT that pins the root for [`Direction::Down`]
    /// (`page` for a page-root ref, else `page#sel`).
    pub selector: String,
    /// The edge's pinned rev — `None` for a declared-only (grey) item.
    pub rev: Option<String>,
    /// The edge's computed color (green / red / grey, each with its reason).
    pub color: Color,
    /// Hops from the root (a direct edge is depth `1`).
    pub depth: u32,
}

/// One doc rev the walk read — the honesty citation (§2.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevCitation {
    /// The page path.
    pub path: String,
    /// The page's document-root rev at read time.
    pub doc_rev: String,
}

/// A completed walk — the listing plus its rev citations (§2.4). Owned by the
/// caller; the engine stores none of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalkReport {
    /// The direction walked.
    pub direction: Direction,
    /// The root page the walk started from.
    pub root: String,
    /// The depth bound applied (`None` = unbounded); no entry exceeds it.
    pub depth_bound: Option<u32>,
    /// The listing — BFS order, ascending depth then discovery order.
    pub entries: Vec<WalkEntry>,
    /// Every doc rev the listing rests on, path order (§2.4 honesty law).
    pub revs_read: Vec<RevCitation>,
}

/// A walk that cannot answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WalkError {
    /// The root page is not in the corpus.
    RootNotFound(String),
    /// An in-snapshot cycle reachable from the root in the walk direction
    /// (§2.4). Carries the page loop (`start … back-to-start`) for the refusal.
    Cycle(Vec<String>),
}

/// Walk the `^inputs` pin graph from `root` in `direction`, bounded to
/// `depth_bound` hops (`None` = unbounded). Computed per query, never stored.
///
/// `root` is a page path; a trailing `#fragment` is stripped (U2.3 page grain).
///
/// # Errors
/// [`WalkError::RootNotFound`] when `root`'s page is not in `docs`;
/// [`WalkError::Cycle`] when the pin graph has a cycle reachable from `root` in
/// `direction` (§2.4: in-snapshot cycles are errors).
pub fn walk(
    docs: &BTreeMap<String, Document>,
    root: &str,
    direction: Direction,
    depth_bound: Option<u32>,
) -> Result<WalkReport, WalkError> {
    let root_page = page_of(root).to_string();
    if !docs.contains_key(&root_page) {
        return Err(WalkError::RootNotFound(root_page));
    }

    // Parse every page's `^inputs` ONCE (the shared parser), so both directions
    // read the SAME edge facts. `forward[src] = src's declared edges`.
    let forward = forward_edges(docs);

    // Cycle check on the page-level adjacency in the walk direction (§2.4),
    // before emitting — an in-snapshot cycle is an error, not a silent stop.
    if let Some(cycle) = find_cycle(&page_adjacency(&forward, docs, direction), &root_page) {
        return Err(WalkError::Cycle(cycle));
    }

    let mut entries = Vec::new();
    // The listing's rev citations: the root always, plus every page the listing
    // names and every live target a color rested on (§2.4).
    let mut read: BTreeSet<String> = BTreeSet::new();
    read.insert(root_page.clone());

    // BFS, page-keyed traversal; entries deduped by the ROW they would print —
    // `(selector, rev, color_label)` — at min depth (BFS visits shallower first,
    // so the first sighting is the min depth).
    //
    // The key is the whole row, never the selector alone: two pins on ONE ref
    // (one live, one drifted) share a canonical selector and carry DIFFERENT
    // verdicts, and a selector-keyed dedupe dropped the second one — so a
    // measured red vanished behind a green and the listing exited 0 while
    // `mrd status` rolled the same corpus up red. A dedupe may collapse rows
    // that say the same thing; it may never collapse a verdict.
    let mut queue: VecDeque<(String, u32)> = VecDeque::new();
    let mut enqueued: BTreeSet<String> = BTreeSet::new();
    let mut emitted: BTreeSet<(String, Option<String>, String)> = BTreeSet::new();
    queue.push_back((root_page.clone(), 0));
    enqueued.insert(root_page.clone());

    while let Some((page, depth)) = queue.pop_front() {
        let next_depth = depth + 1;
        if depth_bound.is_some_and(|bound| next_depth > bound) {
            continue; // the bound is reached — do not expand this page further
        }
        for step in steps_from(docs, &forward, &page, direction) {
            read.insert(step.color_target.clone());
            read.insert(page_of(&step.selector).to_string());
            let row = (
                step.selector.clone(),
                step.pinned_rev.clone(),
                color_label(&step.color),
            );
            if emitted.insert(row) {
                entries.push(WalkEntry {
                    selector: step.selector,
                    rev: step.pinned_rev,
                    color: step.color,
                    depth: next_depth,
                });
            }
            if enqueued.insert(step.next_page.clone()) {
                queue.push_back((step.next_page, next_depth));
            }
        }
    }

    let revs_read = read
        .into_iter()
        .filter_map(|p| {
            docs.get(&p).map(|d| RevCitation {
                path: p,
                doc_rev: d.root.node_rev.0.clone(),
            })
        })
        .collect();

    Ok(WalkReport {
        direction,
        root: root_page,
        depth_bound,
        entries,
        revs_read,
    })
}

/// One `meridian-lock` row with its computed color — the row shape a status
/// surface rolls up, and the surface a link decorator reads a pin's color from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinColor {
    /// The page whose `meridian-lock` block declares this row.
    pub src_path: String,
    /// The pin's declared ref, verbatim — EMPTY on a lock-refusal row (which
    /// declares no ref; `color` then carries the refusal).
    pub declared_ref: String,
    /// The pinned `fp1.…` CID-token — `None` on a lock-refusal row.
    pub fingerprint: Option<String>,
    /// The computed color, reason-carrying ([`color_label`] renders it).
    pub color: Color,
}

/// Every `meridian-lock` (form-3) row in `docs` with its color — corpus order,
/// then block order. THE surface for a whole-corpus pin roll-up (`mrd status`)
/// and for per-pin decoration; it colors through the same [`edge_color`] the
/// walk uses, so no second computer can disagree with a walk listing.
///
/// The legacy `^inputs` forms are excluded: they are the SQL board's plane and
/// answer a different compare (`node_rev`, not a fingerprint). A page whose lock
/// REFUSED contributes its one grey `lock-refused` row — a corrupt lock is
/// visible here, never silently absent.
#[must_use]
pub fn lock_pin_colors(docs: &BTreeMap<String, Document>) -> Vec<PinColor> {
    let index = corpus_index(docs);
    let mut out = Vec::new();
    for (path, doc) in docs {
        for item in page_lock_items_in_corpus(path, doc, &index, docs) {
            if item.fingerprint.is_none() && item.lock_refusal.is_none() {
                continue; // a legacy `^inputs` row — the board's plane, not this one
            }
            out.push(PinColor {
                src_path: path.clone(),
                declared_ref: item.declared_ref.clone(),
                fingerprint: item.fingerprint.clone(),
                color: edge_color(docs, &item),
            });
        }
    }
    out
}

/// One `objects:`-plane row of a page's `meridian-lock` block — a blob sha the
/// lock references, with the page and key that reference it.
///
/// The `objects:` plane is the RETRIEVAL plane (#8 §2, git's world): whole-file
/// blob shas, never fingerprints. It answers a different question from the
/// `pins:` plane [`PinColor`] carries — not "did the content drift" but "does
/// this blob still exist anywhere durable" — so it is projected separately and
/// carries no color.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockObject {
    /// The page whose `meridian-lock` block declares this object.
    pub src_path: String,
    /// The `objects:` key, verbatim (what the blob is FOR).
    pub key: String,
    /// The blob sha, verbatim — an object id in git's world, not the engine's.
    pub blob_sha: String,
}

/// Every `meridian-lock` `objects:` entry in `docs` — corpus order, then block
/// order. THE surface for a whole-corpus reachability gauge (`mrd status`'s
/// vibe-debt meter), which asks git whether each of these blobs is reachable
/// from a commit.
///
/// [`lock::find`] is the parser, exactly as it is for the `pins:` plane — one
/// owner for the lock grammar, so a page's objects and its pins can never be
/// read by two disagreeing readers. A page whose lock REFUSED contributes NO
/// objects here: its plane is unreadable, and that damage is already named by
/// the grey `lock-refused` row [`lock_pin_colors`] projects for the same page.
#[must_use]
pub fn lock_objects(docs: &BTreeMap<String, Document>) -> Vec<LockObject> {
    let mut out = Vec::new();
    for (path, doc) in docs {
        let Ok(Some(found)) = lock::find(doc) else {
            continue;
        };
        for (key, blob_sha) in found.lock.objects {
            out.push(LockObject {
                src_path: path.clone(),
                key,
                blob_sha,
            });
        }
    }
    out
}

/// Whether the listing carries any red edge — the finding signal (a broken pin
/// in the context, or a dependent whose pin no longer resolves).
#[must_use]
pub fn has_red(report: &WalkReport) -> bool {
    report
        .entries
        .iter()
        .any(|e| matches!(e.color, Color::Red(_)))
}

/// The tone of a color (`green` / `grey` / `red`) — the stable output word.
///
/// Re-exported from [`model::selector::color_tone`], where it sits beside the
/// `Color` it names: stage-2 S10's claim-link decorator needs the same word on
/// a crate that cannot depend on this one, and two `match`es over one enum is
/// how a board and a walk start disagreeing.
pub use model::selector::color_tone;

/// The reason word behind a non-green color (`None` for green) — the stable
/// output reason, shared by the human render and the `--json` shape.
#[must_use]
pub fn color_reason(color: &Color) -> Option<&'static str> {
    match color {
        Color::Green => None,
        Color::Grey(GreyReason::ImmutableRoot) => Some("immutable-root"),
        Color::Grey(GreyReason::DeclaredUnpinned) => Some("declared-unpinned"),
        Color::Grey(GreyReason::Ambiguous) => Some("ambiguous"),
        Color::Grey(GreyReason::SupersededAlgo) => Some("superseded-algo"),
        Color::Grey(GreyReason::UnverifiableFingerprint { .. }) => Some("unverifiable-fingerprint"),
        Color::Grey(GreyReason::MalformedFingerprint) => Some("malformed-fingerprint"),
        Color::Grey(GreyReason::LockRefused { .. }) => Some("lock-refused"),
        Color::Red(RedReason::Drifted) => Some("content-drifted"),
        Color::Red(RedReason::DanglingAnchor { .. }) => Some("dangling-anchor"),
        Color::Red(RedReason::SelectorUnresolved { .. }) => Some("selector-unresolved"),
    }
}

/// The detail a reason carries beyond its word (`None` when the word says it
/// all) — WHICH fingerprint-triple member is unknown, or WHY the lock refused.
/// Split from [`color_reason`] so the reason stays a stable enum-like token for
/// machines while the human render still names the specific damage.
#[must_use]
pub fn color_detail(color: &Color) -> Option<String> {
    match color {
        Color::Grey(GreyReason::UnverifiableFingerprint { unknown }) => {
            Some(format!("unknown {}", unknown.join(", ")))
        }
        Color::Grey(GreyReason::LockRefused { reason }) => Some(reason.clone()),
        _ => None,
    }
}

/// The full color label (`green`, `red content-drifted`, `grey immutable-root`,
/// `grey unverifiable-fingerprint (unknown version)`, …) — tone, reason, and the
/// reason's detail when it carries one. The human-render word.
#[must_use]
pub fn color_label(color: &Color) -> String {
    let tone = color_tone(color);
    match (color_reason(color), color_detail(color)) {
        (Some(reason), Some(detail)) => format!("{tone} {reason} ({detail})"),
        (Some(reason), None) => format!("{tone} {reason}"),
        (None, _) => tone.to_string(),
    }
}

// ---------------------------------------------------------------------------
// internals
// ---------------------------------------------------------------------------

/// One reachable hop the BFS emits and follows.
struct Step {
    /// The canonical selector reported in the entry (target for up, dependent
    /// page for down).
    selector: String,
    /// The pinned rev of the edge (`None` = declared-only / grey).
    pinned_rev: Option<String>,
    /// The edge's computed color.
    color: Color,
    /// The live target whose rev the color rested on (the pinned page).
    color_target: String,
    /// The page to traverse next in the walk direction.
    next_page: String,
}

/// Parse every page's declared pin edges once (the shared parser — all three
/// forms), each edge's `to_path` resolved against the corpus so a form-2
/// `[[wikilink]]`-by-NAME ref points at a real `node.path` (the U3.4 wikilink
/// wiring — else the target is unfindable and a native-algo form-2 pin can never
/// verify green).
fn forward_edges(docs: &BTreeMap<String, Document>) -> BTreeMap<String, Vec<LockItem>> {
    let index = corpus_index(docs);
    docs.iter()
        .map(|(path, doc)| {
            (
                path.clone(),
                page_lock_items_in_corpus(path, doc, &index, docs),
            )
        })
        .collect()
}

/// The hops out of `page` in `direction`.
///
/// - **Up**: `page`'s own declared edges — each points at a target it draws
///   from. The entry names the target; the next hop follows the target page.
/// - **Down**: the reverse — every `(src, edge)` whose edge targets `page`. The
///   entry names the dependent `src`; the color rests on the pinned target
///   (`page`); the next hop follows `src`.
fn steps_from(
    docs: &BTreeMap<String, Document>,
    forward: &BTreeMap<String, Vec<LockItem>>,
    page: &str,
    direction: Direction,
) -> Vec<Step> {
    match direction {
        Direction::Up => forward
            .get(page)
            .into_iter()
            .flatten()
            .map(|edge| Step {
                selector: step_selector(page, edge),
                pinned_rev: edge.pinned_rev.clone(),
                color: edge_color(docs, edge),
                color_target: edge.to_path.clone(),
                next_page: edge.to_path.clone(),
            })
            .collect(),
        Direction::Down => {
            let mut steps = Vec::new();
            for (src, edges) in forward {
                for edge in edges {
                    if edge.to_path == page {
                        steps.push(Step {
                            selector: src.clone(),
                            pinned_rev: edge.pinned_rev.clone(),
                            color: edge_color(docs, edge),
                            color_target: edge.to_path.clone(),
                            next_page: src.clone(),
                        });
                    }
                }
            }
            steps
        }
    }
}

/// The listing name of one hop out of `src`: the edge's canonical target
/// address, or — for a lock-refusal row, which declares no target — the PAGE
/// whose lock refused, so the entry says WHICH page is unreadable instead of
/// rendering a blank address. The refusal row's empty `to_path` also keeps it
/// out of the reverse index and the page adjacency by construction: a refused
/// lock names no edge, so it is a leaf the walk never traverses.
fn step_selector(src: &str, edge: &LockItem) -> String {
    if edge.lock_refusal.is_some() {
        return src.to_string();
    }
    canonical_ref(&edge.to_path, &edge.to_sel)
}

/// Color one edge with the U2.2 law: parse the target selector, wrap the pinned
/// rev, and classify against the live target document.
///
/// One check rides ahead of the rev compare: a pin minted under a NAMED
/// `hash-algo` this engine does not compute (present and not engine-native per
/// [`model::is_native_algo`] — the `{node-rev, v2}` set) is grey
/// `superseded-algo` — readable, unverifiable here. A foreign rev can neither
/// equal a live node-rev (a false green) nor be measured as drift (a false red),
/// so it renders grey before classification (d2 §6.3; U0.2/U3.4). The v1→v2
/// supersede keeps the node-rev value under the `v2` contract label, so a `v2`
/// pin verifies through the SAME compare as `node-rev`. An absent header
/// defaults to native (the engine mints `node-rev`); a declared-only item (no
/// rev) has no algo to supersede — it stays declared-unpinned grey.
///
/// Two rows never reach the rev compare, because neither declares a
/// `node_rev`-comparable edge:
///
/// - a **lock-refusal row** ([`LockItem::lock_refusal`]) — the page's whole
///   `meridian-lock` block is unreadable, so it is grey `lock-refused` with the
///   refusal carried; and
/// - a **`meridian-lock` pin** (form-3), which pins a `fp1.…` CID-token. That
///   token is not `node_rev`-comparable in either direction, so the FINGERPRINT
///   plane answers it ([`model::selector::classify_pin`] over
///   [`LockItem::fingerprint`], the typed slot) — the same address law, a
///   different compare. Before this, such a pin fell into the foreign-algo
///   short-circuit below and rendered grey `superseded-algo` — visible but
///   permanently unverified.
fn edge_color(docs: &BTreeMap<String, Document>, edge: &LockItem) -> Color {
    if let Some(reason) = &edge.lock_refusal {
        return Color::Grey(GreyReason::LockRefused {
            reason: reason.clone(),
        });
    }
    let selector = Selector::parse(&canonical_ref(&edge.to_path, &edge.to_sel));
    if let Some(token) = &edge.fingerprint {
        return classify_pin(&selector, token, docs.get(&edge.to_path));
    }
    if edge.pinned_rev.is_some()
        && edge
            .hash_algo
            .as_deref()
            .is_some_and(|a| !model::is_native_algo(a))
    {
        return Color::Grey(GreyReason::SupersededAlgo);
    }
    let pinned = edge.pinned_rev.as_ref().map(|r| model::NodeRev(r.clone()));
    classify_edge(&selector, pinned.as_ref(), docs.get(&edge.to_path))
}

/// The page-level adjacency in the walk direction, for the cycle check — only
/// corpus-present pages (transcript / unmanaged targets are leaves, never
/// traversed, so they can never close a cycle).
fn page_adjacency(
    forward: &BTreeMap<String, Vec<LockItem>>,
    docs: &BTreeMap<String, Document>,
    direction: Direction,
) -> BTreeMap<String, Vec<String>> {
    let mut adj: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (src, edges) in forward {
        for edge in edges {
            if !docs.contains_key(&edge.to_path) {
                continue;
            }
            match direction {
                Direction::Up => adj
                    .entry(src.clone())
                    .or_default()
                    .push(edge.to_path.clone()),
                Direction::Down => adj
                    .entry(edge.to_path.clone())
                    .or_default()
                    .push(src.clone()),
            }
        }
    }
    adj
}

/// Find a cycle reachable from `start` in `adj` (DFS with a gray on-stack set);
/// returns the page loop (`node … back-to-node`) or `None` for a DAG.
fn find_cycle(adj: &BTreeMap<String, Vec<String>>, start: &str) -> Option<Vec<String>> {
    let mut gray: BTreeSet<String> = BTreeSet::new();
    let mut black: BTreeSet<String> = BTreeSet::new();
    let mut path: Vec<String> = Vec::new();
    dfs_cycle(start, adj, &mut gray, &mut black, &mut path)
}

fn dfs_cycle(
    node: &str,
    adj: &BTreeMap<String, Vec<String>>,
    gray: &mut BTreeSet<String>,
    black: &mut BTreeSet<String>,
    path: &mut Vec<String>,
) -> Option<Vec<String>> {
    gray.insert(node.to_string());
    path.push(node.to_string());
    for next in adj.get(node).into_iter().flatten() {
        if gray.contains(next) {
            // Back-edge to a node on the current stack — close the loop.
            let from = path.iter().position(|p| p == next).unwrap_or(0);
            let mut cycle = path[from..].to_vec();
            cycle.push(next.clone());
            return Some(cycle);
        }
        if !black.contains(next)
            && let Some(cycle) = dfs_cycle(next, adj, gray, black, path)
        {
            return Some(cycle);
        }
    }
    path.pop();
    gray.remove(node);
    black.insert(node.to_string());
    None
}

/// The page part of a selector (`a.md#Heading` → `a.md`, `a.md` → `a.md`).
fn page_of(selector: &str) -> &str {
    selector.split_once('#').map_or(selector, |(page, _)| page)
}

/// Canonical `page#sel` (or bare `page` for the doc root) — the full ref
/// [`Selector::parse`] classifies and the entry reports.
fn canonical_ref(to_path: &str, to_sel: &str) -> String {
    if to_sel.is_empty() {
        to_path.to_string()
    } else {
        format!("{to_path}#{to_sel}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(raw: &str) -> Document {
        model::build(raw.to_string(), syntax::parse(raw))
    }

    /// The three-doc chain fixture `a.md -> b.md -> c.md`, every pin GREEN: build
    /// `c`, embed its live root rev in `b`'s `^inputs`, build `b`, embed its live
    /// root rev in `a`'s `^inputs`, build `a`. Returns the corpus + the three
    /// live doc revs so a test asserts byte-exact entries and citations.
    fn three_doc_chain() -> (BTreeMap<String, Document>, String, String, String) {
        let c = doc("# C\n\nleaf body\n");
        let c_rev = c.root.node_rev.0.clone();

        let b_raw = format!(
            "# B\n\ndraws from c\n\n```yaml ^inputs\nhash-algo: node-rev\nitems:\n  - {{ref: 'c.md', rev: '{c_rev}', rev_class: 'content'}}\n```\n"
        );
        let b = doc(&b_raw);
        let b_rev = b.root.node_rev.0.clone();

        let a_raw = format!(
            "# A\n\ndraws from b\n\n```yaml ^inputs\nhash-algo: node-rev\nitems:\n  - {{ref: 'b.md', rev: '{b_rev}', rev_class: 'content'}}\n```\n"
        );
        let a = doc(&a_raw);
        let a_rev = a.root.node_rev.0.clone();

        let mut docs = BTreeMap::new();
        docs.insert("a.md".to_string(), a);
        docs.insert("b.md".to_string(), b);
        docs.insert("c.md".to_string(), c);
        (docs, a_rev, b_rev, c_rev)
    }

    /// The Test (gate 1): the three-doc chain's `walk` up-output is byte-expected
    /// — ordered `{selector, rev, color, depth}` entries with depth tags, plus
    /// the rev citations for every doc the listing rests on (§2.4 honesty).
    #[test]
    fn three_doc_chain_up_is_byte_expected() {
        let (docs, a_rev, b_rev, c_rev) = three_doc_chain();
        let report = walk(&docs, "a.md", Direction::Up, None).expect("walk up");

        assert_eq!(
            report.entries,
            vec![
                WalkEntry {
                    selector: "b.md".to_string(),
                    rev: Some(b_rev.clone()),
                    color: Color::Green,
                    depth: 1,
                },
                WalkEntry {
                    selector: "c.md".to_string(),
                    rev: Some(c_rev.clone()),
                    color: Color::Green,
                    depth: 2,
                },
            ],
            "up = the context walk, depth-tagged: b at 1, c at 2"
        );

        // Every answer cites the doc revs it read (§2.4): root a, plus b and c.
        assert_eq!(
            report.revs_read,
            vec![
                RevCitation {
                    path: "a.md".to_string(),
                    doc_rev: a_rev,
                },
                RevCitation {
                    path: "b.md".to_string(),
                    doc_rev: b_rev,
                },
                RevCitation {
                    path: "c.md".to_string(),
                    doc_rev: c_rev,
                },
            ]
        );
    }

    /// The Test (gate 2): `--down --depth 1` returns EXACTLY the direct
    /// dependents — no transitive leak. The unbounded down walk returns the
    /// transitive dependents too, proving the bound is load-bearing: if the
    /// depth bound did not fire, the `depth 1` walk would also carry `a.md` and
    /// this assertion would FAIL.
    #[test]
    fn down_depth_one_is_exactly_direct_dependents() {
        let (docs, _a, _b, c_rev) = three_doc_chain();

        // Direct dependents of c: exactly b (b pins c). Never a (transitive).
        let direct = walk(&docs, "c.md", Direction::Down, Some(1)).expect("walk down d1");
        assert_eq!(
            direct.entries,
            vec![WalkEntry {
                selector: "b.md".to_string(),
                rev: Some(c_rev.clone()),
                color: Color::Green,
                depth: 1,
            }],
            "--down --depth 1 = exactly the direct dependents, no transitive leak"
        );

        // Unbounded down: b at depth 1 AND a at depth 2 — the bound above dropped
        // exactly this transitive `a`, so the bound demonstrably fires.
        let full = walk(&docs, "c.md", Direction::Down, None).expect("walk down full");
        let reached: Vec<(&str, u32)> = full
            .entries
            .iter()
            .map(|e| (e.selector.as_str(), e.depth))
            .collect();
        assert_eq!(reached, vec![("b.md", 1), ("a.md", 2)]);
    }

    /// Never-stored (gate 3): `walk` mutates nothing — the corpus is byte-identical
    /// after the call (it takes a SHARED borrow, has no writer/Connection/fs in
    /// its signature), and it is idempotent: two walks yield identical reports, so
    /// no verb memoizes a walk (§2.4 / §3).
    #[test]
    fn walk_stores_nothing_and_is_idempotent() {
        let (docs, ..) = three_doc_chain();
        // Snapshot the corpus bytes + revs (Document has no PartialEq); a walk
        // that stored anything would have to mutate one of these.
        let snapshot = |d: &BTreeMap<String, Document>| -> Vec<(String, String, String)> {
            d.iter()
                .map(|(p, doc)| (p.clone(), doc.raw.clone(), doc.root.node_rev.0.clone()))
                .collect()
        };
        let before = snapshot(&docs);

        let first = walk(&docs, "a.md", Direction::Up, None).expect("walk");
        let second = walk(&docs, "a.md", Direction::Up, None).expect("walk again");

        assert_eq!(
            snapshot(&docs),
            before,
            "the corpus is byte-identical — the walk stores nothing"
        );
        assert_eq!(
            first, second,
            "the walk is idempotent — nothing is memoized"
        );
    }

    /// A red pin in the context surfaces as a red entry and is the finding signal.
    /// Edit c after b pinned it: b's pin drifts (`red content-drifted`).
    #[test]
    fn drifted_pin_renders_red_and_is_a_finding() {
        let (mut docs, ..) = three_doc_chain();
        // Rewrite c.md so its live root rev no longer matches b's pin.
        docs.insert("c.md".to_string(), doc("# C\n\nEDITED body — drift\n"));

        let report = walk(&docs, "a.md", Direction::Up, None).expect("walk");
        let c_entry = report
            .entries
            .iter()
            .find(|e| e.selector == "c.md")
            .expect("c edge present");
        assert_eq!(c_entry.color, Color::Red(RedReason::Drifted));
        assert!(has_red(&report), "a drifted pin is a walk finding (exit 1)");
    }

    /// A declared-but-unpinned input renders `grey declared-unpinned`, never red,
    /// and is not a finding (the first pin is how grey turns green).
    #[test]
    fn declared_unpinned_input_renders_grey() {
        let a = doc(
            "# A\n\n```yaml ^inputs\nitems:\n  - {ref: 'b.md', claim: 'declared-only, no rev'}\n```\n",
        );
        let b = doc("# B\n\nbody\n");
        let mut docs = BTreeMap::new();
        docs.insert("a.md".to_string(), a);
        docs.insert("b.md".to_string(), b);

        let report = walk(&docs, "a.md", Direction::Up, None).expect("walk");
        assert_eq!(
            report.entries,
            vec![WalkEntry {
                selector: "b.md".to_string(),
                rev: None,
                color: Color::Grey(GreyReason::DeclaredUnpinned),
                depth: 1,
            }]
        );
        assert!(!has_red(&report), "grey is never a finding");
    }

    /// An in-snapshot cycle is an error (§2.4), never an infinite walk: `x` pins
    /// `y`, `y` pins `x`.
    #[test]
    fn in_snapshot_cycle_is_an_error() {
        let x = doc(
            "# X\n\n```yaml ^inputs\nitems:\n  - {ref: 'y.md', rev: 'deadbeefdeadbeef'}\n```\n",
        );
        let y = doc(
            "# Y\n\n```yaml ^inputs\nitems:\n  - {ref: 'x.md', rev: 'feedfacefeedface'}\n```\n",
        );
        let mut docs = BTreeMap::new();
        docs.insert("x.md".to_string(), x);
        docs.insert("y.md".to_string(), y);

        let err = walk(&docs, "x.md", Direction::Up, None).expect_err("cycle is an error");
        let WalkError::Cycle(loop_pages) = err else {
            panic!("expected a cycle error, got {err:?}");
        };
        assert_eq!(
            loop_pages.first(),
            loop_pages.last(),
            "the loop closes on itself"
        );
        assert!(loop_pages.contains(&"x.md".to_string()));
        assert!(loop_pages.contains(&"y.md".to_string()));
    }

    /// The walk root must be in the corpus; a missing root is an error, not an
    /// empty walk (fail-closed). A `#fragment` in the arg is stripped to the page.
    #[test]
    fn missing_root_is_an_error_and_fragment_is_stripped() {
        let (docs, ..) = three_doc_chain();
        assert_eq!(
            walk(&docs, "gone.md", Direction::Up, None),
            Err(WalkError::RootNotFound("gone.md".to_string()))
        );
        // `a.md#Whatever` resolves to page a.md (page-grain traversal).
        let report = walk(&docs, "a.md#Section", Direction::Up, None).expect("walk");
        assert_eq!(report.root, "a.md");
    }

    /// A transcript ref (`session#seq-N`) renders `grey immutable-root` and is a
    /// walk leaf — recognized, never resolved, never traversed (§2.2 / §2.4).
    #[test]
    fn transcript_input_renders_grey_immutable_root() {
        let a = doc(
            "# A\n\n```yaml ^inputs\nitems:\n  - {ref: '22-01-session#seq-160', claim: 'transcript root'}\n```\n",
        );
        let mut docs = BTreeMap::new();
        docs.insert("a.md".to_string(), a);

        let report = walk(&docs, "a.md", Direction::Up, None).expect("walk");
        assert_eq!(
            report.entries,
            vec![WalkEntry {
                selector: "22-01-session#seq-160".to_string(),
                rev: None,
                color: Color::Grey(GreyReason::ImmutableRoot),
                depth: 1,
            }]
        );
    }

    #[test]
    fn color_words_are_stable() {
        assert_eq!(color_label(&Color::Green), "green");
        assert_eq!(
            color_label(&Color::Red(RedReason::Drifted)),
            "red content-drifted"
        );
        assert_eq!(
            color_label(&Color::Grey(GreyReason::ImmutableRoot)),
            "grey immutable-root"
        );
        assert_eq!(
            color_tone(&Color::Grey(GreyReason::DeclaredUnpinned)),
            "grey"
        );
        assert_eq!(
            color_reason(&Color::Red(RedReason::SelectorUnresolved {
                candidates: vec![]
            })),
            Some("selector-unresolved")
        );
        assert_eq!(
            color_label(&Color::Grey(GreyReason::SupersededAlgo)),
            "grey superseded-algo"
        );
    }

    /// Archive fixture (U3.4): a pin minted under `hash-algo: v1` — merkle-v1, an
    /// algo this engine does not compute — renders `grey superseded-algo`, NEVER
    /// red. A v1 (sha256) rev can never equal a live node-rev, so without the
    /// algo gate the engine would cry drift (a false red) over an archived block
    /// it must leave grey forever (d2 §6.3; U0.2/U3.4).
    #[test]
    fn archived_v1_lock_renders_grey_superseded_algo() {
        let a = doc(
            "# A\n\n```yaml ^inputs\nhash-algo: v1\nitems:\n  - {ref: 'b.md', rev: 'a1b2c3d4e5f60718', rev_class: content}\n```\n",
        );
        let b = doc("# B\n\nbody\n");
        let mut docs = BTreeMap::new();
        docs.insert("a.md".to_string(), a);
        docs.insert("b.md".to_string(), b);

        let report = walk(&docs, "a.md", Direction::Up, None).expect("walk");
        let b_entry = report
            .entries
            .iter()
            .find(|e| e.selector == "b.md")
            .expect("b edge present");
        assert_eq!(
            b_entry.color,
            Color::Grey(GreyReason::SupersededAlgo),
            "a v1-algo pin is grey superseded-algo, never red drift"
        );
        assert!(
            !has_red(&report),
            "a superseded-algo pin is never a walk finding"
        );
    }

    /// Control: the SAME edge under the engine's native `hash-algo: node-rev` is
    /// VERIFIED — green when the rev matches live, red drift when it does not.
    /// The algo gate keys on the algo, not on any unfamiliar-looking rev.
    #[test]
    fn native_node_rev_lock_is_verified_not_superseded() {
        let b = doc("# B\n\nbody\n");
        let b_rev = b.root.node_rev.0.clone();
        let a_green = doc(&format!(
            "# A\n\n```yaml ^inputs\nhash-algo: node-rev\nitems:\n  - {{ref: 'b.md', rev: '{b_rev}', rev_class: content}}\n```\n"
        ));
        let a_red = doc(
            "# A\n\n```yaml ^inputs\nhash-algo: node-rev\nitems:\n  - {ref: 'b.md', rev: 'a1b2c3d4e5f60718', rev_class: content}\n```\n",
        );

        let mut g = BTreeMap::new();
        g.insert("a.md".to_string(), a_green);
        g.insert("b.md".to_string(), doc("# B\n\nbody\n"));
        let gr = walk(&g, "a.md", Direction::Up, None).expect("walk");
        assert_eq!(gr.entries[0].color, Color::Green, "native + match = green");

        let mut r = BTreeMap::new();
        r.insert("a.md".to_string(), a_red);
        r.insert("b.md".to_string(), b);
        let rr = walk(&r, "a.md", Direction::Up, None).expect("walk");
        assert_eq!(
            rr.entries[0].color,
            Color::Red(RedReason::Drifted),
            "native + mismatch = red drift"
        );
    }

    /// U3.4 (form-2 reader): the ratified SCHEMA.md effect-receipt chain — a plain
    /// `` ```yaml `` block, block-SEQUENCE body, `hash-algo: v1`, trailing `^inputs`
    /// anchor. Pre-U3.4 mrd was BLIND to this form: a `walk` printed `(nothing)`.
    /// Now the edge PARSES and renders `grey superseded-algo` (v1 is an algo this
    /// engine does not compute). The entry list is non-empty — the pre-change
    /// behavior was zero items.
    #[test]
    fn form2_chain_block_renders_grey_superseded_algo() {
        let raw = "## Chain\n\n```yaml\n- ref: '[[llm-wiki-skill-compilation]]'\n  claim:\n  hash: 'merkle-v1:247e292cc3c62e103424ad04cecb36517711cdfe42bc245ef516cfe54b83073d'\nhash-algo: v1\n```\n\n^inputs\n";
        let mut docs = BTreeMap::new();
        docs.insert("effect.md".to_string(), doc(raw));

        let report = walk(&docs, "effect.md", Direction::Up, None).expect("walk");
        assert!(
            !report.entries.is_empty(),
            "form-2 now parses — pre-U3.4 this walk was empty (mrd printed `(nothing)`)",
        );
        assert_eq!(report.entries.len(), 1);
        let entry = &report.entries[0];
        assert_eq!(entry.selector, "llm-wiki-skill-compilation");
        assert_eq!(
            entry.rev.as_deref(),
            Some("merkle-v1:247e292cc3c62e103424ad04cecb36517711cdfe42bc245ef516cfe54b83073d"),
            "the `hash:` line is the pinned rev — the `merkle-v1:` prefix is kept",
        );
        assert_eq!(
            entry.color,
            Color::Grey(GreyReason::SupersededAlgo),
            "a v1-algo form-2 pin is grey superseded-algo, never red",
        );
        assert!(
            !has_red(&report),
            "a superseded-algo pin is never a finding"
        );
    }

    /// A form-2 chain with MULTIPLE `- ref:` block-sequence items parses ALL of
    /// them (count assertion), each grey superseded-algo under the shared `v1`
    /// header — proving the block-sequence reader is not one-shot.
    #[test]
    fn form2_multiple_refs_all_parse() {
        let raw = "## Chain\n\n```yaml\n- ref: '[[alpha]]'\n  hash: 'merkle-v1:aaaa'\n- ref: '[[beta]]'\n  claim:\n  hash: 'merkle-v1:bbbb'\n- ref: '[[gamma]]'\n  hash: 'merkle-v1:cccc'\nhash-algo: v1\n```\n\n^inputs\n";
        let mut docs = BTreeMap::new();
        docs.insert("effect.md".to_string(), doc(raw));

        let report = walk(&docs, "effect.md", Direction::Up, None).expect("walk");
        assert_eq!(
            report.entries.len(),
            3,
            "all three form-2 block-sequence items parse",
        );
        assert_eq!(
            report
                .entries
                .iter()
                .map(|e| e.selector.as_str())
                .collect::<Vec<_>>(),
            vec!["alpha", "beta", "gamma"],
        );
        assert!(
            report
                .entries
                .iter()
                .all(|e| e.color == Color::Grey(GreyReason::SupersededAlgo)),
            "every v1-algo item is grey superseded-algo",
        );
    }

    /// U3.4 (wikilink wiring): a form-2 ref is a `[[wikilink]]`-by-NAME, NOT a
    /// path. Here the ref `[[target-page]]` must resolve to the real fixture path
    /// `sources/target-page.md` (`getFirstLinkpathDest` by basename) for the walk
    /// to find the target and verify. Native algo, `hash:` == the target's live
    /// `node_rev` ⇒ GREEN. Pre-wiring the bare NAME matched no `node.path`, so the
    /// same pin rendered red `selector-unresolved` — the wiring is load-bearing.
    #[test]
    fn form2_wikilink_by_name_resolves_and_verifies_green() {
        let target = doc("# Target\n\nbody\n");
        let target_rev = target.root.node_rev.0.clone();
        // The ref is the bare NAME `target-page`, but the real path is nested.
        let a_raw = format!(
            "## Chain\n\n```yaml\n- ref: '[[target-page]]'\n  claim:\n  hash: '{target_rev}'\nhash-algo: node-rev\n```\n\n^inputs\n"
        );
        let mut docs = BTreeMap::new();
        docs.insert("effects/a.md".to_string(), doc(&a_raw));
        docs.insert("sources/target-page.md".to_string(), target);

        let report = walk(&docs, "effects/a.md", Direction::Up, None).expect("walk");
        assert_eq!(report.entries.len(), 1);
        assert_eq!(
            report.entries[0].selector, "sources/target-page.md",
            "the wikilink NAME resolved to the real corpus path",
        );
        assert_eq!(
            report.entries[0].color,
            Color::Green,
            "resolved wikilink ref, rev == live node_rev ⇒ green",
        );
        assert!(!has_red(&report));
    }

    /// A `[[wikilink]]`-by-NAME ref that resolves to NOTHING keeps its bare name
    /// and renders red `selector-unresolved` — unresolved is first-class, never a
    /// false green (the resolver returns the input unchanged when the name is
    /// absent from the corpus).
    #[test]
    fn form2_unresolvable_wikilink_is_red_unresolved_not_green() {
        let a_raw = "## Chain\n\n```yaml\n- ref: '[[no-such-page]]'\n  hash: 'a1b2c3d4e5f60718'\nhash-algo: node-rev\n```\n\n^inputs\n";
        let mut docs = BTreeMap::new();
        docs.insert("effects/a.md".to_string(), doc(a_raw));

        let report = walk(&docs, "effects/a.md", Direction::Up, None).expect("walk");
        assert_eq!(report.entries.len(), 1);
        assert_eq!(report.entries[0].selector, "no-such-page");
        assert_eq!(
            report.entries[0].color,
            Color::Red(RedReason::SelectorUnresolved { candidates: vec![] }),
            "an unresolvable wikilink is red selector-unresolved, never a false green",
        );
    }

    /// U3.4 (v1→v2 supersede): a form-2 pin RE-LABELED `hash-algo: v2` — the
    /// design-2 §6.3 supersede keeps the node-rev VALUE under the effect-page `v2`
    /// contract label — verifies through the SAME `node_rev` compare as native
    /// `node-rev`: GREEN when the value equals live, red when it drifts. It is NOT
    /// grey superseded-algo (v2 is in the native set `{node-rev, v2}`). This is
    /// the post-sweep mrd-v2 leg of Gate B.
    #[test]
    fn form2_v2_algo_verifies_green_not_superseded() {
        let target = doc("# Target\n\nbody\n");
        let target_rev = target.root.node_rev.0.clone();
        let a_green = doc(&format!(
            "## Chain\n\n```yaml\n- ref: '[[target-page]]'\n  hash: '{target_rev}'\nhash-algo: v2\n```\n\n^inputs\n"
        ));
        let a_red = doc(
            "## Chain\n\n```yaml\n- ref: '[[target-page]]'\n  hash: 'a1b2c3d4e5f60718'\nhash-algo: v2\n```\n\n^inputs\n",
        );

        let mut g = BTreeMap::new();
        g.insert("effects/a.md".to_string(), a_green);
        g.insert(
            "sources/target-page.md".to_string(),
            doc("# Target\n\nbody\n"),
        );
        let gr = walk(&g, "effects/a.md", Direction::Up, None).expect("walk");
        assert_eq!(
            gr.entries[0].color,
            Color::Green,
            "v2 algo + rev == live node_rev ⇒ green (native compare, not superseded)",
        );

        let mut r = BTreeMap::new();
        r.insert("effects/a.md".to_string(), a_red);
        r.insert("sources/target-page.md".to_string(), target);
        let rr = walk(&r, "effects/a.md", Direction::Up, None).expect("walk");
        assert_eq!(
            rr.entries[0].color,
            Color::Red(RedReason::Drifted),
            "v2 algo + mismatch ⇒ red drift (a v2 pin is verified, never greyed)",
        );
    }

    /// S3 (form-3): a page whose ONLY declared inputs live in a `meridian-lock`
    /// block is visible to the walk. Pre-S3 this walk was a SILENT ABSENCE — the
    /// listing was empty, `mrd walk` printed `(nothing)` and exited 0 (clean),
    /// so a real pin looked like a page with no inputs at all.
    ///
    /// The edge is now VERIFIED, not greyed: S9 routes a row carrying a
    /// `fingerprint` to the fingerprint plane, so a token that does not equal
    /// the target's live fingerprint is measured drift. (S3 asserted grey
    /// `superseded-algo` here — the placeholder this unit replaced, which could
    /// never distinguish a correct pin from a drifted one.)
    #[test]
    fn meridian_lock_page_is_visible_to_the_walk() {
        let token = format!("fp1.span2.b3.{}", "ab".repeat(32));
        let mut lock_block = lock::Lock::new();
        lock_block.set_object("sources/target-page.md", "9ae3f1deadbeef");
        lock_block.upsert_pin(lock::PinEntry {
            declared_ref: "sources/target-page.md".to_string(),
            fingerprint: token.clone(),
        });
        let effect = format!(
            "# Effect\n\ndraws from it\n\n{}\n",
            lock::render(&lock_block)
        );

        let mut docs = BTreeMap::new();
        docs.insert("effect.md".to_string(), doc(&effect));
        docs.insert(
            "sources/target-page.md".to_string(),
            doc("# Target\n\nbody\n"),
        );

        let report = walk(&docs, "effect.md", Direction::Up, None).expect("walk up");
        assert_eq!(
            report.entries,
            vec![WalkEntry {
                selector: "sources/target-page.md".to_string(),
                rev: Some(token.clone()),
                color: Color::Red(RedReason::Drifted),
                depth: 1,
            }],
            "the lock pin IS the edge — pre-S3 this vec was empty",
        );
        assert!(
            has_red(&report),
            "a pin that no longer matches its target IS a finding",
        );

        // And the reverse direction sees it too: the target's dependents now
        // include the page that pinned it (the blast radius was blind pre-S3).
        let down =
            walk(&docs, "sources/target-page.md", Direction::Down, Some(1)).expect("walk down d1");
        assert_eq!(
            down.entries
                .iter()
                .map(|e| e.selector.as_str())
                .collect::<Vec<_>>(),
            vec!["effect.md"],
        );
    }

    /// Control: a form-2 chain whose `hash-algo` is the engine's native `node-rev`,
    /// pinning a real fixture page (`b.md`) at a rev EQUAL to its live `node_rev`,
    /// renders GREEN. This proves the reader feeds the NORMAL verify path and that
    /// superseded-algo keys on the ALGO alone, not on the form-2 shape.
    #[test]
    fn form2_native_node_rev_verifies_green() {
        let b = doc("# B\n\nbody\n");
        let b_rev = b.root.node_rev.0.clone();
        let a_raw = format!(
            "## Chain\n\n```yaml\n- ref: 'b.md'\n  claim:\n  hash: '{b_rev}'\nhash-algo: node-rev\n```\n\n^inputs\n"
        );
        let mut docs = BTreeMap::new();
        docs.insert("a.md".to_string(), doc(&a_raw));
        docs.insert("b.md".to_string(), b);

        let report = walk(&docs, "a.md", Direction::Up, None).expect("walk");
        assert_eq!(report.entries.len(), 1);
        assert_eq!(report.entries[0].selector, "b.md");
        assert_eq!(
            report.entries[0].color,
            Color::Green,
            "native-algo form-2, rev == live node_rev ⇒ green (normal verify path)",
        );
        assert!(!has_red(&report));
    }

    // ── S9: the `meridian-lock` pin color (green / red / grey) ───────────────

    /// A corpus of one effect page pinning `sources/target.md` at `token`, and
    /// the target page built from `target_raw`.
    fn pinned_corpus(token: &str, target_raw: &str) -> BTreeMap<String, Document> {
        pinned_corpus_ref("sources/target.md", token, target_raw)
    }

    /// [`pinned_corpus`] with the declared ref spelled explicitly (so a test can
    /// pin a selector, or a target that is not there at all).
    fn pinned_corpus_ref(
        declared_ref: &str,
        token: &str,
        target_raw: &str,
    ) -> BTreeMap<String, Document> {
        let mut lock_block = lock::Lock::new();
        lock_block.upsert_pin(lock::PinEntry {
            declared_ref: declared_ref.to_string(),
            fingerprint: token.to_string(),
        });
        let effect = format!(
            "# Effect\n\ndraws from it\n\n{}\n",
            lock::render(&lock_block)
        );
        let mut docs = BTreeMap::new();
        docs.insert("effect.md".to_string(), doc(&effect));
        docs.insert("sources/target.md".to_string(), doc(target_raw));
        docs
    }

    /// The one entry a single-pin corpus walks up to.
    fn only_entry(docs: &BTreeMap<String, Document>) -> WalkEntry {
        let report = walk(docs, "effect.md", Direction::Up, None).expect("walk up");
        assert_eq!(report.entries.len(), 1, "one pin, one entry");
        report.entries[0].clone()
    }

    /// The live fingerprint token of a page root — what a CORRECT pin holds.
    fn live_token(raw: &str) -> String {
        let d = doc(raw);
        model::fingerprint::fingerprint(&d, &d.root)
            .expect("the fixture page has content")
            .into_string()
    }

    /// GATE 1 — the five rendered states are DISTINCT: no two of green /
    /// red(drifted) / red(dangling) / grey(unverifiable) / grey(malformed) share
    /// a label, and each names its own reason. A drift face whose states collide
    /// cannot be acted on.
    #[test]
    fn the_five_pin_states_each_render_distinctly() {
        let body = "# Target\n\nbody v1\n";
        let hex64 = "0".repeat(64);

        let green = only_entry(&pinned_corpus(&live_token(body), body));
        let drifted = only_entry(&pinned_corpus(&live_token(body), "# Target\n\nbody v2\n"));
        let dangling = only_entry(&pinned_corpus_ref(
            "sources/target.md#^goal",
            &format!("fp1.span2.b3.{hex64}"),
            body,
        ));
        let unverifiable = only_entry(&pinned_corpus(&format!("fp9.span2.b3.{hex64}"), body));
        let malformed = only_entry(&pinned_corpus("780d2fb4cf68f60f", body));

        let labels: Vec<String> = [&green, &drifted, &dangling, &unverifiable, &malformed]
            .iter()
            .map(|e| color_label(&e.color))
            .collect();
        assert_eq!(
            labels,
            vec![
                "green",
                "red content-drifted",
                "red dangling-anchor",
                "grey unverifiable-fingerprint (unknown version)",
                "grey malformed-fingerprint",
            ],
        );
        let distinct: BTreeSet<&String> = labels.iter().collect();
        assert_eq!(
            distinct.len(),
            labels.len(),
            "two states collided: {labels:?}"
        );

        // The tones roll up honestly: the two reds are findings, the two greys
        // are not (grey = never measured, so never a claim of breakage either).
        assert_eq!(
            labels
                .iter()
                .map(|l| l.split(' ').next().unwrap())
                .collect::<Vec<_>>(),
            vec!["green", "red", "red", "grey", "grey"],
        );
    }

    /// GATE 2 (LOAD-BEARING) — grey never renders green. Each token below
    /// carries the target's CORRECT live digest under a version / codec / hashfn
    /// this build does not implement, plus the superseded bare-digest spelling.
    /// A digest that happens to match is not a verification.
    #[test]
    fn an_unverifiable_pin_never_renders_green() {
        let body = "# Target\n\nbody v1\n";
        let live = live_token(body);
        let digest = live.rsplit('.').next().expect("digest");

        for token in [
            format!("fp9.span2.b3.{digest}"),
            format!("fp1.zzz9.b3.{digest}"),
            format!("fp1.span2.xx.{digest}"),
            digest.to_string(),
        ] {
            let entry = only_entry(&pinned_corpus(&token, body));
            assert_eq!(
                color_tone(&entry.color),
                "grey",
                "{token} must render grey, got {}",
                color_label(&entry.color),
            );
            assert_ne!(entry.color, Color::Green, "{token} rendered a false green");
        }

        // The control: the SAME digest under the implemented triple is green —
        // the greys above are about the triple, not about a broken compare.
        assert_eq!(only_entry(&pinned_corpus(&live, body)).color, Color::Green);
    }

    /// GATE 3 — an `fp9.span2.b3` grey NAMES the version as the unknown member.
    /// Reporting only `codec=span2, hashfn=b3` (both live-looking) could not say
    /// which member this build does not implement.
    #[test]
    fn the_unverifiable_grey_names_the_unknown_triple_member() {
        let body = "# Target\n\nbody v1\n";
        let hex64 = "0".repeat(64);
        let cases = [
            (
                "fp9.span2.b3",
                "grey unverifiable-fingerprint (unknown version)",
            ),
            (
                "fp1.zzz9.b3",
                "grey unverifiable-fingerprint (unknown codec)",
            ),
            (
                "fp1.span2.xx",
                "grey unverifiable-fingerprint (unknown hashfn)",
            ),
            (
                "fp9.zzz9.xx",
                "grey unverifiable-fingerprint (unknown version, codec, hashfn)",
            ),
        ];
        for (triple, expected) in cases {
            let entry = only_entry(&pinned_corpus(&format!("{triple}.{hex64}"), body));
            assert_eq!(color_label(&entry.color), expected, "{triple}");
        }
    }

    /// D8 — an unreadable target is RED with its reason, never grey and never
    /// green: the vanished-anchor and vanished-page cases both.
    #[test]
    fn a_dangling_pin_target_renders_red_never_green() {
        let hex64 = "0".repeat(64);
        let token = format!("fp1.span2.b3.{hex64}");

        // The pinned anchor is gone from a live page.
        let gone = only_entry(&pinned_corpus_ref(
            "sources/target.md#^goal",
            &token,
            "# Target\n\nbody with no anchor\n",
        ));
        assert!(matches!(
            gone.color,
            Color::Red(RedReason::DanglingAnchor { .. })
        ));

        // The pinned PAGE is not in the corpus at all.
        let mut lock_block = lock::Lock::new();
        lock_block.upsert_pin(lock::PinEntry {
            declared_ref: "sources/vanished.md#^goal".to_string(),
            fingerprint: token.clone(),
        });
        let mut docs = BTreeMap::new();
        docs.insert(
            "effect.md".to_string(),
            doc(&format!("# E\n\n{}\n", lock::render(&lock_block))),
        );
        let entry = only_entry(&docs);
        assert_eq!(color_label(&entry.color), "red dangling-anchor");
    }

    /// GATE 2b — a MALFORMED lock renders grey `lock-refused`, NOT absent.
    /// Before this, `lock::find`'s refusal projected zero rows, so a corrupt
    /// lock and a page that never pinned anything were the same walk output:
    /// `(nothing)`, exit 0.
    #[test]
    fn a_malformed_lock_renders_grey_not_absent() {
        let malformed = "# Effect\n\n```meridian-lock\nversion: 1\ngarbage here\n```\n";
        let mut docs = BTreeMap::new();
        docs.insert("effect.md".to_string(), doc(malformed));

        let entry = only_entry(&docs);
        assert_eq!(
            entry.selector, "effect.md",
            "the row names the damaged page"
        );
        assert_eq!(
            color_label(&entry.color),
            "grey lock-refused (malformed at line 3: unrecognized line (canonical order: version, objects, pins))",
            "the refusal reason is carried, not just the tone",
        );
        assert!(entry.rev.is_none(), "a refusal pins nothing");

        // Grey, so it is not a finding — the ledger measured nothing here.
        let report = walk(&docs, "effect.md", Direction::Up, None).expect("walk");
        assert!(!has_red(&report));
    }

    /// GATE 2b — TWO lock blocks on one page renders grey `lock-refused`, NOT
    /// absent. `lock::find` calls two blocks corruption; the read face reports
    /// that verdict rather than guessing which block is the lock.
    #[test]
    fn a_double_block_lock_renders_grey_not_absent() {
        let block = lock::render(&{
            let mut l = lock::Lock::new();
            l.upsert_pin(lock::PinEntry {
                declared_ref: "sources/target.md".to_string(),
                fingerprint: format!("fp1.span2.b3.{}", "0".repeat(64)),
            });
            l
        });
        let mut docs = BTreeMap::new();
        docs.insert(
            "effect.md".to_string(),
            doc(&format!("# Effect\n\n{block}\n\n{block}\n")),
        );
        docs.insert("sources/target.md".to_string(), doc("# T\n\nbody\n"));

        let entry = only_entry(&docs);
        assert_eq!(entry.selector, "effect.md");
        assert_eq!(
            color_label(&entry.color),
            "grey lock-refused (more than one meridian-lock block on the page)",
        );
    }

    /// A refusal row declares NO edge: it never enters the reverse index, never
    /// enters the page adjacency, and is never traversed. Without this a
    /// refusal row pointing at its own page would make every walk over that page
    /// refuse as an in-snapshot cycle.
    #[test]
    fn a_refusal_row_is_a_leaf_never_an_edge() {
        let malformed = "# Effect\n\n```meridian-lock\nversion: 1\ngarbage here\n```\n";
        let mut docs = BTreeMap::new();
        docs.insert("effect.md".to_string(), doc(malformed));
        docs.insert("sources/target.md".to_string(), doc("# T\n\nbody\n"));

        // Up terminates (no cycle refusal) at depth 1 and expands no further.
        let up = walk(&docs, "effect.md", Direction::Up, None).expect("up must not cycle");
        assert_eq!(up.entries.len(), 1);
        assert_eq!(up.entries[0].depth, 1);

        // Down from any page never sees the refusal — it names no target.
        let down = walk(&docs, "sources/target.md", Direction::Down, None).expect("down");
        assert!(down.entries.is_empty(), "a refusal is nobody's dependent");
        let down_self = walk(&docs, "effect.md", Direction::Down, None).expect("down self");
        assert!(down_self.entries.is_empty());
    }

    // ── S11: the `objects:` retrieval plane ─────────────────────────────────

    /// [`lock_objects`] projects the `objects:` plane of every page's lock in
    /// corpus order, carries key and sha verbatim, and stays SEPARATE from the
    /// `pins:` plane: a lock with pins but no objects projects nothing here, and
    /// a page with no lock projects nothing at all.
    #[test]
    fn lock_objects_projects_the_objects_plane_verbatim() {
        let mut with_both = lock::Lock::new();
        with_both.set_object("vibe.md", &"a".repeat(40));
        with_both.set_object("second.md", &"b".repeat(40));
        with_both.upsert_pin(lock::PinEntry {
            declared_ref: "sources/target.md".to_string(),
            fingerprint: format!("fp1.span2.b3.{}", "0".repeat(64)),
        });
        let mut pins_only = lock::Lock::new();
        pins_only.upsert_pin(lock::PinEntry {
            declared_ref: "sources/target.md".to_string(),
            fingerprint: format!("fp1.span2.b3.{}", "0".repeat(64)),
        });

        let mut docs = BTreeMap::new();
        docs.insert(
            "effect.md".to_string(),
            doc(&format!("# Effect\n\n{}\n", lock::render(&with_both))),
        );
        docs.insert(
            "other.md".to_string(),
            doc(&format!("# Other\n\n{}\n", lock::render(&pins_only))),
        );
        docs.insert("plain.md".to_string(), doc("# Plain\n\nno lock here\n"));

        let objects = lock_objects(&docs);
        assert_eq!(objects.len(), 2, "only the objects plane, once each");
        assert_eq!(objects[0].src_path, "effect.md");
        assert_eq!(objects[0].key, "vibe.md");
        assert_eq!(objects[0].blob_sha, "a".repeat(40));
        assert_eq!(objects[1].key, "second.md");
        assert_eq!(objects[1].blob_sha, "b".repeat(40));
    }

    /// A REFUSED lock projects no objects — its plane is unreadable, and the
    /// damage is already named by the grey `lock-refused` row the pin plane
    /// projects for the same page. The gauge must never read a corrupt lock's
    /// objects as "none owed" WITHOUT that row beside it.
    #[test]
    fn a_refused_lock_projects_no_objects_but_still_projects_its_refusal_row() {
        let malformed = "# Effect\n\n```meridian-lock\nversion: 1\ngarbage here\n```\n";
        let mut docs = BTreeMap::new();
        docs.insert("effect.md".to_string(), doc(malformed));

        assert!(
            lock_objects(&docs).is_empty(),
            "an unreadable plane is empty"
        );
        let rows = lock_pin_colors(&docs);
        assert_eq!(rows.len(), 1, "the refusal is still visible: {rows:?}");
        assert_eq!(color_tone(&rows[0].color), "grey");
    }
}
