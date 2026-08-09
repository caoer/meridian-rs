//! The engine-side receipt renderer: armed facts → the default md receipt line
//! (wire-contract.md §6.3; standing design, decision 014).
//!
//! Owns: rendering one batch's armed facts as one markdown list-item block —
//! the shipped default template (replaceable, D-C10 §6.4: the armed-fact set
//! is normative, the presentation is not) — and the receipt block-anchor mint
//! format (`r-NNNNNN`, decision 011). Stage-2 S6 adds the read-is-the-mint
//! ledger ([`read_mint`]).
//!
//! Never does: I/O, batching, validation, span math. Rendered bytes join the
//! batch before validation (§6.1) and ride the sealed batch's single root
//! advance (D-C3). Dependencies are `wire` only — a leaf crate.
//!
//! Receipt laws (§6.1–§6.3): per-request, rendered only when the splice named
//! a `receipt:{path,anchor}` address; the line carries the armed response's
//! facts, never delivery claims (A7); `root_before` only — a receipt cannot
//! contain the root it produces (§6.2); absent inputs produce absent facts
//! (§9).
//!
//! Field law (fix9): the renderer emits identifiers, never free text. Every
//! interpolated value goes through [`render_field`], so a rendered line
//! carries no `[` the frozen template did not put there — no wikilink can
//! form, no claim-link position exists, and `syntax::fp_removals` is empty by
//! construction. Guarding the charset also excludes whitespace and line
//! endings (which would forge token/row boundaries) and backticks (which
//! would close the escape span early).
//!
//! Segment law (§6.7 rule 2): where a template writes the §2.1 JSON target
//! form, the JSON punctuation is TEMPLATE bytes and only a segment's heading
//! text is interpolated — through [`render_hpath_segment_text`], never
//! [`render_field`]. The two renderers differ on exactly one character: the
//! field charset PERMITS `"`, which between template quotes would close the
//! JSON string early and forge the target object.

use std::borrow::Cow;
use std::fmt::Write;

pub mod anchor;
pub mod read_mint;

/// May `c` stand verbatim in a receipt line — a byte that cannot become
/// markdown structure or a token boundary?
///
/// ASCII graphic (no whitespace, no line ending), minus `[` and `]` (no
/// wikilink or embed can form), minus the backtick and backslash
/// [`render_field`] spends as escape delimiters.
#[must_use]
pub fn is_receipt_ident_char(c: char) -> bool {
    c.is_ascii_graphic() && !matches!(c, '[' | ']' | '`' | '\\')
}

/// Is `s` renderable verbatim — every char in the receipt-identifier charset?
///
/// An empty string qualifies: it renders as an empty token, which is a
/// degenerate fact, not a forged one.
#[must_use]
pub fn is_receipt_ident(s: &str) -> bool {
    s.chars().all(is_receipt_ident_char)
}

/// One receipt-line field, rendered.
///
/// An identifier renders byte-identical. Anything else renders as an inline
/// code span with out-of-charset characters escaped `\u{…}` and `\` doubled —
/// reversible, so the value is preserved exactly (§5.2). The escaped content
/// carries no backtick (the span closes where this function put it) and no
/// `[` (the claim grammar cannot form).
#[must_use]
pub fn render_field(s: &str) -> Cow<'_, str> {
    if is_receipt_ident(s) {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len() + 2);
    out.push('`');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            c if is_receipt_ident_char(c) => out.push(c),
            c => {
                let _ = write!(out, "\\u{{{:x}}}", c as u32);
            }
        }
    }
    out.push('`');
    Cow::Owned(out)
}

/// May `c` stand verbatim inside a template-quoted JSON string — receipt
/// identifier text that is not the quote the template owns?
fn is_hpath_segment_verbatim(c: char) -> bool {
    is_receipt_ident_char(c) && c != '"'
}

/// Render one hpath segment's heading text as the BODY of a JSON string —
/// the template owns the surrounding quotes (§6.7 rule 2).
///
/// Receipt-identifier text other than `"` stands verbatim; every other
/// character — the quote, the backslash, the brackets, the backtick, spaces,
/// control characters, everything non-ASCII — becomes a JSON `\u` escape
/// (surrogate pairs above U+FFFF). The result therefore carries no `"` and no
/// backslash that does not open a complete six-byte escape, so the string
/// cannot be closed early and no markdown structure can form inside it. The
/// escape is JSON's own, so a strict parser recovers `s` byte-for-byte.
///
/// This is NOT [`render_field`]: that charset permits `"`, and a heading
/// carrying one would forge the target object rather than be quoted by it.
#[must_use]
pub fn render_hpath_segment_text(s: &str) -> Cow<'_, str> {
    if s.chars().all(is_hpath_segment_verbatim) {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        if is_hpath_segment_verbatim(c) {
            out.push(c);
            continue;
        }
        let mut buf = [0u16; 2];
        for unit in c.encode_utf16(&mut buf) {
            let _ = write!(out, "\\u{unit:04x}");
        }
    }
    Cow::Owned(out)
}

