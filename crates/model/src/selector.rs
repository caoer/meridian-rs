//! Selector grammar (four classes), edge-color law, ambiguity teaching refusal
//! (U2.2).
//!
//! # Grammar (d2 §2.2)
//! | class | form | color |
//! |---|---|---|
//! | [`Selector::Page`] | `page` | document root |
//! | [`Selector::Heading`] | `page#Heading` (`/`-joined) | section |
//! | [`Selector::Block`] | `page#^block-id` | block anchor |
//! | [`Selector::ImmutableRoot`] | `session-id#seq-N` | grey `immutable-root`, never resolved |
//!
//! # Edge-color law (d2 §2.3; decision #9)
//! Per edge, computed never stored: **green** (live == pinned), **red**
//! (resolves-but-drifted or address fails), **grey** (ledger cannot verify).
//! Address-failure reds are distinct from content-drift:
//!
//! - [`RedReason::Drifted`] — resolves, content moved.
//! - [`RedReason::DanglingAnchor`] — pinned `^block-id` vanished.
//! - [`RedReason::SelectorUnresolved`] — heading/page resolves to nothing.
//!
//! Address failures carry nearest-candidate hints (d1 F6b) — never auto-repair.
//!
//! Address half: [`resolve_selector`]. Compare: [`classify_pin`] via
//! [`crate::fingerprint::verify_content_span`].
//!
//! **Grey never green** — ledger did not measure; unknown codec, unreadable
//! token, refused lock stay grey regardless of digest.

use crate::fingerprint::ContentVerdict;
use crate::{Document, Node, NodeKind, Ref, ResolveError, Target, resolve};

/// The four selector classes (d2 §2.2), parsed by [`Selector::parse`].
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
    /// Classify a ref string into its selector class.
    ///
    /// The fragment after the first `#` decides the class: `^…` → [`Block`],
    /// `seq-<digits>` → [`ImmutableRoot`], anything else → [`Heading`]
    /// (`/`-split). No `#` → [`Page`].
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

/// One edge's computed color (d2 §2.3) — three tones; reds and greys carry
/// their reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Color {
    /// `live_rev == pinned_rev` — the pinned content is current.
    Green,
    /// The ledger cannot verify this edge; the reason names why.
    Grey(GreyReason),
    /// The pinned edge is wrong; the reason distinguishes content drift from an
    /// address that no longer resolves (decision #9).
    Red(RedReason),
}

/// Why an edge renders grey (the ledger cannot verify it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GreyReason {
    /// A `session-id#seq-N` transcript hop — recognized, not verified; the
    /// address class cannot drift by construction (d2 §2.2/§2.3).
    ImmutableRoot,
    /// The pinned selector became ambiguous (a duplicate appeared) — the ledger
    /// cannot say which target the pin meant (d1 § selector ambiguity, point 3).
    Ambiguous,
    /// A `meridian-lock` pin whose fingerprint token parses but names a
    /// version / codec / hashfn this build does not implement
    /// ([`crate::fingerprint::ContentVerdict::Unverifiable`]). `unknown` names
    /// which triple members are unknown, so the render never prints a
    /// live-looking triple. Grey, never green and never red.
    UnverifiableFingerprint { unknown: Vec<&'static str> },
    /// Cross-root address naming a root this machine does not bind
    /// (`docs/address-grammar.md` § 8 M6). **Grey, never red** — outside sight,
    /// nothing drifted. Distinct from `file_not_found` (measured absence in a
    /// mounted root). Carries the name so the refusal can teach the fix (D8).
    ///
    /// **R-3 — grey outranks red:** unmounting a formerly-green root goes grey,
    /// never red, and grey → exit 0 is refused (S3-R7 ③). Not unified with
    /// `mrd check` cannot-assess (address vs validity plane).
    Unmounted {
        /// Canonical root name this machine does not bind.
        root: addr::MountName,
    },
    /// Root declared but path unreadable / absent / empty corpus
    /// (`docs/address-grammar.md` § 8 M6). Distinct from
    /// [`GreyReason::Unmounted`] (S3-R43): the root is declared, so the refusal
    /// names the **path**, never "declare it".
    PathUnseeable {
        /// Canonical name the file declares.
        root: addr::MountName,
        /// Declared path.
        path: String,
        /// Underlying reason, verbatim.
        detail: String,
    },
    /// Pinned value is not a fingerprint token
    /// ([`crate::fingerprint::ContentVerdict::Malformed`]). Grey, never red.
    MalformedFingerprint,
    /// Whole `meridian-lock` refused parse (malformed / multi-block). Projects
    /// this row rather than zero pins — corrupt lock must not read as "no pins".
    LockRefused { reason: String },
    /// Fail-closed: lock row with neither fingerprint nor refusal. Grey — no
    /// measure; if this renders, the render is the finding. Guarded by
    /// `lock::a_pin_row_missing_a_mandatory_field_refuses_at_parse`.
    Uncolourable,
}

