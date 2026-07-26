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
//!
//! # Two compares, ONE color law
//! The law splits into an ADDRESS half and a COMPARE half. [`resolve_selector`]
//! owns the address half; two compares are built on it, and there is no third
//! color computer:
//!
//! - [`classify_edge`] — the legacy `^inputs` plane: `live node_rev` vs the
//!   pinned `rev`.
//! - [`classify_pin`] — the `meridian-lock` plane: the pinned `fp1.…`
//!   CID-token through [`crate::fingerprint::verify_content_span`], whose five
//!   arms map onto these same tones (green / red `content-drifted` / grey
//!   `unverifiable-fingerprint` / grey `malformed-fingerprint` / red
//!   `content-drifted` for an empty normalized span, R31).
//!
//! **Grey never renders green.** Every grey means the ledger did not measure
//! this edge — an unverified claim dressed as attested is the one dishonest
//! color, so an unknown codec, an unreadable token, and a refused lock block all
//! stay grey no matter what their digest says.

use crate::fingerprint::ContentVerdict;
use crate::{Document, Node, NodeKind, NodeRev, Ref, ResolveError, Target, resolve};

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
    /// The lock is pinned under a `hash-algo` this engine does not compute
    /// (anything but [`crate::NODE_REV_ALGO`]) — readable, unverifiable here.
    /// A `hash-algo: vN` mismatch is a mechanical re-hash trigger, never
    /// invalidation, so it renders grey — NEVER red (a false drift the engine
    /// never measured) and NEVER green (an unverified claim dressed as attested).
    /// Archived v1 blocks render this forever (d2 §6.3; U0.2/U3.4;
    /// wire-contract-v2-colors-amendment § Colors).
    SupersededAlgo,
    /// A `meridian-lock` pin whose fingerprint token PARSES but names a
    /// version / codec / hashfn this build does not implement
    /// ([`crate::fingerprint::ContentVerdict::Unverifiable`]). `unknown` names
    /// WHICH triple members are unknown, so the render never prints a
    /// live-looking triple. The fingerprint plane's `superseded-algo`: grey,
    /// never green (an unverified claim dressed as attested) and never red (a
    /// drift nobody measured).
    UnverifiableFingerprint { unknown: Vec<&'static str> },
    /// **A cross-root address naming a root this machine does not bind** — or
    /// binds without being able to read (`docs/address-grammar.md` § 8 M6).
    ///
    /// **Grey, never red:** nothing drifted; the ledger cannot see from here
    /// (`2026-07-24-cross-root-addressing.md` §1a). Carries the missing name so
    /// the refusal can teach the fix (D8), and it is a DISTINCT class from
    /// `file_not_found` — a missing file in a MOUNTED root is a measured
    /// absence, while an unmounted root is outside sight. Conflating them is the
    /// false negative this variant exists to prevent.
    ///
    /// **R-3 — grey OUTRANKS red.** A cross-root pin that was green and whose
    /// root is later unmounted becomes grey, never red. The inverse (grey → exit
    /// 0) is refused categorically: it would make unmounting a root a way to
    /// convert a red into a pass through an edit to `~/MERIDIAN.md`, which cannot
    /// itself be attested (S3-R7 ③).
    ///
    /// **D8a — deliberately NOT unified with `mrd check`'s cannot-assess.** Two
    /// subsystems, ONE shared meaning: this is the *address* plane, routed
    /// through the reason-carrying grey model; cannot-assess is a verb-level
    /// exit state on the *validity* plane. What they share is the law — outside
    /// sight never renders as verified (R26) — not the type.
    Unmounted {
        /// The canonical root name the address named and this machine does not
        /// bind.
        root: addr::MountName,
    },
    /// **A cross-root address naming a root the file DECLARES but this machine
    /// cannot READ** — the path is absent, unreadable, or holds no corpus
    /// (`docs/address-grammar.md` § 8 M6).
    ///
    /// **A DIFFERENT cause from [`GreyReason::Unmounted`], and S3-R43 rules it a
    /// different reason word.** Both are grey and both refuse on exit 1; what
    /// separates them is what a reader must DO. `Unmounted`'s refusal says
    /// *"declare it in `~/MERIDIAN.md`"*; here the root **is** declared, so that
    /// sentence is false and its fix is already done. This one names **the
    /// PATH** — the thing that is actually wrong.
    ///
    /// > A teaching refusal that prescribes a COMPLETED ACTION is worse than a
    /// > bare class: it spends the user's trust AND their time, and leaves no
    /// > signal pointing at the real cause.
    ///
    /// S3-R6 does not license collapsing the two: it established
    /// `grey(unmounted)` and `grey(cannot-assess)` as TWO words inside ONE
    /// vocabulary at ONE exit code. It forbids re-spelling one CONCEPT across
    /// crates; it does not require two CAUSES to share one word.
    PathUnseeable {
        /// The canonical name the file declares.
        root: addr::MountName,
        /// The path it is declared at — what the refusal tells a reader to check.
        path: String,
        /// The underlying reason, verbatim.
        detail: String,
    },
    /// A `meridian-lock` pin whose pinned value is not a fingerprint token at
    /// all ([`crate::fingerprint::ContentVerdict::Malformed`]). Unreadable, so
    /// it was never measured — grey, never red.
    MalformedFingerprint,
    /// The page's whole `meridian-lock` block REFUSED to parse — malformed, or
    /// more than one block on one page. The lock is outside sight, so the page
    /// projects THIS row instead of zero rows: a corrupt lock must never read
    /// as "no pins". Carries the refusal reason verbatim.
    LockRefused { reason: String },
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

/// The tone of a color (`green` / `grey` / `red`) — the stable output word.
///
/// It lives beside the [`Color`] it names so there is ONE vocabulary, exactly
/// as there is one color law: the walk/status render (`view::walk::color_tone`
/// re-exports this) and the stage-2 claim-link decorator are different surfaces
/// answering the same question, and a second `match` would be a second answer
/// waiting to disagree.
#[must_use]
pub fn color_tone(color: &Color) -> &'static str {
    match color {
        Color::Green => "green",
        Color::Grey(_) => "grey",
        Color::Red(_) => "red",
    }
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
    match resolve_selector(selector, target) {
        // Resolves — the only question left is content drift (source 2).
        Ok((_, t)) if &t.node_rev == pinned => Color::Green,
        Ok(_) => Color::Red(RedReason::Drifted),
        Err(color) => color,
    }
}

