//! `mrd retire` — the type-2 retirement DSL.
//!
//! ```text
//! mrd retire report [--id ID] [--json]
//! mrd retire mark   [--id ID] [--dry-run] [--expect-root ROOT] [--json]
//! ```
//!
//! # What this verb is
//! Type-2 retirement is grep-and-change with a marker, so the engine can later run
//! over the markers, gather information, and report the declared migration routes.
//! The marker is markdown `~~` plus a replacer, pointing at a section that holds
//! the replaced content — visible, non-destructive, one idempotent logic unit.
//!
//! # The one indirection
//! The link syntax is array hpath. The pointer to the holding section lives once,
//! in the `meridian-retire` block, as a [`wire::HpathSeg`] array; the call site
//! carries a key, not an address.
//!
//! # The declaration block is human-authored, so it stays visible
//! `meridian-retire` registers no [`lock::EngineEmitted`] writer, so the render
//! face shows it. Nothing is ever written back into the block.
//!
//! # Idempotence is structural, never remembered
//! The mark operation's image lies inside a marker span the recognizer excludes,
//! so `f(f(x)) = f(x)` holds by construction. Every count is re-derived from the
//! documents on every run; a second run makes zero splice calls and leaves the
//! workspace merkle root byte-identical.
//!
//! # What this verb refuses to do
//! It carries exactly one detector — the positive control — and refuses to close
//! a retirement without declared evidence for the rest. It never inspects a test.
//! Every number this verb prints is labelled `measured` or `declared`, never
//! mixed.
//!
//! Exits: **0** every declaration clean / **1** a finding (a refusal below, or a
//! retirement still `open`) / **2** bad invocation.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use serde_json::{Value, json};
use wire::{Edit, EditShape, HpathSeg, Path as WirePath, Root, SecRef};
use wire_serve::write::{SpliceArgs, splice};

use crate::{Fail, Format};

/// The declaration block's fence language. Reserved in the `meridian-*` namespace
/// ([`lock::is_meridian_lang`]) and deliberately not registered as engine-emitted.
pub(crate) const RETIRE_LANG: &str = "meridian-retire";

/// The no-partial clause every sweep refusal carries — the write-side twin of
/// `config::NO_PARTIAL_LOAD_CLAUSE` and `wire_serve::NO_PARTIAL_WRITE_CLAUSE`.
pub(crate) const NO_MARK_CLAUSE: &str = "No file was marked; the sweep is refused whole.";

/// The marker's machine half, opening literal. The id follows, then `)`.
const KEY_OPEN: &str = "(retired: ";

/// Why a retirement refused. The closed reason set — a reason word is a `&'static str` from
/// [`Reason::word`], never free text, so a refusal's spelling cannot drift.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// A `meridian-retire` block the parser cannot read.
    BlockMalformed,
    /// R1 — the declared positive control matched nothing, so the term's zero
    /// proves nothing.
    ControlSilent,
    /// R2 — the holding section's array hpath addresses no section.
    HoldingUnresolvable,
    /// R3 — one id, two declarations.
    IdAmbiguous,
    /// R4 — the term occurs inside bytes the ENGINE writes.
    WouldCorruptEngineBlock,
    /// R5 — a marker in the prose whose id no declaration carries.
    MarkerOrphaned,
    /// A `(retired: id)` token with no `~~…~~` before it on its line.
    MarkerMalformed,
    /// R6 — the term matched nothing and no marker carries its id. A wrong pattern, a
    /// term already removed by hand, and a retirement declared early are indistinguishable
    /// here, so the name states only what is detected.
    TermNeverMatched,
    /// R7 — a hash-domain member the corpus does not serve (not UTF-8,
    /// node-rev-merkle-spec §3 per-file degradation). The sweep certifies
    /// ABSENCE, and it cannot certify absence over a corpus it did not fully
    /// read: a term or control occurrence inside the unserved member is
    /// invisible to every scan loop, so a clean count would be a false clean
    /// sweep.
    MemberUnserved,
}

impl Reason {
    /// Every reason word this surface can emit, in declaration order — what the
    /// coverage census reads.
    pub const ALL: [Reason; 9] = [
        Reason::BlockMalformed,
        Reason::ControlSilent,
        Reason::HoldingUnresolvable,
        Reason::IdAmbiguous,
        Reason::WouldCorruptEngineBlock,
        Reason::MarkerOrphaned,
        Reason::MarkerMalformed,
        Reason::TermNeverMatched,
        Reason::MemberUnserved,
    ];

    /// The reason word, one spelling across the human line and `--json`.
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Reason::BlockMalformed => "retire-block-malformed",
            Reason::ControlSilent => "retire-control-silent",
            Reason::HoldingUnresolvable => "retire-holding-unresolvable",
            Reason::IdAmbiguous => "retire-id-ambiguous",
            Reason::WouldCorruptEngineBlock => "retire-would-corrupt-engine-block",
            Reason::MarkerOrphaned => "retire-marker-orphaned",
            Reason::MarkerMalformed => "retire-marker-malformed",
            Reason::TermNeverMatched => "retire-term-never-matched",
            Reason::MemberUnserved => "retire-member-unserved",
        }
    }
}

/// The tripwire that keeps [`Reason::ALL`] from falling behind the variants: a new
/// variant fails to compile here and in [`Reason::word`], and the accompanying test
/// checks the mapping is a bijection onto `ALL`'s indices.
#[cfg(test)]
const fn index_in_all(r: Reason) -> usize {
    match r {
        Reason::BlockMalformed => 0,
        Reason::ControlSilent => 1,
        Reason::HoldingUnresolvable => 2,
        Reason::IdAmbiguous => 3,
        Reason::WouldCorruptEngineBlock => 4,
        Reason::MarkerOrphaned => 5,
        Reason::MarkerMalformed => 6,
        Reason::TermNeverMatched => 7,
        Reason::MemberUnserved => 8,
    }
}

/// One refusal, assembled to the house four-property contract: subject · cause ·
/// partial state · a runnable fix.
#[derive(Debug, Clone)]
pub(crate) struct Refusal {
    pub(crate) reason: Reason,
    pub(crate) subject: String,
    pub(crate) cause: String,
    pub(crate) partial: String,
    pub(crate) fix: String,
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "refused: {} {} {} Fix: {}",
            self.subject, self.cause, self.partial, self.fix
        )
    }
}

