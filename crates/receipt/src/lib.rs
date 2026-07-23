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
//! homed here).
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

use std::fmt::Write;

pub mod anchor;
pub mod journal;

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
    let _ = write!(out, "- splice {}", facts.path.0);
    if let Some(id) = facts.id {
        let _ = write!(out, " id={id}");
    }
    if let Some(actor) = facts.actor {
        let _ = write!(out, " actor={actor}");
    }
    if let Some(now) = facts.now {
        let _ = write!(out, " now={now}");
    }
    let _ = write!(
        out,
        " root_before={} edits={}",
        facts.root_before.0,
        facts.edits.len()
    );
    for edit in &facts.edits {
        let _ = write!(
            out,
            " {} {} {}->{}",
            target_display(edit.target),
            shape_display(edit.shape),
            edit.before.0,
            edit.after.0
        );
    }
    let _ = write!(out, " ^{}", facts.anchor);
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
