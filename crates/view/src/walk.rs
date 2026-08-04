//! U2.3 — the walk plane: the context-assembly listing (d2 §2.4 / §3).
//!
//! The walk computes — **per query, never stored** — the reachability listing
//! over the declared pin graph: every edge [`crate::read_face::page_lock_items`]
//! parses (the engine's own `meridian-lock` block — the legacy `^inputs`
//! form-1/form-2 readers were retired with the vocabulary, R1.3, and
//! the engine's own `meridian-lock` block). One traversal, two directions:
//!
//! - **[`Direction::Up`]** — ancestors: what the root draws from, transitively —
//!   d2 §2.4's context-assembly walk (the retired "pack" noun avoided).
//! - **[`Direction::Down`]** — descendants: who pins the root — the dependents
//!   renderer and dry-run blast radius (`--depth 1` = the direct dependents).
//!
//! Each reached edge is one [`WalkEntry`] `{selector, rev, color, depth}`, its
//! color computed by [`model::selector::classify_pin`] (U2.2). Every report
//! cites the doc revs it read ([`WalkReport::revs_read`]) — the honesty law
//! (§2.4: every answer cites the doc revs it read; a walk output is itself a
//! pinnable fact). In-snapshot cycles are errors ([`WalkError::Cycle`]).
//!
//! # Grain
//! U2.3 traverses at PAGE grain: the root is a page (a `#fragment` in the arg is
//! stripped), and a hop follows a page's whole pin set. Each entry still
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
use model::selector::{Color, GreyReason, RedReason, Selector, classify_pin};

use crate::read_face::{LockItem, corpus_index, page_lock_items_in_rooted_corpus};

/// Which way the walk runs over the lock pin graph.
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

/// Walk the lock pin graph from `root` in `direction`, bounded to
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
    walk_rooted(
        &model::RootedCorpus::ambient(docs),
        &addr::MountSet::default(),
        root,
        direction,
        depth_bound,
    )
}

/// [`walk`] against a ROOT-KEYED corpus and a mount table — the cross-root form.
///
/// The walk root itself is always an AMBIENT page (it is a workspace-relative
/// path the user names); what becomes root-aware is every EDGE it reaches. An
/// edge into a mounted root is colored against that root's documents, and an
/// edge into an unmounted one renders grey `unmounted` with the missing name.
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

    // Parse every page's pins ONCE (the shared parser), so both directions
    // read the SAME edge facts. `forward[src] = src's declared edges`.
    let forward = forward_edges(corpus, mounts);

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
/// One form only: the legacy `^inputs` readers were retired with the
/// vocabulary (R1.3), so there is no second plane to exclude. A page whose lock
/// REFUSED contributes its one grey `lock-refused` row — a corrupt lock is
/// visible here, never silently absent.
#[must_use]
pub fn lock_pin_colors(docs: &BTreeMap<String, Document>) -> Vec<PinColor> {
    lock_pin_colors_rooted(
        &model::RootedCorpus::ambient(docs),
        &addr::MountSet::default(),
    )
}