// ---------------------------------------------------------------------------
// The declaration
// ---------------------------------------------------------------------------

/// One parsed `meridian-retire` block.
#[derive(Debug, Clone)]
pub(crate) struct Decl {
    pub(crate) id: String,
    pub(crate) term: String,
    pub(crate) replacer: String,
    pub(crate) control: String,
    pub(crate) holding: String,
    pub(crate) hpath: Vec<HpathSeg>,
    pub(crate) route: String,
    /// Declared, never verified: the mutation that must redden a named pin.
    pub(crate) proofs: Vec<String>,
    /// Where this block was found — its own page and 1-based fence line.
    pub(crate) page: String,
    pub(crate) line: usize,
}

/// The canonical field order, enforced so one fact has one spelling.
const FIELDS: [&str; 8] = [
    "version", "id", "term", "replacer", "control", "holding", "route", "proof",
];

fn malformed(path: &str, line: usize, cause: String, mut fix: String) -> Refusal {
    Refusal {
        reason: Reason::BlockMalformed,
        subject: format!("the {RETIRE_LANG} block at {path} line {line}"),
        cause,
        partial: format!("No declaration was loaded. {NO_MARK_CLAUSE}"),
        // Every parse refusal ends at a runnable command, appended here rather
        // than repeated at every call site.
        fix: {
            fix.push_str(" Then run `mrd retire report --json` to confirm the block parses.");
            fix
        },
    }
}

/// The raw field harvest of one block body — every stated key, in the order
/// stated, with the holding entry's two halves lifted out.
#[derive(Default)]
struct Harvest {
    seen: BTreeMap<&'static str, String>,
    holding_path: Option<String>,
    holding_hpath: Vec<HpathSeg>,
    proofs: Vec<String>,
}

/// Read one block body (line number, text) into a [`Harvest`].
///
/// Hand-rolled over an ordered field cursor: the block is a fixed-shape declaration,
/// not free YAML, and a general parser would accept spellings this grammar has no
/// meaning for. The line grammar lives here; the completeness law in [`parse_decl`].
#[allow(clippy::too_many_lines)]
fn harvest(path: &str, body: &[(usize, String)]) -> Result<Harvest, Refusal> {
    let mut seen: BTreeMap<&'static str, String> = BTreeMap::new();
    let mut order: Vec<&str> = Vec::new();
    let mut holding_path: Option<String> = None;
    let mut holding_hpath: Vec<HpathSeg> = Vec::new();
    let mut proofs: Vec<String> = Vec::new();
    let mut in_holding = false;
    let mut in_hpath = false;

    for (line, text) in body {
        if text.trim().is_empty() {
            continue;
        }
        let indented = text.starts_with(' ');
        if !indented {
            in_holding = false;
            in_hpath = false;
        }
        let trimmed = text.trim();

        // A nested `- h: …` row of the holding hpath array.
        if in_hpath && let Some(rest) = trimmed.strip_prefix("- ") {
            let (k, v) = split_kv(rest).ok_or_else(|| {
                malformed(
                    path,
                    *line,
                    format!("`{trimmed}` is not a `key: value` row of the holding hpath array."),
                    "write each segment as `- h: <raw heading text>`, optionally followed by an indented `n: <1-based occurrence>`.".to_owned(),
                )
            })?;
            if k != "h" {
                return Err(malformed(
                    path,
                    *line,
                    format!(
                        "an hpath segment opens with `{k}:` — a segment's first key is `h`, the RAW heading text."
                    ),
                    "write the segment as `- h: <raw heading text>`.".to_owned(),
                ));
            }
            holding_hpath.push(HpathSeg { h: v, n: None });
            continue;
        }
        // The optional occurrence index on the segment just pushed.
        if in_hpath && trimmed.starts_with("n:") {
            let raw = trimmed.trim_start_matches("n:").trim();
            let n: u32 = raw.parse().map_err(|_| {
                malformed(
                    path,
                    *line,
                    format!("an hpath segment's `n:` is `{raw}`, which is not a 1-based integer."),
                    "write `n:` as a positive whole number, or omit it to take the first match."
                        .to_owned(),
                )
            })?;
            match holding_hpath.last_mut() {
                Some(seg) => seg.n = Some(n),
                None => {
                    return Err(malformed(
                        path,
                        *line,
                        "an `n:` appears before any `- h:` segment, so it indexes nothing."
                            .to_owned(),
                        "put `n:` on the line after the `- h:` segment it indexes.".to_owned(),
                    ));
                }
            }
            continue;
        }

        let (key, value) = split_kv(trimmed).ok_or_else(|| {
            malformed(
                path,
                *line,
                format!("`{trimmed}` is not a `key: value` line."),
                format!(
                    "the block's fields are {} (in that order).",
                    FIELDS.join(", ")
                ),
            )
        })?;

        if in_holding && indented {
            match key.as_str() {
                "path" => holding_path = Some(value),
                "hpath" => in_hpath = true,
                other => {
                    return Err(malformed(
                        path,
                        *line,
                        format!(
                            "`holding` carries `{other}:` — a holding entry is `path` then `hpath`."
                        ),
                        "spell the two holding keys `path:` and `hpath:`.".to_owned(),
                    ));
                }
            }
            continue;
        }

        if key == "proof" {
            proofs.push(value);
            order.push("proof");
            continue;
        }

        let Some(slot) = FIELDS.iter().position(|f| *f == key) else {
            return Err(malformed(
                path,
                *line,
                format!(
                    "unknown field `{key}` in a {RETIRE_LANG} block — legal fields are {} (in that order).",
                    FIELDS.join(", ")
                ),
                "remove the line or spell the field you meant.".to_owned(),
            ));
        };
        if seen.contains_key(FIELDS[slot]) {
            return Err(malformed(
                path,
                *line,
                format!(
                    "`{key}` appears twice in this {RETIRE_LANG} block — last-wins and first-wins are both silent choices about which of two stated intents you meant."
                ),
                format!("delete one of the two `{key}:` lines."),
            ));
        }
        if let Some(last) = order.last()
            && let Some(prev) = FIELDS.iter().position(|f| f == last)
            && prev > slot
        {
            return Err(malformed(
                path,
                *line,
                format!(
                    "this {RETIRE_LANG} block's fields are out of canonical order — `{key}` follows `{last}`."
                ),
                format!("reorder the block's lines to {}.", FIELDS.join(", ")),
            ));
        }
        if key == "holding" {
            in_holding = true;
        }
        order.push(FIELDS[slot]);
        seen.insert(FIELDS[slot], value);
    }

    Ok(Harvest {
        seen,
        holding_path,
        holding_hpath,
        proofs,
    })
}