/// Why an edge renders red — the reasons kept distinct (decision #9).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RedReason {
    /// The address resolves, but `live_rev != pinned_rev` — measured content
    /// drift. A `red(drifted)` pin never refuses; `realise` converges it (U2.5).
    Drifted,
    /// A pinned `page#^block-id` whose target anchor vanished (delete/rename).
    /// Carries the live toc's nearest block-id candidates (never auto-repair).
    DanglingAnchor { candidates: Vec<String> },
    /// A pinned heading/page selector that resolves to nothing. Carries the
    /// live toc's nearest heading candidates.
    SelectorUnresolved { candidates: Vec<String> },
    /// Measured absence (U21): root bound+readable, path missing in corpus.
    /// Red, not grey — the engine looked (grey = outside sight, see
    /// [`GreyReason::Unmounted`]). Not [`SelectorUnresolved`], which asserts the
    /// page resolved and the selector failed.
    ///
    /// [`SelectorUnresolved`]: RedReason::SelectorUnresolved
    FileNotFound {
        /// Root of the miss — never the ambient root.
        root: addr::MountName,
        /// Path missing inside that root.
        path: String,
        /// Selector as declared (`None` = page grain). Joined at render only (R1.6).
        selector: Option<String>,
    },
}

/// Color tone word (`green` / `grey` / `red`) — sole vocabulary for all
/// surfaces (walk/status, claim-link).
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
        Color::Grey(GreyReason::Ambiguous) => Some("ambiguous"),
        Color::Grey(GreyReason::Uncolourable) => Some("uncolourable"),
        Color::Grey(GreyReason::UnverifiableFingerprint { .. }) => Some("unverifiable-fingerprint"),
        Color::Grey(GreyReason::MalformedFingerprint) => Some("malformed-fingerprint"),
        Color::Grey(GreyReason::LockRefused { .. }) => Some("lock-refused"),
        Color::Grey(GreyReason::Unmounted { .. }) => Some("unmounted"),
        // Shared word (S3-R49); a config compile-assert fails the build on drift.
        Color::Grey(GreyReason::PathUnseeable { .. }) => Some(addr::PATH_UNSEEABLE_REASON_WORD),
        Color::Red(RedReason::Drifted) => Some("content-drifted"),
        Color::Red(RedReason::DanglingAnchor { .. }) => Some("dangling-anchor"),
        Color::Red(RedReason::SelectorUnresolved { .. }) => Some("selector-unresolved"),
        Color::Red(RedReason::FileNotFound { .. }) => Some("file-not-found"),
    }
}

/// Extra detail beyond the reason word (`None` when the word is enough) —
/// unknown fingerprint-triple members, lock refusal why, etc. The stable reason
/// token stays in [`color_reason`].
///
/// The two address-failure reds surface their nearest-candidate hints here
/// (d1 F6b — a hint, never auto-repair): before this, [`resolve_selector`]
/// computed the ranked candidates and no render surface ever showed them.
/// Capped at [`NEAREST_SHOWN`]; an empty candidate list (no live doc to draw
/// from) stays `None` so the bare label keeps meaning "nothing to offer".
#[must_use]
pub fn color_detail(color: &Color) -> Option<String> {
    match color {
        Color::Grey(GreyReason::UnverifiableFingerprint { unknown }) => {
            Some(format!("unknown {}", unknown.join(", ")))
        }
        Color::Red(RedReason::DanglingAnchor { candidates }) if !candidates.is_empty() => {
            let shown: Vec<String> = candidates
                .iter()
                .take(NEAREST_SHOWN)
                .map(|c| format!("^{c}"))
                .collect();
            Some(format!("nearest live anchors: {}", shown.join(", ")))
        }
        Color::Red(RedReason::SelectorUnresolved { candidates }) if !candidates.is_empty() => {
            let shown: Vec<&str> = candidates
                .iter()
                .take(NEAREST_SHOWN)
                .map(String::as_str)
                .collect();
            Some(format!("nearest live headings: {}", shown.join(", ")))
        }
        Color::Grey(GreyReason::LockRefused { reason }) => Some(reason.clone()),
        Color::Grey(GreyReason::Unmounted { root }) => Some(format!("root '{root}'")),
        // Path, not the mount entry (S3-R43 distinction).
        Color::Grey(GreyReason::PathUnseeable { path, detail, .. }) => {
            Some(format!("{path} ({detail})"))
        }
        // Root + path: root alone omits what is missing; path alone reads ambient.
        Color::Red(RedReason::FileNotFound { root, path, .. }) => {
            Some(format!("root '{root}' holds no '{path}'"))
        }
        Color::Grey(GreyReason::Uncolourable) => Some(
            "this row carried neither a fingerprint nor a refusal; \
             its appearance here is itself the defect — report it"
                .to_string(),
        ),
        _ => None,
    }
}

/// Full teaching refusal when the reason word is not enough; `None` otherwise.
/// `address` is the ref as declared (author text, not resolution output).
#[must_use]
pub fn color_teaching(color: &Color, address: &str) -> Option<String> {
    match color {
        Color::Grey(GreyReason::Unmounted { root }) => Some(render_unmounted(root, address)),
        Color::Grey(GreyReason::PathUnseeable { root, path, detail }) => {
            Some(render_path_unseeable(root, path, detail))
        }
        // Parts are joined inside the renderer; `address` is not forwarded — the
        // root must match the miss, not a caller-supplied spelling.
        Color::Red(RedReason::FileNotFound {
            root,
            path,
            selector,
        }) => Some(render_file_not_found(
            root,
            path,
            selector.as_deref(),
            target_is_non_markdown(path),
        )),
        _ => None,
    }
}