/// Resolve a pinned selector against the live target — the ADDRESS half of the
/// color law, shared by the two compares built on it: the `node_rev` compare
/// ([`classify_edge`], the legacy `^inputs` plane) and the fingerprint compare
/// ([`classify_pin`], the `meridian-lock` plane). One owner, so an address
/// failure can never render one color on one plane and another on the other.
///
/// `Ok` carries the live document and the resolved [`Target`] (span + rev);
/// `Err` carries the color the address failure itself dictates:
///
/// - the transcript class is grey `immutable-root` — never resolved (d2 §2.2);
/// - a vanished target PAGE, or a fragment that resolves to nothing: a block
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
    // The whole-page selector resolves to the DOCUMENT ROOT (== `file_rev`); it
    // has no mint-plane `Ref` form, so it is read directly. A present page never
    // dangles — only drifts (a vanished page is the `target: None` case).
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
        // The address failed. Block → dangling-anchor; heading/page →
        // selector-unresolved. Each with the live toc's nearest candidates.
        Err(ResolveError::NotFound) => Err(match selector {
            Selector::Block(id) => Color::Red(RedReason::DanglingAnchor {
                candidates: nearest(id, &live_anchors(doc)),
            }),
            _ => Color::Red(RedReason::SelectorUnresolved {
                candidates: nearest(&selector_display(selector), &live_headings(doc)),
            }),
        }),
        // The pinned selector became ambiguous — grey, unknowable (d1 point 3).
        Err(ResolveError::Ambiguous(_)) => Err(Color::Grey(GreyReason::Ambiguous)),
    }
}

