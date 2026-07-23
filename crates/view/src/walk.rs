//! U2.3 — the walk plane: the context-assembly listing (d2 §2.4 / §3).
//!
//! The walk computes — **per query, never stored** — the reachability listing
//! over the `^inputs` pin graph (the edges [`crate::read_face::page_lock_items`]
//! parses). One traversal, two directions:
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

use crate::read_face::{LockItem, page_lock_items};

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

/// Parse every page's `^inputs` edges once (the shared parser).
fn forward_edges(docs: &BTreeMap<String, Document>) -> BTreeMap<String, Vec<LockItem>> {
    docs.iter()
        .map(|(path, doc)| (path.clone(), page_lock_items(doc)))
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
fn edge_color(docs: &BTreeMap<String, Document>, edge: &LockItem) -> Color {
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
            "# B\n\ndraws from c\n\n```yaml ^inputs\nhash-algo: statusd-file-rev\nitems:\n  - {{ref: 'c.md', rev: '{c_rev}', rev_class: 'content'}}\n```\n"
        );
        let b = doc(&b_raw);
        let b_rev = b.root.node_rev.0.clone();

        let a_raw = format!(
            "# A\n\ndraws from b\n\n```yaml ^inputs\nhash-algo: statusd-file-rev\nitems:\n  - {{ref: 'b.md', rev: '{b_rev}', rev_class: 'content'}}\n```\n"
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
    }
}