/// Resolve a pinned selector against the live target — the address half of the
/// color law, and the sole owner of it (the fingerprint compare,
/// [`classify_pin`], builds on this).
///
/// `Ok` carries the live document and the resolved [`Target`] (span + rev);
/// `Err` carries the color the address failure dictates:
///
/// - the transcript class is grey `immutable-root` — never resolved (d2 §2.2);
/// - a vanished target page, or a fragment that resolves to nothing: a block
///   address dangles, a heading/page address is selector-unresolved (decision
///   #9, each with the live toc's nearest candidates — empty when there is no
///   live doc to draw them from);
/// - an ambiguous selector is grey, unknowable (d1 point 3).
///
/// # Errors
/// The [`Color`] an unresolvable address renders — see above.
pub fn resolve_selector<'a>(
    selector: &Selector,
    target: Option<&'a Document>,
) -> Result<(&'a Document, Target), Color> {
    if selector.is_immutable_root() {
        return Err(Color::Grey(GreyReason::ImmutableRoot));
    }
    let Some(doc) = target else {
        return Err(match selector {
            Selector::Block(_) => Color::Red(RedReason::DanglingAnchor {
                candidates: Vec::new(),
            }),
            _ => Color::Red(RedReason::SelectorUnresolved {
                candidates: Vec::new(),
            }),
        });
    };
    // The whole-page selector resolves to the document root (== `file_rev`); it
    // has no mint-plane `Ref` form, so it is read directly.
    if matches!(selector, Selector::Page) {
        return Ok((
            doc,
            Target {
                span: doc.root.span.clone(),
                node_rev: doc.root.node_rev.clone(),
            },
        ));
    }
    match resolve(doc, &selector_ref(selector)) {
        Ok(t) => Ok((doc, t)),
        Err(ResolveError::NotFound) => Err(match selector {
            Selector::Block(id) => Color::Red(RedReason::DanglingAnchor {
                candidates: nearest(id, &live_anchors(doc)),
            }),
            _ => Color::Red(RedReason::SelectorUnresolved {
                candidates: nearest(&selector_display(selector), &live_headings(doc)),
            }),
        }),
        Err(ResolveError::Ambiguous(_)) => Err(Color::Grey(GreyReason::Ambiguous)),
    }
}

/// `meridian-lock` pin color: address ([`resolve_selector`]) then fingerprint
/// compare on `pinned_token` via [`fingerprint::verify_content_span`].
///
/// | verdict | color |
/// |---|---|
/// | `Green` | green |
/// | `Red{actual}` | red `content-drifted` |
/// | `Unverifiable` | grey `unverifiable-fingerprint` (names unknown members) |
/// | `Malformed` | grey `malformed-fingerprint` |
/// | `EmptySpan` | red `content-drifted` |
///
/// Unverifiable arms are grey, never green (unmeasured ≠ attested).
///
/// **R31 empty-span is red, not grey:** the address resolved and the live bytes
/// canonicalize empty, so the content the pin claims is measurably absent. Mint
/// refuses empty spans, so the pin is wrong (`content-drifted`).
///
/// **D8:** unresolved target → red-with-reason (never grey/green).
/// **D12:** content-addressed per-pin; `root:` only chooses which document.
#[must_use]
pub fn classify_pin(selector: &Selector, pinned_token: &str, target: Option<&Document>) -> Color {
    let (doc, resolved) = match resolve_selector(selector, target) {
        Ok(found) => found,
        Err(color) => return color,
    };
    let verdict = crate::fingerprint::verify_content_span(doc, &resolved.span, pinned_token);
    match verdict {
        ContentVerdict::Green => Color::Green,
        // Both verdicts are measured: `Red` read different content, `EmptySpan`
        // read none under a token mint could only have made over some (R31).
        ContentVerdict::Red { .. } | ContentVerdict::EmptySpan => Color::Red(RedReason::Drifted),
        ContentVerdict::Unverifiable { .. } => Color::Grey(GreyReason::UnverifiableFingerprint {
            unknown: verdict.unknown_members(),
        }),
        ContentVerdict::Malformed => Color::Grey(GreyReason::MalformedFingerprint),
    }
}

/// Project a resolvable selector to its mint-plane [`Ref`] — Heading and Block
/// only; Page and the transcript class are guarded in [`resolve_selector`] and
/// never reach here (the empty hpath is an inert fallback resolving `NotFound`).
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
        // the color plane never mints the charset-guard refusal.
        Selector::Block(id) => Ref::Anchor(id.clone()),
        Selector::Page | Selector::ImmutableRoot { .. } => Ref::Hpath(Vec::new()),
    }
}

/// A human display of a selector's address, in d1's `Task/Objective`
/// heading-path spelling.
#[must_use]
pub fn selector_display(selector: &Selector) -> String {
    match selector {
        Selector::Page => "(page)".to_string(),
        Selector::Heading(hpath) => hpath.join("/"),
        Selector::Block(id) => format!("^{id}"),
        Selector::ImmutableRoot { session, seq } => format!("{session}#seq-{seq}"),
    }
}

/// Every block-id anchor name in a document, in document order — the
/// dangling-anchor hint candidates.
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