/// Turn a [`Harvest`] into a [`Decl`], enforcing the completeness law: every field a retirement
/// cannot act without is present, and the holding link is a non-empty array.
fn parse_decl(path: &str, fence_line: usize, body: &[(usize, String)]) -> Result<Decl, Refusal> {
    let Harvest {
        seen,
        holding_path,
        holding_hpath,
        proofs,
    } = harvest(path, body)?;
    let need = |k: &str| -> Result<String, Refusal> {
        seen.get(k).filter(|v| !v.is_empty()).cloned().ok_or_else(|| {
            malformed(
                path,
                fence_line,
                format!("this {RETIRE_LANG} block states no `{k}`, so the retirement it declares is incomplete."),
                format!("add a `{k}:` line; the block's fields are {} (in that order).", FIELDS.join(", ")),
            )
        })
    };
    let version = need("version")?;
    if version != "1" {
        return Err(malformed(
            path,
            fence_line,
            format!("this block declares `version: {version}`, which this engine does not speak."),
            "write `version: 1`, the only version this engine reads.".to_owned(),
        ));
    }
    let id = need("id")?;
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
        return Err(malformed(
            path,
            fence_line,
            format!(
                "the id `{id}` is outside the marker id charset `[A-Za-z0-9-]` — the same charset the `^anchor` plane uses, so a marker cannot carry it."
            ),
            "respell the id using ASCII letters, digits and hyphens only.".to_owned(),
        ));
    }
    let holding_path = holding_path.ok_or_else(|| {
        malformed(
            path,
            fence_line,
            format!("the `holding` entry of `{id}` states no `path`, so the replaced content has no page."),
            "add an indented `path: <workspace-relative page>` under `holding:`.".to_owned(),
        )
    })?;
    if holding_hpath.is_empty() {
        return Err(malformed(
            path,
            fence_line,
            format!(
                "the `holding` entry of `{id}` carries no hpath segments — the ruled link syntax is an array hpath, and an empty array addresses nothing."
            ),
            format!(
                "run `mrd read {holding_path} --json` and copy the holding section's `hpath` array under `holding: hpath:` as `- h: …` rows."
            ),
        ));
    }

    Ok(Decl {
        id,
        term: need("term")?,
        replacer: need("replacer")?,
        control: need("control")?,
        holding: holding_path,
        hpath: holding_hpath,
        route: need("route")?,
        proofs,
        page: path.to_owned(),
        line: fence_line,
    })
}

/// `key: value`, with the value taken verbatim after the first `: `.
fn split_kv(line: &str) -> Option<(String, String)> {
    let (k, v) = line.split_once(':')?;
    let k = k.trim();
    if k.is_empty() || k.contains(' ') {
        return None;
    }
    Some((k.to_owned(), v.trim().to_owned()))
}

// ---------------------------------------------------------------------------
// Locating blocks and spans in a document
// ---------------------------------------------------------------------------

/// One fenced block located through the model tree, never by scanning lines, so a
/// nested fence or an indented snippet needs no special case.
struct Fence {
    lang: String,
    span: model::ByteSpan,
    line: usize,
    body: Vec<(usize, String)>,
}

fn collect_fences(node: &model::Node, out: &mut Vec<(model::ByteSpan, String)>) {
    if let model::NodeKind::CodeBlock { lang, .. } = &node.kind {
        out.push((node.span.clone(), lang.clone()));
    }
    for child in &node.children {
        collect_fences(child, out);
    }
}

fn fences(raw: &str, root: &model::Node) -> Vec<Fence> {
    let mut spans: Vec<(model::ByteSpan, String)> = Vec::new();
    collect_fences(root, &mut spans);
    spans.sort_by_key(|(s, _)| s.start);
    spans
        .into_iter()
        .map(|(span, lang)| {
            let fence_line = line_at(raw, span.start);
            let slice = raw.get(span.clone()).unwrap_or_default();
            let mut lines: Vec<&str> = slice.lines().collect();
            if !lines.is_empty() {
                lines.remove(0);
            }
            if lines.last().is_some_and(|l| l.trim_end() == "```") {
                lines.pop();
            }
            Fence {
                // The first whitespace token decides the language, matching
                // every other reader in the repo.
                lang: lang.split_whitespace().next().unwrap_or("").to_owned(),
                span,
                line: fence_line,
                body: lines
                    .into_iter()
                    .enumerate()
                    .map(|(i, l)| (fence_line + 1 + i, l.to_owned()))
                    .collect(),
            }
        })
        .collect()
}

/// 1-based line number of a byte offset.
fn line_at(raw: &str, byte: usize) -> usize {
    let mut end = byte.min(raw.len());
    while end > 0 && !raw.is_char_boundary(end) {
        end -= 1;
    }
    raw[..end].split('\n').count()
}

/// One marker already in the prose: its full span and the id it carries.
#[derive(Debug, Clone)]
struct Marker {
    span: model::ByteSpan,
    id: String,
}

/// A `(retired: id)` token with no `~~…~~` before it on its line.
#[derive(Debug, Clone)]
struct BadMarker {
    line: usize,
    id: String,
}

