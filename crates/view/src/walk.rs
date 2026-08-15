//! Walk plane — context-assembly listing over the declared pin graph
//! (d2 §2.4 / §3). **Per query, never stored.**
//!
//! Edges from [`crate::read_face::page_lock_items`] (`meridian-lock` only).
//! - **[`Direction::Up`]** — ancestors (what the root draws from).
//! - **[`Direction::Down`]** — descendants / blast radius (`--depth 1` = direct).
//!
//! Each entry is `{selector, rev, color, depth}` via
//! [`model::selector::classify_pin`]. Report cites doc revs read
//! ([`WalkReport::revs_read`]; §2.4 honesty). In-snapshot cycles are errors.
//!
//! # Grain
//! Page grain: root is a page (`#fragment` stripped); hop follows whole pin set.
//! Entries still carry full selector. Selector-grain deferred to wire.
//!
//! # Never stored
//! Pure function of shared-borrowed corpus — no writer, Connection, or fs
//! handle; cannot persist by construction.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use model::Document;
use model::selector::{Color, GreyReason, RedReason, Selector, classify_pin};

use crate::read_face::{LockItem, corpus_index, page_lock_items_in_rooted_corpus};

/// Walk direction over the lock pin graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Ancestors — what the root draws from.
    Up,
    /// Descendants — who pins the root (blast radius).
    Down,
}

impl Direction {
    /// Stable lowercase label (`up` / `down`).
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Direction::Up => "up",
            Direction::Down => "down",
        }
    }
}

/// One reached edge: depth-tagged, color-computed, never stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalkEntry {
    /// Canonical selector of the reached end (target for Up, dependent for Down).
    pub selector: String,
    /// Pinned rev (`None` = declared-only / grey).
    pub rev: Option<String>,
    /// Computed color with reason.
    pub color: Color,
    /// Hops from root (direct edge = 1).
    pub depth: u32,
}

/// Doc rev the walk read — §2.4 honesty citation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevCitation {
    pub path: String,
    /// Document-root rev at read time.
    pub doc_rev: String,
}

/// Completed walk — listing + rev citations. Caller-owned; engine stores none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalkReport {
    pub direction: Direction,
    pub root: String,
    /// Depth bound (`None` = unbounded); no entry exceeds it.
    pub depth_bound: Option<u32>,
    /// BFS order: ascending depth then discovery.
    pub entries: Vec<WalkEntry>,
    /// Doc revs the listing rests on, path order (§2.4).
    pub revs_read: Vec<RevCitation>,
}

/// Walk that cannot answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WalkError {
    /// Root page not in corpus.
    RootNotFound(String),
    /// In-snapshot cycle reachable from root (§2.4). Page loop for the refusal.
    Cycle(Vec<String>),
}

/// Walk lock pin graph from `root` in `direction`, optional `depth_bound`.
/// Per query, never stored. `#fragment` stripped (page grain).
///
/// # Errors
/// [`WalkError::RootNotFound`] / [`WalkError::Cycle`].
pub fn walk(
    docs: &BTreeMap<String, Document>,
    root: &str,
    direction: Direction,
    depth_bound: Option<u32>,
) -> Result<WalkReport, WalkError> {
    walk_rooted(
        &model::RootedCorpus::ambient(docs),
        &addr::MountSet::default(),
        root,
        direction,
        depth_bound,
    )
}

