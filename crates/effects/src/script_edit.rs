//! The script plane's armed-edit list: wire `splice.plan_edits[]` items armed by
//! `put()`, inert until the consumer plane commits them in ONE guarded splice.
//!
//! The shapes are [`wire::PlanEdit`] **verbatim** — the engine never re-types
//! what the wire owns, so a new plan verb costs this module nothing. That is the
//! whole content of ruling (B′)
//! (`decisions/2026-08-07-script-put-builtin-edit-grammar.md`): `put()` speaks
//! the wire's second edit dialect, so no third grammar is minted and the wire
//! schema is untouched.

use wire::{HpathSeg, PlanEdit};

/// One armed edit: the wire plan-edit shape, the file it targets, and where in
/// the source it was armed.
///
/// `line`/`depth` are trace facts. Depth is recorded and **never suppresses**:
/// an applied effect renders at any nesting depth (plan v1.2 § Echo semantics —
/// the echo/quiet rule governs reads, not arms).
// `Eq` is absent because `wire::PlanEdit` is `PartialEq` only — the wire type is
// carried verbatim, never re-typed to widen its traits.
#[derive(Debug, Clone, PartialEq)]
pub struct ArmedEdit {
    /// The content path this edit writes.
    pub path: String,
    /// The wire shape, carried opaquely into `splice.plan_edits[]`.
    pub edit: PlanEdit,
    /// 1-based source line of the `put()` call that armed it.
    pub line: u32,
    /// Call-stack nesting depth at arm time; 0 at module top level.
    pub depth: u32,
}

/// Why the arm-time law refused. Consumer-plane typed — never a §8 wire code,
/// so the closed refusal taxonomy stays closed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ArmRefusal {
    /// A second CONTENT path was armed (v1 law: one content path per commit).
    MultiFileWriteSet {
        line: u32,
        first: String,
        second: String,
    },
    /// The armed-edit ceiling was reached — refused, never truncated.
    ArmedBudget { line: u32, limit: usize },
}

/// Split a `section=` address into §2.1 segments. Addresses are segments, never
/// a joined sanitized string (`docs/laws.md` Law 3).
pub(crate) fn section_segments(section: &str) -> Vec<HpathSeg> {
    section
        .split('/')
        .filter(|s| !s.is_empty())
        .map(|s| HpathSeg {
            h: s.to_owned(),
            n: None,
        })
        .collect()
}

/// Build the plan items one `put()` call arms, in the order they reach the wire:
/// properties first as one sorted group, then the body op — the same batch order
/// `wire-serve::plan::lower` and the MCP `put` face already build.
///
/// # Errors
/// The caller's own message when the kwargs do not address a wire shape: a
/// `props`-less, body-less call, or an `append` with no `section`. A bare
/// `append` has no wire target — `PlanEdit::Append` carries an hpath and an
/// empty one refuses `NotFound` in both dialects
/// (`decisions/2026-08-07-script-bare-append-target.md`).
pub(crate) fn plan_items(
    props: &[(String, String)],
    section: Option<&str>,
    append: Option<&str>,
) -> Result<Vec<PlanEdit>, String> {
    if props.is_empty() && append.is_none() {
        return Err("put() arms nothing: pass props={…} to set frontmatter, or \
             section=\"…\", append=\"…\" to append to a section"
            .to_owned());
    }
    let mut items: Vec<PlanEdit> = props
        .iter()
        .map(|(key, value)| PlanEdit::SetProperty {
            key: key.clone(),
            value: value.clone(),
            rev: None,
        })
        .collect();
    if let Some(body) = append {
        let Some(section) = section.filter(|s| !s.is_empty()) else {
            return Err(
                "put(append=…) addresses a section — pass section=\"<Section>\". An append \
                 targets the containing heading path; the wire carries no document-grain \
                 append"
                    .to_owned(),
            );
        };
        items.push(PlanEdit::Append {
            hpath: section_segments(section),
            body: body.to_owned(),
            rev: None,
        });
    } else if section.is_some() {
        return Err(
            "put(section=…) needs a body op — pass append=\"…\". A frontmatter write \
             (props={…}) is file-grain and takes no section"
                .to_owned(),
        );
    }
    Ok(items)
}