/// Every heading-path in a document (`/`-joined), in document order — the
/// selector-unresolved hint candidates.
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

/// How many nearest candidates a rendered hint names — enough to catch a
/// rename, few enough that a hostile page cannot flood a refusal.
pub const NEAREST_SHOWN: usize = 3;

/// Rank `candidates` by nearest to `want` (a hint, never auto-repair — d1 F6b):
/// a shared-bigram score, ties broken by document order.
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

/// The count of character bigrams `a` and `b` share.
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

// Ambiguity teaching refusal (d1 — verbatim anchor)

/// d1 teaching-refusal exemplar, verbatim. [`render_ambiguity`] interpolates
/// the real selector/candidates; drift fails tests.
pub const D1_TEACHING_REFUSAL_EXEMPLAR: &str = "refused: selector 'Task/Objective' is ambiguous — 2 matches: n=1.2 (^a1b2c3), n=1.5 (^d4e5f6). Unambiguous writes to this file remain served. Fix: address the duplicate by block id or node index and rename one heading; see [[selector-grammar]].";

/// One ambiguous candidate: its 1-based node index (the occurrence `n=`, the
/// §2.1 addressable disambiguator) and its block id if it carries one (`^block`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AmbiguityCandidate {
    /// The occurrence index — the node-index address that disambiguates the
    /// duplicate (`n=`).
    pub node_index: u32,
    /// The candidate's block anchor id (`^block`), when it has one.
    pub block: Option<String>,
}

/// Render the d1 teaching refusal for an ambiguous selector, naming each
/// candidate by node index (`n=`) and block id (`^block`) when it has one.
/// Wording is carried verbatim from [`D1_TEACHING_REFUSAL_EXEMPLAR`].
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

/// Anchor-plane ambiguity teaching refusal, verbatim exemplar
/// ([`render_anchor_ambiguity`] interpolates the real selector/count; drift
/// fails tests). Unlike [`D1_TEACHING_REFUSAL_EXEMPLAR`] it lists no
/// candidates: duplicate block ids share one spelling and the anchor grammar
/// carries no occurrence index (`n` disambiguates hpath segments;
/// `{"anchor":id}` has no `n` slot), so nothing machine-addressable exists to
/// name — and the remedy speaks the anchor grammar, never "rename one heading"
/// (wire-contract A.3, door symmetry over duplicate block ids).
pub const ANCHOR_AMBIGUITY_REFUSAL_EXEMPLAR: &str = "refused: selector '^same-id' is ambiguous — 2 blocks carry this id. Unambiguous reads and writes to this file remain served. Fix: give each duplicate block a distinct id (a block id addresses exactly one block in its file), or address the enclosing section by heading path; see [[selector-grammar]].";

/// Render the anchor-plane ambiguity refusal for a duplicated block id.
/// Wording is carried verbatim from [`ANCHOR_AMBIGUITY_REFUSAL_EXEMPLAR`].
#[must_use]
pub fn render_anchor_ambiguity(selector: &str, count: usize) -> String {
    format!(
        "refused: selector '{selector}' is ambiguous — {count} blocks carry this id. \
         Unambiguous reads and writes to this file remain served. Fix: give each \
         duplicate block a distinct id (a block id addresses exactly one block in \
         its file), or address the enclosing section by heading path; see \
         [[selector-grammar]]."
    )
}

// Cross-vault measured-absence refusal (verbatim anchors)

/// Resolve-plane partial-state clause (with config/wire-serve siblings).
pub const NO_PARTIAL_RESOLVE_CLAUSE: &str = "Nothing was resolved for this ref and no rev was minted; every other ref on this page is unaffected.";

/// Cross-vault measured-absence refusal, verbatim.
/// [`render_file_not_found`] interpolates root/path.
pub const RED_FILE_NOT_FOUND_REFUSAL_EXEMPLAR: &str = "red(file-not-found): the address 'sessions:24-01-retro/notes.md#Design' names root 'sessions', which this machine binds and reads — and that root's corpus holds no '24-01-retro/notes.md'. The root is visible, so this is a measured absence, not grey. Nothing was resolved for this ref and no rev was minted; every other ref on this page is unaffected. Fix: check the path inside 'sessions' — `mrd config` names where it is mounted — or repoint the link; see [[address-grammar]].";

/// Whether `path` is outside the markdown-only v1 corpus. No extension ⇒
/// treated as markdown (`.md` append rule).
#[must_use]
pub fn target_is_non_markdown(path: &str) -> bool {
    std::path::Path::new(path)
        .extension()
        .is_some_and(|ext| !ext.eq_ignore_ascii_case("md"))
}

#[must_use]
pub fn render_file_not_found(
    root: &addr::MountName,
    missing: &str,
    selector: Option<&str>,
    md_only: bool,
) -> String {
    let limit = if md_only {
        " Cross-vault links are markdown-only in v1, so a non-.md target is not addressable even when the file exists."
    } else {
        ""
    };
    // Subject = address as written; absence = page path (not the selector).
    let address = match selector {
        Some(sel) => format!("{root}:{missing}#{sel}"),
        None => format!("{root}:{missing}"),
    };
    format!(
        "red(file-not-found): the address '{address}' names root '{root}', \
         which this machine binds and reads — and that root's corpus holds no \
         '{missing}'. The root is visible, so this is a measured absence, not \
         grey.{limit} {NO_PARTIAL_RESOLVE_CLAUSE} Fix: check the path inside \
         '{root}' — `mrd config` names where it is mounted — or repoint the \
         link; see [[address-grammar]]."
    )
}