/// Compute a **`meridian-lock` pin's** color: the same address law as
/// [`classify_edge`] ([`resolve_selector`]), then the FINGERPRINT compare
/// instead of the `node_rev` compare.
///
/// `pinned_token` is the lock's `fingerprint` CID-token verbatim. A `fp1.…`
/// token is not `node_rev`-comparable in either direction, so routing a lock pin
/// through [`classify_edge`] could only ever produce a false red or a grey; the
/// verdict belongs to [`fingerprint::verify_content_span`], whose four arms map
/// onto this one color model:
///
/// | verdict | color |
/// |---|---|
/// | `Green` | green |
/// | `Red{actual}` | red `content-drifted` |
/// | `Unverifiable` | grey `unverifiable-fingerprint`, NAMING the unknown triple member |
/// | `Malformed` | grey `malformed-fingerprint` |
/// | `EmptySpan` | red `content-drifted` |
///
/// Both unverifiable arms are grey and NEVER green — an unreadable or
/// unimplemented pin was never measured, so claiming it verified would be the
/// one dishonest color (grey = outside sight).
///
/// **R31 — why the empty-span arm is RED and not grey.** Grey means "outside
/// sight"; this is inside it. The address resolved, the engine read the live
/// bytes, and they canonicalize to nothing — so the content this pin claims to
/// cover is measurably not there. A pinned token can never have been minted
/// over an empty span ([`fingerprint::fingerprint_span`] refuses), so the pin is
/// wrong about its target, which is exactly `content-drifted`. It reuses that
/// existing reason deliberately: the class is a *false green being closed*, not
/// a new colour to teach, so no reason code, render label, or golden row moves.
/// The consequence to know: `realise` converges an ordinary `red(drifted)` pin,
/// but converging THIS one would have to re-mint over the empty span, so it
/// refuses at the mint door instead — honestly, and by the same owner.
///
/// **D8:** a target that no longer resolves is red-with-reason
/// (`dangling-anchor` / `selector-unresolved`), never grey and never green —
/// [`resolve_selector`]'s answer, unchanged for this plane.
///
/// **D12:** the verdict is content-addressed and per-pin — the token and the
/// target's own bytes, nothing root-scoped. A later `root:` prefix changes which
/// document the caller hands in, never this computation.
#[must_use]
pub fn classify_pin(selector: &Selector, pinned_token: &str, target: Option<&Document>) -> Color {
    let (doc, resolved) = match resolve_selector(selector, target) {
        Ok(found) => found,
        Err(color) => return color,
    };
    let verdict = crate::fingerprint::verify_content_span(doc, &resolved.span, pinned_token);
    match verdict {
        ContentVerdict::Green => Color::Green,
        // One body for two verdicts, and they genuinely are one colour: the
        // address resolved and the engine read the live bytes in both, so both
        // are measured. `Red` measured different content; `EmptySpan` measured
        // NO content under a token that can only have been minted over some
        // (R31) — the pin is wrong about its target either way.
        ContentVerdict::Red { .. } | ContentVerdict::EmptySpan => Color::Red(RedReason::Drifted),
        ContentVerdict::Unverifiable { .. } => Color::Grey(GreyReason::UnverifiableFingerprint {
            unknown: verdict.unknown_members(),
        }),
        ContentVerdict::Malformed => Color::Grey(GreyReason::MalformedFingerprint),
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

// ---------------------------------------------------------------------------
// The unmounted-root teaching refusal (D8 — carried VERBATIM)
// ---------------------------------------------------------------------------

/// The unmounted-root teaching refusal, carried VERBATIM as the provenance
/// anchor (`2026-07-24-cross-root-addressing.md` §1a). [`render_unmounted`]
/// reproduces this wording with the real root and address interpolated; this
/// const pins the exemplar so a drift in the wording is a visible test failure —
/// the same shape as [`D1_TEACHING_REFUSAL_EXEMPLAR`].
///
/// It **names the missing mount** and **teaches the fix** (D8), and it carries
/// §1a's ratified sentence — *"Not red: nothing drifted, you just cannot see
/// from here"* — rather than paraphrasing it.
pub const GREY_UNMOUNTED_REFUSAL_EXEMPLAR: &str = "grey(unmounted): root 'assets' is not mounted — the address 'assets:domains/media/logo.md#Design' names a root this machine does not bind. Not red: nothing drifted, you just cannot see from here. Refs to mounted roots remain served. Fix: declare 'assets' in ~/MERIDIAN.md as a mount entry (name / path / kind); see [[address-grammar]].";

/// Render the D8 teaching refusal for an address naming an unmounted root,
/// naming the missing mount and the offending address. The wording is carried
/// verbatim from [`GREY_UNMOUNTED_REFUSAL_EXEMPLAR`] with the real values
/// spliced in — refuse-unmounted-only: *"Refs to mounted roots remain served."*
///
/// The leading reason word is **`grey(unmounted)`** — S3-R6's vocabulary, shared
/// with `grey(cannot-assess)` and distinct from `red(...)`. It is not re-spelled
/// locally: the same ruling binds u14i, U14 and U15.
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

/// The DECLARED-but-unreadable teaching refusal, carried VERBATIM as the
/// provenance anchor (S3-R43 / S3-R49). [`render_path_unseeable`] reproduces
/// this wording with the real root, path and reason interpolated; this const
/// pins the exemplar so a drift in the wording is a visible test failure — the
/// same shape as [`D1_TEACHING_REFUSAL_EXEMPLAR`] and
/// [`GREY_UNMOUNTED_REFUSAL_EXEMPLAR`].
///
/// **The word is REUSED, not minted (S3-R49).** Round 2 was ruled a new word,
/// `root-unreachable`; the enumeration this unit was obliged to run found that
/// `config::mount::MountState::PathUnseeable` already ships
/// **`grey(path-unseeable)`** for the same observed state, citing the same M6
/// row, with a teaching that already names the path. The bare word is
/// [`addr::PATH_UNSEEABLE_REASON_WORD`] — **one source**, wrapped by each plane's
/// own renderer rather than spelled twice.
///
/// **It names the PATH, never the mount entry**, because the mount entry is
/// already correct — that is the whole distinction S3-R43 draws.
pub const GREY_PATH_UNSEEABLE_REFUSAL_EXEMPLAR: &str = "grey(path-unseeable): root 'assets' is declared, but the path it binds could not be read: /Volumes/media/assets (No such file or directory (os error 2)). Not red: nothing drifted, you just cannot see from here. Refs to readable roots remain served. Fix: check that /Volumes/media/assets exists and is readable, or change the mount entry's path; see [[address-grammar]].";

/// Render the S3-R43 teaching refusal for an address naming a DECLARED root this
/// machine cannot read. Wording carried verbatim from
/// [`GREY_PATH_UNSEEABLE_REFUSAL_EXEMPLAR`].
///
/// **Deliberately never says "declare it".** The root IS declared; prescribing
/// the declaration again is the defect this refusal exists to remove.
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

    /// **D8 — the unmounted-root refusal carries its teaching tail VERBATIM**,
    /// exactly as `render_ambiguity_carries_d1_teaching_verbatim` pins D1's. The
    /// exemplar const and every rendered refusal share the same tail, so a drift
    /// in the wording is a visible failure rather than a silent divergence
    /// between the documented refusal and the shipped one.
    #[test]
    fn render_unmounted_carries_d8_teaching_verbatim() {
        const TEACH_TAIL: &str = ". Not red: nothing drifted, you just cannot see from here. \
             Refs to mounted roots remain served. Fix: declare 'assets' in ~/MERIDIAN.md \
             as a mount entry (name / path / kind); see [[address-grammar]].";
        // The pinned exemplar carries the verbatim teaching tail.
        assert!(
            GREY_UNMOUNTED_REFUSAL_EXEMPLAR.ends_with(TEACH_TAIL),
            "the D8 exemplar const must carry the verbatim teaching tail",
        );
        // And the renderer REPRODUCES the exemplar exactly, on the exemplar's
        // own inputs — the assertion that makes the const a pin rather than a
        // decorative copy of prose nothing checks.
        let assets = addr::MountName::parse("assets").expect("a canonical name");
        let msg = render_unmounted(&assets, "assets:domains/media/logo.md#Design");
        assert_eq!(
            msg, GREY_UNMOUNTED_REFUSAL_EXEMPLAR,
            "the renderer must reproduce the pinned exemplar byte for byte",
        );

        // On DIFFERENT inputs it names those, and still carries the tail shape.
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

    /// The grey vocabulary must not COLLIDE with its siblings (S3-R6). One
    /// reason word per concept, distinct in the human line.
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

    // ── the `meridian-lock` pin plane (`classify_pin`) ───────────────────────

    /// The live fingerprint token of a resolvable selector — what a correct pin
    /// holds (mint through the SAME owner the verdict recomputes with).
    fn live_token(d: &Document, sel: &Selector) -> String {
        let (_, t) = resolve_selector(sel, Some(d)).expect("resolves");
        crate::fingerprint::fingerprint_span(d, &t.span, &syntax::anchor_removals(&d.raw))
            .expect("fixture target has content")
            .into_string()
    }

    /// All four `ContentVerdict` arms map onto the ONE color model, each with
    /// its own reason — and the two unverifiable arms are GREY, never green.
    #[test]
    fn classify_pin_maps_every_content_verdict_arm() {
        let d = doc("# Task\n\n## Objective\n\nbody v1\n");
        let sel = Selector::Heading(vec!["Task".into(), "Objective".into()]);
        let token = live_token(&d, &sel);

        assert_eq!(classify_pin(&sel, &token, Some(&d)), Color::Green);

        // The content moved — the verdict is measured drift.
        let drifted = doc("# Task\n\n## Objective\n\nbody v2 edited\n");
        assert_eq!(
            classify_pin(&sel, &token, Some(&drifted)),
            Color::Red(RedReason::Drifted)
        );

        // An unimplemented triple member — grey, NAMING which member.
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

    /// LOAD-BEARING (grey = outside sight): no unverifiable pin — unknown
    /// version, codec, or hashfn, or an unreadable token — may render green,
    /// even when its digest happens to equal the live one.
    #[test]
    fn an_unverifiable_pin_never_renders_green() {
        let d = doc("# A\n\nbody\n");
        let sel = Selector::Page;
        let live = live_token(&d, &sel);
        let digest = live.rsplit('.').next().expect("digest");

        // Each token carries the CORRECT live digest under an unknown member.
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

    /// D8 — an address that no longer resolves is RED with its reason, never
    /// grey and never green: a vanished block dangles, a vanished heading is
    /// selector-unresolved, a vanished PAGE fails by its selector's class.
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
        // A heading that resolves to nothing is selector-unresolved, with the
        // live toc's nearest candidates as the re-pin hint.
        let Color::Red(RedReason::SelectorUnresolved { candidates }) =
            classify_pin(&Selector::Heading(vec!["Taskk".into()]), &token, Some(&d))
        else {
            panic!("a vanished heading must render red selector-unresolved");
        };
        assert_eq!(candidates, vec!["Task".to_string()]);

        // The pin plane shares the address law with the rev plane (one owner).
        assert!(matches!(
            classify_pin(&Selector::parse("22-01-session#seq-9"), &token, Some(&d)),
            Color::Grey(GreyReason::ImmutableRoot)
        ));
    }
}