/// Find every marker in `raw`. Hand-scanned rather than regex-matched: `mrd` carries no regex
/// dependency, and a hand scan lets the malformed case be reported rather than silently
/// unmatched.
fn markers(raw: &str) -> (Vec<Marker>, Vec<BadMarker>) {
    let bytes = raw.as_bytes();
    let mut found = Vec::new();
    let mut bad = Vec::new();
    let mut at = 0usize;
    while let Some(rel) = raw[at..].find(KEY_OPEN) {
        let open = at + rel;
        let id_start = open + KEY_OPEN.len();
        let Some(close_rel) = raw[id_start..].find(')') else {
            break;
        };
        let close = id_start + close_rel;
        let id = &raw[id_start..close];
        at = close + 1;
        if id.is_empty() || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
            continue;
        }
        // Walk back to the start of this line, then find the last `~~` pair
        // that closes before the key.
        let line_start = raw[..open].rfind('\n').map_or(0, |n| n + 1);
        let head = &raw[line_start..open];
        let Some(close_tilde) = head.rfind("~~") else {
            bad.push(BadMarker {
                line: line_at(raw, open),
                id: id.to_owned(),
            });
            continue;
        };
        let Some(open_tilde) = head[..close_tilde].rfind("~~") else {
            bad.push(BadMarker {
                line: line_at(raw, open),
                id: id.to_owned(),
            });
            continue;
        };
        debug_assert!(bytes.get(line_start + open_tilde).is_some());
        found.push(Marker {
            span: (line_start + open_tilde)..(close + 1),
            id: id.to_owned(),
        });
    }
    (found, bad)
}

/// The innermost `Section` node whose span contains `range`, and the raw heading
/// chain that addresses it.
fn innermost_section(node: &model::Node, range: &model::ByteSpan) -> Option<Vec<String>> {
    if node.span.start > range.start || node.span.end < range.end {
        return None;
    }
    for child in &node.children {
        if let Some(deeper) = innermost_section(child, range) {
            return Some(deeper);
        }
    }
    match (&node.kind, &node.hpath) {
        (model::NodeKind::Section { .. }, Some(hpath)) if !hpath.is_empty() => Some(hpath.clone()),
        _ => None,
    }
}

/// The span of the section addressed by `hpath`, if it exists — the R2 resolver.
fn walk_sections(node: &model::Node, want: &[&str], out: &mut Option<model::ByteSpan>) {
    if let (model::NodeKind::Section { .. }, Some(chain)) = (&node.kind, &node.hpath)
        && chain.len() == want.len()
        && chain.iter().zip(want).all(|(a, b)| a == b)
        && out.is_none()
    {
        *out = Some(node.span.clone());
    }
    for child in &node.children {
        walk_sections(child, want, out);
    }
}

fn section_span(node: &model::Node, hpath: &[HpathSeg]) -> Option<model::ByteSpan> {
    let want: Vec<&str> = hpath.iter().map(|s| s.h.as_str()).collect();
    let mut out = None;
    walk_sections(node, &want, &mut out);
    out
}

// ---------------------------------------------------------------------------
// The scan
// ---------------------------------------------------------------------------

/// The measured counts for one retirement — every one of them re-derived from
/// the documents on this run, none of them remembered.
#[derive(Debug, Default, Clone)]
pub(crate) struct Counts {
    pub(crate) marked: usize,
    pub(crate) already: usize,
    pub(crate) foreign: usize,
    pub(crate) remaining: usize,
    pub(crate) control: usize,
    pub(crate) in_code: usize,
    pub(crate) in_holding: usize,
    pub(crate) unaddressable: usize,
}

/// One file's planned rewrite: the contiguous region covering every markable
/// occurrence, its replacement, and the section that addresses it.
struct Plan {
    path: String,
    hpath: Vec<String>,
    old: String,
    new: String,
    hits: usize,
}

/// One byte range in a document that a marker will wrap.
struct Hit {
    span: model::ByteSpan,
    coded: bool,
}

/// Is `at` inside an inline code span on its own line? Returns the enclosing backtick-delimited
/// range when so. `~~` inside a code span is literal text: a marker written as `` `~~term~~` ``
/// renders the four tilde characters and strikes nothing.
fn code_span(raw: &str, at: &model::ByteSpan) -> Option<model::ByteSpan> {
    let line_start = raw[..at.start].rfind('\n').map_or(0, |n| n + 1);
    let line_end = raw[at.end..].find('\n').map_or(raw.len(), |n| at.end + n);
    let before = &raw[line_start..at.start];
    if before.matches('`').count().is_multiple_of(2) {
        return None;
    }
    let open = line_start + before.rfind('`')?;
    let close = at.end + raw[at.end..line_end].find('`')?;
    Some(open..(close + 1))
}