// Unmounted-root teaching refusal (D8 — verbatim anchor)

/// Unmounted-root teaching refusal, verbatim. Names the mount and the fix (D8).
pub const GREY_UNMOUNTED_REFUSAL_EXEMPLAR: &str = "grey(unmounted): root 'assets' is not mounted — the address 'assets:domains/media/logo.md#Design' names a root this machine does not bind. Not red: nothing drifted, you just cannot see from here. Refs to mounted roots remain served. Fix: declare 'assets' in ~/MERIDIAN.md as a mount entry (name / path / kind); see [[address-grammar]].";

/// D8 teaching refusal for an unmounted root; wording from
/// [`GREY_UNMOUNTED_REFUSAL_EXEMPLAR`].
#[must_use]
pub fn render_unmounted(root: &addr::MountName, address: &str) -> String {
    format!(
        "grey(unmounted): root '{root}' is not mounted — the address '{address}' \
         names a root this machine does not bind. Not red: nothing drifted, you \
         just cannot see from here. Refs to mounted roots remain served. \
         Fix: declare '{root}' in ~/MERIDIAN.md as a mount entry \
         (name / path / kind); see [[address-grammar]]."
    )
}

/// Declared-but-unreadable teaching refusal, verbatim (S3-R43/R49). The reason
/// word comes from [`addr::PATH_UNSEEABLE_REASON_WORD`].
pub const GREY_PATH_UNSEEABLE_REFUSAL_EXEMPLAR: &str = "grey(path-unseeable): root 'assets' is declared, but the path it binds could not be read: /Volumes/media/assets (No such file or directory (os error 2)). Not red: nothing drifted, you just cannot see from here. Refs to readable roots remain served. Fix: check that /Volumes/media/assets exists and is readable, or change the mount entry's path; see [[address-grammar]].";

/// S3-R43 teaching refusal for a declared root that cannot be read. Never says
/// "declare it" — root is already declared.
#[must_use]
pub fn render_path_unseeable(root: &addr::MountName, path: &str, detail: &str) -> String {
    let word = addr::PATH_UNSEEABLE_REASON_WORD;
    format!(
        "grey({word}): root '{root}' is declared, but the path it binds could not \
         be read: {path} ({detail}). Not red: nothing drifted, you just cannot \
         see from here. Refs to readable roots remain served. Fix: check that \
         {path} exists and is readable, or change the mount entry's path; \
         see [[address-grammar]]."
    )
}

