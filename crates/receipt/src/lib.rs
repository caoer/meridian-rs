//! The engine-side receipt renderer: armed facts → the default md receipt
//! line (wire-contract-v2 §6.3; FROZEN 2026-07-18, decision 014).
//!
//! # Charter
//! **Owns:** rendering one batch's armed facts as one markdown list-item
//! block — the shipped DEFAULT template. The template is replaceable
//! (D-C10, §6.4): the normative receipt content is the armed-fact set
//! defined by the wire response shape; a non-ccc consumer renders it any
//! way it likes. Also owns the receipt block-anchor mint format
//! (`r-NNNNNN`, in-charset per decision 011 — the CHARSET-GUARD position
//! homed here). Stage-2 S6 adds the read-is-the-mint ledger
//! ([`read_mint`]) — the ephemeral in-memory read-receipt fact, the same
//! receipt FAMILY at the read plane (stage-3 unifies its representation with
//! the persisted `^receipt` projection this module renders).
//!
//! **Never does:** I/O, batching, validation, span math. Rendered bytes
//! join the batch BEFORE validation (the caller's law, §6.1): the append
//! rides inside the sealed batch and the single root advance (D-C3).
//! Dependencies are `wire` only, by gate — a leaf crate; placement per the
//! repo growth rule (not in the bin; `model`/`wire-map` charter "never
//! does: body formatting").
//!
//! # Receipt laws this crate renders TO (v2 §6.1–§6.3)
//! - Receipts are PER-REQUEST, never a wire requirement: this renderer runs
//!   only when the splice named a `receipt:{path,anchor}` address.
//! - The line carries the same facts as the armed response: op, target
//!   identities, rev transitions, `root_before`, actor, now, request id.
//!   Facts about what was ARMED — never delivery claims (A7).
//! - `root_before` only — a receipt cannot contain the root it produces
//!   (§6.2, the no-self-rooting law, stated as a limit).
//! - Absent inputs produce absent facts (§9): a request without `actor`/
//!   `now`/`id` renders a line without those tokens.
//!
//! # The field law (fix9): this renderer emits IDENTIFIERS, never free text
//! Every value a receipt line interpolates arrives from outside — `actor` and
//! the target `hpath`/`fm_key` are caller-supplied wire strings, `path` passes
//! only the §1 path law (which admits `[`, `]`, `@` and line endings). Rendered
//! raw, those bytes become MARKDOWN: `actor=[[guide#^goal@green.b3af12cd|G]]`
//! is an `@fp` claim in a claim-link position — a claim nobody computed, in
//! stored bytes, on a plane no candidate strip sees (the receipt rides
//! `ValidatedBatch.receipt`, beside `.edits`, never inside the document the
//! `@fp` strip judges).
//!
//! So every field goes through [`render_field`], and the invariant is one
//! sentence: **a rendered receipt line carries no `[` the frozen template did
//! not put there** (it puts none). No wikilink can form, so no claim-link
//! position exists, so `syntax::fp_removals` is empty by CONSTRUCTION rather
//! than by a strip that would have to re-spell the dialect grammar in a crate
//! whose charter forbids the dependency.
//!
//! The same law closes three siblings of the `@fp` instance for free, because
//! it guards the CHARSET rather than one token shape: whitespace (which forges
//! `key=value` token boundaries), line endings (which forge a whole row), and
//! backticks (which would close the escape span early).

use std::borrow::Cow;
use std::fmt::Write;

pub mod anchor;
pub mod read_mint;

/// May `c` stand VERBATIM in a receipt line — a byte that cannot become
/// markdown structure or a token boundary?
///
/// ASCII graphic (so no whitespace and no line ending: one row stays one row,
/// one `key=value` stays one token), minus `[` and `]` (so no wikilink or embed
/// can form, which is what makes a claim-link position unreachable), minus the
/// backtick and backslash [`render_field`] spends as escape delimiters.
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
/// An identifier renders as itself — **byte-identical**, so every §6.3 frozen
/// line and every actor the daemon derives is unchanged. Anything else renders
/// as an inline code span with its out-of-charset characters escaped `\u{…}`
/// (Rust's own spelling, borrowed rather than invented) and `\` doubled.
///
/// The escaped content carries no backtick, so the span always closes exactly
/// where this function put it; and no `[`, so the claim grammar cannot form
/// even before the code masking R22 ratified applies. The value is preserved
/// exactly — reversibly, not lossily — which is what §5.2's "recorded exactly
/// as given, never invented" requires of a record that must still be legible as
/// one line. The normative facts are the armed response's (D-C10: template
/// replaceable, facts normative); this is the shipped template's presentation
/// of one of them.
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

/// Render the default receipt line — the block-leaf BYTES (no line
/// terminator: the block span excludes it, v1 leaf law; the batch writer
/// owns terminators and file joining).
///
/// Worked shape (§6.3 E3, byte-exact under test):
/// `- splice notes/plan.md id=42 actor=… now=… root_before=b3:… edits=1
/// Goals>Q3 match 33d5…->41f6… ^r-000042`
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
        " root_before={} edits={}",
        render_field(&facts.root_before.0),
        facts.edits.len()
    );
    for edit in &facts.edits {
        let _ = write!(
            out,
            " {} {} {}->{}",
            render_field(&target_display(edit.target)),
            shape_display(edit.shape),
            render_field(&edit.before.0),
            render_field(&edit.after.0)
        );
    }
    let _ = write!(out, " ^{}", render_field(facts.anchor));
    out
}

/// Mint the receipt block anchor for a batch counter: `r-NNNNNN`,
/// zero-padded to six digits, widening beyond — always inside the ONE
/// block-id charset `[A-Za-z0-9-]+` (§2.4, decision 011).
#[must_use]
pub fn anchor(n: u64) -> String {
    format!("r-{n:06}")
}

/// Target display text — DISPLAY inside the default template, never a
/// second address grammar (§6.4): hpath segments joined `>`, an occurrence
/// index as `(n)`, anchors as `^id`, frontmatter keys bare.
pub(crate) fn target_display(target: &wire::SecRef) -> String {
    match target {
        wire::SecRef::Hpath { hpath } => {
            let mut out = String::new();
            for (i, seg) in hpath.iter().enumerate() {
                if i > 0 {
                    out.push('>');
                }
                out.push_str(&seg.h);
                if let Some(n) = seg.n {
                    let _ = write!(out, "({n})");
                }
            }
            out
        }
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