/// Scan one document for one declaration.
#[allow(clippy::too_many_lines)]
fn scan_doc(
    decl: &Decl,
    path: &str,
    doc: &model::Document,
    all_ids: &BTreeSet<String>,
) -> Result<(Counts, Option<Plan>, Vec<Refusal>), Refusal> {
    let raw = &doc.raw;
    let mut counts = Counts::default();
    let mut refusals = Vec::new();
    let (marks, bad) = markers(raw);
    let blocks = fences(raw, &doc.root);

    for b in &bad {
        refusals.push(Refusal {
            reason: Reason::MarkerMalformed,
            subject: format!("{path} line {} carries `(retired: {})`", b.line, b.id),
            cause: "with no `~~…~~` before it on that line — the machine half of a marker with no visible half, so the next run cannot tell the occurrence was already retired and would mark it a second time.".to_owned(),
            partial: NO_MARK_CLAUSE.to_owned(),
            fix: format!("strike the retired words on that line as `~~<term>~~ <replacer> (retired: {})`, or delete the token; then run `mrd retire report --json` to confirm the marker reads.", b.id),
        });
    }
    for m in &marks {
        if !all_ids.contains(&m.id) {
            refusals.push(Refusal {
                reason: Reason::MarkerOrphaned,
                subject: format!(
                    "{path} line {} carries marker `(retired: {})`",
                    line_at(raw, m.span.start),
                    m.id
                ),
                cause: "and no declaration has that id — a reader who follows the marker reaches nothing.".to_owned(),
                partial: "The report above is complete; nothing was written.".to_owned(),
                fix: format!(
                    "correct the id in the page, or add a {RETIRE_LANG} block declaring it — `mrd retire report --json` lists every orphan id with its file and line."
                ),
            });
        }
    }

    // Every `meridian-retire` block's own bytes are outside both scans: a block states
    // `control:` and `term:` as literal text, which a whole-file scan would match.
    let declared: Vec<model::ByteSpan> = blocks
        .iter()
        .filter(|f| f.lang == RETIRE_LANG)
        .map(|f| f.span.clone())
        .collect();
    let outside = |span: &model::ByteSpan| !declared.iter().any(|d| overlaps(d, span));

    // The positive control, measured in the same pass over the same file set as the
    // term — anywhere else the two numbers describe different worlds.
    let mut ctl = 0usize;
    let mut ctl_at = 0usize;
    while let Some(rel) = raw[ctl_at..].find(&decl.control) {
        let start = ctl_at + rel;
        let span = start..(start + decl.control.len());
        ctl_at = span.end;
        if outside(&span) {
            ctl += 1;
        }
    }
    counts.control += ctl;

    // The retirement's own holding section is excluded from its own sweep: it must name the
    // retired term to explain it, and marking it there would strike out the explanation.
    let holding_span = if path == decl.holding {
        section_span(&doc.root, &decl.hpath)
    } else {
        None
    };

    let mut hits: Vec<Hit> = Vec::new();
    let mut at = 0usize;
    while let Some(rel) = raw[at..].find(&decl.term) {
        let start = at + rel;
        let span = start..(start + decl.term.len());
        at = span.end;

        if !outside(&span) {
            // Inside a declaration block: the sweep never marks the surface that
            // declares it.
            continue;
        }
        if let Some(m) = marks.iter().find(|m| overlaps(&m.span, &span)) {
            if m.id == decl.id {
                counts.already += 1;
            } else {
                counts.foreign += 1;
            }
            continue;
        }
        if let Some(h) = &holding_span
            && overlaps(h, &span)
        {
            counts.in_holding += 1;
            continue;
        }
        if let Some(fence) = blocks.iter().find(|f| overlaps(&f.span, &span)) {
            if lock::is_engine_emitted(&fence.lang) {
                return Err(Refusal {
                    reason: Reason::WouldCorruptEngineBlock,
                    subject: format!(
                        "{path} line {} carries `{}` inside a `{}` block",
                        line_at(raw, span.start),
                        decl.term,
                        fence.lang
                    ),
                    cause: "the engine writes those bytes, and a marker in them would corrupt an engine artifact rather than annotate prose.".to_owned(),
                    partial: NO_MARK_CLAUSE.to_owned(),
                    fix: "narrow `term:` so it cannot match engine-written bytes, or migrate that block through its own writer first — `mrd walk --json` lists the pages carrying it.".to_owned(),
                });
            }
            counts.in_code += 1;
            continue;
        }
        let coded = code_span(raw, &span);
        hits.push(Hit {
            span: coded.clone().unwrap_or(span),
            coded: coded.is_some(),
        });
    }

    counts.remaining = hits.len();
    if hits.is_empty() {
        return Ok((counts, None, refusals));
    }

    // One edit per file, targeting the innermost section that contains every hit: one target
    // cannot overlap itself, so the disjointness law holds by construction.
    let region = hits[0].span.start..hits[hits.len() - 1].span.end;
    let Some(hpath) = innermost_section(&doc.root, &region) else {
        counts.unaddressable += hits.len();
        counts.remaining = 0;
        return Ok((counts, None, refusals));
    };

    let mut new = String::with_capacity(region.len() + hits.len() * 32);
    let mut cursor = region.start;
    for hit in &hits {
        new.push_str(&raw[cursor..hit.span.start]);
        let original = &raw[hit.span.clone()];
        // The `~~` wraps the term's full inline span, backticks included — see
        // `code_span`.
        if hit.coded {
            let _ = write!(
                new,
                "~~{original}~~ `{}` {KEY_OPEN}{})",
                decl.replacer, decl.id
            );
        } else {
            let _ = write!(
                new,
                "~~{original}~~ {} {KEY_OPEN}{})",
                decl.replacer, decl.id
            );
        }
        cursor = hit.span.end;
    }
    new.push_str(&raw[cursor..region.end]);

    Ok((
        counts,
        Some(Plan {
            path: path.to_owned(),
            hpath,
            old: raw[region].to_owned(),
            new,
            hits: hits.len(),
        }),
        refusals,
    ))
}

fn overlaps(a: &model::ByteSpan, b: &model::ByteSpan) -> bool {
    a.start < b.end && b.start < a.end
}

// ---------------------------------------------------------------------------
// The verb
// ---------------------------------------------------------------------------

/// Dispatch `mrd retire <report|mark> …`. Errors [`Fail`] exit 2 on a bad invocation; exit 1 on
/// any refusal above or on a retirement that is still `open`.
pub(crate) fn dispatch(args: &[String]) -> Result<(), Fail> {
    match args.first().map(String::as_str) {
        Some("report") => run(&args[1..], false),
        Some("mark") => run(&args[1..], true),
        Some(other) => Err(Fail::usage(format!("unknown retire subcommand: {other}"))),
        None => Err(Fail::usage("retire needs a subcommand".to_owned())),
    }
}

struct Parsed {
    id: Option<String>,
    dry_run: bool,
    expect_root: Option<String>,
    format: Format,
}

impl Parsed {
    fn parse(args: &[String], writing: bool) -> Result<Self, Fail> {
        let (mut id, mut dry_run, mut json, mut expect_root) = (None, false, false, None);
        let mut it = args.iter();
        while let Some(arg) = it.next() {
            match arg.as_str() {
                "--json" => json = true,
                "--dry-run" if writing => dry_run = true,
                "--id" => {
                    id = Some(
                        it.next()
                            .ok_or_else(|| Fail::tool("--id needs a value".to_owned()))?
                            .clone(),
                    );
                }
                "--expect-root" if writing => {
                    expect_root = Some(
                        it.next()
                            .ok_or_else(|| Fail::tool("--expect-root needs a value".to_owned()))?
                            .clone(),
                    );
                }
                flag if flag.starts_with('-') => {
                    return Err(Fail::tool(format!("unknown flag: {flag}")));
                }
                value => return Err(Fail::tool(format!("unexpected argument: {value}"))),
            }
        }
        Ok(Parsed {
            id,
            dry_run,
            expect_root,
            format: if json { Format::Json } else { Format::Human },
        })
    }
}