/// The first block-id anchor name contained within a byte span; `None` when the
/// span carries no anchor. Shared with the wire-serve write door, which enriches
/// the `ambiguous_ref` candidates.
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
        // Grey even with no target doc — never resolved (d2 §2.2).
        assert_eq!(
            resolve_selector(&sel, None).expect_err("the transcript class never resolves"),
            Color::Grey(GreyReason::ImmutableRoot)
        );
    }

    /// The whole-page selector resolves to the document root's rev.
    #[test]
    fn page_selector_resolves_to_the_document_root() {
        let d = doc("# Task\n\n## Objective\n\nbody\n");
        let (_, t) =
            resolve_selector(&Selector::Page, Some(&d)).expect("the page selector resolves");
        assert_eq!(t.node_rev, d.root.node_rev);
    }

    /// A dangling `^block-id` is `red(dangling-anchor)` — distinct from
    /// `red(drifted)` — and carries the live toc's nearest block-id candidates
    /// (decision #9).
    #[test]
    fn dangling_anchor_distinct_from_drifted() {
        // ^kept exists, ^gone does not.
        let d = doc("# Task\n\n## Objective\n\nbody ^kept\n\n## Notes\n\nmore\n");

        let dangling = resolve_selector(&Selector::Block("gone".into()), Some(&d))
            .expect_err("a vanished anchor does not resolve");
        let Color::Red(RedReason::DanglingAnchor { candidates }) = &dangling else {
            panic!("a vanished pinned anchor must render red(dangling-anchor): {dangling:?}");
        };
        assert!(
            candidates.contains(&"kept".to_string()),
            "the nearest-candidate hint lists the live toc's block ids: {candidates:?}"
        );

        // A heading that resolves is not an address failure at all.
        assert!(
            resolve_selector(
                &Selector::Heading(vec!["Task".into(), "Objective".into()]),
                Some(&d)
            )
            .is_ok(),
            "a live heading resolves; drift is the compare's question, not the address's"
        );

        assert_ne!(dangling, Color::Red(RedReason::Drifted));
    }

    /// A pinned heading that resolves to nothing is `red(selector-unresolved)`
    /// with the live toc's nearest heading candidates — distinct from drift.
    #[test]
    fn selector_unresolved_for_missing_heading() {
        let d = doc("# Task\n\n## Objective\n\nbody\n");
        let sel = Selector::Heading(vec!["Task".into(), "Goalz".into()]);
        let c = resolve_selector(&sel, Some(&d)).expect_err("a missing heading does not resolve");
        let Color::Red(RedReason::SelectorUnresolved { candidates }) = &c else {
            panic!("a missing heading selector must render red(selector-unresolved): {c:?}");
        };
        assert!(
            candidates.contains(&"Task/Objective".to_string()),
            "candidates list the live heading paths: {candidates:?}"
        );
        assert_ne!(c, Color::Red(RedReason::Drifted));
    }

    /// The address-failure reds render their nearest-candidate hints through
    /// `color_detail` (d1 F6b) — before this, the candidates were computed on
    /// every miss and no surface ever showed them. Capped at [`NEAREST_SHOWN`];
    /// an empty candidate list stays `None` (bare label = nothing to offer).
    #[test]
    fn color_detail_renders_the_nearest_candidate_hints() {
        let dangling = Color::Red(RedReason::DanglingAnchor {
            candidates: vec!["kept".into(), "goal".into(), "gate".into(), "extra".into()],
        });
        assert_eq!(
            color_detail(&dangling).as_deref(),
            Some("nearest live anchors: ^kept, ^goal, ^gate"),
            "anchor ids render with their marker, capped at {NEAREST_SHOWN}"
        );
        let unresolved = Color::Red(RedReason::SelectorUnresolved {
            candidates: vec!["Task/Objective".into()],
        });
        assert_eq!(
            color_detail(&unresolved).as_deref(),
            Some("nearest live headings: Task/Objective"),
        );
        // No live doc to draw from → no hint, never an empty parenthesis.
        for empty in [
            Color::Red(RedReason::DanglingAnchor {
                candidates: Vec::new(),
            }),
            Color::Red(RedReason::SelectorUnresolved {
                candidates: Vec::new(),
            }),
        ] {
            assert_eq!(color_detail(&empty), None, "{empty:?}");
        }
    }

    /// The nearest-candidate rank surfaces a renamed heading first (d1 F6b).
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

    /// The exemplar const and every rendered refusal share the same d1 teaching
    /// tail verbatim, naming each candidate by node index and block id.
    #[test]
    fn render_ambiguity_carries_d1_teaching_verbatim() {
        const TEACH_TAIL: &str = ". Unambiguous writes to this file remain served. \
             Fix: address the duplicate by block id or node index and rename one \
             heading; see [[selector-grammar]].";
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

    /// The anchor-plane renderer reproduces its exemplar byte for byte on the
    /// exemplar's inputs, and its remedy never speaks the heading grammar
    /// (wire-contract A.3, door symmetry over duplicate block ids).
    #[test]
    fn render_anchor_ambiguity_carries_the_exemplar_verbatim() {
        assert_eq!(
            render_anchor_ambiguity("^same-id", 2),
            ANCHOR_AMBIGUITY_REFUSAL_EXEMPLAR,
            "the renderer must reproduce the pinned exemplar byte for byte"
        );
        let other = render_anchor_ambiguity("^r-000042", 3);
        assert!(
            other.starts_with("refused: selector '^r-000042' is ambiguous — 3 blocks carry"),
            "different inputs are named, same shape: {other}"
        );
        assert!(
            !other.contains("rename one heading"),
            "the anchor remedy never speaks the heading grammar: {other}"
        );
    }

    /// The unmounted-root refusal carries its teaching tail verbatim (D8) — the
    /// exemplar const and every rendered refusal share the same tail.
    #[test]
    fn render_unmounted_carries_d8_teaching_verbatim() {
        const TEACH_TAIL: &str = ". Not red: nothing drifted, you just cannot see from here. \
             Refs to mounted roots remain served. Fix: declare 'assets' in ~/MERIDIAN.md \
             as a mount entry (name / path / kind); see [[address-grammar]].";
        assert!(
            GREY_UNMOUNTED_REFUSAL_EXEMPLAR.ends_with(TEACH_TAIL),
            "the D8 exemplar const must carry the verbatim teaching tail",
        );
        // The renderer reproduces the exemplar exactly, on the exemplar's inputs.
        let assets = addr::MountName::parse("assets").expect("a canonical name");
        let msg = render_unmounted(&assets, "assets:domains/media/logo.md#Design");
        assert_eq!(
            msg, GREY_UNMOUNTED_REFUSAL_EXEMPLAR,
            "the renderer must reproduce the pinned exemplar byte for byte",
        );

        // Different inputs: names those, still carries the tail shape.
        let sessions = addr::MountName::parse("sessions").expect("a canonical name");
        let other = render_unmounted(&sessions, "sessions:24-01-retro/notes.md#Design");
        assert!(
            other.starts_with("grey(unmounted): root 'sessions' is not mounted — "),
            "the reason word is S3-R6's `grey(unmounted)`, and the refusal NAMES the \
             missing mount (D8): {other}",
        );
        assert!(
            other.contains("sessions:24-01-retro/notes.md#Design"),
            "the refusal echoes the offending address: {other}",
        );
        assert!(
            other.contains("Fix: declare 'sessions' in ~/MERIDIAN.md"),
            "and it teaches the fix in terms of the missing mount: {other}",
        );
        assert!(
            !other.contains("red("),
            "grey is never spelled as red — nothing drifted",
        );
    }

    /// The grey vocabulary must not collide with its siblings (S3-R6).
    #[test]
    fn the_unmounted_reason_word_is_distinct_from_its_siblings() {
        let assets = addr::MountName::parse("assets").expect("a canonical name");
        let msg = render_unmounted(&assets, "assets:x.md");
        assert!(msg.starts_with("grey(unmounted):"));
        assert!(
            !msg.contains("cannot-assess"),
            "D8a — the address plane's grey is NOT `mrd check`'s verb-level \
             cannot-assess; two subsystems, one shared meaning, two types",
        );
        assert!(
            !msg.contains("file_not_found"),
            "an unmounted root is outside sight, never a measured absence",
        );
    }

    /// A duplicate with no block id is named by node index alone.
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
        let sel = Selector::Heading(vec!["Task".into(), "Objective".into()]);
        let span = resolve(&d, &selector_ref(&sel)).unwrap().span;
        assert_eq!(first_anchor_in_span(&d, &span), Some("a1b2c3".to_string()));
    }

    // meridian-lock pin plane (`classify_pin`)

    /// Live fingerprint token of a resolvable selector (same mint as verdict).
    fn live_token(d: &Document, sel: &Selector) -> String {
        let (_, t) = resolve_selector(sel, Some(d)).expect("resolves");
        crate::fingerprint::fingerprint_span(d, &t.span, &syntax::anchor_removals(&d.raw))
            .expect("fixture target has content")
            .into_string()
    }

    /// All four `ContentVerdict` arms map onto the color model, each with its
    /// own reason; the two unverifiable arms are grey, never green.
    #[test]
    fn classify_pin_maps_every_content_verdict_arm() {
        let d = doc("# Task\n\n## Objective\n\nbody v1\n");
        let sel = Selector::Heading(vec!["Task".into(), "Objective".into()]);
        let token = live_token(&d, &sel);

        assert_eq!(classify_pin(&sel, &token, Some(&d)), Color::Green);

        let drifted = doc("# Task\n\n## Objective\n\nbody v2 edited\n");
        assert_eq!(
            classify_pin(&sel, &token, Some(&drifted)),
            Color::Red(RedReason::Drifted)
        );

        // An unimplemented triple member — grey, naming which member.
        let hex64 = "0".repeat(64);
        assert_eq!(
            classify_pin(&sel, &format!("fp9.span2.b3.{hex64}"), Some(&d)),
            Color::Grey(GreyReason::UnverifiableFingerprint {
                unknown: vec!["version"]
            })
        );

        // Not a fingerprint token at all — grey, unreadable.
        assert_eq!(
            classify_pin(&sel, "780d2fb4cf68f60f", Some(&d)),
            Color::Grey(GreyReason::MalformedFingerprint)
        );
    }

    /// No unverifiable pin — unknown version, codec, or hashfn, or an
    /// unreadable token — may render green, even when its digest equals the
    /// live one.
    #[test]
    fn an_unverifiable_pin_never_renders_green() {
        let d = doc("# A\n\nbody\n");
        let sel = Selector::Page;
        let live = live_token(&d, &sel);
        let digest = live.rsplit('.').next().expect("digest");

        // Each token carries the correct live digest under an unknown member.
        for token in [
            format!("fp9.span2.b3.{digest}"),
            format!("fp1.zzz9.b3.{digest}"),
            format!("fp1.span2.xx.{digest}"),
            digest.to_string(), // the superseded bare-digest spelling
        ] {
            let color = classify_pin(&sel, &token, Some(&d));
            assert!(
                matches!(color, Color::Grey(_)),
                "{token} must render grey, got {color:?}"
            );
            assert_ne!(color, Color::Green, "{token} rendered a false green");
        }
    }

    /// D8 — an address that no longer resolves is red with its reason, never
    /// grey and never green.
    #[test]
    fn classify_pin_reds_a_dangling_target() {
        let d = doc("# Task\n\nbody ^goal\n");
        let hex64 = "0".repeat(64);
        let token = format!("fp1.span2.b3.{hex64}");

        // The anchor vanished from a live page.
        let gone = doc("# Task\n\nbody\n");
        assert!(matches!(
            classify_pin(&Selector::Block("goal".into()), &token, Some(&gone)),
            Color::Red(RedReason::DanglingAnchor { .. })
        ));
        // The whole page vanished — a block address still dangles.
        assert!(matches!(
            classify_pin(&Selector::Block("goal".into()), &token, None),
            Color::Red(RedReason::DanglingAnchor { .. })
        ));
        // A heading that resolves to nothing is selector-unresolved.
        let Color::Red(RedReason::SelectorUnresolved { candidates }) =
            classify_pin(&Selector::Heading(vec!["Taskk".into()]), &token, Some(&d))
        else {
            panic!("a vanished heading must render red selector-unresolved");
        };
        assert_eq!(candidates, vec!["Task".to_string()]);

        // The pin plane shares the address law with the rev plane.
        assert!(matches!(
            classify_pin(&Selector::parse("22-01-session#seq-9"), &token, Some(&d)),
            Color::Grey(GreyReason::ImmutableRoot)
        ));
    }
}

