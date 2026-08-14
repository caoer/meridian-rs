//! The script plane's armed-edit list: wire `splice.plan_edits[]` items armed by
//! `put()`, inert until the consumer plane commits them in ONE guarded splice.
//!
//! The shapes are [`wire::PlanEdit`] **verbatim** — the engine never re-types
//! what the wire owns, so a new plan verb costs this module nothing. That is the
//! whole content of ruling (B′)
//! (`decisions/2026-08-07-script-put-builtin-edit-grammar.md`): `put()` speaks
//! the wire's second edit dialect, so no third grammar is minted and the wire
//! schema is untouched.

use wire::{HpathSeg, PlanEdit, ReadSel};

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
    /// The armed-edit ceiling was reached — refused, never truncated.
    ArmedBudget { line: u32, limit: usize },
}

/// Parse a `section=` address into §2.1 segments, through the ONE
/// human-string→selector door ([`ReadSel::parse`]) — the same door
/// `read(path, section=…)` goes through on the read face.
///
/// Addresses are segments, never a joined sanitized string (`docs/laws.md`
/// Law 3), and one grammar means one parser: a raw `split('/')` here would
/// silently coerce `^r-000118` — an address the toc face itself PUBLISHES and
/// the read face accepts — into a heading literally named `^r-000118`, arming a
/// write at an address no document has.
///
/// # Errors
/// The two spellings that are real addresses but not heading paths: an `^id`
/// block address and a dewey ordinal. `PlanEdit::Append` carries an hpath, so
/// neither has a wire target, and each refuses in its own words rather than
/// being flattened into a heading.
pub(crate) fn section_segments(section: &str) -> Result<Vec<HpathSeg>, String> {
    match ReadSel::parse(section) {
        ReadSel::Hpath { hpath } => Ok(hpath.into_iter().filter(|s| !s.h.is_empty()).collect()),
        ReadSel::Anchor { anchor } => Err(format!(
            "put(section=\"^{anchor}\") addresses a BLOCK, and an append addresses a heading \
             path — the wire's append carries an hpath, so a block anchor has no target on it. \
             ^{anchor} is a real address on the read face: read(path, section=\"^{anchor}\") \
             works. To append, name the heading path the block sits under"
        )),
        ReadSel::Dewey { n } => Err(format!(
            "put(section=\"{n}\") addresses a row of a table you are holding, not a document — a \
             dewey ordinal is positional and does not survive an edit. Name the heading path"
        )),
    }
}

/// Build the plan items one `put()` call arms, in the order they reach the wire:
/// properties first as one sorted group, then the body op — the same batch order
/// `wire-serve::plan::lower` and the MCP `put` face already build.
///
/// # Errors
/// The caller's own message when the kwargs do not address a wire shape: a
/// `props`-less, body-less call, an `append` with no `section`, or a `section=`
/// whose address is a block anchor or a dewey ordinal ([`section_segments`]). A bare
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
            hpath: section_segments(section)?,
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

/// Does a RECORDED section spelling address the same node as an armed row's
/// `hpath`? The one matcher both threading lanes use (the CLI lane's
/// last-read threading and the § A.7 entry-rev threading), so a licensed row
/// cannot depend on which lane evaluated it.
///
/// **Both sides go through [`wire::ReadSel::parse`]** — the one
/// human-string→selector door. The non-heading spellings answer `false`: an
/// `^anchor` row and a dewey ordinal are real addresses that an `append`
/// (which carries an hpath) cannot be pointed at, so they match no armed row.
/// Empty segments are filtered on both sides — `ReadSel::parse("/A")` yields
/// `["", "A"]`, and one leading slash must not decide a CAS token.
#[must_use]
pub fn hpath_addresses(recorded: &str, hpath: &[HpathSeg]) -> bool {
    let ReadSel::Hpath { hpath: parsed } = ReadSel::parse(recorded) else {
        return false;
    };
    let mut recorded = parsed
        .iter()
        .map(|seg| seg.h.as_str())
        .filter(|h| !h.is_empty());
    let mut armed = hpath
        .iter()
        .map(|seg| seg.h.as_str())
        .filter(|h| !h.is_empty());
    loop {
        match (recorded.next(), armed.next()) {
            (None, None) => return true,
            (a, b) if a != b => return false,
            _ => {}
        }
    }
}