/// Render an hpath as the §2.1 JSON array a receipt template writes —
/// `[{"h":"Goals"},{"h":"Beta","n":2}]`.
///
/// Every byte of punctuation here is template text; only the heading text
/// (through [`render_hpath_segment_text`]) and the occurrence index come from
/// the data. Absent `n` writes no key, matching the wire spelling (§2.1).
#[must_use]
pub fn render_hpath_json(hpath: &[wire::HpathSeg]) -> String {
    let mut out = String::from("[");
    for (i, seg) in hpath.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let _ = write!(out, "{{\"h\":\"{}\"", render_hpath_segment_text(&seg.h));
        if let Some(n) = seg.n {
            let _ = write!(out, ",\"n\":{n}");
        }
        out.push('}');
    }
    out.push(']');
    out
}

/// The armed-fact set for one batch, borrowed from the request + armed
/// response pair the caller already holds (§6.1 fact list).
#[derive(Debug, Clone)]
pub struct ArmedFacts<'a> {
    pub id: Option<wire::RequestId>,
    pub path: &'a wire::Path,
    pub actor: Option<&'a str>,
    pub now: Option<&'a str>,
    pub root_before: &'a wire::Root,
    /// The receipt's own block anchor (from the request's `receipt.anchor`).
    pub anchor: &'a str,
    pub edits: Vec<EditFact<'a>>,
}

/// One edit's facts: the request-side target + shape and the armed rev
/// transition (request and armed edits align 1:1, same order — §4.4).
#[derive(Debug, Clone)]
pub struct EditFact<'a> {
    pub target: &'a wire::SecRef,
    pub shape: &'a wire::EditShape,
    pub before: &'a wire::NodeRev,
    pub after: &'a wire::NodeRev,
}

/// Render the default receipt line — the block-leaf bytes, no line terminator
/// (the batch writer owns terminators and file joining).
///
/// Worked shape (§6.3 E3, byte-exact under test):
/// `- splice notes/plan.md id=42 actor=… now=… fingerprint_before=b3:…
/// edits=1 target.hpath=[{"h":"Goals"},{"h":"Q3"}] match 33d5…->41f6…
/// ^r-000042`
///
/// The token spells `fingerprint_before`, §6.1's standing noun, over the
/// `root_before` fact the wire response carries — the two name one value and
/// only the rendered spelling is this template's to choose.
#[must_use]
pub fn render_line(facts: &ArmedFacts<'_>) -> String {
    let mut out = String::new();
    let _ = write!(out, "- splice {}", render_field(&facts.path.0));
    if let Some(id) = facts.id {
        let _ = write!(out, " id={id}");
    }
    if let Some(actor) = facts.actor {
        let _ = write!(out, " actor={}", render_field(actor));
    }
    if let Some(now) = facts.now {
        let _ = write!(out, " now={}", render_field(now));
    }
    let _ = write!(
        out,
        " fingerprint_before={} edits={}",
        render_field(&facts.root_before.0),
        facts.edits.len()
    );
    for edit in &facts.edits {
        let _ = write!(
            out,
            " {} {} {}->{}",
            target_token(edit.target),
            shape_display(edit.shape),
            render_field(&edit.before.0),
            render_field(&edit.after.0)
        );
    }
    let _ = write!(out, " ^{}", render_field(facts.anchor));
    out
}

/// Mint the receipt block anchor for a batch counter: `r-NNNNNN`,
/// zero-padded to six digits, widening beyond — always inside the block-id
/// charset `[A-Za-z0-9-]+` (§2.4, decision 011).
#[must_use]
pub fn anchor(n: u64) -> String {
    format!("r-{n:06}")
}

/// The target token the default template writes for one edit.
///
/// An hpath renders as `target.hpath=` plus the §2.1 JSON array — the form
/// §6.3 mandates and the pretty join it forbids in the same breath. The array
/// goes in WHOLE and never through [`render_field`]: that charset excludes
/// `[` and `]`, so routing the address through it would escape the address
/// into a code span. Its punctuation is template bytes and only the heading
/// text is interpolated, through [`render_hpath_segment_text`] (§6.7 rule 2).
///
/// The anchor and `fm_key` forms are single tokens, not joins, so they keep
/// rule 1's escaping and their existing spelling.
fn target_token(target: &wire::SecRef) -> String {
    match target {
        wire::SecRef::Hpath { hpath } => format!("target.hpath={}", render_hpath_json(hpath)),
        other => render_field(&target_display(other)).into_owned(),
    }
}

/// Target display text for the non-hpath forms — anchors as `^id`,
/// frontmatter keys bare. Never a second address grammar (§6.4).
pub(crate) fn target_display(target: &wire::SecRef) -> String {
    match target {
        wire::SecRef::Hpath { hpath } => render_hpath_json(hpath),
        wire::SecRef::Anchor { anchor } => format!("^{anchor}"),
        wire::SecRef::FmKey { fm_key } => fm_key.clone(),
    }
}

pub(crate) fn shape_display(shape: &wire::EditShape) -> &'static str {
    match shape {
        wire::EditShape::Match { .. } => "match",
        wire::EditShape::Put { at, .. } => match at {
            wire::PutAt::All => "put:all",
            wire::PutAt::Content => "put:content",
            wire::PutAt::End => "put:end",
            wire::PutAt::Upsert => "put:upsert",
        },
    }
}