#[cfg(test)]
mod u21_file_not_found {
    use super::*;

    /// The four-property refusal contract: subject · cause at its grain ·
    /// partial state · a runnable fix — plus the negative, never an internal
    /// name.
    #[test]
    fn the_refusal_meets_the_house_four_property_bar() {
        let root = addr::MountName::parse("sessions").expect("a name");
        let m = render_file_not_found(&root, "24-01-retro/notes.md", Some("Design"), false);

        // 1. SUBJECT — the address, the root, and the path, all three named.
        assert!(
            m.contains("sessions:24-01-retro/notes.md"),
            "names the address: {m}"
        );
        assert!(m.contains("root 'sessions'"), "names the root: {m}");

        // 2. CAUSE AT ITS GRAIN — bound and readable, yet the corpus lacks it.
        assert!(m.contains("binds and reads"), "names the cause: {m}");
        assert!(
            m.contains("measured absence, not grey"),
            "pre-empts the grey misreading: {m}"
        );

        // 3. PARTIAL STATE — single-sourced.
        assert!(
            m.contains(NO_PARTIAL_RESOLVE_CLAUSE),
            "discloses partial state: {m}"
        );

        // 4. FIX — an act, not a restatement of the problem.
        assert!(
            m.contains("Fix: check the path inside 'sessions'"),
            "carries a fix: {m}"
        );

        // The negative — no internal vocabulary leaks into a user's sentence.
        for internal in [
            "RefResolution",
            "NotFound",
            "resolve_ref",
            "three_rules",
            "CorpusIndex",
        ] {
            assert!(
                !m.contains(internal),
                "leaks an internal name '{internal}': {m}"
            );
        }
    }

