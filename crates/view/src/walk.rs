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
use model::selector::{Color, GreyReason, RedReason, Selector, classify_edge};

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

    // BFS, page-keyed traversal; entries deduped by canonical selector at min
    // depth (BFS visits shallower first, so the first sighting is the min depth).
    let mut queue: VecDeque<(String, u32)> = VecDeque::new();
    let mut enqueued: BTreeSet<String> = BTreeSet::new();
    let mut emitted: BTreeSet<String> = BTreeSet::new();
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
            if emitted.insert(step.selector.clone()) {
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
#[must_use]
pub fn color_tone(color: &Color) -> &'static str {
    match color {
        Color::Green => "green",
        Color::Grey(_) => "grey",
        Color::Red(_) => "red",
    }
}

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
        Color::Red(RedReason::Drifted) => Some("content-drifted"),
        Color::Red(RedReason::DanglingAnchor { .. }) => Some("dangling-anchor"),
        Color::Red(RedReason::SelectorUnresolved { .. }) => Some("selector-unresolved"),
    }
}

/// The full color label (`green`, `red content-drifted`, `grey immutable-root`,
/// …) — tone plus reason, the human-render word.
#[must_use]
pub fn color_label(color: &Color) -> String {
    match color_reason(color) {
        Some(reason) => format!("{} {reason}", color_tone(color)),
        None => color_tone(color).to_string(),
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
                selector: canonical_ref(&edge.to_path, &edge.to_sel),
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
/// A `meridian-lock` pin (form-3) reaches this gate carrying its fingerprint
/// token's own version as the algo label (`fp1`) — likewise non-native, because
/// a `fp1.…` CID-token is not `node_rev`-comparable in either direction. So a
/// lock pin renders grey here: VISIBLE and honestly unverified, never a false
/// green and never a false drift. Turning that grey into the real fingerprint
/// verdict is the pin-color unit's job, over [`LockItem::fingerprint`].
fn edge_color(docs: &BTreeMap<String, Document>, edge: &LockItem) -> Color {
    if edge.pinned_rev.is_some()
        && edge
            .hash_algo
            .as_deref()
            .is_some_and(|a| !model::is_native_algo(a))
    {
        return Color::Grey(GreyReason::SupersededAlgo);
    }
    let selector = Selector::parse(&canonical_ref(&edge.to_path, &edge.to_sel));
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
    /// The edge lands grey: a `fp1.…` fingerprint is not `node_rev`-comparable,
    /// so it is honestly unverified HERE (the fingerprint verdict belongs to the
    /// pin-color unit) — visible, never a false green, never a false drift.
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
                color: Color::Grey(GreyReason::SupersededAlgo),
                depth: 1,
            }],
            "the lock pin IS the edge — pre-S3 this vec was empty",
        );
        assert!(!has_red(&report), "an unverified lock pin is not a finding");

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
}