/// [`walk`] against root-keyed corpus + mount table. Root is ambient; edges
/// are root-aware (mounted root coloured against that root; unmounted → grey).
///
/// # Errors
/// As [`walk`].
pub fn walk_rooted(
    corpus: &model::RootedCorpus<'_>,
    mounts: &addr::MountSet,
    root: &str,
    direction: Direction,
    depth_bound: Option<u32>,
) -> Result<WalkReport, WalkError> {
    let docs = corpus.ambient_docs();
    let root_page = page_of(root).to_string();
    if !docs.contains_key(&root_page) {
        return Err(WalkError::RootNotFound(root_page));
    }

    // Shared parser once — both directions read the same edge facts.
    let forward = forward_edges(corpus, mounts);

    // In-snapshot cycle is an error, not a silent stop (§2.4).
    if let Some(cycle) = find_cycle(&page_adjacency(&forward, docs, direction), &root_page) {
        return Err(WalkError::Cycle(cycle));
    }

    let mut entries = Vec::new();
    // Citations: root + every named page + every live colour target (§2.4).
    let mut read: BTreeSet<String> = BTreeSet::new();
    read.insert(root_page.clone());

    // BFS, page-keyed; dedupe by whole row `(selector, rev, color_label)` at min
    // depth. Selector alone would collapse two pins on one ref with different
    // verdicts (measured red vanishing behind green).
    let mut queue: VecDeque<(String, u32)> = VecDeque::new();
    let mut enqueued: BTreeSet<String> = BTreeSet::new();
    let mut emitted: BTreeSet<(String, Option<String>, String)> = BTreeSet::new();
    queue.push_back((root_page.clone(), 0));
    enqueued.insert(root_page.clone());

    while let Some((page, depth)) = queue.pop_front() {
        let next_depth = depth + 1;
        if depth_bound.is_some_and(|bound| next_depth > bound) {
            continue;
        }
        for step in steps_from(corpus, &forward, &page, direction) {
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

/// Every mount root the corpus's `meridian-lock` addresses name — the exact
/// set of roots worth building a corpus for. A pin's root is a property of
/// its address, not of the tree it points into, so the set is knowable from
/// the ambient corpus alone. The root is read off
/// [`LockItem::declared_addr`], the structural owner, so nothing re-splits
/// `declared_ref`. A row with no address contributes no root.
/// Every mount root the corpus's wikilink/embed targets name — the set of
/// roots whose pages the link plane (and the SQL projection of it) can
/// resolve into. Mounted root corpora exist so `resolve_ref` can answer a
/// rooted spelling, so a workspace carrying none needs zero.
///
/// `path` mirrors the links plane: `None` scans the whole ambient corpus;
/// `Some` scans that one file. The root name is read from
/// [`addr::Addr::parse`] of each target — the same grammar the resolver
/// peels. A target outside the grammar contributes no root.
///
/// Shared owner for the CLI verbs (`mrd sql`, `mrd links`) and the § A.11
/// wire serve path — two assemblies would hand two corpora to one resolver.
#[must_use]
pub fn link_addressed_roots(
    docs: &BTreeMap<String, Document>,
    path: Option<&str>,
) -> BTreeSet<addr::MountName> {
    let mut roots = BTreeSet::new();
    for (source, doc) in docs {
        if path.is_some_and(|p| p != source.as_str()) {
            continue;
        }
        collect_link_roots(&doc.root, &mut roots);
    }
    roots
}

fn collect_link_roots(node: &model::Node, roots: &mut BTreeSet<addr::MountName>) {
    match &node.kind {
        model::NodeKind::Wikilink { target, .. } | model::NodeKind::Embed { target, .. } => {
            if let Ok(addr) = addr::Addr::parse(target)
                && let Some(root) = addr.root()
            {
                roots.insert(root.clone());
            }
        }
        _ => {}
    }
    for child in &node.children {
        collect_link_roots(child, roots);
    }
}

#[must_use]
pub fn lock_addressed_roots(docs: &BTreeMap<String, Document>) -> BTreeSet<addr::MountName> {
    let mut roots = BTreeSet::new();
    for doc in docs.values() {
        for item in crate::read_face::page_lock_items(doc) {
            if let Some(root) = item.declared_addr.as_ref().and_then(addr::Addr::root) {
                roots.insert(root.clone());
            }
        }
    }
    roots
}

/// One `meridian-lock` row with its computed color (status roll-up / decorator).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinColor {
    pub src_path: String,
    /// Declared ref verbatim; empty on lock-refusal.
    pub declared_ref: String,
    /// Pinned `fp1.…` token; `None` on lock-refusal.
    pub fingerprint: Option<String>,
    pub color: Color,
}

/// Every `meridian-lock` row with colour — same [`edge_color`] as walk. Refused
/// lock contributes one grey `lock-refused` (never silent absence).
#[must_use]
pub fn lock_pin_colors(docs: &BTreeMap<String, Document>) -> Vec<PinColor> {
    lock_pin_colors_rooted(
        &model::RootedCorpus::ambient(docs),
        &addr::MountSet::default(),
    )
}

/// [`lock_pin_colors`] with real mount table — **the** colour computer.
/// Caller supplies mounts + rooted corpus so cross-root pins are not
/// insensitively greyed as `unmounted` (F6).
#[must_use]
pub fn lock_pin_colors_rooted(
    corpus: &model::RootedCorpus<'_>,
    mounts: &addr::MountSet,
) -> Vec<PinColor> {
    lock_pin_colors_rooted_with_sources(corpus, mounts, &BTreeMap::new())
}

/// [`lock_pin_colors_rooted`] over ADDITIONAL pin SOURCES — pages that hold
/// pins but whose bytes the hash domain does not carry.
///
/// `mrd pin` admits an out-of-domain holder and mints the pin at exit 0, so a
/// face that reads its rows only from the hashed corpus asserts a universal
/// over a population it silently narrowed. `extra_sources` are read for their
/// `meridian-lock` rows and for nothing else.
///
/// ⚠️ **The corpus is NOT widened, and that is the load-bearing part.** The
/// index and the corpus below are built from the ambient docs alone, so a pin's
/// TARGET resolves in exactly the world it resolved in before: an out-of-domain
/// target stays `grey(outside-hash-domain)` — reported, never gated — and no
/// ambient link resolves that did not resolve already. Holder and target are
/// independent axes (session decision 0045); this widens the holder axis only.
#[must_use]
pub fn lock_pin_colors_rooted_with_sources(
    corpus: &model::RootedCorpus<'_>,
    mounts: &addr::MountSet,
    extra_sources: &BTreeMap<String, Document>,
) -> Vec<PinColor> {
    let docs = corpus.ambient_docs();
    let index = corpus_index(docs);
    let mut out = Vec::new();
    for (path, doc) in docs.iter().chain(extra_sources.iter()) {
        for item in page_lock_items_in_rooted_corpus(path, doc, &index, corpus, mounts) {
            if !item.is_colourable() {
                // Fail-closed: uncolourable is skip, never green. Same predicate
                // as board projection so residue counts exactly these rows.
                continue;
            }
            out.push(PinColor {
                src_path: path.clone(),
                declared_ref: item.declared_ref.clone(),
                fingerprint: item.fingerprint.clone(),
                color: edge_color(corpus, &item),
            });
        }
    }
    out
}

/// Retrieval-plane row: blob sha a lock references (git world, no colour).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockObject {
    pub src_path: String,
    /// Pin `object` (path without `.md`).
    pub key: String,
    /// Blob sha (git object id).
    pub blob_sha: String,
}

/// Every blob pinned by a `meridian-lock` in `docs`. Same parser as colours.
/// Refused lock contributes nothing (damage named by grey lock-refused).
/// Deduped by `(src_path, object, hash)` — whole-file lock pins same blob twice.
#[must_use]
pub fn lock_objects(docs: &BTreeMap<String, Document>) -> Vec<LockObject> {
    let mut out = Vec::new();
    let mut seen: BTreeSet<(String, String, String)> = BTreeSet::new();
    for (path, doc) in docs {
        let Ok(Some(found)) = lock::find(doc) else {
            continue;
        };
        for pin in found.lock.pins {
            if !seen.insert((path.clone(), pin.object.clone(), pin.hash.clone())) {
                continue;
            }
            out.push(LockObject {
                src_path: path.clone(),
                key: pin.object,
                blob_sha: pin.hash,
            });
        }
    }
    out
}

/// Whether the listing carries any red edge (finding signal).
#[must_use]
pub fn has_red(report: &WalkReport) -> bool {
    report
        .entries
        .iter()
        .any(|e| matches!(e.color, Color::Red(_)))
}

/// Color tone (`green` / `grey` / `red`). Re-exported from model so board and
/// walk cannot disagree via two matches.
pub use model::selector::color_tone;

/// Reason / detail / teaching words — same re-export rationale as [`color_tone`].
pub use model::selector::{color_detail, color_reason, color_teaching};

/// Full human color label: tone, reason, optional detail.
#[must_use]
pub fn color_label(color: &Color) -> String {
    let tone = color_tone(color);
    match (color_reason(color), color_detail(color)) {
        (Some(reason), Some(detail)) => format!("{tone} {reason} ({detail})"),
        (Some(reason), None) => format!("{tone} {reason}"),
        (None, _) => tone.to_string(),
    }
}

/// One BFS hop.
struct Step {
    selector: String,
    pinned_rev: Option<String>,
    color: Color,
    /// Live target the colour rested on.
    color_target: String,
    next_page: String,
}

/// Parse every page's pins once (shared parser), `to_path` resolved.
fn forward_edges(
    corpus: &model::RootedCorpus<'_>,
    mounts: &addr::MountSet,
) -> BTreeMap<String, Vec<LockItem>> {
    let docs = corpus.ambient_docs();
    let index = corpus_index(docs);
    docs.iter()
        .map(|(path, doc)| {
            (
                path.clone(),
                page_lock_items_in_rooted_corpus(path, doc, &index, corpus, mounts),
            )
        })
        .collect()
}

/// Hops out of `page`: Up = own edges; Down = reverse ambient edges only.
fn steps_from(
    corpus: &model::RootedCorpus<'_>,
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
                color: edge_color(corpus, edge),
                // Root-qualified so cross-root is a leaf, not ambient same-path.
                color_target: edge_page(edge),
                next_page: edge_page(edge),
            })
            .collect(),
        Direction::Down => {
            let mut steps = Vec::new();
            for (src, edges) in forward {
                for edge in edges {
                    // Ambient-only reverse: cross-root same path is a different doc.
                    if edge.to_root.is_none() && edge.root_refusal.is_none() && edge.to_path == page
                    {
                        steps.push(Step {
                            selector: src.clone(),
                            pinned_rev: edge.pinned_rev.clone(),
                            color: edge_color(corpus, edge),
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

/// Listing name of one hop: canonical target, or the page on lock-refusal
/// (refusal is a leaf — empty `to_path` keeps it out of reverse/adjacency).
fn step_selector(src: &str, edge: &LockItem) -> String {
    if edge.lock_refusal.is_some() {
        return src.to_string();
    }
    // Unmounted: name by declared address (only honest name).
    if edge.root_refusal.is_some() {
        return edge.declared_ref.clone();
    }
    edge_address(edge)
}

/// Canonical root-qualified address. Qualification is load-bearing: without it
/// BFS walks ambient same-path (wrong document). Cross-root is a leaf by
/// construction (`root:path` absent from ambient keys).
///
/// Display rides the live grammar — `path §selector`, never the retired
/// `path#selector` join (ZT ruling 2026-08-14, walk-wire boundary: ONE
/// grammar everywhere, display values included).
fn edge_address(edge: &LockItem) -> String {
    let canonical = display_ref(&edge.to_path, &edge.to_sel);
    match &edge.to_root {
        Some(root) => format!("{root}:{canonical}"),
        None => canonical,
    }
}

/// Page an edge points at, root-qualified — BFS key; same leaf property.
fn edge_page(edge: &LockItem) -> String {
    match &edge.to_root {
        Some(root) => format!("{root}:{}", edge.to_path),
        None => edge.to_path.clone(),
    }
}

/// Colour one edge on the fingerprint plane.
///
/// Pre-compare arms: lock-refusal → grey; unmounted root → grey (R-3, first);
/// target absent from the ambient disk → red `file-not-found` (decisions 0049
/// and 0054, absence outranks domain membership on BOTH planes); target outside
/// the hash domain and present → grey `outside-hash-domain` (R-3, before every
/// red); root absence → red `file-not-found`; else `classify_pin` on
/// fingerprint.
fn edge_color(corpus: &model::RootedCorpus<'_>, edge: &LockItem) -> Color {
    if let Some(reason) = &edge.lock_refusal {
        return Color::Grey(GreyReason::LockRefused {
            reason: reason.clone(),
        });
    }
    // R-3: grey outranks red — checked FIRST. Unmounted target is ABSENT to
    // later arms (would mis-classify as red selector-unresolved).
    if let Some(reason) = &edge.root_refusal {
        return Color::Grey(reason.clone());
    }
    // ABSENCE OUTRANKS DOMAIN MEMBERSHIP, ON BOTH PLANES (decisions 0049 and
    // 0054): THE ORDER OF QUESTIONS IS THE ORDER OF FACTS. Existence is a fact
    // about the DISK, and the disk does not know the domain — so this arm is
    // asked FIRST, ahead of the domain arm below and every address arm after
    // it, rather than nested inside the out-of-domain branch it arrived in.
    //
    // It is a DISK READ of the named path at the root it resolves under
    // (decision 0045's mechanism), asked of the corpus's builder. The corpus
    // map cannot stand in for it on EITHER plane: out of domain, an excluded
    // target is missing from the map whether it is on disk or deleted (0049's
    // defect); in domain, the map does record the absence, but the arm that
    // reads it asserts a resolution — `selector-unresolved` claims the page
    // resolved, `dangling-anchor` claims its anchor vanished — and with the page
    // gone there is no page to resolve (0054's defect). `root_absence` is not
    // the signal either: `read_face` sets it only for a miss inside a MOUNTED
    // root. `None` here = the face supplied no disk: cannot say, so every
    // pre-0049 verdict stands rather than a guess.
    if corpus.on_ambient_disk(edge.to_root.as_ref(), &edge.to_path) == Some(false) {
        return Color::Red(RedReason::FileNotFound {
            root: None,
            path: edge.to_path.clone(),
            selector: (!edge.to_sel.is_empty()).then(|| edge.to_sel.clone()),
        });
    }
    // R-3: the hash domain gates HASHING, not addressing, so a target it
    // excludes was never loaded into this corpus and every red below would
    // assert an absence nobody measured (§12.1 verdict-plane clause, decision
    // 0034). `pin` attests such a target at rc=0, so red here sends the caller
    // to destroy the engine's own attestation. The grey is scoped to a target
    // that EXISTS and cannot be hashed — the arm above already took the absent
    // case. `None` = the face supplied no domain: cannot say, so nothing
    // changes.
    if corpus.in_hash_domain(edge.to_root.as_ref(), &edge.to_path) == Some(false) {
        return Color::Grey(GreyReason::OutsideHashDomain {
            path: edge.to_path.clone(),
        });
    }
    // U21: root reached, file absent — before ambient miss → wrong red cause.
    if let Some(root) = &edge.root_absence {
        return Color::Red(RedReason::FileNotFound {
            root: Some(root.clone()),
            path: edge.to_path.clone(),
            selector: (!edge.to_sel.is_empty()).then(|| edge.to_sel.clone()),
        });
    }
    // Bytes from the resolved root — never ambient (wrong-bytes success).
    let target = match &edge.to_root {
        Some(root) => corpus
            .root(root)
            .and_then(|mounted| mounted.docs().get(&edge.to_path)),
        None => corpus.ambient_docs().get(&edge.to_path),
    };
    // Structure, never joined string (heading with `/` must not re-split).
    let selector = match &edge.selector {
        Some(structural) => model_selector(&edge.object, structural),
        None => Selector::parse(&canonical_ref(&edge.to_path, &edge.to_sel)),
    };
    if let Some(token) = &edge.fingerprint {
        return classify_pin(&selector, token, target);
    }
    // FAIL-CLOSED TAIL. Live population zero under R4 (parser refuses missing
    // fields) but structurally reachable — callers do not all filter. Deleting
    // this arm makes fall-through green: fail-OPEN on a reachable path.
    // Guard: `lock::a_pin_row_missing_a_mandatory_field_refuses_at_parse`.
    Color::Grey(GreyReason::Uncolourable)
}

/// R4 structural selector → model address selector (translation for
/// `classify_pin`). Arms: `path:[]` → Page; sole `^id` → Block; sole `seq-N` +
/// object → `ImmutableRoot` (needs object — without it, grey becomes false red);
/// else Heading segment-for-segment, each segment read through the R4
/// occurrence spelling (`"Dup#2"` → `n: Some(2)`, r8 D3 — this door is the one
/// place the stored spelling becomes an address; [`lock::parse_occurrence`]
/// owns the grammar). Public (U22) so historical repair shares one grammar
/// reading.
#[must_use]
pub fn model_selector(object: &str, selector: &lock::Selector) -> Selector {
    let lock::Selector::Path(segments) = selector else {
        return Selector::Page;
    };
    match segments.split_first() {
        None => Selector::Page,
        Some((only, [])) => {
            if let Some(id) = only.strip_prefix('^') {
                return Selector::Block(id.to_string());
            }
            if let Some(seq) = only
                .strip_prefix("seq-")
                .and_then(|n| n.parse::<u64>().ok())
            {
                return Selector::ImmutableRoot {
                    session: object.to_string(),
                    seq,
                };
            }
            Selector::Heading(heading_segments(segments))
        }
        Some(_) => Selector::Heading(heading_segments(segments)),
    }
}

/// R4 path elements → occurrence-aware heading segments, one spelling owner
/// ([`lock::parse_occurrence`]).
fn heading_segments(segments: &[String]) -> Vec<model::HpathSeg> {
    segments
        .iter()
        .map(|seg| {
            let (h, n) = lock::parse_occurrence(seg);
            model::HpathSeg {
                h: h.to_string(),
                n,
            }
        })
        .collect()
}

/// Page-level adjacency for cycle check — corpus-present pages only (leaves
/// never close a cycle).
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

/// Cycle reachable from `start` (DFS gray set) → page loop, or `None` for DAG.
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
            // Back-edge on stack — close the loop.
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

/// Page part of a selector (`a.md#Heading` → `a.md`).
fn page_of(selector: &str) -> &str {
    selector.split_once('#').map_or(selector, |(page, _)| page)
}

/// Canonical `page#sel` (or bare `page` for doc root).
fn canonical_ref(to_path: &str, to_sel: &str) -> String {
    if to_sel.is_empty() {
        to_path.to_string()
    } else {
        format!("{to_path}#{to_sel}")
    }
}

/// The SERVED spelling of a target+selector pair: `path §selector` — the live
/// grammar every teaching surface speaks since the stale-teaching sweep.
/// [`canonical_ref`]'s `#` join stays internal (it feeds `Selector::parse`,
/// the stored-plane grammar); it never rides a served row.
fn display_ref(to_path: &str, to_sel: &str) -> String {
    if to_sel.is_empty() {
        to_path.to_string()
    } else {
        format!("{to_path} §{to_sel}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(raw: &str) -> Document {
        model::build(raw.to_string(), syntax::parse(raw))
    }

    /// `a.md → b.md → c.md` chain, every pin green via live fingerprint tokens.
    fn three_doc_chain() -> (BTreeMap<String, Document>, String, String, String) {
        let c_raw = "# C\n\nleaf body\n".to_string();
        let c_token = live_token(&c_raw);

        let b_raw = format!("# B\n\ndraws from c\n\n{}\n", chain_block("c", &c_token));
        let b_token = live_token(&b_raw);

        let a_raw = format!("# A\n\ndraws from b\n\n{}\n", chain_block("b", &b_token));
        let a_token = live_token(&a_raw);

        let mut docs = BTreeMap::new();
        docs.insert("a.md".to_string(), doc(&a_raw));
        docs.insert("b.md".to_string(), doc(&b_raw));
        docs.insert("c.md".to_string(), doc(&c_raw));
        (docs, a_token, b_token, c_token)
    }

    /// Whole-body pin on `object` at `token` (`path: []` = document root).
    fn chain_block(object: &str, token: &str) -> String {
        let mut l = lock::Lock::new();
        l.upsert_pin(lock::PinEntry::new(
            object,
            "9ae3f1deadbeef",
            lock::Selector::Path(Vec::new()),
            token,
        ));
        lock::render(&l)
    }

    /// Gate 1: three-doc up-walk is byte-expected + §2.4 rev citations.
    #[test]
    fn three_doc_chain_up_is_byte_expected() {
        let (docs, _a_token, b_rev, c_rev) = three_doc_chain();
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

        // §2.4: citations are doc node revs (not pin tokens).
        let doc_rev = |p: &str| docs[p].root.node_rev.0.clone();
        assert_eq!(
            report.revs_read,
            vec![
                RevCitation {
                    path: "a.md".to_string(),
                    doc_rev: doc_rev("a.md"),
                },
                RevCitation {
                    path: "b.md".to_string(),
                    doc_rev: doc_rev("b.md"),
                },
                RevCitation {
                    path: "c.md".to_string(),
                    doc_rev: doc_rev("c.md"),
                },
            ]
        );
    }

    /// Gate 2: `--down --depth 1` is direct dependents only (bound load-bearing).
    #[test]
    fn down_depth_one_is_exactly_direct_dependents() {
        let (docs, _a, _b, c_rev) = three_doc_chain();

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

        // Unbounded includes transitive a@2 — proves the bound fired above.
        let full = walk(&docs, "c.md", Direction::Down, None).expect("walk down full");
        let reached: Vec<(&str, u32)> = full
            .entries
            .iter()
            .map(|e| (e.selector.as_str(), e.depth))
            .collect();
        assert_eq!(reached, vec![("b.md", 1), ("a.md", 2)]);
    }

    /// Drifted pin is red and a finding.
    #[test]
    fn drifted_pin_renders_red_and_is_a_finding() {
        let (mut docs, ..) = three_doc_chain();
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

    /// In-snapshot cycle is an error (§2.4), never infinite walk.
    #[test]
    fn in_snapshot_cycle_is_an_error() {
        // Cycle is a traversal fact — colour irrelevant.
        let hex = "0".repeat(64);
        let x = doc(&format!(
            "# X\n\ndraws from y\n\n{}\n",
            chain_block("y", &format!("fp1.span2.b3.{hex}"))
        ));
        let y = doc(&format!(
            "# Y\n\ndraws from x\n\n{}\n",
            chain_block("x", &format!("fp1.span2.b3.{hex}"))
        ));
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

    /// Transcript `session#seq-N` → grey immutable-root leaf (§2.2 / §2.4).
    #[test]
    fn transcript_input_renders_grey_immutable_root() {
        // model_selector needs object: without it seq-N looks like a heading.
        let token = "fp1.span2.b3.".to_string() + &"0".repeat(64);
        let mut block = lock::Lock::new();
        block.upsert_pin(pin_from_spelling("22-01-session#seq-160", &token));
        let a = doc(&format!(
            "# A\n\ndraws from a transcript\n\n{}\n",
            lock::render(&block)
        ));
        let mut docs = BTreeMap::new();
        docs.insert("a.md".to_string(), a);

        let report = walk(&docs, "a.md", Direction::Up, None).expect("walk");
        assert_eq!(
            report.entries,
            vec![WalkEntry {
                // Live grammar (2026-08-14 ruling): the SERVED row spells
                // `path §selector`; the `#` in the pin spelling above is the
                // stored plane's and stays.
                selector: "22-01-session.md §seq-160".to_string(),
                rev: Some(token),
                color: Color::Grey(GreyReason::ImmutableRoot),
                depth: 1,
            }],
            "the transcript class is grey immutable-root and a LEAF — the \
             display spelling moved with R4, the classification did not"
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
        assert_eq!(color_tone(&Color::Grey(GreyReason::Uncolourable)), "grey");
        assert_eq!(
            color_reason(&Color::Red(RedReason::SelectorUnresolved {
                candidates: vec![]
            })),
            Some("selector-unresolved")
        );
    }

    /// Uncolourable render names itself as the defect (not a target fact).
    #[test]
    fn uncolourable_render_says_its_appearance_is_the_defect() {
        let label = color_label(&Color::Grey(GreyReason::Uncolourable));
        assert!(
            label.starts_with("grey uncolourable ("),
            "the sentinel keeps the tone/reason shape every other colour uses: {label}"
        );
        assert!(
            label.contains("itself the defect"),
            "the rendered text must say its own appearance is the finding: {label}"
        );
        assert!(
            label.contains("neither a fingerprint nor a refusal"),
            "and must name the row shape that produced it: {label}"
        );
    }

    /// meridian-lock-only page is visible; mismatched token is measured drift.
    #[test]
    fn meridian_lock_page_is_visible_to_the_walk() {
        let token = format!("fp1.span2.b3.{}", "ab".repeat(32));
        let mut lock_block = lock::Lock::new();
        lock_block.upsert_pin(pin_from_spelling("sources/target-page.md", &token));
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

    /// Effect page pins `sources/target.md` at `token`.
    fn pinned_corpus(token: &str, target_raw: &str) -> BTreeMap<String, Document> {
        pinned_corpus_ref("sources/target.md", token, target_raw)
    }

    /// [`pinned_corpus`] with explicit declared ref.
    fn pinned_corpus_ref(
        declared_ref: &str,
        token: &str,
        target_raw: &str,
    ) -> BTreeMap<String, Document> {
        let mut lock_block = lock::Lock::new();
        lock_block.upsert_pin(pin_from_spelling(declared_ref, token));
        let effect = format!(
            "# Effect\n\ndraws from it\n\n{}\n",
            lock::render(&lock_block)
        );
        let mut docs = BTreeMap::new();
        docs.insert("effect.md".to_string(), doc(&effect));
        docs.insert("sources/target.md".to_string(), doc(target_raw));
        docs
    }

    /// Fixture: split `page[#A/B]` spelling into object + selector array.
    fn pin_from_spelling(spelling: &str, token: &str) -> lock::PinEntry {
        let (target, fragment) = match spelling.split_once('#') {
            Some((t, f)) => (t, f),
            None => (spelling, ""),
        };
        let object = target.strip_suffix(".md").unwrap_or(target);
        let selector = if fragment.is_empty() {
            lock::Selector::Path(Vec::new())
        } else {
            lock::Selector::Path(fragment.split('/').map(str::to_string).collect())
        };
        lock::PinEntry::new(object, "9ae3f1deadbeef", selector, token)
    }

    fn only_entry(docs: &BTreeMap<String, Document>) -> WalkEntry {
        let report = walk(docs, "effect.md", Direction::Up, None).expect("walk up");
        assert_eq!(report.entries.len(), 1, "one pin, one entry");
        report.entries[0].clone()
    }

    /// Live fingerprint of a page root (correct pin).
    fn live_token(raw: &str) -> String {
        let d = doc(raw);
        model::fingerprint::fingerprint(&d, &d.root)
            .expect("the fixture page has content")
            .into_string()
    }

    /// The served row's display grammar (ZT ruling 2026-08-14, walk-wire
    /// boundary): a section-scoped claim spells `path §selector` — the
    /// retired `path#selector` join never rides a served entry, whatever the
    /// stored pin spelled.
    #[test]
    fn a_section_scoped_entry_spells_the_live_grammar_never_the_hash_join() {
        let body = "# Target\n\ncontent\n";
        let hex64 = "0".repeat(64);
        let entry = only_entry(&pinned_corpus_ref(
            "sources/target.md#Target",
            &format!("fp1.span2.b3.{hex64}"),
            body,
        ));
        assert_eq!(entry.selector, "sources/target.md §Target");
        assert!(
            !entry.selector.contains(".md#"),
            "the retired join never rides a served row: {}",
            entry.selector
        );
    }

    /// Gate 1: five pin states render distinct labels.
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

        // Reds are findings; greys are not (never measured).
        assert_eq!(
            labels
                .iter()
                .map(|l| l.split(' ').next().unwrap())
                .collect::<Vec<_>>(),
            vec!["green", "red", "red", "grey", "grey"],
        );
    }

    /// Gate 2 (load-bearing): unverifiable pin never greens (matched digest ≠ verify).
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

        // Control: same digest under implemented triple is green.
        assert_eq!(only_entry(&pinned_corpus(&live, body)).color, Color::Green);
    }

    /// Gate 3: unverifiable grey names which triple member is unknown.
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

    /// Unreadable target is red (never grey/green): vanished anchor and page.
    #[test]
    fn a_dangling_pin_target_renders_red_never_green() {
        let hex64 = "0".repeat(64);
        let token = format!("fp1.span2.b3.{hex64}");

        let gone = only_entry(&pinned_corpus_ref(
            "sources/target.md#^goal",
            &token,
            "# Target\n\nbody with no anchor\n",
        ));
        assert!(matches!(
            gone.color,
            Color::Red(RedReason::DanglingAnchor { .. })
        ));

        let mut lock_block = lock::Lock::new();
        lock_block.upsert_pin(pin_from_spelling("sources/vanished.md#^goal", &token));
        let mut docs = BTreeMap::new();
        docs.insert(
            "effect.md".to_string(),
            doc(&format!("# E\n\n{}\n", lock::render(&lock_block))),
        );
        let entry = only_entry(&docs);
        assert_eq!(color_label(&entry.color), "red dangling-anchor");
    }

    /// Malformed lock → grey `lock-refused`, not silent absence.
    #[test]
    fn a_malformed_lock_renders_grey_not_absent() {
        let malformed = "# Effect\n\n```meridian-lock\nversion: 2\ngarbage here\n```\n";
        let mut docs = BTreeMap::new();
        docs.insert("effect.md".to_string(), doc(malformed));

        let entry = only_entry(&docs);
        assert_eq!(
            entry.selector, "effect.md",
            "the row names the damaged page"
        );
        assert_eq!(
            color_label(&entry.color),
            "grey lock-refused (malformed at line 3: unrecognized line (canonical order: version, pins))",
            "the refusal reason is carried, not just the tone",
        );
        assert!(entry.rev.is_none(), "a refusal pins nothing");

        let report = walk(&docs, "effect.md", Direction::Up, None).expect("walk");
        assert!(!has_red(&report));
    }

    /// Two lock blocks → grey `lock-refused`, not silent absence.
    #[test]
    fn a_double_block_lock_renders_grey_not_absent() {
        let block = lock::render(&{
            let mut l = lock::Lock::new();
            l.upsert_pin(pin_from_spelling(
                "sources/target.md",
                &format!("fp1.span2.b3.{}", "0".repeat(64)),
            ));
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

    /// Refusal is a leaf: no reverse index, no adjacency, no traversal (else cycle).
    #[test]
    fn a_refusal_row_is_a_leaf_never_an_edge() {
        let malformed = "# Effect\n\n```meridian-lock\nversion: 2\ngarbage here\n```\n";
        let mut docs = BTreeMap::new();
        docs.insert("effect.md".to_string(), doc(malformed));
        docs.insert("sources/target.md".to_string(), doc("# T\n\nbody\n"));

        let up = walk(&docs, "effect.md", Direction::Up, None).expect("up must not cycle");
        assert_eq!(up.entries.len(), 1);
        assert_eq!(up.entries[0].depth, 1);

        let down = walk(&docs, "sources/target.md", Direction::Down, None).expect("down");
        assert!(down.entries.is_empty(), "a refusal is nobody's dependent");
        let down_self = walk(&docs, "effect.md", Direction::Down, None).expect("down self");
        assert!(down_self.entries.is_empty());
    }

    /// [`lock_objects`] dedupes by `(page, object, hash)` — whole-file lock
    /// pins same blob twice; two rows would double-count debt.
    #[test]
    fn lock_objects_dedupes_one_blob_per_object_never_per_pin() {
        let token = format!("fp1.span2.b3.{}", "0".repeat(64));
        let mut whole_file = lock::Lock::new();
        // Whole-file: body + frontmatter pins, one blob.
        whole_file.upsert_pin(lock::PinEntry::new(
            "vibe",
            &"a".repeat(40),
            lock::Selector::Path(Vec::new()),
            &token,
        ));
        whole_file.upsert_pin(lock::PinEntry::new(
            "vibe",
            &"a".repeat(40),
            lock::Selector::Properties(Vec::new()),
            &token,
        ));
        // Control: distinct object must survive dedup.
        whole_file.upsert_pin(lock::PinEntry::new(
            "second",
            &"b".repeat(40),
            lock::Selector::Path(Vec::new()),
            &token,
        ));

        let mut docs = BTreeMap::new();
        docs.insert(
            "effect.md".to_string(),
            doc(&format!("# Effect\n\n{}\n", lock::render(&whole_file))),
        );
        docs.insert("plain.md".to_string(), doc("# Plain\n\nno lock here\n"));

        let objects = lock_objects(&docs);
        assert_eq!(
            objects.len(),
            2,
            "three pins, two blobs — the whole-file lock is ONE debt: {objects:?}"
        );
        assert_eq!(objects[0].src_path, "effect.md");
        assert_eq!(objects[0].key, "vibe");
        assert_eq!(objects[0].blob_sha, "a".repeat(40));
        assert_eq!(objects[1].key, "second");
        assert_eq!(objects[1].blob_sha, "b".repeat(40));

        // A page with no lock projects nothing at all.
        let only_plain =
            BTreeMap::from([("plain.md".to_string(), doc("# Plain\n\nno lock here\n"))]);
        assert!(lock_objects(&only_plain).is_empty());
    }

    /// A REFUSED lock projects no blobs — its plane is unreadable, and the
    /// damage is already named by the grey `lock-refused` row the pin plane
    /// projects for the same page. The gauge must never read a corrupt lock's
    /// blobs as "none owed" WITHOUT that row beside it.
    #[test]
    fn a_refused_lock_projects_no_objects_but_still_projects_its_refusal_row() {
        let malformed = "# Effect\n\n```meridian-lock\nversion: 2\ngarbage here\n```\n";
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

    /// The §12.1 verdict-plane law, as a family — the gate for the P0 where the
    /// engine mints a pin and two commands later calls it broken.
    ///
    /// A domain that excludes exactly one path, asserted over the WHOLE family
    /// on ONE corpus: the excluded target greys with its own reason word, and
    /// every other outcome is UNCHANGED. The controls are the point — a fix that
    /// greys everything passes the excluded arm alone.
    #[test]
    fn an_out_of_domain_target_greys_and_every_other_verdict_is_unchanged() {
        let target_raw = "# Target\n\nbody v1\n";
        let token = live_token(target_raw);

        // Four pinning pages, one per arm, all pinning at the SAME live token.
        let mut docs = BTreeMap::new();
        docs.insert(
            "excluded-src.md".to_string(),
            doc(&format!(
                "# S\n\n{}\n",
                chain_block("ignored/target", &token)
            )),
        );
        docs.insert(
            "green-src.md".to_string(),
            doc(&format!("# S\n\n{}\n", chain_block("kept", &token))),
        );
        docs.insert(
            "drift-src.md".to_string(),
            doc(&format!("# S\n\n{}\n", chain_block("drifted", &token))),
        );
        docs.insert(
            "miss-src.md".to_string(),
            doc(&format!("# S\n\n{}\n", chain_block("gone", &token))),
        );
        // The in-domain targets. `ignored/target.md` is deliberately ABSENT from
        // the map exactly as the hash-domain walk leaves it, and `gone.md` is
        // absent because it does not exist — the same corpus fact, two causes.
        docs.insert("kept.md".to_string(), doc(target_raw));
        docs.insert(
            "drifted.md".to_string(),
            doc("# Target\n\nbody v2 edited\n"),
        );

        let domain = Excluding(&["ignored/target.md"]);
        let corpus = model::RootedCorpus::ambient(&docs).with_hash_domain(&domain);
        let colors: BTreeMap<String, Color> =
            lock_pin_colors_rooted(&corpus, &addr::MountSet::default())
                .into_iter()
                .map(|pin| (pin.src_path, pin.color))
                .collect();

        // The arm under test: policy, not blindness — and it names the path.
        let excluded = &colors["excluded-src.md"];
        assert_eq!(
            excluded,
            &Color::Grey(GreyReason::OutsideHashDomain {
                path: "ignored/target.md".to_string()
            }),
            "an out-of-domain target is grey: the engine did not look"
        );
        assert_eq!(color_reason(excluded), Some("outside-hash-domain"));

        // The three controls, on the same run and the same corpus.
        assert_eq!(
            colors["green-src.md"],
            Color::Green,
            "an in-domain pin at its live token still reads green"
        );
        assert_eq!(
            colors["drift-src.md"],
            Color::Red(RedReason::Drifted),
            "in-domain drift is still red — greying it would hide real drift"
        );
        assert!(
            matches!(colors["miss-src.md"], Color::Red(_)),
            "an in-domain target that is genuinely absent stays RED: there the \
             engine DID look. Got {:?}",
            colors["miss-src.md"]
        );
    }

    /// Without a domain the corpus cannot say, and cannot-say must not become
    /// grey: the pre-0034 verdict stands rather than a guess.
    #[test]
    fn a_corpus_with_no_domain_colours_exactly_as_before() {
        let token = live_token("# Target\n\nbody v1\n");
        let mut docs = BTreeMap::new();
        docs.insert(
            "src.md".to_string(),
            doc(&format!(
                "# S\n\n{}\n",
                chain_block("ignored/target", &token)
            )),
        );
        let corpus = model::RootedCorpus::ambient(&docs);
        let rows = lock_pin_colors_rooted(&corpus, &addr::MountSet::default());
        assert!(
            matches!(rows[0].color, Color::Red(_)),
            "no domain supplied ⇒ no claim about the domain: {:?}",
            rows[0].color
        );
    }

    /// A domain that holds everything except the paths it is told to exclude.
    #[derive(Debug)]
    struct Excluding(&'static [&'static str]);
    impl model::HashDomain for Excluding {
        fn contains(&self, rel: &str) -> bool {
            !self.0.contains(&rel)
        }
    }

    /// A disk holding exactly the paths it is given, and nothing else. Every
    /// answer is MEASURED (`Some`) — `None` is the separate cannot-say world
    /// its own test below owns.
    #[derive(Debug)]
    struct Holding(&'static [&'static str]);
    impl model::AmbientDisk for Holding {
        fn exists(&self, rel: &str) -> Option<bool> {
            Some(self.0.contains(&rel))
        }
    }

    /// Pin on `object` at `token` with an explicit selector path (`[]` = page).
    fn chain_block_at(object: &str, token: &str, segments: &[&str]) -> String {
        let mut l = lock::Lock::new();
        l.upsert_pin(lock::PinEntry::new(
            object,
            "9ae3f1deadbeef",
            lock::Selector::Path(segments.iter().map(|s| (*s).to_string()).collect()),
            token,
        ));
        lock::render(&l)
    }

    /// The one corpus the absence family is read over: seven declaring pages,
    /// one pin each, and the in-domain PRESENT targets.
    ///
    /// `ignored/present.md` is ON DISK and absent from this map because the
    /// domain excluded it; `in-gone.md` is absent because it is not there. **That
    /// the map cannot tell those two apart is the whole reason the disk is
    /// asked** — and it is why the fixture below supplies a disk rather than
    /// trusting the corpus.
    fn absence_family_corpus() -> BTreeMap<String, Document> {
        let live_raw = "# Task\n\nbody v1\n";
        let token = live_token(live_raw);
        let mut docs = BTreeMap::new();
        let mut src = |name: &str, object: &str, segments: &[&str]| {
            docs.insert(
                format!("{name}.md"),
                doc(&format!(
                    "# S\n\n{}\n",
                    chain_block_at(object, &token, segments)
                )),
            );
        };
        // Absent targets — the two planes, at page grain and at block grain.
        src("indomain-gone-src", "in-gone", &[]);
        src("indomain-gone-block-src", "in-gone-block", &["^a1"]);
        src("excluded-gone-src", "ignored/gone", &[]);
        // Present targets — the verdicts that must NOT move.
        src("excluded-present-src", "ignored/present", &[]);
        src("moved-heading-src", "live", &["Taskk"]);
        src("green-src", "live", &[]);
        src("drift-src", "drifted", &[]);
        docs.insert("live.md".to_string(), doc(live_raw));
        docs.insert("drifted.md".to_string(), doc("# Task\n\nbody v2 edited\n"));
        docs
    }

    /// §12.1's absence law AS A FAMILY (decisions 0049 + 0054): an absent page
    /// is `file-not-found` WHEREVER it is absent — on BOTH planes, and ahead of
    /// every address arm.
    ///
    /// Seven arms on ONE corpus, ONE domain, ONE disk, ONE run, because the
    /// finding this fixture gates is a DISAGREEMENT BETWEEN ARMS and only a
    /// shared run can show one. The pairing is the point twice over:
    ///
    /// - `indomain-gone` vs `excluded-gone` — same deletion, same bytes, the
    ///   ONLY variable is whether the domain excludes the path. Before this fix
    ///   the verdict word FLIPPED across that pair; the law says it must not.
    /// - `excluded-gone` vs `excluded-present` — same exclusion, the only
    ///   variable is the disk. The word MUST flip across that pair (0049), and
    ///   a fix that reddens everything absent from the corpus map breaks it.
    ///
    /// `moved-heading` is the control that keeps `selector-unresolved` honest:
    /// the word survives, with its candidate list, exactly where its claim is
    /// TRUE — the page resolved and the selector failed. A fix that retired the
    /// word instead of scoping it fails there.
    #[test]
    fn an_absent_target_is_file_not_found_on_both_planes_and_presence_keeps_its_verdict() {
        let docs = absence_family_corpus();
        let domain = Excluding(&["ignored/gone.md", "ignored/present.md"]);
        let disk = Holding(&[
            "live.md",
            "drifted.md",
            "ignored/present.md",
            "indomain-gone-src.md",
            "indomain-gone-block-src.md",
            "excluded-gone-src.md",
            "excluded-present-src.md",
            "moved-heading-src.md",
            "green-src.md",
            "drift-src.md",
        ]);
        let corpus = model::RootedCorpus::ambient(&docs)
            .with_hash_domain(&domain)
            .with_ambient_disk(&disk);
        let colors: BTreeMap<String, Color> =
            lock_pin_colors_rooted(&corpus, &addr::MountSet::default())
                .into_iter()
                .map(|pin| (pin.src_path, pin.color))
                .collect();

        // ── The absent family: one word on both planes, both grains. ──
        let absent = |path: &str| {
            Color::Red(RedReason::FileNotFound {
                root: None,
                path: path.to_string(),
                selector: None,
            })
        };
        assert_eq!(
            colors["indomain-gone-src.md"],
            absent("in-gone.md"),
            "IN-DOMAIN and absent is file-not-found (0054): with the page gone \
             there is no page to resolve, so selector-unresolved would assert a \
             resolution that did not occur"
        );
        assert_eq!(
            colors["excluded-gone-src.md"],
            absent("ignored/gone.md"),
            "OUT-OF-DOMAIN and absent is file-not-found (0049), unchanged"
        );
        assert_eq!(
            color_reason(&colors["indomain-gone-src.md"]),
            color_reason(&colors["excluded-gone-src.md"]),
            "THE PAIR IS THE FINDING: same deletion, same bytes, the only \
             variable is domain membership — and the verdict word must NOT flip"
        );
        assert_eq!(
            colors["indomain-gone-block-src.md"],
            Color::Red(RedReason::FileNotFound {
                root: None,
                path: "in-gone-block.md".to_string(),
                selector: Some("^a1".to_string()),
            }),
            "the existence question runs ahead of EVERY address arm, so a block \
             address on an absent page is file-not-found and not dangling-anchor \
             — dangling-anchor asserts the page resolved just as loudly"
        );

        // ── The present family: every verdict UNCHANGED. ──
        assert_eq!(
            colors["excluded-present-src.md"],
            Color::Grey(GreyReason::OutsideHashDomain {
                path: "ignored/present.md".to_string()
            }),
            "OUT-OF-DOMAIN and PRESENT stays grey (0049's other state): a fix \
             that reddens everything missing from the corpus map breaks here"
        );
        let Color::Red(RedReason::SelectorUnresolved { candidates }) =
            &colors["moved-heading-src.md"]
        else {
            panic!(
                "a moved heading on a page that EXISTS is still \
                 selector-unresolved — the word is scoped, not retired. Got {:?}",
                colors["moved-heading-src.md"]
            );
        };
        assert_eq!(
            candidates,
            &vec!["Task".to_string()],
            "and it carries the candidate list that the absent case structurally \
             cannot — which is why the two worlds needed two words"
        );
        assert_eq!(
            colors["green-src.md"],
            Color::Green,
            "an in-domain pin at its live token still reads green"
        );
        assert_eq!(
            colors["drift-src.md"],
            Color::Red(RedReason::Drifted),
            "in-domain drift is still red — hiding it behind absence would be \
             the same collapse in the other direction"
        );
    }

    /// Without a disk the corpus cannot say, and cannot-say must not become a
    /// red: the pre-0049 verdict stands rather than a guess — the mirror of
    /// `a_corpus_with_no_domain_colours_exactly_as_before`.
    ///
    /// The no-fire control for the arm above: same absent in-domain target,
    /// same run shape, and the ONLY variable removed is the disk.
    #[test]
    fn a_corpus_with_no_disk_never_mints_an_absence_it_did_not_measure() {
        let token = live_token("# Task\n\nbody v1\n");
        let mut docs = BTreeMap::new();
        docs.insert(
            "src.md".to_string(),
            doc(&format!("# S\n\n{}\n", chain_block("in-gone", &token))),
        );
        let domain = Excluding(&[]);
        let corpus = model::RootedCorpus::ambient(&docs).with_hash_domain(&domain);
        let rows = lock_pin_colors_rooted(&corpus, &addr::MountSet::default());
        assert_eq!(
            color_reason(&rows[0].color),
            Some("selector-unresolved"),
            "no disk supplied ⇒ no measured absence ⇒ no file-not-found: {:?}",
            rows[0].color
        );
    }
}