#[allow(clippy::too_many_lines)]
fn run(args: &[String], writing: bool) -> Result<(), Fail> {
    let parsed = Parsed::parse(args, writing)?;
    // The sweep carries a file-set-grain CAS — `splice`'s existing world guard;
    // the verb stays `Origin::InProcess`.
    if writing && !parsed.dry_run && parsed.expect_root.is_none() {
        return Err(Fail::tool(format!(
            "`mrd retire mark` writes without a world guard unless one is stated — a vault moves under a sweep whenever the fleet is not quiesced, and a half-swept vault is what the pre-sweep commit exists to undo. {NO_MARK_CLAUSE} Fix: run `mrd retire report --json` for the current `fingerprint`, quiesce the fleet, commit the vault, then re-run with `--expect-root <fingerprint>`; or use `--dry-run`."
        )));
    }

    let cwd = crate::current_dir()?;
    let resolved = crate::resolve::resolve_runtime(&cwd).map_err(|e| {
        Fail::tool(format!(
            "cannot resolve workspace for {}: {e}",
            cwd.display()
        ))
    })?;
    let canonical = workspace::canonicalize(&resolved.workspace).map_err(|e| {
        Fail::tool(format!(
            "cannot resolve workspace {} ({e})",
            resolved.workspace.display()
        ))
    })?;
    let root = fs::WorkspaceRoot(canonical);
    // `wire_serve::domain_snapshot`, not `fs::`: it folds the root into the wire's
    // `Root` — the same token `splice`'s world guard compares `--expect-root` against.
    let (files, fingerprint) = wire_serve::domain_snapshot(&root).map_err(|e| {
        Fail::tool(format!(
            "cannot read the corpus: {}",
            e.message.unwrap_or_default()
        ))
    })?;
    let (_index, docs, unserved) = fs::build_corpus(files);
    // The denominator counts files the scan READS. The raw domain count also
    // held members the corpus does not serve — folding those in is how
    // "matched 0 of N scanned files" overclaimed coverage by exactly the
    // files no loop below ever saw.
    let scanned = docs.len();

    // The OTHER population this sweep cannot see: markdown under the root that
    // the hash domain excludes (a dot-segment path, a `meridian/domain.md`
    // ignore rule). It is enumerated, never scanned — an out-of-domain page's
    // bytes cannot move the fingerprint this answer is stamped `as_of`, so a row
    // for it would sit under a stamp that does not cover it. Excluding it is
    // lawful; excluding it SILENTLY is not: this verb CERTIFIES ABSENCE, so a
    // reference living in such a page would be reported retired
    // (session decision 0017; `docs/wire-contract.md` §12.1, enumerator clause;
    // charter 03's absence split). Same discipline as `files_unserved` above —
    // published BESIDE the denominator, never folded into it.
    let excluded: Vec<String> = fs::walk(&root)
        .unwrap_or_default()
        .iter()
        .filter_map(|rel| rel.to_str().map(str::to_owned))
        .filter(|rel| !docs.contains_key(rel) && !unserved.contains_key(rel))
        .collect();

    // 1 · collect every declaration in the vault.
    let mut decls: Vec<Decl> = Vec::new();
    let mut refusals: Vec<Refusal> = Vec::new();

    // R7 — the corpus must serve the whole hash domain, or the sweep cannot
    // certify absence. Minted per member and ahead of every scan: the refusal
    // gates `mark` regardless of which id is selected.
    for (member, condition) in &unserved {
        refusals.push(Refusal {
            reason: Reason::MemberUnserved,
            subject: format!("hash-domain member `{member}` {condition}"),
            cause: "— the corpus does not serve it, so this sweep cannot see inside it; a clean count over a partial corpus certifies nothing, and a reference surviving in that file would be reported retired.".to_owned(),
            partial: NO_MARK_CLAUSE.to_owned(),
            fix: format!(
                "re-encode {member} as valid UTF-8, or move it out of the hash domain; then run `mrd retire report --json` and confirm `files_unserved` is 0."
            ),
        });
    }
    for (path, doc) in &docs {
        for fence in fences(&doc.raw, &doc.root) {
            if fence.lang != RETIRE_LANG {
                continue;
            }
            match parse_decl(path, fence.line, &fence.body) {
                Ok(decl) => decls.push(decl),
                Err(r) => refusals.push(r),
            }
        }
    }
    decls.sort_by(|a, b| a.id.cmp(&b.id));

    // R3 — one id, two declarations.
    for pair in decls.windows(2) {
        if pair[0].id == pair[1].id {
            refusals.push(Refusal {
                reason: Reason::IdAmbiguous,
                subject: format!("retirement id `{}` is declared twice", pair[0].id),
                cause: format!(
                    "— {} line {} and {} line {}; one id must name one holding section, or a marker in the prose points at two.",
                    pair[0].page, pair[0].line, pair[1].page, pair[1].line
                ),
                partial: NO_MARK_CLAUSE.to_owned(),
                fix: "rename one declaration's `id:`, then run `mrd retire report --json` to confirm one entry per id.".to_owned(),
            });
        }
    }

    let all_ids: BTreeSet<String> = decls.iter().map(|d| d.id.clone()).collect();
    let selected: Vec<&Decl> = decls
        .iter()
        .filter(|d| parsed.id.as_ref().is_none_or(|want| *want == d.id))
        .collect();

    // 2 · scan.
    let mut per_decl: Vec<(&Decl, Counts, Vec<Plan>)> = Vec::new();
    for decl in &selected {
        let mut counts = Counts::default();
        let mut plans = Vec::new();
        for (path, doc) in &docs {
            match scan_doc(decl, path, doc, &all_ids) {
                Ok((c, plan, mut rs)) => {
                    counts.already += c.already;
                    counts.foreign += c.foreign;
                    counts.remaining += c.remaining;
                    counts.control += c.control;
                    counts.in_code += c.in_code;
                    counts.in_holding += c.in_holding;
                    counts.unaddressable += c.unaddressable;
                    refusals.append(&mut rs);
                    if let Some(p) = plan {
                        plans.push(p);
                    }
                }
                Err(r) => refusals.push(r),
            }
        }

        // R2 — the holding section must resolve, or every marker this sweep
        // writes points at nothing.
        let holding_ok = docs
            .get(&decl.holding)
            .and_then(|d| section_span(&d.root, &decl.hpath))
            .is_some();
        if !holding_ok {
            refusals.push(Refusal {
                reason: Reason::HoldingUnresolvable,
                subject: format!("retirement `{}` holds its replaced content at {} § `{}`", decl.id, decl.holding, display_hpath(&decl.hpath)),
                cause: "and no section is addressed by that hpath — every marker this sweep writes would point at nothing.".to_owned(),
                partial: NO_MARK_CLAUSE.to_owned(),
                fix: format!("run `mrd read {} --json` and copy that section's `hpath` array into the block's `holding.hpath`.", decl.holding),
            });
        }

        // R1 — the positive control. Measured in the same pass as the term.
        if counts.control == 0 {
            refusals.push(Refusal {
                reason: Reason::ControlSilent,
                subject: format!("retirement `{}`'s positive control `{}` matched 0 of {scanned} scanned files", decl.id, decl.control),
                cause: "— a control that never matches cannot tell a clean vault from a broken pattern, so the term's zero count would prove nothing.".to_owned(),
                partial: NO_MARK_CLAUSE.to_owned(),
                fix: format!("run `mrd retire report --id {} --json` for the scanned file set, then declare a `control:` that is present in it today.", decl.id),
            });
        }

        // R6 — the arm the control cannot see: a wrong `term` and an absent one
        // produce a byte-identical row.
        let would_mark: usize = plans.iter().map(|p| p.hits).sum();
        if would_mark == 0 && counts.already == 0 {
            refusals.push(Refusal {
                reason: Reason::TermNeverMatched,
                subject: format!(
                    "retirement `{}`'s term `{}` matched nothing in {scanned} scanned files, and no marker in the vault carries its id",
                    decl.id, decl.term
                ),
                cause: "— so either the pattern is wrong, or the work is already done by other means; a retirement THIS tool completed would have left its markers behind and `already` would be above zero. A clean sweep and a pattern that never matched are the same row, and this tool will not report one as the other.".to_owned(),
                partial: NO_MARK_CLAUSE.to_owned(),
                fix: format!(
                    "run `mrd walk --json` and search for `{}` by hand: if it is present, `term:` is wrong — correct it; if it is genuinely absent, the retirement is already done and its block should be deleted or its `term:` corrected to what remains. Then re-run `mrd retire report --id {}`.",
                    decl.term, decl.id
                ),
            });
        }

        counts.marked = would_mark;
        per_decl.push((decl, counts, plans));
    }

    // 3 · write, or not.
    let mut wrote = 0usize;
    if writing && refusals.is_empty() {
        let mut expected = parsed.expect_root.clone().map(Root);
        for (_decl, _counts, plans) in &per_decl {
            for plan in plans {
                let edits = vec![Edit {
                    target: SecRef::Hpath {
                        hpath: plan
                            .hpath
                            .iter()
                            .map(|h| HpathSeg {
                                h: h.clone(),
                                n: None,
                            })
                            .collect(),
                    },
                    edit: EditShape::Match {
                        old: plan.old.clone(),
                        new: plan.new.clone(),
                    },
                    if_node_rev: None,
                }];
                let args = SpliceArgs {
                    id: None,
                    origin: wire_serve::guard::Origin::InProcess,
                    path: WirePath(plan.path.clone()),
                    actor: None,
                    now: None,
                    receipt: None,
                    // The guard chains across the sweep: each splice guards on the
                    // root the previous one produced, so anything else writing
                    // mid-sweep refuses `root_mismatch` rather than landing a
                    // half-swept vault.
                    if_root: expected.clone(),
                    dry: parsed.dry_run,
                    force: false,
                    edits,
                    plan_edits: Vec::new(),
                    pin: None,
                };
                // `mrd` is a CLI client and mints no notification sequence, so no
                // `SeqSink` is passed.
                let outcome = splice(&root, None, &args, &[], None).map_err(|e| {
                    crate::engine::json_refusal(parsed.format, &resolved.workspace, &e)
                })?;
                if let wire::ResponseBody::Splice { root_after, .. } = &outcome.body {
                    expected.clone_from(root_after);
                }
                wrote += 1;
            }
        }
    }

    // 4 · report.
    let open: Vec<&str> = per_decl
        .iter()
        .filter(|(d, _, _)| d.proofs.is_empty())
        .map(|(d, _, _)| d.id.as_str())
        .collect();

    match parsed.format {
        Format::Json => println!(
            "{}",
            serde_json::to_string_pretty(&to_json(
                scanned,
                unserved.len(),
                &excluded,
                &fingerprint,
                &per_decl,
                &refusals,
                &open
            ))
            .expect("json")
        ),
        Format::Human => print!(
            "{}",
            render_human(
                scanned,
                unserved.len(),
                &excluded,
                &fingerprint,
                &per_decl,
                &refusals,
                &open,
                writing,
                wrote
            )
        ),
    }

    if let Some(first) = refusals.first() {
        return Err(Fail::findings(first.to_string()));
    }
    if !open.is_empty() {
        return Err(Fail::findings(format!(
            "{} retirement(s) are open — no mutation-proof is declared for: {}. {} Fix: add a `proof:` line to each block naming the pin and the mutation that must redden it; this tool records it and never verifies it.",
            open.len(),
            open.join(", "),
            "Every measured count above is complete."
        )));
    }
    Ok(())
}