    /// The exemplar is produced by the renderer, not asserted as prose.
    #[test]
    fn the_pinned_exemplar_is_reproduced_by_the_renderer() {
        let root = addr::MountName::parse("sessions").expect("a name");
        let produced = render_file_not_found(&root, "24-01-retro/notes.md", Some("Design"), false);
        assert_eq!(
            produced, RED_FILE_NOT_FOUND_REFUSAL_EXEMPLAR,
            "the renderer and the pinned exemplar must not drift apart",
        );
    }

    /// Two different roots must produce two different refusals (address-grammar
    /// § 5.2 F4), or the scoping is decorative.
    #[test]
    fn the_refusal_is_scoped_to_the_root_that_missed() {
        let sessions = addr::MountName::parse("sessions").expect("a name");
        let assets = addr::MountName::parse("assets").expect("a name");
        let a = render_file_not_found(&sessions, "notes.md", None, false);
        let b = render_file_not_found(&assets, "notes.md", None, false);
        assert_ne!(a, b, "one path, two roots, two refusals");
        assert!(a.contains("'sessions'") && !a.contains("'assets'"), "{a}");
        assert!(b.contains("'assets'") && !b.contains("'sessions'"), "{b}");
    }

    /// The md-only teaching leg: a refusal that would imply absence instead
    /// names the limit. Asserted with its negative half.
    #[test]
    fn a_non_markdown_target_names_the_v1_limit_instead_of_implying_absence() {
        let root = addr::MountName::parse("assets").expect("a name");
        let png = render_file_not_found(&root, "media/logo.png", None, true);
        assert!(
            png.contains("markdown-only in v1")
                && png.contains("not addressable even when the file exists"),
            "a non-.md target must name the limit, never imply absence: {png}"
        );
        // The negative half — an ordinary .md miss must not carry it.
        let md = render_file_not_found(&root, "notes.md", None, false);
        assert!(
            !md.contains("markdown-only"),
            "an .md miss must not claim a markdown limit: {md}"
        );
    }

    /// `resolve_ref` reports which root missed, on both arms; the ambient arm
    /// reports `None`.
    #[test]
    fn resolve_ref_reports_which_root_the_miss_happened_inside() {
        use std::collections::BTreeMap;
        let docs: BTreeMap<String, Document> = BTreeMap::new();
        let index = crate::CorpusIndex::new();
        let corpus = crate::RootedCorpus::ambient(&docs);
        let mounts = addr::MountSet::default();

        // Ambient miss — no root; the refusal's parts come back from the
        // resolver rather than being re-split by the caller.
        assert_eq!(
            index.resolve_ref("absent.md", "from.md", &corpus, &mounts),
            crate::RefResolution::NotFound {
                root: None,
                path: "absent.md".to_owned(),
                selector: None,
            },
            "an ambient miss names no root, and says so explicitly",
        );

        // A miss inside a mounted root names that root and hands back the path
        // and selector separately, so `file_not_found` never re-splits a joined
        // string address (R1.6).
        let sessions = addr::MountName::parse("sessions").expect("a name");
        let empty: BTreeMap<String, Document> = BTreeMap::new();
        let corpus = crate::RootedCorpus::ambient(&docs).with_root(
            sessions.clone(),
            crate::RootKind::Vault,
            &empty,
        );
        let mounts = addr::MountSet::new([sessions.clone()]);
        assert_eq!(
            index.resolve_ref("sessions:notes.md#Design", "from.md", &corpus, &mounts),
            crate::RefResolution::NotFound {
                root: Some(sessions),
                path: "notes.md".to_owned(),
                selector: Some("Design".to_owned()),
            },
            "a rooted miss hands back the root, the path INSIDE it, and the \
             selector — the three parts the refusal names",
        );
    }
}