/// [`lock_pin_colors`] against a ROOT-KEYED corpus and the REAL mount table —
/// **the computer**, of which [`lock_pin_colors`] is the ambient-only case.
///
/// # Why the mount table had to become a parameter (F6)
/// Resolution is a mount lookup (U11), and this roll-up resolved every row
/// against `MountSet::default()` — an EMPTY table — while `mrd walk` one call
/// away resolved the same rows against the loaded one. The consequence was not
/// disagreement, which a comparison would have caught: it was
/// **INSENSITIVITY**. On a BOUND root a cross-root pin answered
/// `grey(unmounted)` when its target MATCHED, when it had DRIFTED, and when it
/// was RESTORED — three states, one answer, from the plane sitting under the
/// pre-commit fence. *An axis whose instrument cannot vary on that axis is
/// unevidenced (S3-R72), and a fence reading it inherits the blindness.*
///
/// The one-computer structure is unchanged and is why this was one edit: there
/// is still exactly one place a pin colour is computed, and `check`, `status`
/// and `walk` still agree BY CONSTRUCTION. **What was wrong was its INPUT, not
/// its shape** — the two blind arguments were the item resolution (an empty
/// mount table) and the target lookup (an ambient-only corpus), and both are
/// supplied by the caller now.
#[must_use]
pub fn lock_pin_colors_rooted(
    corpus: &model::RootedCorpus<'_>,
    mounts: &addr::MountSet,
) -> Vec<PinColor> {
    let docs = corpus.ambient_docs();
    let index = corpus_index(docs);
    let mut out = Vec::new();
    for (path, doc) in docs {
        for item in page_lock_items_in_rooted_corpus(path, doc, &index, corpus, mounts) {
            if !item.is_colourable() {
                // Unreachable under R4 — every pin row carries a fingerprint and
                // every refusal carries its reason. Kept as a fail-CLOSED guard:
                // a row that somehow carried neither is uncolourable, and
                // skipping it is the honest answer, not colouring it green.
                //
                // The predicate is [`LockItem::is_colourable`] and NOT a local
                // spelling of it: the board projection reads the SAME one to
                // decide which rows get a verdict, and its residue disclosure
                // counts exactly the rows skipped here. Two copies of this
                // condition could drift with nothing failing.
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

/// One RETRIEVAL-plane row of a page's `meridian-lock` block — a blob sha the
/// lock references, with the page and object that reference it.
///
/// The retrieval plane (#8 §2, git's world) is whole-file blob shas, never
/// fingerprints. It answers a different question from the CLAIM plane
/// [`PinColor`] carries — not "did the content drift" but "does this blob still
/// exist anywhere durable" — so it is projected separately and carries no color.
///
/// **R4 retired the shared `objects:` table**; the blob sha now rides the pin row
/// as its `hash`, so this projection reads the same `pins:` plane the colors read
/// and no page can carry a blob its pins do not account for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockObject {
    /// The page whose `meridian-lock` block declares this object.
    pub src_path: String,
    /// The pin's `object` — the wiki-link inner text, verbatim (what the blob is
    /// FOR). The target's vault path WITHOUT `.md`.
    pub key: String,
    /// The blob sha, verbatim — an object id in git's world, not the engine's.
    pub blob_sha: String,
}

/// Every blob a `meridian-lock` block in `docs` pins — corpus order, then pin
/// order. THE surface for a whole-corpus reachability gauge (`mrd status`'s
/// vibe-debt meter), which asks git whether each of these blobs is reachable
/// from a commit.
///
/// [`lock::find`] is the parser, exactly as it is for the colors — one owner for
/// the lock grammar, so a page's blobs and its pins can never be read by two
/// disagreeing readers. A page whose lock REFUSED contributes NOTHING here: its
/// plane is unreadable, and that damage is already named by the grey
/// `lock-refused` row [`lock_pin_colors`] projects for the same page.
///
/// # One blob, one row (R4)
/// R4 moved the sha onto the pin row, so the whole-file lock — `path: []` and
/// `properties: []` on one object — pins the SAME blob twice. Deduped by
/// `(src_path, object, hash)`: two rows naming one blob would make the vibe-debt
/// meter count one debt as two, and `check::layer0` would report one orphan as
/// two findings.
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
/// **The full TEACHING REFUSAL for a color that has one** — `None` when the
/// reason word already says everything.
///
/// Re-exported from [`model::selector`] for the SAME reason [`color_tone`] is:
/// the §4.6 link plane's refusal rows are minted in `wire-serve`, which cannot
/// depend on this crate, and two `match`es over one enum is how a board and a
/// walk start disagreeing about one address.
pub use model::selector::{color_detail, color_reason, color_teaching};

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

/// The hops out of `page` in `direction`.
///
/// - **Up**: `page`'s own declared edges — each points at a target it draws
///   from. The entry names the target; the next hop follows the target page.
/// - **Down**: the reverse — every `(src, edge)` whose edge targets `page`. The
///   entry names the dependent `src`; the color rests on the pinned target
///   (`page`); the next hop follows `src`.
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
                // Root-qualified, so a cross-root target is a leaf and is never
                // confused with an ambient file of the same path.
                color_target: edge_page(edge),
                next_page: edge_page(edge),
            })
            .collect(),
        Direction::Down => {
            let mut steps = Vec::new();
            for (src, edges) in forward {
                for edge in edges {
                    // Only an AMBIENT edge reverses into an ambient page: a
                    // cross-root edge whose in-root path happens to equal an
                    // ambient path names a different document entirely.
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
    // An UNMOUNTED edge never resolved, so it is named by the address the page
    // declared — the only honest name available, and the one the refusal teaches
    // against.
    if edge.root_refusal.is_some() {
        return edge.declared_ref.clone();
    }
    edge_address(edge)
}

/// One edge's canonical, ROOT-QUALIFIED address: `root:path[#sel]` for an edge
/// that resolved into a mounted root, `path[#sel]` for an ambient one.
///
/// **The qualification is load-bearing, not cosmetic.** A cross-root target
/// resolves to a path INSIDE its own root — `notes.md` in the `sessions` root —
/// and the ambient corpus may hold its own `notes.md`. An unqualified name would
/// (a) print an address that reads as the ambient file, and (b) let the BFS
/// traverse into the ambient corpus at that key, which is FINDING 03 reappearing
/// one layer up: the right verdict on the right bytes, followed by a walk into
/// the wrong document.
///
/// Qualifying makes a cross-root target a LEAF by construction: the ambient
/// corpus holds no key spelled `root:path`, so `docs.contains_key` is false and
/// the page is never expanded or reversed into. Walking INTO another root's own
/// pin graph is a separate capability no Core unit asks for.
fn edge_address(edge: &LockItem) -> String {
    let canonical = canonical_ref(&edge.to_path, &edge.to_sel);
    match &edge.to_root {
        Some(root) => format!("{root}:{canonical}"),
        None => canonical,
    }
}

/// The PAGE an edge points at, root-qualified — [`edge_address`] without the
/// selector. The BFS traverses on this, so it carries the same leaf property.
fn edge_page(edge: &LockItem) -> String {
    match &edge.to_root {
        Some(root) => format!("{root}:{}", edge.to_path),
        None => edge.to_path.clone(),
    }
}

/// Color one edge: parse the target selector, resolve it against the root the
/// address named, and answer on the FINGERPRINT plane.
///
/// Three rows are answered before the fingerprint compare, each because it
/// declares no verifiable edge at all:
///
/// - a **lock-refusal row** ([`LockItem::lock_refusal`]) — the page's whole
///   `meridian-lock` block is unreadable, so it is grey `lock-refused` with the
///   refusal carried;
/// - an **unmounted / unseeable root** ([`LockItem::root_refusal`]) — grey
///   outranks red (R-3), because nothing drifted; the ledger stopped being able
///   to measure; and
/// - a **`meridian-lock` pin** (form-3), which pins a `fp1.…` CID-token
///   answered by [`model::selector::classify_pin`] over
///   [`LockItem::fingerprint`], the typed slot.
///
/// **The retired foreign-algo short-circuit.** A pin minted under a `hash-algo`
/// this engine does not compute used to render grey `superseded-algo` ahead of
/// the `node_rev` compare. R4 removed both: every pin now carries a
/// self-describing `fp1.…` token, so the FINGERPRINT plane owns that case and
/// spells it `unverifiable-fingerprint`, NAMING which triple member is unknown.
/// The subject moved planes; it was not dropped.
fn edge_color(corpus: &model::RootedCorpus<'_>, edge: &LockItem) -> Color {
    if let Some(reason) = &edge.lock_refusal {
        return Color::Grey(GreyReason::LockRefused {
            reason: reason.clone(),
        });
    }
    // **R-3 — grey OUTRANKS red, and it is checked FIRST.** An address naming an
    // unmounted root is grey WHATEVER ELSE IS TRUE of the target: a cross-root
    // pin that was green and whose root is later unmounted becomes grey, never
    // red. Nothing drifted — the ledger simply stopped being able to measure.
    //
    // Ordering matters here rather than being incidental. Every arm below
    // classifies against a target document, and the target document of an
    // unmounted root is ABSENT — which classifies as red `selector-unresolved`.
    // Checking grey second would therefore render exactly the plausible-looking
    // wrong answer this unit exists to remove.
    if let Some(reason) = &edge.root_refusal {
        return Color::Grey(reason.clone());
    }
    // U21 — the root was REACHED and the file is not in it. This must be read
    // before the target lookup below, because that lookup finds nothing and
    // classifies as red `selector-unresolved` — the plausible-looking wrong
    // cause, asserting that the page resolved when the page is what is absent.
    if let Some(root) = &edge.root_absence {
        return Color::Red(RedReason::FileNotFound {
            root: root.clone(),
            path: edge.to_path.clone(),
            selector: (!edge.to_sel.is_empty()).then(|| edge.to_sel.clone()),
        });
    }
    // The target's bytes come from the root the address RESOLVED INTO — never
    // the ambient corpus. Reading the ambient one here is FINDING 03's wrong
    // success wearing a verdict.
    let target = match &edge.to_root {
        Some(root) => corpus
            .root(root)
            .and_then(|mounted| mounted.docs().get(&edge.to_path)),
        None => corpus.ambient_docs().get(&edge.to_path),
    };
    // **STRUCTURE, never the joined string.** R4's selector is an array, and
    // `Selector::parse` on the `/`-joined display spelling re-splits it — which
    // turns a heading whose own text contains `/` into two headings naming a
    // section that does not exist. The structural arm is read whenever the row
    // carries one; the joined parse survives only for rows that never had an
    // array to begin with.
    let selector = match &edge.selector {
        Some(structural) => model_selector(&edge.object, structural),
        None => Selector::parse(&canonical_ref(&edge.to_path, &edge.to_sel)),
    };
    if let Some(token) = &edge.fingerprint {
        return classify_pin(&selector, token, target);
    }
    // **THE FAIL-CLOSED TAIL.** Arms 1-3 are the whole colour law for a lock
    // row: a refusal, an unmounted root, or a fingerprint. Reaching here means
    // the row carried NONE of them — it names no evidence and reports no failure
    // to read any, so neither plane has a compare that can answer it.
    //
    // **GUARDED AT ONE POINT — not impossible.** Every R4 pin row carries a
    // fingerprint and every refusal carries its reason, which is what makes this
    // unreachable from live input. That invariant is ONE PARSER RULE with ONE
    // TEST on it:
    // `lock::a_pin_row_missing_a_mandatory_field_refuses_at_parse`
    // (`crates/lock/src/lib.rs`). One rule, one carrier — a single point of
    // failure stated as one, so the next reader inherits "guarded at one point"
    // rather than "cannot happen".
    //
    // **DO NOT DELETE THIS ARM AS DEAD CODE.** Its live population is zero
    // because arms 1 and 3 are jointly exhaustive over the two `LockItem`
    // producers (`read_face::collect_lock_pins`) — that answers "can a row reach
    // here TODAY", NOT "is the tail unreachable in principle". Two of
    // `edge_color`'s three callers (`steps_from`, both directions) do not filter,
    // so the arm is structurally reachable; and the function must yield a
    // `Color`, so removing it makes the fall-through green — a fail-OPEN default
    // on a reachable path, reporting success.
    Color::Grey(GreyReason::Uncolourable)
}

/// Translate R4's structural selector into the model's address selector — the
/// TRANSLATION LAYER `classify_pin` needs, not a change to its signature.
///
/// The three arms are R4's own, and each is named in the schema:
/// - `path: []` — the whole body without frontmatter, which is the document root;
/// - `path: ["^id"]` — the ANCHOR pin: a `^id` as the SOLE element, block-grain,
///   never widened to the host section (R4 provenance note 2). A `^` reaching
///   here in any other position was refused at mint (U8: mixed arrays are
///   unruled and refused loudly), so the sole-element test is the whole rule;
/// - `path: [...]` — a heading chain, carried SEGMENT FOR SEGMENT. Nothing is
///   split and nothing is joined, so a heading containing `/` survives — the
///   case the array form exists for.
///
/// `properties:` has no analogue in the address enum: the frontmatter is not a
/// body node. It maps to the document root, where its `props1` token meets the
/// SPAN verifier and is refused loudly as unverifiable rather than being answered
/// wrongly — the never-conflate-hash-planes law doing its job at read time.
///
/// # The transcript arm is why this takes the `object`
/// `Selector::parse` recognized `session#seq-N` as [`Selector::ImmutableRoot`]
/// from the FRAGMENT — a class that is *recognized, stored opaque, rendered grey
/// `immutable-root`, and never resolved* (d2 §2.2). Read structurally without the
/// object, `path: ["seq-160"]` is indistinguishable from a heading named
/// `seq-160`, and a transcript pin would classify as an unresolved HEADING —
/// **grey turning into red**, a false finding on a ref the engine is ruled never
/// to resolve. The object carries the session id, so the arm needs both halves.
fn model_selector(object: &str, selector: &lock::Selector) -> Selector {
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
            Selector::Heading(segments.clone())
        }
        Some(_) => Selector::Heading(segments.clone()),
    }
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

    /// The three-doc chain fixture `a.md -> b.md -> c.md`, every pin GREEN, in
    /// R4 `meridian-lock` form: build `c`, pin its LIVE fingerprint token in
    /// `b`'s lock block, build `b`, pin `b`'s live token in `a`, build `a`.
    /// Returns the corpus + the three pinned TOKENS so a test asserts byte-exact
    /// entries.
    ///
    /// Pre-R4 this chain was two `^inputs` blocks pinning `node_rev`s. The rev
    /// plane is retired with the vocabulary (R1.3), so the chain now greens
    /// through the FINGERPRINT compare — the same law, the surviving carrier.
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

    /// One whole-body pin on `object` at `token` — `path: []` is the body
    /// without frontmatter, which is the document root the token was minted over.
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

    /// The Test (gate 1): the three-doc chain's `walk` up-output is byte-expected
    /// — ordered `{selector, rev, color, depth}` entries with depth tags, plus
    /// the rev citations for every doc the listing rests on (§2.4 honesty).
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

        // Every answer cites the doc revs it read (§2.4): root a, plus b and c.
        // A citation is the containing doc's NODE REV — distinct from the pin
        // TOKEN the entry carries above, and the two must not be conflated: the
        // citation says which bytes were read, the token says what was claimed.
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

    /// An in-snapshot cycle is an error (§2.4), never an infinite walk: `x` pins
    /// `y`, `y` pins `x`.
    #[test]
    fn in_snapshot_cycle_is_an_error() {
        // The pins need not be GREEN — a cycle is a traversal fact, and the walk
        // must refuse to loop whatever colour the edges carry.
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

    /// A transcript ref (`session#seq-N`) renders `grey immutable-root` and is a
    /// walk leaf — recognized, never resolved, never traversed (§2.2 / §2.4).
    #[test]
    fn transcript_input_renders_grey_immutable_root() {
        // R4 spells this as the object `22-01-session` with `path: ["seq-160"]`.
        // Read WITHOUT the object the array is indistinguishable from a heading
        // named `seq-160`, and this pin would render red `selector-unresolved` —
        // a false finding on a ref the engine is ruled never to resolve. That is
        // why `model_selector` takes the object.
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
                selector: "22-01-session.md#seq-160".to_string(),
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

    /// The fail-closed sentinel renders its own indictment, not a fact about a
    /// target — condition 3 of the bronze act. A reader who meets this line must
    /// learn from the LINE that it is a defect, without consulting a doc comment.
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

    /// Mint an R4 pin from a `page[#A/B]` ref SPELLING — a fixture convenience
    /// only. The spelling is split into the `object` and the selector ARRAY
    /// here, at the fixture's own door; nothing downstream ever sees the joined
    /// form. `^id` and `seq-N` fragments stay single-element arrays, which is
    /// what makes them the anchor and transcript arms.
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
        lock_block.upsert_pin(pin_from_spelling("sources/vanished.md#^goal", &token));
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

    /// A refusal row declares NO edge: it never enters the reverse index, never
    /// enters the page adjacency, and is never traversed. Without this a
    /// refusal row pointing at its own page would make every walk over that page
    /// refuse as an in-snapshot cycle.
    #[test]
    fn a_refusal_row_is_a_leaf_never_an_edge() {
        let malformed = "# Effect\n\n```meridian-lock\nversion: 2\ngarbage here\n```\n";
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

    // ── S11: the retrieval plane (R4: the per-pin `hash`) ───────────────────

    /// [`lock_objects`] projects the blob every pin references, corpus order,
    /// key and sha verbatim — and **deduped by `(page, object, hash)`**.
    ///
    /// The dedup is the R4 law, not an optimisation. R4 retired the shared
    /// `objects:` table and moved the sha onto the pin row, so the whole-file
    /// lock — `path: []` and `properties: []` on ONE object — references the same
    /// blob twice. Two rows for one blob would make the vibe-debt meter count one
    /// debt as two and `check::layer0` report one orphan as two findings, which
    /// is the "naming one defect twice" trap that plane already refuses.
    ///
    /// The positive control is in the fixture: `effect.md` carries the two-row
    /// whole-file lock AND a distinct second object, so a dedup that collapsed
    /// too much would drop `second` and fail here.
    #[test]
    fn lock_objects_dedupes_one_blob_per_object_never_per_pin() {
        let token = format!("fp1.span2.b3.{}", "0".repeat(64));
        let mut whole_file = lock::Lock::new();
        // The whole-file lock: body and frontmatter, two pins, ONE blob.
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
        // A genuinely different object — the control that keeps the dedup honest.
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
}