fn display_hpath(hpath: &[HpathSeg]) -> String {
    // Display only, for a refusal to name back — nothing parses this string.
    hpath
        .iter()
        .map(|s| match s.n {
            Some(n) => format!("{}#{n}", s.h),
            None => s.h.clone(),
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn to_json(
    scanned: usize,
    unserved: usize,
    excluded: &[String],
    fingerprint: &Root,
    per_decl: &[(&Decl, Counts, Vec<Plan>)],
    refusals: &[Refusal],
    open: &[&str],
) -> Value {
    json!({
        "files_scanned": scanned,
        // The population the scan did NOT read, published beside its
        // denominator — never folded into it.
        "files_unserved": unserved,
        // The domain-excluded population, named rather than implied: markdown
        // under the root this sweep did not read because the hash domain does
        // not carry it (decision 0017 — an enumerator may exclude, never
        // silently).
        "files_excluded": excluded,
        "fingerprint": fingerprint.0,
        "retirements": per_decl.iter().map(|(d, c, _)| json!({
            "id": d.id,
            "route": d.route,
            "holding": { "path": d.holding, "hpath": d.hpath },
            // The two evidence classes are separate keys, never one merged table.
            "measured": {
                "marked": c.marked, "already": c.already, "foreign": c.foreign,
                "remaining": c.remaining, "control": c.control,
                "skipped_code": c.in_code, "skipped_holding": c.in_holding,
                "skipped_unaddressable": c.unaddressable,
            },
            "declared": { "proofs": d.proofs, "verified_by_this_tool": false },
            "state": if d.proofs.is_empty() { "open" } else { "closed" },
        })).collect::<Vec<_>>(),
        // The four properties ride as their own keys, so a consumer never
        // substring-matches `message`.
        "refusals": refusals.iter().map(|r| json!({
            "reason": r.reason.word(),
            "subject": r.subject,
            "cause": r.cause,
            "partial": r.partial,
            "fix": r.fix,
            "message": r.to_string(),
        })).collect::<Vec<_>>(),
        "open": open,
    })
}

#[allow(clippy::too_many_arguments)]
fn render_human(
    scanned: usize,
    unserved: usize,
    excluded: &[String],
    fingerprint: &Root,
    per_decl: &[(&Decl, Counts, Vec<Plan>)],
    refusals: &[Refusal],
    open: &[&str],
    writing: bool,
    wrote: usize,
) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "files scanned: {scanned}   unserved: {unserved}   declarations: {}   fingerprint: {}",
        per_decl.len(),
        fingerprint.0
    );
    if !excluded.is_empty() {
        let _ = writeln!(
            out,
            "outside the hash domain: {} markdown file(s) under this root were NOT scanned — {}. This sweep certifies absence over the domain only: a reference living in one of these files is NOT retired and is NOT reported here (decision 0017).",
            excluded.len(),
            excluded.join(", ")
        );
    }
    for (decl, c, _) in per_decl {
        let _ = writeln!(
            out,
            "{}\n  measured   marked {}   already {}   foreign {}   remaining {}   control {}",
            decl.id, c.marked, c.already, c.foreign, c.remaining, c.control
        );
        if c.in_code + c.in_holding + c.unaddressable > 0 {
            let _ = writeln!(
                out,
                "             skipped-code {}   skipped-holding {}   skipped-unaddressable {}",
                c.in_code, c.in_holding, c.unaddressable
            );
        }
        let _ = writeln!(out, "  route      {} (declared)", decl.route);
        if decl.proofs.is_empty() {
            let _ = writeln!(out, "  declared   (none) — this retirement is open");
        } else {
            for p in &decl.proofs {
                let _ = writeln!(out, "  declared   {p}   (not verified by this tool)");
            }
        }
    }
    for r in refusals {
        let _ = writeln!(out, "{}  {r}", r.reason.word());
    }
    if writing {
        if wrote == 0 {
            out.push_str("no file was written\n");
        } else {
            let _ = writeln!(out, "{wrote} file(s) written");
        }
    }
    if !open.is_empty() {
        let _ = writeln!(out, "open: {}", open.join(", "));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(raw: &str) -> model::Document {
        model::build(raw.to_owned(), syntax::parse(raw))
    }

    #[test]
    fn a_marker_span_excludes_its_own_term_so_the_operation_is_idempotent() {
        let raw = "# H\n\nthe ~~old~~ new (retired: x) field\n";
        let (marks, bad) = markers(raw);
        assert_eq!(marks.len(), 1, "one marker: {marks:?}");
        assert!(bad.is_empty(), "no malformed markers: {bad:?}");
        let term = raw.find("old").expect("term present");
        assert!(
            overlaps(&marks[0].span, &(term..term + 3)),
            "the marker span covers the term it wrapped, which is what makes f(f(x))=f(x)"
        );
    }

    #[test]
    fn a_key_with_no_visible_half_is_malformed_not_a_marker() {
        // The span must not be treated as a marker: excluding a span that does
        // not exist is how an occurrence gets marked twice.
        let raw = "# H\n\nthe old (retired: x) field\n";
        let (marks, bad) = markers(raw);
        assert!(marks.is_empty(), "not a marker: {marks:?}");
        assert_eq!(bad.len(), 1, "reported as malformed: {bad:?}");
    }

    #[test]
    fn a_term_in_an_inline_code_span_expands_to_the_backticks() {
        let raw = "# H\n\nthe `old_name` field\n";
        let at = raw.find("old_name").expect("term");
        let span = code_span(raw, &(at..at + 8)).expect("inside a code span");
        assert_eq!(&raw[span], "`old_name`", "the wrap covers the backticks");
    }

    #[test]
    fn a_term_outside_backticks_is_not_expanded() {
        let raw = "# H\n\nthe old_name field\n";
        let at = raw.find("old_name").expect("term");
        assert!(code_span(raw, &(at..at + 8)).is_none());
    }

    #[test]
    fn the_holding_hpath_resolves_through_the_array_never_a_joined_string() {
        let d = doc("# Guide\n\n## A/B\n\nbody\n");
        let hpath = vec![
            HpathSeg {
                h: "Guide".to_owned(),
                n: None,
            },
            HpathSeg {
                h: "A/B".to_owned(),
                n: None,
            },
        ];
        assert!(
            section_span(&d.root, &hpath).is_some(),
            "a `/`-bearing heading is addressable through the array — the whole point of R1.6"
        );
    }

    /// `Reason::ALL` is a bijection onto the variants: every entry sits at the index
    /// [`index_in_all`] assigns it, and there are exactly as many entries as indices.
    #[test]
    fn all_is_a_bijection_onto_the_reason_variants() {
        for (i, reason) in Reason::ALL.iter().enumerate() {
            assert_eq!(
                index_in_all(*reason),
                i,
                "{} sits at ALL[{i}] but index_in_all says {}",
                reason.word(),
                index_in_all(*reason)
            );
        }
        let mut words: Vec<&str> = Reason::ALL.iter().map(|r| r.word()).collect();
        words.sort_unstable();
        words.dedup();
        assert_eq!(
            words.len(),
            Reason::ALL.len(),
            "no reason word appears twice in ALL"
        );
    }

    #[test]
    fn a_block_out_of_canonical_order_refuses_with_a_teaching_message() {
        let body: Vec<(usize, String)> = vec![
            (2, "version: 1".to_owned()),
            (3, "term: t".to_owned()),
            (4, "id: x".to_owned()),
        ];
        let err = parse_decl("p.md", 1, &body).expect_err("out of order");
        assert_eq!(err.reason, Reason::BlockMalformed);
        assert!(err.fix.contains("reorder"), "teaches the fix: {err}");
    }
}
